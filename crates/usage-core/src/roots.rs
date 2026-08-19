//! OS별 데이터 루트 탐색.
//!
//! - Linux/macOS: `~/.claude`, `~/.codex`, `~/.gemini/antigravity-cli`
//! - Windows: `%USERPROFILE%\.claude` 등 (dirs::home_dir 가 처리)
//!   + 실행 중인 WSL 배포판 안의 리눅스 홈도 병합 ([`wsl_guest_homes`])
//!
//! 반대 방향(WSL **안에서** 앱을 띄우고 `/mnt/c/Users/*` 의 Windows 홈을 병합)은 지원하지
//! 않는다. 배포본이 msi/dmg 뿐이라 최종 사용자가 그렇게 쓸 일이 없고, 개발용으로만 남아
//! 있던 코드다. WSL 안에서 띄워도 그 배포판의 `~/.claude` 등은 그대로 잡힌다.

use std::path::{Path, PathBuf};

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// 존재하는 경로만 남기고 중복 제거.
/// 같은 디렉토리를 다르게 적은 경로(대소문자·`..`·심볼릭 링크)까지 걸러야 같은 파일을
/// 두 번 스캔하지 않으므로, 비교는 canonical 경로로 한다.
fn existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for p in paths {
        if !p.is_dir() {
            continue;
        }
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            out.push(p);
        }
    }
    out
}

/// 자동 탐지 결과에 사용자가 설정에 직접 적은 홈들을 합친다.
///
/// 자동 탐지는 표준 위치와 마커 스캔이 닿는 곳까지만 본다. 홈을 다른 드라이브처럼 스캔
/// 범위 밖으로 옮겨 두면 그 경로가 있다는 사실조차 알 수 없다 — **세 소스 모두 그렇다.**
/// (환경변수로 그걸 메우면 실행 방식에 따라 범위가 달라져서 안 쓴다. `codex_homes` 참고)
/// 설정에 적어 두면 실행 방식과 무관하게 항상 같은 범위를 본다.
/// 합쳐서 생기는 중복은 어댑터의 이벤트 dedup 이 막는다.
fn with_extra(auto: Vec<PathBuf>, extra: &[PathBuf], suffix: &[&str]) -> Vec<PathBuf> {
    let mut all = auto;
    for e in extra {
        let mut p = e.clone();
        for part in suffix {
            p = p.join(part);
        }
        all.push(p);
    }
    existing(all)
}

/// Windows 빌드에서 WSL 배포판 내부의 리눅스 홈 디렉토리 목록
/// (`\\wsl.localhost\<distro>\home\<user>`, `\root`).
/// 같은 머신에서 WSL로 쓴 CLI 사용량을 Windows 앱이 병합하기 위함.
///
/// WSL 조회는 **꺼져 있을 때 지독하게 느리다.** 실측(배포판 2개 Stopped):
///
/// | | 걸린 시간 |
/// |---|---|
/// | `\\wsl.localhost\<distro>\...` 경로 확인 1회 | **110초** |
/// | `wsl.exe -l -q` | 30초 |
/// | `wsl.exe -l -q --running` | 14초 |
///
/// Windows 가 배포판을 깨우려 들기 때문이다. 그래서 세 겹으로 막는다:
///
/// 1. **VM 이 이미 떠 있을 때만** 조회한다 ([`wsl_vm_running`]) — `wsl.exe` 는 인자가
///    무엇이든 실행하는 순간 WSL 서비스를 깨우고, 환경에 따라 배포판까지 딸려 온다.
///    사용량을 보자고 앱을 켰더니 WSL 이 켜지는 건 사용자가 시키지 않은 일이다.
///    VM 이 꺼져 있으면 `--running` 의 답도 어차피 "없음" 이라 잃는 것도 없다.
/// 2. **실행 중인 배포판만** 본다 (`--running` — 사용량 보자고 남의 배포판을 깨울 이유가 없다).
/// 3. 그마저도 [`WSL_PROBE_TIMEOUT`] 안에 안 끝나면 포기한다.
///
/// 포기한 결과는 캐시하지 **않는다** — 나중에 WSL 을 켜고 "다시 검색" 하면 잡히도록.
/// 성공한 결과는 캐시하되 [`forget_wsl_guest_homes`] 로 버릴 수 있다.
#[cfg(windows)]
fn wsl_guest_homes() -> Vec<PathBuf> {
    let cell = wsl_cache();
    if let Some(cached) = cell.lock().unwrap().as_ref() {
        return cached.clone();
    }

    // 꺼져 있으면 조회 자체를 하지 않는다. **이 결과는 캐시하지 않는다** — 앱을 켠 뒤에
    // WSL 을 시작하는 일이 흔하고, 그때 다음 재발견에서 자연히 잡혀야 한다. 검사 비용은
    // 프로세스 목록 조회 한 번이라 매번 해도 싸다.
    if !wsl_vm_running() {
        return vec![];
    }

    // 조회가 매달릴 수 있으므로 별도 스레드에 맡기고 기다리는 쪽에 시간 제한을 건다.
    // 시간이 지나면 그 스레드는 버려둔다 — wsl.exe 가 끝나면 알아서 정리된다.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(collect_wsl_guest_homes());
    });
    let Ok(homes) = rx.recv_timeout(WSL_PROBE_TIMEOUT) else {
        return vec![];
    };
    *cell.lock().unwrap() = Some(homes.clone());
    homes
}

/// WSL 조회를 포기하는 시간. 켜져 있으면 수십 ms 면 끝나므로 넉넉한 값이다.
#[cfg(windows)]
const WSL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(windows)]
fn wsl_cache() -> &'static std::sync::Mutex<Option<Vec<PathBuf>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<Vec<PathBuf>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 캐시된 WSL 조회 결과를 버린다 — **"다시 검색"이 실제로 다시 검색하게** 하는 데 필요하다.
///
/// 조회가 비싸서(꺼진 배포판이 있으면 최대 [`WSL_PROBE_TIMEOUT`]) 성공한 결과는 프로세스
/// 내내 캐시한다. 그런데 그러면 앱을 켠 **뒤에** 시작한 배포판이 영영 안 잡힌다 — 재시작
/// 말고는 방법이 없었다. 사용자가 상황을 바꾸고 다시 누르는 경우가 정확히 이 시나리오다.
///
/// 자동 재발견(홈 추가·제거 등)에서는 부르지 않는다. 매번 최대 3초를 더 쓸 이유가 없다.
pub fn forget_wsl_guest_homes() {
    #[cfg(windows)]
    {
        *wsl_cache().lock().unwrap() = None;
    }
}

/// WSL 게스트 경로(`\\wsl.localhost\...`)를 **지금 읽어도 되는지**.
///
/// 스캔은 발견 때 정한 루트를 10초마다(라이브는 2초) 다시 읽는데, 그 사이 사용자가
/// WSL 을 종료했다면 그 파일 접근 하나하나가 배포판을 깨우라는 요청이 된다 — 모듈
/// 주석의 실측표대로 경로 확인 1회가 `wsl.exe` 호출보다도 비싸다(110초 vs 30초).
/// 그래서 읽기 전에 이걸 묻고, 꺼져 있으면 그 루트는 이번 회차에서 뺀다.
///
/// **캐시는 "꺼짐" 쪽만 한다.** 처음엔 결과를 5초 재사용했는데, 그게 정확히 이 기능을
/// 무력화했다 (실측):
///
/// ```text
/// 12:10:53  wsl --shutdown        vmmemWSL 사라짐
///    +2초   라이브 스레드가 캐시된 "켜짐" 을 믿고 WSL 경로를 읽음 → 배포판 부팅
/// 12:11:03  WSL 부활, 이후 계속 켜진 채    ← 한 번 깨우면 되돌아오지 못한다
/// ```
/// (같은 순서를 앱 없이 하면 90초간 꺼진 채였다 — 앱이 깨운 것이 맞았다.)
///
/// 두 방향의 낡음은 값이 다르다. **"켜짐"이 낡으면 WSL 을 깨운다**(되돌릴 수 없다).
/// **"꺼짐"이 낡으면 한 회차 스캔을 거를 뿐이다**(다음 회차에 따라잡는다). 그래서
/// 켜짐은 캐시하지 않고 매번 확인하며, 꺼짐만 [`WSL_OFF_TTL`] 동안 재사용한다 —
/// WSL 을 꺼 두고 쓰는 사용자에게 검사가 계속 도는 걸 막는다.
pub fn wsl_reachable() -> bool {
    #[cfg(windows)]
    {
        static OFF_UNTIL: std::sync::OnceLock<
            std::sync::Mutex<Option<std::time::Instant>>,
        > = std::sync::OnceLock::new();
        let cell = OFF_UNTIL.get_or_init(|| std::sync::Mutex::new(None));
        let mut until = cell.lock().unwrap();
        if let Some(t) = *until {
            if std::time::Instant::now() < t {
                return false;
            }
        }
        let ok = wsl_vm_running();
        *until = (!ok).then(|| std::time::Instant::now() + WSL_OFF_TTL);
        ok
    }
    // 윈도우가 아니면 WSL 경로 자체가 없다 — 판단할 것이 없다
    #[cfg(not(windows))]
    true
}

/// 이 경로가 WSL 게스트 안인지. 옛 표기(`\\wsl$\`)도 같이 본다 — 사용자가 설정에
/// 직접 넣은 추가 홈은 그 형태일 수 있다.
pub fn is_wsl_path(p: &std::path::Path) -> bool {
    wsl_distro_of(p).is_some()
}

/// 이 경로가 어느 WSL 배포판 안인지 (`\\wsl.localhost\Ubuntu-24.04\home\me` → `Ubuntu-24.04`).
/// WSL 경로가 아니면 `None` — 그래서 [`is_wsl_path`] 가 이걸 그대로 쓴다.
///
/// 이름을 뽑는 이유는 화면 때문이다. 계정 목록에서 "WSL: Ubuntu-24.04" 처럼 **어디에
/// 사는 계정인지** 밝혀야, 그 계정을 켜는 것이 무슨 뜻인지 사용자가 알 수 있다.
pub fn wsl_distro_of(p: &std::path::Path) -> Option<String> {
    let s = p.to_string_lossy().replace('/', "\\");
    let lower = s.to_ascii_lowercase();
    let rest = [r"\\wsl.localhost\", r"\\wsl$\"]
        .into_iter()
        .find_map(|pre| lower.starts_with(pre).then(|| &s[pre.len()..]))?;
    let name = rest.split('\\').next().unwrap_or_default();
    (!name.is_empty()).then(|| name.to_string())
}

/// "WSL 꺼짐" 판정을 재사용하는 시간. 이 방향의 낡음은 스캔 한 회차를 거르게 할 뿐이라
/// 안전하다 — 반대 방향("켜짐")은 캐시하지 않는다 ([`wsl_reachable`] 주석 참고).
/// 검사 비용은 `tasklist` 한 번(실측 ~45ms)이고, WSL 루트가 있는 사용자에게만 든다.
#[cfg(windows)]
const WSL_OFF_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// WSL **가상 머신이 지금 떠 있는지**. 프로세스 목록 조회는 WSL 을 깨우지 않는다 —
/// 이 검사가 `wsl.exe` 대신 존재하는 이유가 그것이다.
///
/// **서비스 상태를 보면 안 된다.** 처음엔 `sc query WSLService` 를 썼는데 그 서비스는
/// 시작 유형이 `AUTO_START` 라 부팅과 함께 뜨고 정지 트리거도 없다 — 실측: 부팅 14초
/// 뒤에 떠서 20시간째 `RUNNING`. `wsl --shutdown` 은 VM 만 끄고 서비스는 건드리지
/// 않으므로, 그 값은 "WSL 이 켜져 있다"가 아니라 **사실상 상수**다. 그걸 신호로 쓰면
/// 게이트가 늘 열려 있다.
///
/// 그래서 **`vmmemWSL` 하나만** 본다 — WSL VM 의 메모리 프로세스라, 이게 있으면 VM 이
/// 떠 있고 없으면 꺼져 있다. 확장자 없는 이름이다.
///
/// **맨 `vmmem` 은 일부러 안 본다.** 그 이름은 Hyper-V 기반 VM 이 다 같이 쓴다 —
/// Docker Desktop(Hyper-V 백엔드)·Windows Sandbox·안드로이드 에뮬레이터가 떠 있으면
/// WSL 이 꺼져 있어도 참이 되고, 그러면 게이트가 열려 WSL 경로를 읽어 깨우게 된다.
/// WSL2 가 아직 `vmmem` 이던 옛 빌드에서는 이 판정이 늘 거짓이라 WSL 안의 사용량이
/// 안 잡힌다 — 화면에 "데이터 없음" 으로 보이므로 조용히 틀리지는 않는다.
#[cfg(windows)]
fn wsl_vm_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("tasklist.exe")
        .args(["/FI", "IMAGENAME eq vmmemWSL", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        // 없으면 "INFO: No tasks are running..." 이 온다 — 이미지 이름 자체를 찾는다
        .is_ok_and(|out| {
            String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains("vmmemwsl")
        })
}

#[cfg(windows)]
fn collect_wsl_guest_homes() -> Vec<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(out) = std::process::Command::new("wsl.exe")
        .args(["-l", "-q", "--running"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return vec![];
    };
    // wsl.exe 출력은 UTF-16LE
    let utf16: Vec<u16> =
        out.stdout.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&utf16);

    let mut homes = vec![];
    for distro in text.lines().map(|l| l.trim().trim_matches('\0')).filter(|l| !l.is_empty()) {
        let base = PathBuf::from(format!(r"\\wsl.localhost\{distro}\home"));
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.filter_map(|e| e.ok()) {
                if e.path().is_dir() {
                    homes.push(e.path());
                }
            }
        }
        let root = PathBuf::from(format!(r"\\wsl.localhost\{distro}\root"));
        if root.is_dir() {
            homes.push(root);
        }
    }
    homes
}

#[cfg(not(windows))]
fn wsl_guest_homes() -> Vec<PathBuf> {
    vec![]
}

/// 소스별 배치 — 홈 아래 어디를 보는가. 대안이 여럿이면 **홈마다 전부** 시도한다.
/// 소스 간 차이를 **코드가 아니라 데이터**로 두면 특정 홈에서만 빠지는 누락이 안 생긴다.
///
/// 지금은 소스마다 하나씩이다. 예전에는 Claude 에 XDG 대안(`~/.config/claude/projects`)이
/// 있었는데 **뺐다** — 그 배치에서는 설치본이 온전히 복원되지 않았다. 발견한 루트에서
/// 홈을 거꾸로 유도하는데(`accounts::standard_installs`), XDG 배치면 홈이 `~/.config` 로
/// 잡혀 트랜스크립트·세션·계정 파일 경로가 전부 어긋나고, 계정 파일을 못 읽으니 경로를
/// 키로 한 **유령 계정**이 목록에 뜬다. 게다가 그 배치를 실제로 관측한 적이 없어서,
/// 계정 파일이 어디 놓이는지도 추측일 수밖에 없다. 실측되면 그때 제대로 넣는다.
const CLAUDE_PROJECT_LAYOUTS: &[&[&str]] = &[&[".claude", "projects"]];
const CLAUDE_SESSION_LAYOUTS: &[&[&str]] = &[&[".claude", "sessions"]];
const CODEX_HOME_LAYOUTS: &[&[&str]] = &[&[".codex"]];
const AGY_HOME_LAYOUTS: &[&[&str]] = &[&[".gemini", "antigravity-cli"]];

/// 훑을 후보 홈 — 로컬 홈 + 실행 중인 WSL 배포판의 리눅스 홈.
/// **세 소스가 이 목록을 공유한다.** 셋 다 WSL 에 따로 설치될 수 있고 세션도 따로 쌓인다.
fn candidate_homes() -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = vec![];
    if let Some(h) = home_dir() {
        homes.push(h);
    }
    homes.extend(wsl_guest_homes());
    homes
}

/// 후보 홈 × 배치의 모든 조합 중 존재하는 것만.
fn candidate_dirs(layouts: &[&[&str]]) -> Vec<PathBuf> {
    dirs_under(candidate_homes(), layouts)
}

/// 홈 목록을 받는 쪽을 갈라 둬서 테스트에서 주입할 수 있다
/// (`home_dir()` 은 실행 환경에 고정돼 있어 그대로면 조합 규칙을 검증할 수 없다).
fn dirs_under(homes: Vec<PathBuf>, layouts: &[&[&str]]) -> Vec<PathBuf> {
    let mut out = vec![];
    for home in homes {
        for layout in layouts {
            let mut p = home.clone();
            for part in *layout {
                p = p.join(part);
            }
            out.push(p);
        }
    }
    existing(out)
}

/// Claude Code 트랜스크립트 루트 (`.claude/projects`)
pub fn claude_project_roots() -> Vec<PathBuf> {
    claude_project_roots_with(&[])
}

/// `extra` 는 추가로 볼 `.claude` 홈 디렉토리 목록 (그 아래 `projects` 를 본다)
pub fn claude_project_roots_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(CLAUDE_PROJECT_LAYOUTS), extra, &["projects"])
}

/// Claude Code 라이브 세션 레지스트리 (`.claude/sessions`)
pub fn claude_session_dirs() -> Vec<PathBuf> {
    claude_session_dirs_with(&[])
}

/// `extra` 는 추가로 볼 `.claude` 홈 디렉토리 목록 (그 아래 `sessions` 를 본다)
pub fn claude_session_dirs_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(CLAUDE_SESSION_LAYOUTS), extra, &["sessions"])
}

/// Codex CLI 홈 (`~/.codex`). 홈 자체가 루트이고 그 아래 `sessions`/`archived_sessions` 가 있다.
///
/// **`CODEX_HOME` 은 보지 않는다.** 공식 문서상 기본값을 옮기는 환경변수가 맞지만, 읽는 순간
/// 앱이 보는 범위가 **실행 방식에 좌우된다** — 터미널에서 띄우면 잡히고 트레이·자동시작으로
/// 띄우면 안 잡혀서, 같은 머신인데 집계가 달라진다. 세 소스 중 Codex 만 그런 예외를 두는
/// 것도 일관성을 해친다 (Claude·agy 는 표준 위치만 본다).
///
/// 재배치된 홈은 다른 두 소스와 **같은 방식**으로 찾는다:
/// - `%APPDATA%`·`%LOCALAPPDATA%`·XDG 아래면 마커 스캔이 찾아낸다 (실측: orca 런타임 홈)
/// - 그 밖(다른 드라이브 등)이면 "홈 추가…"로 직접 등록한다 → `extraCodexHomes`
///
/// 재배치 홈과 `~/.codex` 가 같은 rollout 을 갖고 있어도 중복 집계되지 않는다 — 어댑터가
/// 파일이 아니라 **이벤트 단위**로 dedup 한다 (`codex::tests::same_rollout_in_two_homes_counted_once`).
pub fn codex_homes() -> Vec<PathBuf> {
    codex_homes_with(&[])
}

/// `extra` 는 추가로 볼 Codex 홈 디렉토리 목록 (그 아래 `sessions`/`archived_sessions`).
/// WSL 경계를 넘는 `.codex` 들은 서로 다른 설치본이라 그대로 병합한다.
pub fn codex_homes_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(CODEX_HOME_LAYOUTS), extra, &[])
}

/// Antigravity CLI(`agy`) 홈 (`~/.gemini/antigravity-cli`).
///
/// Gemini CLI 를 대체한 도구지만 `~/.gemini` 아래에 자기 폴더를 따로 쓴다.
/// 홈 자체가 루트이고 그 아래 `conversations/<uuid>.db` 가 대화들이다.
/// 홈을 옮기는 환경변수는 관측되지 않았다 (Codex 의 `CODEX_HOME` 에 해당하는 것이 없다).
/// 어차피 환경변수는 어느 소스에서도 읽지 않는다 — `codex_homes` 참고.
pub fn antigravity_homes() -> Vec<PathBuf> {
    antigravity_homes_with(&[])
}

/// `extra` 는 추가로 볼 `antigravity-cli` 홈 디렉토리 목록.
/// 합쳐서 같은 대화 DB 가 두 번 잡혀도 어댑터의 요청 id dedup 이 중복 집계를 막는다.
pub fn antigravity_homes_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(AGY_HOME_LAYOUTS), extra, &[])
}

/// OS 경계를 넘는 경로 여부 — pid 생존 확인이 불가능한 세션 디렉토리 판단용.
/// Windows 에서 본 WSL 공유(`\\wsl...`)가 주 대상이고, `/mnt/...` 는 자동 탐지로는 더 이상
/// 들어오지 않지만 설정에 직접 적을 수 있어 남겨 둔다 (판정이 경로 모양만 보므로 공짜다).
pub fn is_windows_mount(p: &Path) -> bool {
    if p.starts_with("/mnt/") {
        return true;
    }
    p.to_string_lossy().starts_with(r"\\wsl")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 배포판 이름은 화면에 그대로 나가고(계정 배지), 기본 켜짐/꺼짐 판정도 이걸 쓴다.
    #[test]
    fn wsl_distro_name_is_extracted() {
        let cases = [
            (r"\\wsl.localhost\Ubuntu-24.04\home\me", Some("Ubuntu-24.04")),
            (r"\\wsl$\Debian\root", Some("Debian")),
            (r"\\WSL.LOCALHOST\Ubuntu\home\me\.claude", Some("Ubuntu")),
            (r"C:\Users\me\.claude", None),
            // 배포판 이름이 없는 껍데기 경로 — 여기서 빈 이름을 내보내면 배지가 "WSL: " 이 된다
            (r"\\wsl.localhost\", None),
        ];
        for (p, want) in cases {
            assert_eq!(
                wsl_distro_of(std::path::Path::new(p)).as_deref(),
                want,
                "{p}"
            );
        }
    }

    /// WSL 게스트 경로 판정 — 이 판정이 틀리면 두 방향으로 다 나쁘다.
    /// 못 알아보면 WSL 이 꺼진 뒤에도 스캔이 그 경로를 읽어 배포판을 깨우고,
    /// 로컬 경로를 WSL 로 오인하면 멀쩡한 루트가 통째로 빠진다.
    #[test]
    fn wsl_paths_are_recognized_in_both_spellings() {
        let wsl = [
            r"\\wsl.localhost\Ubuntu\home\me",
            r"\\WSL.LOCALHOST\Ubuntu\root", // 대소문자는 상관없다
            r"\\wsl$\Ubuntu\home\me",       // 옛 표기 (설정에 직접 넣은 홈)
        ];
        for p in wsl {
            assert!(is_wsl_path(std::path::Path::new(p)), "{p}");
        }

        let local = [
            r"C:\Users\me\.claude",
            r"\\nas\share\wsl.localhost\x", // 다른 UNC 안에 이름만 같은 폴더
            "/home/me/.claude",
        ];
        for p in local {
            assert!(!is_wsl_path(std::path::Path::new(p)), "{p}");
        }
    }

    /// Codex 만 환경변수를 보던 예외를 없앤 것에 대한 회귀 방지.
    /// 환경변수를 읽으면 앱을 어떻게 띄웠느냐에 따라 집계 범위가 달라진다.
    #[test]
    fn codex_home_env_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let homes = codex_homes();
        std::env::remove_var("CODEX_HOME");
        assert!(
            !homes.contains(&dir.path().to_path_buf()),
            "CODEX_HOME 은 탐색에 쓰이지 않는다 — 재배치 홈은 마커 스캔이나 설정으로 들어온다"
        );
    }

    /// 대안 배치는 **홈마다 전부** 시도해야 한다. 예전에 Claude 의 XDG 대안을 로컬 홈에만
    /// 붙였다가 WSL 게스트가 빠진 적이 있다 (그 배치 자체는 지금 빠져 있다 —
    /// `CLAUDE_PROJECT_LAYOUTS` 주석 참고). 배치 데이터와 무관하게 **조합 규칙**을 지킨다.
    #[test]
    fn every_home_gets_every_layout() {
        const TWO: &[&[&str]] = &[&["a", "data"], &["b", "c", "data"]];
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        // 홈마다 서로 다른 배치를 쓰는 상황
        std::fs::create_dir_all(one.path().join("a").join("data")).unwrap();
        std::fs::create_dir_all(two.path().join("b").join("c").join("data")).unwrap();

        let out = dirs_under(vec![one.path().to_path_buf(), two.path().to_path_buf()], TWO);
        assert!(out.contains(&one.path().join("a").join("data")));
        assert!(
            out.contains(&two.path().join("b").join("c").join("data")),
            "어떤 홈이든 모든 배치를 시도해야 한다"
        );
        assert_eq!(out.len(), 2, "존재하지 않는 조합은 걸러진다");
    }

    /// 재배치된 홈이 들어오는 유일한 자동 경로는 설정(`extraCodexHomes`)이다.
    /// 세 소스가 모두 같은 규칙을 쓴다.
    #[test]
    fn extra_homes_are_added_for_every_source() {
        let dir = tempfile::tempdir().unwrap();
        let extra = vec![dir.path().to_path_buf()];
        assert!(codex_homes_with(&extra).contains(&dir.path().to_path_buf()));
        assert!(antigravity_homes_with(&extra).contains(&dir.path().to_path_buf()));

        // Claude 만 홈 아래 하위 디렉토리가 루트라 접미사가 붙는다
        let proj = dir.path().join("projects");
        std::fs::create_dir_all(&proj).unwrap();
        assert!(claude_project_roots_with(&extra).contains(&proj));
    }
}

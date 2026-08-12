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
/// Windows 가 배포판을 깨우려 들기 때문이다. 그래서 두 겹으로 막는다:
/// **실행 중인 배포판만** 보고(`--running` — 사용량 보자고 남의 WSL 을 깨울 이유가 없다),
/// 그마저도 [`WSL_PROBE_TIMEOUT`] 안에 안 끝나면 포기한다.
///
/// 포기한 결과는 캐시하지 **않는다** — 나중에 WSL 을 켜고 "다시 검색" 하면 잡히도록.
/// 성공한 결과는 캐시하되 [`forget_wsl_guest_homes`] 로 버릴 수 있다.
#[cfg(windows)]
fn wsl_guest_homes() -> Vec<PathBuf> {
    let cell = wsl_cache();
    if let Some(cached) = cell.lock().unwrap().as_ref() {
        return cached.clone();
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

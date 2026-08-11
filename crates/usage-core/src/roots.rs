//! OS/WSL별 데이터 루트 탐색.
//!
//! - Linux/macOS: `~/.claude`, `~/.config/claude`(XDG 대안), `~/.codex`, `~/.gemini/antigravity-cli`
//! - Windows: `%USERPROFILE%\.claude` 등 (dirs::home_dir 가 처리)
//! - WSL: 위에 더해 `/mnt/c/Users/<user>/.claude` 등 Windows 쪽 홈도 병합

use std::path::{Path, PathBuf};

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// WSL 환경 여부 (리눅스 커널 버전 문자열에 microsoft 포함)
pub fn is_wsl() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        return true;
    }
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// WSL에서 보이는 Windows 사용자 홈 디렉토리 목록 (`/mnt/c/Users/<user>`)
fn wsl_windows_user_homes() -> Vec<PathBuf> {
    if !is_wsl() {
        return vec![];
    }
    let users = Path::new("/mnt/c/Users");
    let Ok(entries) = std::fs::read_dir(users) else {
        return vec![];
    };
    let skip = ["Public", "Default", "Default User", "All Users", "desktop.ini"];
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| !skip.contains(&n))
                .unwrap_or(false)
        })
        .collect()
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
/// 자동 탐지는 프로세스 환경에 의존한다 — 트레이에서 뜬 앱은 터미널의 `CODEX_HOME` 을
/// 물려받지 못해서, 그 경로가 있다는 사실조차 알 수 없다. 설정에 적어 두면 실행 방식과
/// 무관하게 항상 같은 범위를 본다. 합쳐서 생기는 중복은 어댑터의 이벤트 dedup 이 막는다.
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
#[cfg(windows)]
fn wsl_guest_homes() -> Vec<PathBuf> {
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<Option<Vec<PathBuf>>>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(None));
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

/// 후보 홈들에 suffix를 붙여 존재하는 것만 반환
fn candidate_dirs(suffix: &[&str]) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = vec![];
    if let Some(h) = home_dir() {
        homes.push(h);
    }
    homes.extend(wsl_windows_user_homes());
    homes.extend(wsl_guest_homes());

    let mut out = vec![];
    for home in homes {
        let mut p = home;
        for part in suffix {
            p = p.join(part);
        }
        out.push(p);
    }
    existing(out)
}

/// Claude Code 트랜스크립트 루트 (`.claude/projects` + XDG 대안)
pub fn claude_project_roots() -> Vec<PathBuf> {
    claude_project_roots_with(&[])
}

/// `extra` 는 추가로 볼 `.claude` 홈 디렉토리 목록 (그 아래 `projects` 를 본다)
pub fn claude_project_roots_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = candidate_dirs(&[".claude", "projects"]);
    if let Some(h) = home_dir() {
        roots.push(h.join(".config/claude/projects"));
    }
    with_extra(roots, extra, &["projects"])
}

/// Claude Code 라이브 세션 레지스트리 (`.claude/sessions`)
pub fn claude_session_dirs() -> Vec<PathBuf> {
    claude_session_dirs_with(&[])
}

/// `extra` 는 추가로 볼 `.claude` 홈 디렉토리 목록 (그 아래 `sessions` 를 본다)
pub fn claude_session_dirs_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(&[".claude", "sessions"]), extra, &["sessions"])
}

/// Codex CLI 홈. 기본은 `~/.codex` 이고, `CODEX_HOME` 은 그 루트를 **대체**한다.
///
/// 공식 문서(learn.chatgpt.com/docs/config-file/environment-variables)의 `CODEX_HOME` 항목:
/// *"Sets the root for Codex state, including config, auth, logs, sessions, skills, ..."* —
/// 기본값 `~/.codex` 를 옮기는 것이지 추가 탐색 경로가 아니다.
///
/// 예전에는 둘을 합쳐서 봤는데, 같은 rollout 파일이 양쪽에 있으면 사용량이 두 번 집계됐다
/// (실측: 이벤트 4건이 5건으로). 대체 의미로 바꾸면 그 경우가 아예 생기지 않는다.
///
/// `CODEX_HOME` 이 설정됐는데 그 경로가 없으면 빈 목록이다 — 그게 이 설치본의 루트이고,
/// `~/.codex` 로 되돌아가면 방금 없앤 중복 문제가 되살아난다.
pub fn codex_homes() -> Vec<PathBuf> {
    codex_homes_with(&[])
}

/// `extra` 는 추가로 볼 Codex 홈 디렉토리 목록 (그 아래 `sessions`/`archived_sessions`).
/// 자동 탐지분과 합쳐도 어댑터의 이벤트 dedup 이 중복 집계를 막는다.
pub fn codex_homes_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(codex_homes_from(std::env::var_os("CODEX_HOME")), extra, &[])
}

fn codex_homes_from(env_home: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    match env_home {
        Some(h) => existing(vec![PathBuf::from(h)]),
        // 미설정이 일반적인 경우 — WSL 경계를 넘는 `.codex` 들은 서로 다른 설치본이라 그대로 병합한다
        None => candidate_dirs(&[".codex"]),
    }
}

/// Antigravity CLI(`agy`) 홈 (`~/.gemini/antigravity-cli`).
///
/// Gemini CLI 를 대체한 도구지만 `~/.gemini` 아래에 자기 폴더를 따로 쓴다.
/// 홈 자체가 루트이고 그 아래 `conversations/<uuid>.db` 가 대화들이다.
/// Codex 의 `CODEX_HOME` 같은 재배치 환경변수는 관측되지 않았다.
pub fn antigravity_homes() -> Vec<PathBuf> {
    antigravity_homes_with(&[])
}

/// `extra` 는 추가로 볼 `antigravity-cli` 홈 디렉토리 목록.
/// 합쳐서 같은 대화 DB 가 두 번 잡혀도 어댑터의 요청 id dedup 이 중복 집계를 막는다.
pub fn antigravity_homes_with(extra: &[PathBuf]) -> Vec<PathBuf> {
    with_extra(candidate_dirs(&[".gemini", "antigravity-cli"]), extra, &[])
}

/// OS 경계를 넘는 경로 여부 — pid 생존 확인이 불가능한 세션 디렉토리 판단용.
/// WSL에서 본 Windows 마운트(`/mnt/...`)와 Windows에서 본 WSL 공유(`\\wsl...`)가 해당.
pub fn is_windows_mount(p: &Path) -> bool {
    if p.starts_with("/mnt/") {
        return true;
    }
    p.to_string_lossy().starts_with(r"\\wsl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_home_replaces_default_root() {
        let dir = tempfile::tempdir().unwrap();
        let homes = codex_homes_from(Some(dir.path().as_os_str().to_owned()));
        assert_eq!(homes, vec![dir.path().to_path_buf()]);

        // 기본 `.codex` 후보가 섞여 들어오면 같은 rollout 을 두 번 세게 된다
        let home_codex = home_dir().map(|h| h.join(".codex"));
        if let Some(hc) = home_codex {
            assert!(!homes.contains(&hc), "CODEX_HOME 은 대체이지 추가가 아니다");
        }
    }

    #[test]
    fn missing_codex_home_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("nope");
        assert!(codex_homes_from(Some(gone.into_os_string())).is_empty());
    }
}

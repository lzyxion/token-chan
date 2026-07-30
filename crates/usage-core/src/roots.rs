//! OS/WSL별 데이터 루트 탐색.
//!
//! - Linux/macOS: `~/.claude`, `~/.config/claude`(XDG 대안), `~/.codex`, `~/.gemini`
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

/// 존재하는 경로만 남기고 중복 제거
fn existing(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.retain(|p| p.is_dir());
    paths.dedup();
    paths
}

/// Windows 빌드에서 WSL 배포판 내부의 리눅스 홈 디렉토리 목록
/// (`\\wsl.localhost\<distro>\home\<user>`, `\root`).
/// 같은 머신에서 WSL로 쓴 CLI 사용량을 Windows 앱이 병합하기 위함.
#[cfg(windows)]
fn wsl_guest_homes() -> Vec<PathBuf> {
    use std::os::windows::process::CommandExt;
    use std::sync::OnceLock;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let Ok(out) = std::process::Command::new("wsl.exe")
                .args(["-l", "-q"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            else {
                return vec![];
            };
            // wsl.exe 출력은 UTF-16LE
            let utf16: Vec<u16> = out
                .stdout
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
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
        })
        .clone()
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
    let mut roots = candidate_dirs(&[".claude", "projects"]);
    if let Some(h) = home_dir() {
        let xdg = h.join(".config/claude/projects");
        if xdg.is_dir() {
            roots.push(xdg);
        }
    }
    roots.dedup();
    roots
}

/// Claude Code 라이브 세션 레지스트리 (`.claude/sessions`)
pub fn claude_session_dirs() -> Vec<PathBuf> {
    candidate_dirs(&[".claude", "sessions"])
}

/// Codex CLI 홈 (`$CODEX_HOME` 우선, 기본 `~/.codex`)
pub fn codex_homes() -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(env_home) = std::env::var_os("CODEX_HOME") {
        let p = PathBuf::from(env_home);
        if p.is_dir() {
            out.push(p);
        }
    }
    out.extend(candidate_dirs(&[".codex"]));
    out.dedup();
    out
}

/// Gemini CLI 홈 (`~/.gemini`)
pub fn gemini_homes() -> Vec<PathBuf> {
    candidate_dirs(&[".gemini"])
}

/// OS 경계를 넘는 경로 여부 — pid 생존 확인이 불가능한 세션 디렉토리 판단용.
/// WSL에서 본 Windows 마운트(`/mnt/...`)와 Windows에서 본 WSL 공유(`\\wsl...`)가 해당.
pub fn is_windows_mount(p: &Path) -> bool {
    if p.starts_with("/mnt/") {
        return true;
    }
    p.to_string_lossy().starts_with(r"\\wsl")
}

//! Claude Code 라이브 세션 상태 (`~/.claude/sessions/<pid>.json`).
//!
//! 캐릭터 애니메이션(작업 중/대기) 구동용. 규칙:
//! - `status == "busy"` 이고 `updatedAt` 이 최근(기본 10분)이어야 busy 로 간주
//! - 같은 OS 의 세션(pid 확인 가능)은 /proc/<pid> 생존 확인 (unix)
//! - Windows 마운트(/mnt/...) 쪽 세션 파일은 pid 확인 불가 → 신선도만 사용

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Windows 마운트 세션(pid 확인 불가)의 updatedAt 신선도 한계 (ms)
pub const FRESH_MS: i64 = 10 * 60 * 1000;

/// 같은 OS 세션의 updatedAt 허용 한계 (ms).
/// updatedAt 은 상태 전환 시점에만 갱신되므로 긴 턴(>10분) 중에도 busy 를 유지하려면
/// 느슨해야 한다 — 죽은 세션은 pid 생존 확인이 걸러낸다.
pub const FRESH_MS_LOCAL: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Deserialize)]
struct SessionFile {
    pid: Option<u64>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    status: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<i64>,
    name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveSessionView {
    pub name: String,
    pub status: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveState {
    pub busy: bool,
    pub busy_count: usize,
    pub sessions: Vec<LiveSessionView>,
}

pub fn read_live_state(dirs: &[PathBuf], now_ms: i64) -> LiveState {
    let mut state = LiveState::default();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let windows_mount = crate::roots::is_windows_mount(dir);
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(s) = std::fs::read_to_string(&path) else { continue };
            let Ok(sf) = serde_json::from_str::<SessionFile>(&s) else { continue };

            // pid 확인이 가능한 로컬 세션은 pid 생존이 1차 근거 → 신선도는 느슨하게.
            // pid 확인이 불가능한 Windows 마운트 세션은 신선도가 유일한 근거 → 엄격하게.
            let limit = if windows_mount { FRESH_MS } else { FRESH_MS_LOCAL };
            let fresh = sf
                .updated_at
                .map(|t| now_ms.saturating_sub(t) < limit)
                .unwrap_or(false);
            if !fresh {
                continue;
            }
            // 같은 OS 세션이면 pid 생존 확인 (죽은 세션의 잔존 파일 무시)
            if !windows_mount {
                if let Some(pid) = sf.pid {
                    if !pid_alive(pid) {
                        continue;
                    }
                }
            }

            let status = sf.status.unwrap_or_else(|| "unknown".into());
            if status == "busy" {
                state.busy = true;
                state.busy_count += 1;
            }
            state.sessions.push(LiveSessionView {
                name: sf
                    .name
                    .or(sf.session_id.map(|s| s.chars().take(8).collect()))
                    .unwrap_or_else(|| "session".into()),
                status,
                cwd: sf.cwd.unwrap_or_default(),
            });
        }
    }
    state
}

#[cfg(unix)]
fn pid_alive(pid: u64) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(unix))]
fn pid_alive(_pid: u64) -> bool {
    // Windows: pid 확인 생략, updatedAt 신선도에 의존
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn session_json(pid: u64, status: &str, updated_at: i64) -> String {
        format!(
            r#"{{"pid":{pid},"sessionId":"aaaa-bbbb","cwd":"/home/u/proj","startedAt":0,"status":"{status}","updatedAt":{updated_at},"name":"demo"}}"#
        )
    }

    #[test]
    fn busy_when_fresh_and_alive() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        let my_pid = std::process::id() as u64; // 확실히 살아있는 pid
        fs::write(dir.path().join("1.json"), session_json(my_pid, "busy", now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(state.busy);
        assert_eq!(state.busy_count, 1);
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn stale_or_dead_sessions_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        let my_pid = std::process::id() as u64;
        // 로컬 세션 한계(24h)보다 오래된 updatedAt → pid 가 살아있어도 무시
        fs::write(dir.path().join("1.json"), session_json(my_pid, "busy", now_ms - FRESH_MS_LOCAL - 1)).unwrap();
        // 죽은 pid (u32::MAX 근처는 존재할 수 없음) → 무시 (unix)
        fs::write(dir.path().join("2.json"), session_json(4_294_967_000, "busy", now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        #[cfg(unix)]
        {
            assert!(!state.busy);
            assert!(state.sessions.is_empty());
        }
    }

    #[test]
    fn idle_sessions_listed_but_not_busy() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        let my_pid = std::process::id() as u64;
        fs::write(dir.path().join("1.json"), session_json(my_pid, "idle", now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(!state.busy);
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].status, "idle");
    }
}

//! 라이브 세션 상태 — 캐릭터 애니메이션(작업 중/대기) 구동용.
//!
//! Claude 만 세션 레지스트리(`~/.claude/sessions/<pid>.json`)를 남겨 정확히 알 수 있다:
//! - `status == "busy"` 이고 `updatedAt` 이 최근(기본 10분)이어야 busy 로 간주
//! - 같은 OS 의 세션(pid 확인 가능)은 /proc/<pid> 생존 확인 (unix)
//! - Windows 마운트(/mnt/...) 쪽 세션 파일은 pid 확인 불가 → 신선도만 사용
//!
//! Codex·Antigravity 는 그런 레지스트리가 없어서 **트랜스크립트 파일이 방금 쓰였는지**로
//! 유도한다([`infer_busy`]). 둘 다 응답을 스트리밍하며 파일에 계속 덧쓰기 때문에 작업
//! 중이면 mtime 이 계속 움직인다. 정확한 신호가 아니라 유도값이므로 상태 이름도
//! `busy` 가 아니라 `active` 로 구분해 둔다.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Source;

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

/// 트랜스크립트가 마지막으로 쓰인 뒤 이 시간 안이면 작업 중으로 본다.
/// 짧게 잡으면 생각이 긴 턴에서 깜빡이고, 길게 잡으면 끝난 뒤에도 계속 타자를 친다.
/// 두 CLI 모두 스트리밍 중 수 초 간격으로 덧쓰므로 45초면 충분히 여유 있다.
pub const INFERRED_BUSY_MS: i64 = 45 * 1000;

#[derive(Debug, Clone, Serialize)]
pub struct LiveSessionView {
    pub source: Source,
    pub name: String,
    /// `busy`/`idle` 은 세션 레지스트리에서 온 정확한 값, `active` 는 파일 신선도로 유도한 값
    pub status: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveState {
    pub busy: bool,
    pub busy_count: usize,
    pub sessions: Vec<LiveSessionView>,
}

/// 마지막 쓰기 시각으로 작업 중 여부를 유도한다.
/// 미래 시각(시계 어긋남)은 작업 중으로 친다 — 방금 쓰였다는 뜻이다.
pub fn infer_busy(last_write: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    last_write
        .map(|t| (now - t).num_milliseconds() < INFERRED_BUSY_MS)
        .unwrap_or(false)
}

/// 세션 레지스트리가 없는 소스의 활동을 감시 파일 mtime 으로 판정해 상태에 얹는다.
/// `watch` 는 소스별 (감시 파일, 세션 이름) — 어댑터의 `watch_path()` 결과다.
pub fn add_inferred(state: &mut LiveState, watch: &[(Source, PathBuf, String)], now: DateTime<Utc>) {
    for (source, path, name) in watch {
        let written: Option<DateTime<Utc>> = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(Into::into);
        if !infer_busy(written, now) {
            continue;
        }
        state.busy = true;
        state.busy_count += 1;
        state.sessions.push(LiveSessionView {
            source: *source,
            name: name.clone(),
            // 레지스트리에서 온 `busy` 와 구분한다 — 이건 파일 신선도로 유도한 값이다
            status: "active".into(),
            cwd: String::new(),
        });
    }
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
                source: Source::Claude,
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

        let _state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        // pid 생존 확인은 unix 에서만 가능하다 (windows 는 신선도에만 의존)
        #[cfg(unix)]
        {
            assert!(!_state.busy);
            assert!(_state.sessions.is_empty());
        }
    }

    #[test]
    fn inferred_busy_uses_write_freshness() {
        let now = chrono::Utc::now();
        assert!(infer_busy(Some(now - chrono::Duration::seconds(5)), now));
        assert!(infer_busy(Some(now - chrono::Duration::seconds(44)), now));
        assert!(!infer_busy(Some(now - chrono::Duration::seconds(60)), now));
        assert!(!infer_busy(None, now), "쓴 적이 없으면 작업 중이 아니다");
        // 시계가 어긋나 미래로 찍혀도 방금 쓰인 것으로 본다
        assert!(infer_busy(Some(now + chrono::Duration::seconds(10)), now));
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

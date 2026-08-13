//! 라이브 세션 상태 — 캐릭터 애니메이션(작업 중/대기)과 작업 완료 알림 구동용.
//!
//! **세 소스 모두 턴 경계를 파일에서 직접 읽는다.** 유도는 하지 않는다.
//!
//! | 소스 | 작업 중 | 완료 |
//! |---|---|---|
//! | Claude | `~/.claude/sessions/<pid>.json` 의 `status == "busy"` | `busy` → [`CLAUDE_DONE_STATUSES`] |
//! | Codex | rollout 의 `task_started` (→ [`crate::codex::TurnWatcher`]) | `task_complete` |
//! | Antigravity | transcript 의 `USER_INPUT` (→ [`crate::antigravity::TurnWatcher`]) | `tool_calls` 없는 `PLANNER_RESPONSE` |
//!
//! # 부재를 완료로 읽지 않는다
//!
//! 완료는 **양(+)의 신호가 있을 때만** [`LiveState::completed`] 에 실린다. 세션이 목록에서
//! 그냥 사라진 것은 완료가 아니다 — 크래시, 강제 종료, 안전망 타임아웃이 전부 같은 모양이기
//! 때문이다. 이걸 완료로 세면 죽은 세션을 두고 "5분 걸렸어" 라고 말하게 된다.
//!
//! 그래서 **프로세스 생존 확인도 하지 않는다.** 재료가 한 소스에만 있다 — Claude 는
//! `pid`+`procStart` 를 파일에 남기지만(CLI 자신도 그걸로 확인한다) Codex 는 pid 를 어디에도
//! 안 남기고, agy 의 `presence/*.lock` 은 프로세스가 없어도 남아 있는 것이 실측됐다.
//! 셋 중 하나만 정확해지는 판정은 "지금 이 벤더는 믿을 수 있나" 를 사용자가 외우게 만든다.
//! 생존 확인은 부재를 해석하려는 시도였고, 위 원칙이 부재 해석 자체를 그만두므로 필요가 없다.
//!
//! # 크기 변화 유도는 제거됐다
//!
//! 예전에는 레지스트리도 턴 이벤트도 없는 소스를 **감시 파일이 방금 자랐는지**로 유도하고
//! 상태를 `active` 로 표시했다(`add_inferred`·`WatchTracker`). 지웠다 — 그 신호로는 완료·
//! 크래시·승인 대기·긴 도구 실행이 전부 같은 모양이었고, 45초 꼬리 때문에 끝난 뒤에도
//! 한동안 작업 중이라고 말했다. 결정적으로 **소비처가 전부 `busy` 와 똑같이 취급**해서,
//! 애매하다는 이름표를 달아 두고 아무도 그 이름표를 보지 않는 상태였다.
//! 이제 턴 경계를 못 읽는 소스는 아무 말도 하지 않는다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Source;

/// OS 경계를 넘은 세션(`\\wsl...`)의 updatedAt 신선도 한계 (ms).
/// 시계가 어긋날 수 있어 같은 OS 세션보다 엄격하게 본다.
pub const FRESH_MS: i64 = 10 * 60 * 1000;

/// 같은 OS 세션의 updatedAt 허용 한계 (ms).
///
/// `updatedAt` 은 상태 전환 시점에만 갱신되므로, 긴 턴(>10분) 중에도 busy 를 유지하려면
/// 느슨해야 한다. **그 대가로 비정상 종료한 세션이 이 시간만큼 작업 중으로 남아 보인다.**
/// 생존 확인을 하지 않기로 했으므로(모듈 주석) 이게 유일한 방어이고 앞으로도 그렇다.
/// 다만 그 세션이 **완료로 잡히지는 않는다** — 사라지는 것은 완료 신호가 아니다.
pub const FRESH_MS_LOCAL: i64 = 24 * 60 * 60 * 1000;

/// 세션 레지스트리 status 중 **턴이 끝났다고 볼 수 있는** 값.
///
/// 거부목록("busy 가 아니면 완료")이 아니라 **허용목록**인 게 핵심이다. CLI 안의 전체
/// 어휘는 `["busy", "shell", "idle", "waiting"]` 인데(실측) 이건 공개 API 가 아니라 값이
/// 늘 수 있다. 거부목록이면 처음 보는 값이 전부 "완료"가 되지만, 허용목록이면 알림을
/// **놓칠 뿐 거짓말은 안 한다.**
///
/// `waiting` 을 뺀 이유: 사용자 입력·승인을 기다리는 상태로 보인다. 일이 끝난 게 아니라
/// **멈춘 것**이라 "다 끝났어" 는 거짓말이 된다. (셋 중 이 구분이 되는 건 Claude 뿐이다 —
/// Codex 의 승인 대기는 계속 `task_started` 상태라 아예 관측되지 않는다.)
pub const CLAUDE_DONE_STATUSES: [&str; 2] = ["shell", "idle"];

/// 파일에는 `pid`·`procStart` 도 있지만 읽지 않는다 — 생존 확인을 하지 않기 때문 (모듈 주석).
#[derive(Debug, Clone, Deserialize)]
struct SessionFile {
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
    pub source: Source,
    /// 세션 id — [`crate::session::SessionRow::id`] 와 **같은 값**이라 최근 세션 목록의
    /// 어느 줄이 지금 돌고 있는지 짚을 수 있다. 못 알아내면 빈 문자열(짚지 않는다).
    ///
    /// `name` 으로는 짚을 수 없다 — Claude 는 사용자가 붙인 세션 이름이 있으면 그걸 쓴다.
    pub id: String,
    pub name: String,
    /// Claude 는 레지스트리 값 그대로(`busy`/`shell`/`idle`/`waiting`),
    /// Codex·agy 는 턴 감시기가 도는 세션에만 `busy` 를 붙인다.
    /// **유도값(`active`)은 더 이상 없다** — 모듈 주석 참고.
    pub status: String,
    pub cwd: String,
}

/// 이번 회차에 **완료 이벤트로** 끝난 세션.
///
/// 안전망 타임아웃으로 풀린 세션은 여기 없다 — 크래시와 완료를 가르는 유일한 신호다.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedSession {
    pub source: Source,
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveState {
    pub busy: bool,
    pub busy_count: usize,
    pub sessions: Vec<LiveSessionView>,
    /// 이번 회차에 끝난 세션들. `sessions` 에서 빠지는 것과 **같은 회차**에 실리므로
    /// 소비처가 두 값을 나란히 보고 "끝났다"와 "사라졌다"를 가를 수 있다.
    pub completed: Vec<CompletedSession>,
}

/// Claude 세션 하나가 이번 회차에 턴을 마쳤는지.
///
/// 목록에서 **사라진** 경우는 이 함수에 오지 않는다 — 레지스트리는 `<pid>.json` 이라
/// 프로세스가 끝나면 파일째 사라지고, 크래시도 (신선도로 밀려나) 같은 모양이 된다.
/// 둘 다 턴 완료가 아니므로 호출자가 걸러야 한다.
pub fn claude_turn_finished(prev: &str, now: &str) -> bool {
    prev == "busy" && CLAUDE_DONE_STATUSES.contains(&now)
}

/// `offset` 부터 읽어 **완결된 줄만** 돌려준다. 두 번째 값은 소비한 바이트 수 —
/// 마지막 줄이 아직 쓰이는 중일 수 있으므로 개행까지만 전진한다.
///
/// 턴 경계를 파일에서 직접 읽는 두 어댑터(Codex rollout · agy transcript)가 같이 쓴다.
/// 원래 codex.rs 안에 있었는데 agy 도 같은 방식이 되면서 공용 자리인 여기로 올렸다.
pub(crate) fn read_from(path: &Path, offset: u64) -> (Vec<String>, u64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return (vec![], 0) };
    if f.seek(SeekFrom::Start(offset)).is_err() {
        return (vec![], 0);
    }
    let mut buf = String::new();
    // 유효하지 않은 UTF-8 이 섞이면 통째로 실패하므로 바이트로 읽고 손실 변환한다
    let mut raw = vec![];
    if f.read_to_end(&mut raw).is_err() {
        return (vec![], 0);
    }
    let Some(last_nl) = raw.iter().rposition(|b| *b == b'\n') else { return (vec![], 0) };
    let complete = &raw[..=last_nl];
    buf.push_str(&String::from_utf8_lossy(complete));
    let lines = buf.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    (lines, complete.len() as u64)
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

            // OS 경계를 넘은 세션은 시계도 다를 수 있어 더 엄격하게 본다
            let limit = if windows_mount { FRESH_MS } else { FRESH_MS_LOCAL };
            let fresh = sf
                .updated_at
                .map(|t| now_ms.saturating_sub(t) < limit)
                .unwrap_or(false);
            if !fresh {
                continue;
            }

            let id = sf.session_id.clone().unwrap_or_default();
            let status = sf.status.unwrap_or_else(|| "unknown".into());
            if status == "busy" {
                state.busy = true;
                state.busy_count += 1;
            }
            state.sessions.push(LiveSessionView {
                source: Source::Claude,
                id,
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
    fn busy_when_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        fs::write(dir.path().join("1.json"), session_json(std::process::id() as u64, "busy", now_ms - 1000))
            .unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(state.busy);
        assert_eq!(state.busy_count, 1);
        assert_eq!(state.sessions.len(), 1);
    }

    /// 신선도가 유일한 방어다 — 생존 확인을 안 하기로 했으므로 한계를 넘은 세션만 걸러진다.
    #[test]
    fn stale_sessions_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        let my_pid = std::process::id() as u64;
        // 한계(24h)보다 오래된 updatedAt → 무시
        fs::write(dir.path().join("1.json"), session_json(my_pid, "busy", now_ms - FRESH_MS_LOCAL - 1)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(!state.busy);
        assert!(state.sessions.is_empty());
    }

    /// 한계 안이면 프로세스가 죽었어도 작업 중으로 보인다 — **의도된 한계**다.
    /// 생존 확인은 재료가 한 소스에만 있어 지원하지 않기로 했다(모듈 주석).
    /// 대신 이 세션이 사라질 때 **완료로 잡히지는 않는다** — 그게 이 결정의 대가를 갚는다.
    #[test]
    fn dead_process_still_looks_busy_within_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        // 존재할 수 없는 pid (u32::MAX 근처)인데도 신선하면 busy 로 잡힌다
        fs::write(dir.path().join("1.json"), session_json(4_294_967_000, "busy", now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(state.busy, "생존 확인이 없으므로 죽은 세션도 신선하면 busy 다");
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

    /// 완료는 **허용목록**이다. 실측된 어휘 넷 중 둘만 완료로 친다.
    #[test]
    fn only_allowlisted_statuses_finish_a_turn() {
        assert!(claude_turn_finished("busy", "shell"), "실측된 정상 종료 경로");
        assert!(claude_turn_finished("busy", "idle"));
        // 멈춘 것이지 끝난 게 아니다
        assert!(!claude_turn_finished("busy", "waiting"));
        // 아직 도는 중
        assert!(!claude_turn_finished("busy", "busy"));
        // busy 였던 적이 없으면 끝날 것도 없다
        assert!(!claude_turn_finished("shell", "idle"));
    }

    /// 처음 보는 값이 늘어도 **거짓 완료를 만들지 않는다** — 거부목록이었다면 전부 완료가 된다.
    #[test]
    fn unknown_statuses_never_finish_a_turn() {
        for s in ["compacting", "paused", "unknown", ""] {
            assert!(!claude_turn_finished("busy", s), "{s}");
        }
    }
}

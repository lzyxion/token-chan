//! 라이브 세션 상태 — 캐릭터 애니메이션(작업 중/대기)과 작업 완료 알림 구동용.
//!
//! **세 소스 모두 턴 경계를 파일에서 직접 읽는다.** 유도는 하지 않는다.
//!
//! | 소스 | 작업 중 | 완료 |
//! |---|---|---|
//! | Claude (TUI) | `~/.claude/sessions/<pid>.json` 의 `status == "busy"` | `busy` → [`CLAUDE_DONE_STATUSES`] |
//! | Claude (그 외) | 트랜스크립트의 사람 메시지 (→ [`crate::claude::TurnWatcher`]) | `stop_reason:"end_turn"` |
//! | Codex | rollout 의 `task_started` (→ [`crate::codex::TurnWatcher`]) | `task_complete` |
//! | Antigravity | transcript 의 `USER_INPUT` (→ [`crate::antigravity::TurnWatcher`]) | `tool_calls` 없는 `PLANNER_RESPONSE` |
//!
//! # 레지스트리는 Claude 의 전부가 아니다
//!
//! `status` 를 쓰는 건 CLI 의 **터미널 UI 뿐**이다 (실측: 2.1.241 번들에서 그 값을 쓰는
//! 유일한 호출이 Ink 컴포넌트의 `useEffect` 안에 있다). 그래서 TUI 없이 뜬 세션 —
//! Obsidian 플러그인 같은 SDK 진입점(`entrypoint:"sdk-ts"`) — 은 시작할 때 파일만
//! 등록하고 `status`·`updatedAt` 을 영영 안 쓴다. 예전엔 그 둘이 없다는 이유로 세션이
//! **목록에서 통째로 빠졌다**: 사용량과 최근 세션에는 잡히는데 작업 중에는 안 잡히는,
//! 설명하기 어려운 비대칭이었다.
//!
//! 고칠 때 레지스트리를 버리고 셋 다 트랜스크립트로 통일하는 길도 있었지만 **안 갔다.**
//! 지금 살아 있는 세션이 무엇인지 알려주는 건 레지스트리뿐이고, 그게 없으면 수천 개
//! 트랜스크립트 중 어느 것이 살아 있는지를 mtime 으로 골라야 한다 — 아래에서 지웠다고
//! 적은 그 신호다. (Codex 는 `history.jsonl` 이 그 색인 역할을 하지만 Claude 엔 없다.)
//! 그래서 **레지스트리가 감시 대상을 정하고**, 상태를 안 주는 세션만 턴 경계를 읽는다.
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
    /// **없을 수 있다** — 이 값을 쓰는 건 CLI 의 터미널 UI 뿐이라, TUI 없이 뜬 세션
    /// (Obsidian 플러그인 같은 SDK 진입점, `entrypoint:"sdk-ts"`)은 시작할 때 파일만
    /// 등록하고 상태는 영영 안 쓴다. 그런 세션은 [`LiveState::headless`] 로 넘어가
    /// 트랜스크립트에서 턴 경계를 읽는다 ([`crate::claude::TurnWatcher`]).
    ///
    /// **이 값의 유무가 신선도를 적용할지도 가른다** — `read_live_state` 참고.
    status: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<i64>,
    /// `updated_at` 이 없을 때의 신선도 기준 — **보험**이다. 지금 관측되는 진입점 중
    /// `status` 는 쓰면서 `updated_at` 은 안 쓰는 것은 없지만, 그런 게 생기면 방금
    /// 고친 그 버그(신선도에서 통째로 걸림)가 그대로 재발한다.
    #[serde(rename = "startedAt")]
    started_at: Option<i64>,
    name: Option<String>,
}

/// 레지스트리가 `status` 를 안 주는 세션 — 작업 중 여부를 트랜스크립트에서 읽어야 한다.
///
/// 레지스트리를 **대체**하는 게 아니라 **보강**하는 자리다. 지금 살아 있는 세션이
/// 무엇이고 그 `sessionId` 가 뭔지 알려주는 건 여전히 레지스트리뿐이고, 그게 없으면
/// `projects/` 아래 수백~수천 개 트랜스크립트 중 어느 것이 살아 있는지를 mtime 으로
/// 골라야 한다 — 그건 [`crate::codex`] 가 지운 크기 변화 유도와 같은 신호다.
/// 그래서 **감시 대상을 레지스트리가 정하고**, 턴 경계만 파일에서 읽는다.
///
/// 이 세션들은 [`LiveState::sessions`] 에 **바로 실리지 않는다.** Codex·agy 와 똑같이,
/// 턴 감시기가 돌고 있다고 말한 것만 호출자가 목록에 올린다 — 이름과 cwd 를 여기 같이
/// 들고 가는 건 그때 uuid 조각 대신 제대로 된 줄을 그리기 위해서다.
#[derive(Debug, Clone)]
pub struct HeadlessSession {
    /// [`LiveSessionView::id`] 와 같은 값 — 감시기가 결과를 이 id 로 돌려준다
    pub id: String,
    /// 레지스트리가 준 표시 이름 (`obsidian-c0` 같은 파생 이름)
    pub name: String,
    pub cwd: String,
    /// 이 세션의 트랜스크립트가 있을 `<홈>/.claude/projects`
    pub projects_root: PathBuf,
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
    /// 이 세션이 **작업 중으로 관측되기 시작한** 시각 (epoch ms). 지금 도는 세션에만
    /// 값이 있다.
    ///
    /// CLI 가 파일에 적은 턴 시작 시각이 아니라 **우리가 처음 본 시각**이다. 라이브
    /// 스레드 주기(2초) 안에서 갈리므로 세 소스가 같은 정확도를 갖는다.
    ///
    /// `None` 은 "모른다" 다 — 앱을 켠 첫 회차에 이미 돌고 있던 세션이 그렇다. 그때
    /// 시작 시각을 지어내면 "9시간째" 같은 거짓말을 하게 되므로 값을 안 준다
    /// (프론트의 완료 대사가 같은 이유로 `at = 0` 을 쓰는 것과 같은 규칙).
    /// 값을 채우는 곳은 라이브 스레드 하나뿐이다 — 회차 간 상태가 필요해서다.
    pub busy_since: Option<i64>,
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
    /// 레지스트리가 상태를 안 주는 세션들 — 턴 감시기에 넘길 작업 목록이다.
    /// 프론트로 나가는 사실이 아니라 **다음 단계로 넘기는 재료**라 직렬화하지 않는다.
    #[serde(skip)]
    pub headless: Vec<HeadlessSession>,
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

            let id = sf.session_id.clone().unwrap_or_default();
            let name = sf
                .name
                .clone()
                .or_else(|| sf.session_id.clone().map(|s| s.chars().take(8).collect()))
                .unwrap_or_else(|| "session".into());
            let cwd = sf.cwd.clone().unwrap_or_default();

            // 상태를 안 주는 세션은 여기서 갈라져 감시기로 간다.
            //
            // **신선도를 씌우지 않는다.** 이 필터는 오래된 세션이 시시해서가 아니라
            // 남아 있는 `status:"busy"` 를 영원히 믿지 않으려고 있다 — 프로세스 생존
            // 확인을 안 하기로 했으므로(모듈 주석) 그게 유일한 방어이기 때문이다.
            // 그 값이 없는 세션에는 지킬 게 없고, 대신 감시기가 자기 안전망
            // ([`crate::claude::TURN_STALE_MS`] = 5분)을 갖고 있다. 씌우면 오히려
            // 해롭다: 그런 세션은 `updated_at` 도 `started_at` 도 안 움직여서, 멀쩡히
            // 일하는 세션이 켠 지 24시간 만에 사라진다.
            //
            // 목록에도 **바로 싣지 않는다** — Codex·agy 와 같은 규칙이다. 돌고 있다고
            // 감시기가 말한 것만 호출자가 올린다. 그래서 청소되지 않은 죽은 세션의
            // 파일이 남아도 유령 줄이 되지 않는다.
            if sf.status.is_none() {
                // id 를 모르면 감시할 파일을 특정할 수 없다 — 아무 말도 하지 않는다
                if !id.is_empty() {
                    state.headless.push(HeadlessSession {
                        id,
                        name,
                        cwd,
                        // `dir` 은 `<홈>/.claude/sessions` 라 형제가 트랜스크립트 루트다
                        projects_root: dir
                            .parent()
                            .map(|p| p.join("projects"))
                            .unwrap_or_else(|| dir.join("projects")),
                    });
                }
                continue;
            }

            // OS 경계를 넘은 세션은 시계도 다를 수 있어 더 엄격하게 본다
            let limit = if windows_mount { FRESH_MS } else { FRESH_MS_LOCAL };
            let fresh = sf
                .updated_at
                .or(sf.started_at)
                .map(|t| now_ms.saturating_sub(t) < limit)
                .unwrap_or(false);
            if !fresh {
                continue;
            }

            let status = sf.status.unwrap_or_default();
            if status == "busy" {
                state.busy = true;
                state.busy_count += 1;
            }
            state.sessions.push(LiveSessionView {
                source: Source::Claude,
                id,
                name,
                status,
                cwd,
                // 회차 간 상태가 필요해 라이브 스레드가 채운다 (필드 주석)
                busy_since: None,
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

    /// TUI 없이 뜬 세션 — `status`·`updatedAt` 이 **아예 없다**.
    /// (실측: Obsidian 플러그인이 띄운 `entrypoint:"sdk-ts"` 세션의 실제 모양)
    fn headless_json(pid: u64, started_at: i64) -> String {
        format!(
            r#"{{"pid":{pid},"sessionId":"cccc-dddd","cwd":"/home/u/vault","startedAt":{started_at},"entrypoint":"sdk-ts","peerProtocol":1,"name":"obsidian-c0"}}"#
        )
    }

    /// 예전엔 `updatedAt` 이 없다는 이유로 아예 걸러졌다 — 작업 중은커녕 존재조차
    /// 안 보였다. 이제 감시기로 넘어간다. 목록에는 **바로 싣지 않는다** — Codex·agy
    /// 와 같은 규칙으로, 돌고 있다고 감시기가 말한 것만 호출자가 올린다.
    #[test]
    fn status_less_sessions_go_to_the_watcher_not_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        fs::write(dir.path().join("7.json"), headless_json(7, now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(state.sessions.is_empty(), "돌고 있는지 모르는 채로 목록에 올리지 않는다");
        assert!(!state.busy);
        assert_eq!(state.headless.len(), 1, "감시기에 넘길 목록에 실린다");
        assert_eq!(state.headless[0].id, "cccc-dddd");
        assert_eq!(state.headless[0].name, "obsidian-c0", "이름·cwd 를 같이 넘긴다");
        assert_eq!(state.headless[0].cwd, "/home/u/vault");
        assert!(state.headless[0].projects_root.ends_with("projects"));
    }

    /// 레지스트리가 상태를 주면 그 값이 권위다 — 감시기로 넘기지 않는다.
    /// 보강이지 대체가 아니라는 게 여기서 갈린다.
    #[test]
    fn sessions_with_a_status_are_never_watched() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        fs::write(dir.path().join("1.json"), session_json(1, "busy", now_ms - 1000)).unwrap();
        fs::write(dir.path().join("2.json"), session_json(2, "idle", now_ms - 1000)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert_eq!(state.sessions.len(), 2);
        assert!(state.headless.is_empty());
    }

    /// **신선도를 씌우지 않는다.** 그 필터는 남아 있는 `status:"busy"` 를 영원히 믿지
    /// 않으려고 있는데, 이 세션엔 그 값이 없다. 반대로 씌우면 해롭다 — `startedAt` 이
    /// 안 움직여서 며칠째 멀쩡히 일하는 세션이 24시간 만에 사라진다.
    /// 크래시 방어는 감시기의 안전망(`claude::TURN_STALE_MS`)이 맡는다.
    #[test]
    fn a_status_less_session_is_never_aged_out() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        let ancient = now_ms - FRESH_MS_LOCAL * 30;
        fs::write(dir.path().join("7.json"), headless_json(7, ancient)).unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert_eq!(state.headless.len(), 1, "한 달 전에 켠 세션도 계속 감시한다");
        assert!(state.sessions.is_empty(), "그렇다고 작업 중이라고 말하지는 않는다");
    }

    /// 짝이 되는 쪽 — `status` 를 주는 세션은 여전히 밀린다. 그 값이 있으니 낡을 수
    /// 있고, 낡은 `busy` 를 믿으면 펫이 영원히 깜빡인다.
    #[test]
    fn a_session_with_a_status_still_ages_out() {
        let dir = tempfile::tempdir().unwrap();
        let now_ms = 1_000_000_000_000i64;
        fs::write(dir.path().join("1.json"), session_json(1, "busy", now_ms - FRESH_MS_LOCAL - 1))
            .unwrap();

        let state = read_live_state(&[dir.path().to_path_buf()], now_ms);
        assert!(state.sessions.is_empty());
        assert!(!state.busy);
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

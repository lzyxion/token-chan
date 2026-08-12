//! 라이브 세션 상태 — 캐릭터 애니메이션(작업 중/대기) 구동용.
//!
//! Claude 만 세션 레지스트리(`~/.claude/sessions/<pid>.json`)를 남겨 정확히 알 수 있다:
//! - `status == "busy"` 이고 `updatedAt` 이 최근이어야 busy 로 간주
//! - OS 경계를 넘은 세션(`\\wsl...`)은 시계가 다를 수 있어 신선도를 더 엄격하게 본다
//!
//! **프로세스 생존 확인은 하지 않는다.** `/proc/<pid>` 로 보던 코드가 있었는데 그건
//! Linux 에서만 성립하고, 배포되는 번들은 msi/dmg 뿐이라 **실제 사용자 빌드에는 아예
//! 컴파일되지 않는 코드**였다. 그런데도 [`FRESH_MS_LOCAL`] 의 근거로 인용되고 있어
//! "죽은 세션은 걸러진다" 는 잘못된 인상을 줬다. 세 OS 를 전부 지원하는 방식으로
//! 되살릴 수는 있으나(unix `kill(pid,0)`, Windows `OpenProcess`), 같은 필요가 있는
//! Codex 쪽 크래시 판정과 **함께 하나의 공통 층으로** 만드는 편이 낫다.
//!
//! Codex·Antigravity 는 그런 레지스트리가 없어서 **트랜스크립트 파일이 방금 쓰였는지**로
//! 유도한다([`add_inferred`]). 정확한 신호가 아니라 유도값이므로 상태 이름도
//! `busy` 가 아니라 `active` 로 구분해 둔다.
//!
//! **mtime 만 보면 안 된다.** Windows/NTFS 는 파일 핸들이 열려 있는 동안 last-write
//! 타임스탬프 갱신을 미루고 핸들이 닫힐 때 반영한다. Codex 는 세션 내내 rollout 핸들을
//! 열어 둔 채 append 하므로 **작업 중에는 mtime 이 아예 안 움직이고 세션이 끝나야 움직인다**
//! — 신호가 정확히 반대로 뒤집힌다. 실측(2026-08-12, Windows 11):
//!
//! ```text
//! 10:14:57  mtime=09:55:03  size=254494
//! 10:15:27  mtime=09:55:03  size=325952   ← 30초간 +71KB, mtime 은 1초도 안 움직임
//! ```
//!
//! 크기는 즉시 갱신되므로 **직전 관측과 비교해 크기나 mtime 이 달라졌는지**를 같이 본다
//! ([`WatchTracker`]). mtime 신선도도 그대로 남겨 둔다 — 파일이 닫힌 직후처럼 그쪽만
//! 잡히는 경우가 있어 둘의 합집합이 어느 하나보다 넓다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Source;

/// OS 경계를 넘은 세션(`\\wsl...`)의 updatedAt 신선도 한계 (ms).
/// 시계가 어긋날 수 있어 같은 OS 세션보다 엄격하게 본다.
pub const FRESH_MS: i64 = 10 * 60 * 1000;

/// 같은 OS 세션의 updatedAt 허용 한계 (ms).
///
/// `updatedAt` 은 상태 전환 시점에만 갱신되므로, 긴 턴(>10분) 중에도 busy 를 유지하려면
/// 느슨해야 한다. **그 대가로 비정상 종료한 세션이 이 시간만큼 작업 중으로 남아 보인다**
/// — 지금은 프로세스 생존 확인이 없어서 이게 유일한 방어다 (모듈 주석 참고).
/// 좁히면 긴 턴이 중간에 idle 로 튀므로, 생존 확인을 제대로 붙이기 전까지는 이 값이 맞다.
pub const FRESH_MS_LOCAL: i64 = 24 * 60 * 60 * 1000;

/// 파일에는 `pid` 도 있지만 읽지 않는다 — 생존 확인을 하지 않기 때문 (모듈 주석 참고).
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

/// 감시 파일의 직전 관측값. mtime 만으로는 작업 중을 못 잡아서(모듈 주석 참고) 크기 변화도
/// 봐야 하는데, 그건 한 번의 stat 으로 알 수 없고 **호출 사이에 값을 들고 있어야** 한다.
/// 라이브 스레드가 하나 만들어 매 회차 넘겨준다.
#[derive(Default)]
pub struct WatchTracker {
    seen: HashMap<PathBuf, Seen>,
}

struct Seen {
    size: u64,
    mtime: Option<SystemTime>,
    /// 크기·mtime 이 바뀐 것을 마지막으로 **관측한** 시각.
    /// 첫 관측은 비교 대상이 없어 `None` 이다 — 그때는 mtime 신선도만으로 판정한다.
    changed_at: Option<DateTime<Utc>>,
}

/// 세션 레지스트리가 없는 소스의 활동을 감시 파일로 판정해 상태에 얹는다.
/// `watch` 는 소스별 (감시 파일, 세션 이름) — 어댑터의 `watch_path()` 결과다.
pub fn add_inferred(
    state: &mut LiveState,
    watch: &[(Source, PathBuf, String)],
    now: DateTime<Utc>,
    tracker: &mut WatchTracker,
) {
    // 세션이 바뀌면 감시 파일도 바뀐다 — 안 보는 경로는 들고 있을 이유가 없다
    tracker.seen.retain(|p, _| watch.iter().any(|(_, w, _)| w == p));

    for (source, path, name) in watch {
        let meta = std::fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());

        let prev = tracker.seen.get(path);
        let mut changed_at = prev.and_then(|s| s.changed_at);
        if let Some(s) = prev {
            if s.size != size || s.mtime != mtime {
                changed_at = Some(now);
            }
        }
        tracker.seen.insert(path.clone(), Seen { size, mtime, changed_at });

        // 방금 자란 것을 봤거나(핸들이 열려 있어 mtime 이 멈춘 경우) mtime 이 신선하거나
        let grew = changed_at
            .map(|t| (now - t).num_milliseconds() < INFERRED_BUSY_MS)
            .unwrap_or(false);
        if !grew && !infer_busy(mtime.map(Into::into), now) {
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

            // OS 경계를 넘은 세션은 시계도 다를 수 있어 더 엄격하게 본다
            let limit = if windows_mount { FRESH_MS } else { FRESH_MS_LOCAL };
            let fresh = sf
                .updated_at
                .map(|t| now_ms.saturating_sub(t) < limit)
                .unwrap_or(false);
            if !fresh {
                continue;
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

    /// 신선도가 유일한 방어다 — 생존 확인이 없으므로 한계를 넘은 세션만 걸러진다.
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

    /// 한계 안이면 프로세스가 죽었어도 살아 있는 것으로 본다 — 지금의 한계를 못박아 둔다.
    /// 생존 확인을 붙이면 이 테스트가 깨져야 하고, 그때 함께 고치면 된다.
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
    fn inferred_busy_uses_write_freshness() {
        let now = chrono::Utc::now();
        assert!(infer_busy(Some(now - chrono::Duration::seconds(5)), now));
        assert!(infer_busy(Some(now - chrono::Duration::seconds(44)), now));
        assert!(!infer_busy(Some(now - chrono::Duration::seconds(60)), now));
        assert!(!infer_busy(None, now), "쓴 적이 없으면 작업 중이 아니다");
        // 시계가 어긋나 미래로 찍혀도 방금 쓰인 것으로 본다
        assert!(infer_busy(Some(now + chrono::Duration::seconds(10)), now));
    }

    /// mtime 을 고정한 채 파일에 덧쓴다 — Codex 가 핸들을 열어 둔 채 append 할 때
    /// Windows 가 보여주는 모습 그대로다 (크기만 자라고 mtime 은 그대로).
    fn append_keeping_mtime(path: &std::path::Path, bytes: &[u8]) {
        let frozen = fs::metadata(path).unwrap().modified().unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        use std::io::Write;
        f.write_all(bytes).unwrap();
        f.set_times(std::fs::FileTimes::new().set_modified(frozen)).unwrap();
        drop(f);
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), frozen, "mtime 고정 실패");
    }

    fn watch_of(path: &std::path::Path) -> Vec<(Source, PathBuf, String)> {
        vec![(Source::Codex, path.to_path_buf(), "codex".to_string())]
    }

    #[test]
    fn growth_counts_as_busy_even_when_mtime_never_moves() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        fs::write(&f, b"{}\n").unwrap();
        let watch = watch_of(&f);
        let mut tracker = WatchTracker::default();

        // mtime 이 이미 낡은 시점에서 본다 — 신선도만 보면 작업 중이 아니다
        let now = Utc::now() + chrono::Duration::seconds(600);
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch, now, &mut tracker);
        assert!(!state.busy, "첫 관측은 비교 대상이 없어 작업 중이 아니다");

        append_keeping_mtime(&f, b"{}\n");
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch, now, &mut tracker);
        assert!(state.busy, "mtime 이 멈춰 있어도 크기가 자랐으면 작업 중이다");
        assert_eq!(state.sessions[0].status, "active");
    }

    #[test]
    fn growth_goes_stale_after_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        fs::write(&f, b"{}\n").unwrap();
        let watch = watch_of(&f);
        let mut tracker = WatchTracker::default();

        let seen_at = Utc::now() + chrono::Duration::seconds(600);
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch, seen_at, &mut tracker);
        append_keeping_mtime(&f, b"{}\n");
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch, seen_at, &mut tracker);
        assert!(state.busy);

        // 더 자라지 않은 채 창(45초)이 지나면 작업 중이 아니다
        let later = seen_at + chrono::Duration::milliseconds(INFERRED_BUSY_MS + 1);
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch, later, &mut tracker);
        assert!(!state.busy, "마지막 변화 뒤 창이 지나면 풀린다");
    }

    #[test]
    fn fresh_mtime_alone_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        fs::write(&f, b"{}\n").unwrap();
        let mut tracker = WatchTracker::default();

        // 방금 쓴 파일 — 크기 변화를 본 적이 없어도 mtime 신선도로 잡힌다
        let mut state = LiveState::default();
        add_inferred(&mut state, &watch_of(&f), Utc::now(), &mut tracker);
        assert!(state.busy);
    }

    #[test]
    fn dropped_watch_paths_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        fs::write(&f, b"{}\n").unwrap();
        let mut tracker = WatchTracker::default();
        let now = Utc::now() + chrono::Duration::seconds(600);

        add_inferred(&mut LiveState::default(), &watch_of(&f), now, &mut tracker);
        assert_eq!(tracker.seen.len(), 1);
        // 세션이 바뀌어 그 파일을 더는 안 보면 관측값도 버린다
        add_inferred(&mut LiveState::default(), &[], now, &mut tracker);
        assert!(tracker.seen.is_empty());
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

//! Codex CLI 어댑터.
//!
//! ✅ **실데이터 검증됨** (2026-07-31, codex-tui 0.146.0 rollout): `turn_context`
//! top-level 이벤트의 `payload.model`, `event_msg`/`token_count` 의
//! `info.last_token_usage`(델타)·`total_token_usage`(누적) 구조 확인.
//! 실파일엔 `cache_write_input_tokens` 필드도 존재(관측값 0) → cache_write 로 전달.
//! 검증 방법: `cargo run -p usage-core --example scan_codex`
//!
//! 알려진 포맷:
//! - 세션 파일: `<홈>/sessions/YYYY/MM/DD/rollout-*.jsonl` + `archived_sessions/`
//!   (홈은 기본 `~/.codex` — 재배치된 홈은 마커 스캔이나 설정으로 들어온다)
//! - `type:"event_msg"` + `payload.type:"token_count"` 이벤트:
//!   `payload.info.total_token_usage` = 세션 **누적**, `payload.info.last_token_usage` = 요청 **델타**
//!   필드: input_tokens, cached_input_tokens, cache_write_input_tokens,
//!   output_tokens, reasoning_output_tokens, total_tokens
//! - 모델명은 `turn_context` 이벤트의 payload.model
//!
//! 집계 규칙: last_token_usage 델타 우선, 없으면 누적값 차분(음수는 0 클램프).
//! input_tokens 는 cached_input_tokens 를 포함하므로 순수 입력 = input - cached.
//! output_tokens 는 reasoning 포함(OpenAI 관례)으로 가정.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::context::{ContextState, RawContext};
use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};
use crate::plan::{window_label, PlanMeter, PlanUsage};
use crate::pricing::PriceTable;
use crate::session::{dir_label, first_line, is_human_prompt, SessionRow};

#[derive(Default, Clone, Copy, PartialEq)]
struct Counters {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
}

impl Counters {
    fn from_value(v: &Value) -> Self {
        let g = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Self {
            input: g("input_tokens"),
            cached: g("cached_input_tokens"),
            cache_write: g("cache_write_input_tokens"),
            output: g("output_tokens"),
            reasoning: g("reasoning_output_tokens"),
        }
    }
    fn is_zero(&self) -> bool {
        self.input == 0
            && self.cached == 0
            && self.cache_write == 0
            && self.output == 0
            && self.reasoning == 0
    }
    fn delta_from(&self, prev: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(prev.input),
            cached: self.cached.saturating_sub(prev.cached),
            cache_write: self.cache_write.saturating_sub(prev.cache_write),
            output: self.output.saturating_sub(prev.output),
            reasoning: self.reasoning.saturating_sub(prev.reasoning),
        }
    }
}

/// dedup 키와 함께 캐시되는 파싱 결과 (Claude 어댑터와 같은 방식).
/// 같은 rollout 이 여러 루트에 존재할 수 있어 — 예: 재배치된 홈과 `~/.codex` 를 함께 볼 때 —
/// 파일 단위가 아니라 **이벤트 단위**로 걸러야 사용량이 두 번 집계되지 않는다.
struct ParsedEvent {
    dedup_key: String,
    ev: UsageEvent,
}

struct FileCache {
    mtime: SystemTime,
    size: u64,
    events: Vec<ParsedEvent>,
    ctx: RawContext,
    /// 이 파일에서 본 가장 최근 `rate_limits` (있으면)
    limits: Option<(DateTime<Utc>, PlanUsage)>,
    /// 파일이 마지막으로 쓰인 시각 — Codex 에는 Claude 같은 세션 레지스트리가 없어
    /// 작업 중 판정을 이 신선도로 유도한다
    written_at: DateTime<Utc>,
    /// 최근 세션 목록용
    session: Option<SessionRow>,
}

pub struct CodexAdapter {
    homes: Vec<PathBuf>,
    cache: HashMap<PathBuf, FileCache>,
}

impl CodexAdapter {
    pub fn new(homes: Vec<PathBuf>) -> Self {
        Self { homes, cache: HashMap::new() }
    }

    pub fn with_default_roots() -> Self {
        Self::new(crate::roots::codex_homes())
    }

    pub fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        if self.homes.is_empty() {
            return ScanOutcome { events: vec![], status: SourceStatus::NoData };
        }

        let mut seen = std::collections::HashSet::new();
        let mut any_file = false;

        for home in &self.homes {
            for sub in ["sessions", "archived_sessions"] {
                let root = home.join(sub);
                if !root.is_dir() {
                    continue;
                }
                for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !entry.file_type().is_file()
                        || path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                    {
                        continue;
                    }
                    any_file = true;
                    let Ok(meta) = entry.metadata() else { continue };
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    let mtime_dt: DateTime<Utc> = mtime.into();
                    if mtime_dt < since {
                        continue;
                    }
                    let size = meta.len();
                    seen.insert(path.to_path_buf());
                    let needs = match self.cache.get(path) {
                        Some(c) => c.mtime != mtime || c.size != size,
                        None => true,
                    };
                    if needs {
                        let (events, ctx, limits, session) = parse_rollout(path);
                        self.cache.insert(
                            path.to_path_buf(),
                            FileCache { mtime, size, events, ctx, limits, written_at: mtime_dt, session },
                        );
                    } else if let Some(fc) = self.cache.get_mut(path) {
                        // 내용이 그대로여도 mtime 은 갱신될 수 있다 (동일 크기 재기록)
                        fc.written_at = mtime_dt;
                    }
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

        // 전역 dedup 후 병합 — 같은 rollout 이 두 루트에 있어도 한 번만 센다
        let mut dedup: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut events: Vec<UsageEvent> = vec![];
        for fc in self.cache.values() {
            for pe in &fc.events {
                if pe.ev.ts < since {
                    continue;
                }
                if dedup.insert(pe.dedup_key.as_str()) {
                    events.push(pe.ev.clone());
                }
            }
        }
        events.sort_by_key(|e| e.ts);

        let status = if any_file { SourceStatus::Ok } else { SourceStatus::NoData };
        ScanOutcome { events, status }
    }

    /// 지금 작업 중인 Codex 세션의 컨텍스트 창 사용량.
    /// Claude 와 달리 창 크기를 로그(`model_context_window`)가 직접 알려주므로 추론이 없다.
    pub fn context(&self, pricing: &PriceTable) -> Option<ContextState> {
        let best = self
            .cache
            .values()
            .map(|fc| &fc.ctx)
            .filter(|c| !c.is_empty())
            .max_by_key(|c| c.last_activity())?;
        Some(crate::context::resolve(
            Source::Codex,
            best,
            pricing.context_window(&best.model),
        ))
    }

    /// 서버가 알려준 공식 한도. Claude 와 달리 프로세스를 띄우지 않는다 —
    /// 이미 읽고 있는 rollout 안에 들어 있다.
    pub fn plan(&self) -> Option<PlanUsage> {
        self.cache
            .values()
            .filter_map(|fc| fc.limits.as_ref())
            .max_by_key(|(at, _)| *at)
            .map(|(_, p)| p.clone())
    }

    /// 최근 세션 목록
    pub fn sessions(&self) -> Vec<SessionRow> {
        self.cache.values().filter_map(|fc| fc.session.clone()).collect()
    }

    /// 스캔한 rollout 중 가장 최근에 쓰인 시각 — 작업 중 판정용
    pub fn last_activity(&self) -> Option<DateTime<Utc>> {
        self.cache.values().map(|fc| fc.written_at).max()
    }

}

// ─────────────────────────────────────────────────────────────────────────────
// 턴 추적 — 크기 변화 유도 대신 **파일에 적힌 턴 경계**를 읽는다
// ─────────────────────────────────────────────────────────────────────────────

/// 턴이 끝났다는 이벤트가 영영 오지 않을 때(크래시·강제 종료) 풀어 주는 안전망.
///
/// **넉넉해야 한다.** 긴 도구 실행(테스트·빌드) 중에는 rollout 이 몇 분간 조용할 수 있고,
/// 그때 풀어 버리면 작업 중인데 펫이 잠든다. 정상 종료는 `task_complete` 가 즉시 잡으므로
/// 이 값은 평상시 지연에 영향을 주지 않는다 — 오직 크래시 뒤 몇 분을 정한다.
/// (프로세스 생존 확인을 공통 층으로 붙이면 이 안전망을 훨씬 좁힐 수 있다.)
pub const TURN_STALE_MS: i64 = 5 * 60 * 1000;

/// `history.jsonl` 한 줄. 프롬프트 제출 시점에 append 된다 —
/// 실측에서 이 `ts` 가 rollout 의 `task_started` 시각과 일치했다.
#[derive(serde::Deserialize)]
struct HistoryEntry {
    session_id: String,
}

struct SessionTurn {
    /// 이 세션을 발견한 홈 — rollout 을 나중에 다시 찾을 때 쓴다
    home: PathBuf,
    /// 이 세션의 rollout. `history.jsonl` 의 `session_id` 가 파일명에 그대로 들어 있어
    /// 디렉토리를 훑지 않고도 찾을 수 있다 (한 번 찾으면 캐시).
    rollout: Option<PathBuf>,
    /// rollout 에서 읽은 지점 — 매 회차 새로 늘어난 부분만 본다
    offset: u64,
    running: bool,
    /// 마지막으로 뭔가 관측한 시각 (안전망 기준)
    last_activity: DateTime<Utc>,
}

/// [`TurnWatcher::poll`] 결과
pub struct TurnPoll {
    /// `history.jsonl` 을 하나라도 읽을 수 있었는지 — **진단용**이다.
    /// `false` 면 이 방식이 성립하지 않으므로(설정으로 이력을 껐거나 아직 없음) 그 홈의
    /// 세션은 작업 중으로 잡히지 않는다. 예전엔 여기서 크기 변화로 폴백했지만 그 신호로는
    /// 완료·크래시가 구분되지 않아 제거했다 (live.rs 모듈 주석).
    pub covered: bool,
    /// 지금 턴이 돌고 있는 세션 id 들
    pub running: Vec<String>,
    /// 이번 회차에 **`task_complete` 로** 끝난 세션 id 들.
    ///
    /// 안전망 타임아웃(`TURN_STALE_MS`)으로 풀린 것도, 사용자가 끊은 것(`turn_aborted`)도
    /// 여기 없다 — 크래시·취소와 완료를 가르는 유일한 신호다.
    pub completed: Vec<String>,
}

/// Codex 턴 추적.
///
/// **왜 이렇게 하는가** — 크기 변화만 보면 "방금 뭔가 쓰였다"까지만 알 수 있어서 종료를
/// 45초 창으로 때려 맞춰야 했고, 감시할 rollout 을 사용량 스캔(10초)이 찾아 줄 때까지
/// 기다려야 했으며, 한 번에 한 세션만 볼 수 있었다. 실측으로 다음이 확인됐다:
///
/// - `<홈>/history.jsonl` 은 **고정 경로**이고 프롬프트 제출 때 한 줄 append 된다
/// - 그 줄의 `session_id` 가 rollout 파일명의 uuid 와 **정확히 같다**
/// - rollout 에는 `task_started` / `task_complete` / `turn_aborted` 가 짝을 이뤄 들어 있다
///   (9파일 38턴 실측: 시작 38 = 완료 33 + 중단 5)
///
/// 그래서 고정 경로로 시작을 잡고 → 이름으로 rollout 을 특정하고 → 꼬리에서 종료를 읽는다.
/// 세션 id 별로 들고 있으므로 동시 세션도 각각 추적된다.
#[derive(Default)]
pub struct TurnWatcher {
    /// 홈별 `history.jsonl` 읽은 지점
    history: HashMap<PathBuf, u64>,
    sessions: HashMap<String, SessionTurn>,
}

impl TurnWatcher {
    pub fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll {
        let mut covered = false;

        for home in homes {
            let path = home.join("history.jsonl");
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            covered = true;

            let size = meta.len();
            let start = match self.history.get(home).copied() {
                // **첫 관측은 과거 이력을 되짚지 않는다.** 그러면 예전 세션이 전부
                // 방금 시작한 것처럼 보인다. 지금 끝을 기준점으로 삼는다.
                None => size,
                // 파일이 갈아엎히거나 잘렸으면 처음부터
                Some(o) if o > size => 0,
                Some(o) => o,
            };
            let (lines, consumed) = crate::live::read_from(&path, start);
            self.history.insert(home.clone(), start + consumed);

            for line in lines {
                let Ok(e) = serde_json::from_str::<HistoryEntry>(&line) else { continue };
                let entry = self.sessions.entry(e.session_id.clone()).or_insert(SessionTurn {
                    home: home.clone(),
                    rollout: None,
                    offset: 0,
                    running: false,
                    last_activity: now,
                });
                entry.running = true;
                entry.last_activity = now;
                if entry.rollout.is_none() {
                    // 이미 쌓인 rollout 은 **끝에서부터** 본다 — 과거 턴의 완료 이벤트를
                    // 지금 것으로 읽으면 시작하자마자 끝나 버린다.
                    // (프롬프트 직후라 아직 파일이 없을 수도 있다. 그건 아래에서 다시 찾는다.)
                    entry.rollout = find_rollout(home, &e.session_id);
                    entry.offset = entry
                        .rollout
                        .as_ref()
                        .and_then(|p| std::fs::metadata(p).ok())
                        .map(|m| m.len())
                        .unwrap_or(0);
                }
            }
        }

        // 돌고 있는 세션의 rollout 꼬리에서 종료 이벤트를 찾는다
        let mut completed = vec![];
        for (id, turn) in self.sessions.iter_mut() {
            if !turn.running {
                continue;
            }
            let mut finished_cleanly = false;
            if turn.rollout.is_none() {
                // 프롬프트 직후엔 파일이 아직 없을 수 있다. 이번에 찾았다면 갓 생긴
                // 파일이므로 처음부터 읽는다 (건너뛸 과거 턴이 없다).
                turn.rollout = find_rollout(&turn.home, id);
                turn.offset = 0;
            }
            if let Some(path) = turn.rollout.clone() {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if size < turn.offset {
                    turn.offset = 0;
                }
                let (lines, consumed) = crate::live::read_from(&path, turn.offset);
                turn.offset += consumed;
                if consumed > 0 {
                    turn.last_activity = now;
                }
                for line in lines {
                    match turn_boundary(&line) {
                        Some(Boundary::Started) => {
                            turn.running = true;
                            finished_cleanly = false;
                        }
                        // 돌고 있던 턴에 온 완료만 센다 — 중복·유실로 들어온 완료가
                        // 이미 끝난 턴을 한 번 더 알리면 안 된다
                        Some(Boundary::Completed) => {
                            finished_cleanly = turn.running;
                            turn.running = false;
                        }
                        Some(Boundary::Aborted) => {
                            turn.running = false;
                            finished_cleanly = false;
                        }
                        None => {}
                    }
                }
            }
            if finished_cleanly {
                completed.push(id.clone());
            }
            // 완료 이벤트가 영영 안 오는 경우(크래시)의 안전망.
            // **완료 판정 뒤에 와야 한다** — 앞에 두면 타임아웃이 완료로 샌다.
            if turn.running && (now - turn.last_activity).num_milliseconds() > TURN_STALE_MS {
                turn.running = false;
            }
        }

        // 오래 조용한 세션은 잊는다 (메모리 상한)
        self.sessions
            .retain(|_, t| t.running || (now - t.last_activity).num_hours() < 24);

        TurnPoll {
            covered,
            running: self
                .sessions
                .iter()
                .filter(|(_, t)| t.running)
                .map(|(id, _)| id.clone())
                .collect(),
            completed,
        }
    }
}

/// 턴 경계 이벤트의 종류. 종료가 두 갈래인 게 핵심이다 — 예전엔 둘을 `false` 하나로
/// 뭉쳤는데, 그러면 **내가 Ctrl-C 로 끊은 턴을 두고 "다 끝났어" 라고** 말하게 된다.
#[derive(Debug, PartialEq)]
enum Boundary {
    Started,
    /// 정상 완료 — 이것만 완료로 친다
    Completed,
    /// 사용자가 끊었다. 실측 109파일에서 `reason` 은 `interrupted` 한 종류뿐이었다
    Aborted,
}

/// 한 줄이 턴 경계면 그 종류. 아니면 `None`.
fn turn_boundary(line: &str) -> Option<Boundary> {
    // 전체 파싱은 낭비다 — 경계 이벤트는 드물고 줄은 크다(도구 출력 포함)
    if !line.contains("task_started") && !line.contains("task_complete") && !line.contains("turn_aborted")
    {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    match v.get("payload")?.get("type")?.as_str()? {
        "task_started" => Some(Boundary::Started),
        "task_complete" => Some(Boundary::Completed),
        "turn_aborted" => Some(Boundary::Aborted),
        _ => None,
    }
}

/// `session_id` 로 rollout 을 찾는다 — 파일명이 `rollout-<시각>-<session_id>.jsonl` 이다.
/// 세션당 한 번만 부르고 결과를 캐시한다.
fn find_rollout(home: &Path, session_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{session_id}.jsonl");
    for sub in ["sessions", "archived_sessions"] {
        let root = home.join(sub);
        if !root.is_dir() {
            continue;
        }
        for e in WalkDir::new(&root).max_depth(4).into_iter().filter_map(|e| e.ok()) {
            if e.file_type().is_file()
                && e.file_name().to_str().map(|n| n.ends_with(&suffix)).unwrap_or(false)
            {
                return Some(e.into_path());
            }
        }
    }
    None
}

/// `payload.rate_limits` → 공식 한도 미터.
/// 창이 짧은 것부터 나열해야 첫 미터가 "지금 당장 걸리는 한도"가 된다.
fn parse_rate_limits(v: &Value, at: DateTime<Utc>) -> Option<PlanUsage> {
    let mut meters: Vec<(u64, PlanMeter)> = vec![];
    for key in ["primary", "secondary"] {
        let Some(w) = v.get(key).filter(|w| w.is_object()) else { continue };
        let Some(pct) = w.get("used_percent").and_then(Value::as_f64) else { continue };
        let minutes = w.get("window_minutes").and_then(Value::as_u64).unwrap_or(0);
        let resets_at = w
            .get("resets_at")
            .and_then(Value::as_i64)
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0));
        meters.push((
            minutes,
            PlanMeter {
                label: window_label(minutes),
                used_pct: pct.round().clamp(0.0, 100.0) as u8,
                resets_at,
            },
        ));
    }
    if meters.is_empty() {
        return None;
    }
    meters.sort_by_key(|(m, _)| *m);

    // Claude 계정 카드의 플랜 이름과 같은 규칙으로 다듬는다 — 두 소스가 같은 화면에 뜬다
    let detail = v
        .get("plan_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(crate::plan::plan_label)
        .unwrap_or_default();

    Some(PlanUsage {
        source: Source::Codex,
        meters: meters.into_iter().map(|(_, m)| m).collect(),
        detail,
        fetched_at: at,
    })
}

type Rollout = (Vec<ParsedEvent>, RawContext, Option<(DateTime<Utc>, PlanUsage)>, Option<SessionRow>);

fn parse_rollout(path: &Path) -> Rollout {
    let mut ctx = RawContext::default();
    let mut limits: Option<(DateTime<Utc>, PlanUsage)> = None;
    // 작업 위치는 session_meta 에 있다 — 지금까지 읽고 버리던 값이다
    let mut cwd = String::new();
    // git 브랜치도 session_meta 에 있다 (`git.branch`). Claude 와 같은 값을 보여줄 수 있다.
    let mut branch = String::new();
    // 제목은 **첫 사용자 메시지**다. Codex 가 `state_5.sqlite` 의 `threads.title` 에도
    // 같은 값을 넣어 두지만(실측 일치), 그 DB 는 파일명에 스키마 버전이 박혀 있고
    // (`state_5` → 언젠가 `state_6`) 마이그레이션을 돌린다. 같은 값을 이미 읽는 파일에서
    // 얻을 수 있으니 새 스키마 의존을 만들지 않는다.
    let mut title = String::new();
    let (mut last_at, mut tokens) = (None, 0u64);
    // 세션 id 는 dedup 키의 뿌리다. session_meta 가 1행이라 token_count 보다 항상 먼저 나오지만,
    // 없더라도 파일명에 uuid 가 들어 있어 같은 rollout 의 복사본끼리는 같은 값이 된다.
    let mut session = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let Ok(content) = std::fs::read_to_string(path) else { return (vec![], ctx, limits, None) };
    let mut out = vec![];
    let mut prev_total = Counters::default();
    let mut model: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        let p_ty = payload.get("type").and_then(Value::as_str).unwrap_or("");

        // 모델 추적: turn_context (top-level 또는 event_msg 내부 어느 쪽이든)
        if ty == "turn_context" || p_ty == "turn_context" {
            if let Some(m) = payload.get("model").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
            continue;
        }

        if ty == "session_meta" || p_ty == "session_meta" {
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                session = id.to_string();
            }
            if let Some(c) = payload.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                cwd = c.to_string();
            }
            if let Some(b) = payload
                .get("git")
                .and_then(|g| g.get("branch"))
                .and_then(Value::as_str)
                .filter(|b| !b.is_empty())
            {
                branch = b.to_string();
            }
            continue;
        }

        // 제목 = 첫 **사람** 메시지. 뒤엣것은 이어지는 대화라 제목이 아니다.
        // 다른 도구가 Claude 대화를 그대로 입력으로 넣은 세션이 실측돼서(`Codex Desktop`
        // originator) 여기에도 `<command-name>…` 같은 래퍼가 들어온다 — 걸러야 한다.
        if p_ty == "user_message" && title.is_empty() {
            if let Some(m) =
                payload.get("message").and_then(Value::as_str).filter(|m| is_human_prompt(m))
            {
                title = first_line(m);
            }
            continue;
        }

        if p_ty != "token_count" {
            continue;
        }
        let Some(ts) = v
            .get("timestamp")
            .or_else(|| payload.get("timestamp"))
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            continue;
        };

        // 공식 한도는 `info` 가 아니라 payload 바로 아래에 형제로 붙는다.
        // 매 token_count 마다 실려 오므로 가장 나중 것이 최신이다.
        if let Some(p) = payload.get("rate_limits").and_then(|r| parse_rate_limits(r, ts)) {
            if limits.as_ref().map(|(prev, _)| ts >= *prev).unwrap_or(true) {
                limits = Some((ts, p));
            }
        }

        let info = payload.get("info").filter(|i| i.is_object()).unwrap_or(&payload);

        let last = info.get("last_token_usage").filter(|l| l.is_object());
        let total = info.get("total_token_usage").filter(|t| t.is_object());

        // ── 컨텍스트 ──
        // Codex 는 대화 전체를 매 요청에 다시 보내므로 요청 1건의 total_tokens
        // (= input + output, input 은 cached 포함) 이 곧 그 시점의 컨텍스트다.
        // `total_token_usage` 는 세션 **누적**이라 컨텍스트가 아니다 — 절대 쓰면 안 된다.
        // 창 크기는 `model_context_window` 가 직접 알려준다. 이 값은 모델의 창 전체가 아니라
        // 예비분을 뺀 실효 창이다 (models_cache.json 의 272,000 × 95% = 258,400 로 확인).
        if let Some(w) = info.get("model_context_window").and_then(Value::as_u64) {
            ctx.window = Some(w);
        }
        // last_token_usage 가 없는 형식에서는 요청 단위 값을 알 수 없어 컨텍스트를 건너뛴다
        // (누적값으로는 복원되지 않는다). 사용량 집계는 아래에서 그대로 진행된다.
        if let Some(cur) = last.and_then(|l| l.get("total_tokens")).and_then(Value::as_u64) {
            ctx.peak = ctx.peak.max(cur);
            if ctx.at.map(|prev| ts >= prev).unwrap_or(true) {
                ctx.tokens = cur;
                ctx.at = Some(ts);
                ctx.model = model.clone().unwrap_or_default();
            }
        }

        let delta = match (last, total) {
            (Some(l), t) => {
                // 요청 델타가 직접 제공됨. 누적값도 있으면 prev 동기화.
                if let Some(t) = t {
                    prev_total = Counters::from_value(t);
                }
                Counters::from_value(l)
            }
            (None, Some(t)) => {
                let cur = Counters::from_value(t);
                let d = cur.delta_from(&prev_total);
                prev_total = cur;
                d
            }
            (None, None) => {
                // 구버전: payload/info 에 카운터가 직접 있는 경우 → 누적으로 간주
                let cur = Counters::from_value(info);
                if cur.is_zero() {
                    continue;
                }
                let d = cur.delta_from(&prev_total);
                prev_total = cur;
                d
            }
        };

        if delta.is_zero() {
            continue;
        }
        // output_tokens 가 0인데 reasoning 만 있는 비정상 케이스 방어
        let output = if delta.output == 0 { delta.reasoning } else { delta.output };
        let input = delta.input.saturating_sub(delta.cached);
        // Codex 에는 Claude 의 message.id 같은 전역 id 가 없다. 같은 세션·같은 시각·같은
        // 사용량이면 같은 요청으로 본다 — 복사본은 반드시 걸리고, 서로 다른 요청이 이 셋을
        // 모두 공유할 일은 사실상 없다.
        let dedup_key = format!(
            "{session}|{}|{input}|{output}|{}|{}",
            ts.timestamp_millis(),
            delta.cache_write,
            delta.cached
        );
        if last_at.map(|prev| ts >= prev).unwrap_or(true) {
            last_at = Some(ts);
        }
        tokens += input + output + delta.cache_write + delta.cached;

        out.push(ParsedEvent {
            dedup_key,
            ev: UsageEvent {
                source: Source::Codex,
                model: model.clone().unwrap_or_else(|| "codex-unknown".into()),
                ts,
                input,
                output,
                // 실데이터에서 관측값은 아직 0 — input_tokens 포함 여부가 미확인이라 그대로 전달만
                cache_write: delta.cache_write,
                cache_read: delta.cached,
                sidechain: false,
            },
        });
    }

    ctx.session = session;
    let row = last_at.map(|at| SessionRow {
        source: Source::Codex,
        // 제목이 있으면 그걸 쓴다 — 폴더명보다 "무엇을 했는지" 를 알려준다 (agy 와 같은 규칙).
        // 첫 사용자 메시지가 없거나(도구로만 돈 세션) 비어 있으면 폴더명으로 떨어진다.
        label: match (title.is_empty(), cwd.is_empty()) {
            (false, _) => title,
            (true, false) => dir_label(&cwd),
            (true, true) => ctx.session.chars().take(8).collect(),
        },
        id: ctx.session.clone(),
        cwd,
        model: model.unwrap_or_default(),
        branch,
        at,
        tokens,
    });
    (out, ctx, limits, row)
}

#[cfg(test)]
mod turn_tests {
    use super::*;
    use std::fs;

    const SID: &str = "019ff450-7640-7c82-91a1-707b3469216b";

    fn ev(kind: &str) -> String {
        format!(r#"{{"timestamp":"2026-08-12T04:54:24.000Z","type":"event_msg","payload":{{"type":"{kind}"}}}}"#)
    }

    /// `<홈>/history.jsonl` 과 `<홈>/sessions/2026/08/12/rollout-…-<sid>.jsonl` 을 만든다
    fn home_with_session(rollout_lines: &[String]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions").join("2026").join("08").join("12");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join(format!("rollout-2026-08-12T13-52-21-{SID}.jsonl")), rollout_lines.join("\n") + "\n")
            .unwrap();
        fs::write(dir.path().join("history.jsonl"), "").unwrap();
        dir
    }

    fn submit_prompt(home: &Path, sid: &str) {
        let mut s = fs::read_to_string(home.join("history.jsonl")).unwrap_or_default();
        s.push_str(&format!(r#"{{"session_id":"{sid}","ts":1786000000,"text":"hi"}}"#));
        s.push('\n');
        fs::write(home.join("history.jsonl"), s).unwrap();
    }

    fn append(path: &Path, line: &str) {
        let mut s = fs::read_to_string(path).unwrap();
        s.push_str(line);
        s.push('\n');
        fs::write(path, s).unwrap();
    }

    fn rollout_of(home: &Path) -> PathBuf {
        find_rollout(home, SID).expect("rollout")
    }

    #[test]
    fn history_line_starts_a_turn() {
        let home = home_with_session(&[ev("task_started"), ev("task_complete")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];

        // 첫 관측은 기준점만 잡는다 — 과거 이력을 되짚으면 예전 세션이 전부 살아난다
        let first = w.poll(&homes, now);
        assert!(first.covered);
        assert!(first.running.is_empty(), "첫 관측은 과거 이력을 재생하지 않는다");

        submit_prompt(home.path(), SID);
        let second = w.poll(&homes, now);
        assert_eq!(second.running, vec![SID.to_string()]);
    }

    #[test]
    fn task_complete_ends_the_turn_immediately() {
        let home = home_with_session(&[ev("task_started"), ev("task_complete")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        assert!(!w.poll(&homes, now).running.is_empty());

        // 새 턴이 끝났다 — 45초 창을 기다리지 않고 바로 풀린다
        append(&rollout_of(home.path()), &ev("task_complete"));
        let p = w.poll(&homes, now);
        assert!(p.running.is_empty());
        assert_eq!(p.completed, vec![SID.to_string()], "정상 완료는 completed 로 보고된다");
    }

    /// **크래시를 완료로 보고하면 안 된다.** 안전망 타임아웃도 `running` 에서 빠지는 건
    /// 같지만, 그건 완료 신호를 못 받았다는 뜻이지 끝났다는 뜻이 아니다.
    #[test]
    fn stale_timeout_is_not_reported_as_completed() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        assert!(!w.poll(&homes, now).running.is_empty());

        // 완료 이벤트가 영영 안 온다 (크래시)
        let later = now + chrono::Duration::milliseconds(TURN_STALE_MS + 1);
        let p = w.poll(&homes, later);
        assert!(p.running.is_empty(), "안전망이 풀어 준다");
        assert!(p.completed.is_empty(), "그러나 완료는 아니다");
    }

    /// 내가 Ctrl-C 로 끊은 턴을 두고 "다 끝났어" 라고 할 이유가 없다.
    /// 실측 109파일에서 `turn_aborted` 의 사유는 `interrupted` 한 종류뿐이었다.
    #[test]
    fn aborted_turn_is_not_reported_as_completed() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        assert!(!w.poll(&homes, now).running.is_empty());

        append(&rollout_of(home.path()), &ev("turn_aborted"));
        let p = w.poll(&homes, now);
        assert!(p.running.is_empty(), "턴은 끝난다");
        assert!(p.completed.is_empty(), "그러나 완료는 아니다");
    }

    #[test]
    fn turn_aborted_also_ends_the_turn() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        w.poll(&homes, now);

        append(&rollout_of(home.path()), &ev("turn_aborted"));
        assert!(w.poll(&homes, now).running.is_empty(), "Ctrl-C 로 끊어도 풀려야 한다");
    }

    #[test]
    fn past_turns_in_the_rollout_do_not_end_the_new_one() {
        // 이미 완료된 턴이 파일에 있는 상태에서 새 프롬프트가 들어온 경우
        let home = home_with_session(&[ev("task_started"), ev("task_complete")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);

        let p = w.poll(&homes, now);
        assert_eq!(p.running, vec![SID.to_string()], "과거 턴의 완료 이벤트를 읽어 바로 끝내면 안 된다");
    }

    #[test]
    fn crashed_turn_is_released_by_the_safety_net() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        assert!(!w.poll(&homes, now).running.is_empty());

        // 완료 이벤트가 영영 안 온다 (크래시)
        let later = now + chrono::Duration::milliseconds(TURN_STALE_MS + 1);
        assert!(w.poll(&homes, later).running.is_empty());
    }

    #[test]
    fn two_sessions_are_tracked_independently() {
        let other = "019ff361-987f-7d32-940b-7ab6a69ed686";
        let home = home_with_session(&[ev("task_started")]);
        let day = home.path().join("sessions").join("2026").join("08").join("12");
        fs::write(day.join(format!("rollout-2026-08-12T09-31-27-{other}.jsonl")), format!("{}\n", ev("task_started")))
            .unwrap();

        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        submit_prompt(home.path(), other);
        let p = w.poll(&homes, now);
        assert_eq!(p.running.len(), 2, "동시 세션은 각각 추적된다");

        // 한쪽만 끝나도 나머지는 그대로
        append(&find_rollout(home.path(), other).unwrap(), &ev("task_complete"));
        let p = w.poll(&homes, now);
        assert_eq!(p.running, vec![SID.to_string()]);
    }

    #[test]
    fn no_history_file_is_not_covered() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = TurnWatcher::default();
        let p = w.poll(&[dir.path().to_path_buf()], Utc::now());
        assert!(!p.covered, "이력이 없으면 호출자가 폴백해야 한다");
        assert!(p.running.is_empty());
    }

    #[test]
    fn half_written_line_is_not_consumed() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        submit_prompt(home.path(), SID);
        w.poll(&homes, now);

        // 아직 개행이 안 붙은 줄 — 다음 회차에 온전해지면 그때 읽혀야 한다
        let roll = rollout_of(home.path());
        let mut s = fs::read_to_string(&roll).unwrap();
        s.push_str(&ev("task_complete")[..20]);
        fs::write(&roll, s).unwrap();
        assert!(!w.poll(&homes, now).running.is_empty(), "잘린 줄로 판정하면 안 된다");

        let mut s = fs::read_to_string(&roll).unwrap();
        s.truncate(s.len() - 20);
        s.push_str(&ev("task_complete"));
        s.push('\n');
        fs::write(&roll, s).unwrap();
        assert!(w.poll(&homes, now).running.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_delta_and_cumulative_token_counts() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/07/30");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            // 세션 메타 (무시됨)
            r#"{"timestamp":"2026-07-30T01:00:00.000Z","type":"session_meta","payload":{"id":"s1","cwd":"/x"}}"#.to_string(),
            // 모델 컨텍스트
            r#"{"timestamp":"2026-07-30T01:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5-codex","cwd":"/x"}}"#.to_string(),
            // 1) last_token_usage 제공 (델타 직접 사용)
            r#"{"timestamp":"2026-07-30T01:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":600,"cache_write_input_tokens":40,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1200},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":600,"cache_write_input_tokens":40,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1200},"model_context_window":272000}}}"#.to_string(),
            // 2) last 없음 → 누적 차분 (input 2500-1000=1500, cached 1500-600=900, output 500-200=300)
            r#"{"timestamp":"2026-07-30T01:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2500,"cached_input_tokens":1500,"output_tokens":500,"reasoning_output_tokens":120,"total_tokens":3000},"last_token_usage":null}}}"#.to_string(),
        ];
        fs::write(day.join("rollout-2026-07-30-abc.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());

        assert_eq!(out.status, SourceStatus::Ok);
        assert_eq!(out.events.len(), 2);

        let e1 = &out.events[0];
        assert_eq!(e1.model, "gpt-5-codex");
        assert_eq!(e1.input, 400); // 1000 - 600(cached)
        assert_eq!(e1.cache_read, 600);
        assert_eq!(e1.cache_write, 40);
        assert_eq!(e1.output, 200);

        let e2 = &out.events[1];
        assert_eq!(e2.input, 600); // (2500-1000) - (1500-600)
        assert_eq!(e2.cache_read, 900);
        assert_eq!(e2.output, 300);
    }

    /// 실파일(rollout-2026-07-31, codex 0.146.0) 에서 관측한 형태 그대로.
    /// `model_context_window` 258,400 = models_cache.json 의 272,000 × 95%.
    #[test]
    fn context_uses_last_turn_and_reported_window() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/07/31");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-07-31T02:00:00.000Z","type":"session_meta","payload":{"id":"019fb5e6","cwd":"/x"}}"#.to_string(),
            r#"{"timestamp":"2026-07-31T02:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.6-terra","cwd":"/x"}}"#.to_string(),
            r#"{"timestamp":"2026-07-31T02:03:12.104Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"total_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"cache_write_input_tokens":0,"output_tokens":12,"reasoning_output_tokens":0,"total_tokens":13261},"last_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"cache_write_input_tokens":0,"output_tokens":12,"reasoning_output_tokens":0,"total_tokens":13261}}}}"#.to_string(),
            // 두 번째 턴: 누적은 커지지만 컨텍스트는 요청 단위 값이어야 한다
            r#"{"timestamp":"2026-07-31T02:05:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"total_token_usage":{"input_tokens":33249,"cached_input_tokens":19984,"output_tokens":112,"reasoning_output_tokens":0,"total_tokens":33361},"last_token_usage":{"input_tokens":20000,"cached_input_tokens":10000,"output_tokens":100,"reasoning_output_tokens":0,"total_tokens":20100}}}}"#.to_string(),
        ];
        fs::write(day.join("rollout-x.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let c = adapter.context(&crate::pricing::PriceTable::builtin()).unwrap();

        assert_eq!(c.source, Source::Codex);
        assert_eq!(c.session, "019fb5e6");
        assert_eq!(c.model, "gpt-5.6-terra");
        // 누적(33,361)이 아니라 마지막 요청의 total_tokens 여야 한다
        assert_eq!(c.tokens, 20_100);
        assert_eq!(c.window, 258_400, "로그가 알려준 실효 창을 그대로 써야 함");
        assert!(!c.window_inferred);
        assert!(!c.interim);
    }

    #[test]
    fn context_absent_without_last_token_usage() {
        // 요청 단위 값이 없는 형식에서는 컨텍스트를 만들어내지 않는다 (사용량 집계는 계속)
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/07/30");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-07-30T01:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}"#,
            r#"{"timestamp":"2026-07-30T01:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":200,"total_tokens":1200},"last_token_usage":null}}}"#,
        ];
        fs::write(day.join("rollout-y.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 1, "사용량 집계는 그대로 되어야 함");
        assert!(adapter.context(&crate::pricing::PriceTable::builtin()).is_none());
    }

    /// 같은 rollout 이 두 홈에 있을 때 (예: 재배치된 홈 + `~/.codex`) 한 번만 세어야 한다.
    /// 실측으로 이 상황이 있었고, dedup 전에는 이벤트가 4건 대신 5건으로 잡혔다.
    #[test]
    fn same_rollout_in_two_homes_counted_once() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let line = r#"{"timestamp":"2026-07-31T02:03:12.104Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"total_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"output_tokens":12,"total_tokens":13261},"last_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"output_tokens":12,"total_tokens":13261}}}}"#;
        let meta = r#"{"timestamp":"2026-07-31T02:00:00.000Z","type":"session_meta","payload":{"id":"019fb5e6"}}"#;
        for root in [a.path(), b.path()] {
            let day = root.join("sessions/2026/07/31");
            fs::create_dir_all(&day).unwrap();
            fs::write(day.join("rollout-same.jsonl"), format!("{meta}\n{line}")).unwrap();
        }

        let mut adapter = CodexAdapter::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 1, "같은 rollout 이 두 홈에 있으면 중복 집계된다");

        // 서로 다른 요청은 그대로 남아야 한다 (dedup 이 과하게 먹으면 안 됨)
        let day = b.path().join("sessions/2026/07/31");
        let other = line.replace("02:03:12", "02:09:12");
        fs::write(day.join("rollout-same.jsonl"), format!("{meta}\n{line}\n{other}")).unwrap();
        let mut adapter = CodexAdapter::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
        assert_eq!(adapter.scan(DateTime::UNIX_EPOCH.into()).events.len(), 2);
    }

    #[test]
    fn no_home_reports_no_data() {
        let mut adapter = CodexAdapter::new(vec![]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.status, SourceStatus::NoData);
    }

    /// 실파일(rollout-2026-08-11, free 플랜)에서 관측한 `rate_limits` 그대로.
    /// 43,200분 = 30일 창이고 free 플랜에선 secondary 가 null 이다.
    #[test]
    fn official_limits_come_from_the_rollout_itself() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/11");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-08-11T01:00:00.000Z","type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#,
            // 이전 값 — 나중 이벤트가 이겨야 한다
            r#"{"timestamp":"2026-08-11T01:20:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":3.0,"window_minutes":43200,"resets_at":1789004769},"secondary":null,"plan_type":"free"}}}"#,
            r#"{"timestamp":"2026-08-11T01:46:50.803Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":26930,"cached_input_tokens":25344,"output_tokens":112,"total_tokens":27042},"model_context_window":258400},"rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":43200,"resets_at":1789004769},"secondary":null,"credits":{"has_credits":false,"unlimited":false,"balance":null},"plan_type":"free"}}}"#,
        ];
        fs::write(day.join("rollout-limits.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let p = adapter.plan().unwrap();

        assert_eq!(p.source, Source::Codex);
        assert_eq!(p.detail, "Free", "Claude 계정 카드와 같은 표기 규칙");
        assert_eq!(p.meters.len(), 1, "free 플랜은 secondary 가 null");
        assert_eq!(p.meters[0].label, "월간");
        assert_eq!(p.meters[0].used_pct, 4, "마지막 이벤트 값이어야 함");
        // 문자열 파싱 없이 정확한 시각이 온다 (Claude 와의 차이)
        assert_eq!(p.meters[0].resets_at.unwrap().timestamp(), 1_789_004_769);
        assert_eq!(adapter.last_activity().is_some(), true);
    }

    /// **합성 픽스처다** — 두 창이 오는 플랜은 아직 관측하지 못했다 (free 30일 단일,
    /// plus 7일 단일, 둘 다 `secondary` 는 null). 그래도 정렬 규칙 자체는 지켜야 한다:
    /// 두 창이 오면 짧은 쪽이 먼저여야 첫 미터가 "지금 당장 걸리는 한도"가 된다.
    #[test]
    fn two_windows_are_ordered_shortest_first() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/11");
        fs::create_dir_all(&day).unwrap();
        let line = r#"{"timestamp":"2026-08-11T01:46:50.803Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}},"rate_limits":{"primary":{"used_percent":12.4,"window_minutes":10080,"resets_at":1789004769},"secondary":{"used_percent":61.0,"window_minutes":300,"resets_at":1789000000},"plan_type":"pro"}}}"#;
        fs::write(day.join("rollout-two.jsonl"), line).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let p = adapter.plan().unwrap();

        assert_eq!(p.meters.len(), 2);
        assert_eq!(p.meters[0].label, "5시간");
        assert_eq!(p.meters[0].used_pct, 61);
        assert_eq!(p.meters[1].label, "주간");
        assert_eq!(p.meters[1].used_pct, 12, "12.4 → 반올림 12");
        assert_eq!(p.session_pct(), Some(61), "가장 짧은 창이 세션 게이지");
    }

    /// 세션 목록 라벨은 폴더명이 아니라 **첫 사용자 메시지**여야 한다 (agy 와 같은 규칙).
    /// 브랜치도 `session_meta.git.branch` 에 있다 — "Codex 는 브랜치를 안 남긴다" 는
    /// 예전 주석은 틀렸다.
    #[test]
    fn session_row_uses_first_message_and_git_branch() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/12");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4c7","cwd":"C:\\Users\\u\\projects\\token-chan","git":{"branch":"main","commit_hash":"d6728af"}}}"#,
            // 실측 그대로 — 첫 줄 뒤에 붙여넣은 JSON 이 이어진다
            r#"{"timestamp":"2026-08-12T01:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"상태 정보를 가져오는데\n{\n  \"a\": 1\n}"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:02.000Z","type":"event_msg","payload":{"type":"user_message","message":"두 번째 메시지는 제목이 아니다"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#,
        ];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4c7.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let rows = adapter.sessions();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "상태 정보를 가져오는데", "첫 줄만, 첫 메시지만");
        assert_eq!(rows[0].branch, "main");
    }

    /// 다른 도구가 Claude 대화를 그대로 입력으로 넣은 세션이 실측됐다
    /// (`originator: "Codex Desktop"`, 50건). 그 첫 메시지는 Claude Code 의 명령 래퍼라
    /// 제목이 되면 안 된다 — 걸러서 폴더명으로 떨어져야 한다.
    #[test]
    fn session_row_skips_injected_wrappers() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/12");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4e0","cwd":"C:\\Users\\u\\projects\\token-chan\\src-tauri"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"<command-name>/usage</command-name>\n<command-message>usage</command-message>"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#,
        ];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4e0.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(adapter.sessions()[0].label, "src-tauri", "래퍼는 제목이 아니다");
    }

    /// 첫 사용자 메시지가 없으면(도구로만 돈 세션) 예전처럼 폴더명으로 떨어진다
    #[test]
    fn session_row_falls_back_to_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/12");
        fs::create_dir_all(&day).unwrap();
        let lines = vec![
            r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4c8","cwd":"/home/u/projects/api"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#,
        ];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4c8.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let rows = adapter.sessions();

        assert_eq!(rows[0].label, "api");
        assert_eq!(rows[0].branch, "", "git 정보가 없으면 빈 값");
    }

    #[test]
    fn rollout_without_limits_has_no_plan() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/07/30");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout-nolimits.jsonl"),
            r#"{"timestamp":"2026-07-30T01:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#,
        )
        .unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        assert!(adapter.plan().is_none());
    }
}

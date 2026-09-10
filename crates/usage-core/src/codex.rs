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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::context::{ContextState, RawContext};
use crate::model::{
    source_status, ParseDiagnostics, ScanOutcome, Source, SourceStatus, UsageEvent,
};
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
    diagnostics: ParseDiagnostics,
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
        let mut diagnostics = ParseDiagnostics::default();

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
                    let Ok(meta) = entry.metadata() else {
                        diagnostics.files_failed += 1;
                        continue;
                    };
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
                        let (events, ctx, limits, session, diagnostics) = parse_rollout(path);
                        self.cache.insert(
                            path.to_path_buf(),
                            FileCache {
                                mtime,
                                size,
                                events,
                                ctx,
                                limits,
                                written_at: mtime_dt,
                                session,
                                diagnostics,
                            },
                        );
                    } else if let Some(fc) = self.cache.get_mut(path) {
                        // 내용이 그대로여도 mtime 은 갱신될 수 있다 (동일 크기 재기록)
                        fc.written_at = mtime_dt;
                    }
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));
        for fc in self.cache.values() {
            diagnostics.add(fc.diagnostics);
        }

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

        let status = source_status(any_file, diagnostics);
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

/// 계약 가입 ([`crate::adapter`]) — 인헌트 메서드에 위임만 한다.
/// Codex 는 스캔한 파일(rollout)에 공식 한도가 실려 오는 유일한 소스라 `plan` 을 구현한다.
impl crate::adapter::SourceAdapter for CodexAdapter {
    fn source(&self) -> Source {
        Source::Codex
    }
    fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        CodexAdapter::scan(self, since)
    }
    fn context(&self, pricing: &PriceTable) -> Option<ContextState> {
        CodexAdapter::context(self, pricing)
    }
    fn sessions(&self) -> Vec<SessionRow> {
        CodexAdapter::sessions(self)
    }
    fn plan(&self) -> Option<PlanUsage> {
        CodexAdapter::plan(self)
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

struct SessionTurn {
    /// 이 세션의 rollout. 파일명이 곧 `session_id` 라 발견 시점에 이미 알고 있다.
    path: PathBuf,
    /// rollout 에서 읽은 지점 — 매 회차 새로 늘어난 부분만 본다
    offset: u64,
    running: bool,
    /// 마지막으로 뭔가 관측한 시각 (안전망 기준)
    last_activity: DateTime<Utc>,
}

/// [`TurnWatcher::poll`] 결과 — agy 와 같은 모양이라 계약 모듈이 정의를 갖는다.
/// 여기서 `covered` 는 "`sessions/` 를 열 수 있었는가", `completed` 는
/// "`task_complete` 로 끝났는가"다 (`turn_aborted` 와 안전망 해제는 포함되지 않는다).
pub use crate::adapter::TurnPoll;

/// Codex 턴 추적.
///
/// **감시 대상은 rollout 파일 목록이 정한다.** 예전에는 `<홈>/history.jsonl` 에 줄이
/// 붙는 것을 턴 시작으로 삼고 그 `session_id` 로 rollout 을 찾아갔는데, 그 파일은
/// **TUI 입력창의 프롬프트 이력**이라 터미널로 친 프롬프트만 남는다. 실측(2026-08-25,
/// 같은 홈의 하루치 rollout 7개):
///
/// ```text
/// originator    source    history.jsonl 등재
/// codex-tui     cli       2/2  O
/// claudian      vscode    0/3  X   (Obsidian 플러그인)
/// codex_exec    exec      0/2  X
/// ```
///
/// 그래서 TUI 밖에서 띄운 세션은 **영영 감시 목록에 들어가지 못했다.** 사용량 집계는
/// rollout 을 직접 훑어 정상이었으므로, "사용량엔 잡히는데 작업 중엔 안 잡힌다"는
/// 비대칭만 남았다 (Claude 가 겪던 것과 같은 꼴 — [`crate::live`] 모듈 주석).
///
/// 지금은 `history.jsonl` 을 아예 보지 않는다. **중복이었기 때문이다** — 프롬프트 제출은
/// rollout 의 `task_started` 로 이미 적히고(실측에서 두 시각이 일치했다), 그쪽은 진입점을
/// 가리지 않는다. 색인을 버리고 날짜 폴더를 직접 훑는다.
///
/// Claude 와 달리 **버리는** 쪽인 이유: 저쪽 레지스트리는 `status`(판정 자체)와
/// "지금 살아 있음"을 들고 있어 대체 불가였지만, 이쪽 색인에는 그런 정보가 없다.
#[derive(Default)]
pub struct TurnWatcher {
    /// `session_id` → 상태. **키가 세션 id 인 게 중요하다** — 홈이 둘 이상이고 서로
    /// 하드링크 미러면(실측: Orca 가 주입한 `CODEX_HOME` 과 `~/.codex`) 같은 rollout 이
    /// 두 경로로 잡힌다. 경로를 키로 쓰면 한 세션이 둘로 세어져 busy 가 두 배가 되고
    /// 완료가 두 번 나간다.
    sessions: HashMap<String, SessionTurn>,
    /// 첫 회차를 돌았는가. 첫 회차에 **이미 있던** rollout 은 과거이므로 끝에서 시작하고,
    /// 그 뒤에 나타난 파일은 방금 시작한 세션이므로 처음부터 읽는다 — 안 그러면 새 세션의
    /// `task_started` 를 놓쳐 첫 턴이 통째로 안 잡힌다.
    seeded: bool,
}

/// 계약 가입 ([`crate::adapter::TurnWatch`]) — 인헌트 `poll` 에 위임만 한다.
impl crate::adapter::TurnWatch for TurnWatcher {
    fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll {
        TurnWatcher::poll(self, homes, now)
    }
}

impl TurnWatcher {
    pub fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll {
        let (covered, found) = discover(homes, now);
        let seeded = self.seeded;
        let mut completed = vec![];
        let mut seen = HashSet::new();

        for (id, path) in found {
            seen.insert(id.clone());
            let entry = self.sessions.entry(id.clone()).or_insert_with(|| SessionTurn {
                offset: if seeded {
                    // 회차 도중에 나타난 파일 = 방금 만들어진 세션. 처음부터 읽는다
                    0
                } else {
                    std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
                },
                path,
                running: false,
                last_activity: now,
            });

            let size = std::fs::metadata(&entry.path).map(|m| m.len()).unwrap_or(0);
            if size < entry.offset {
                entry.offset = 0;
            }
            // 자란 게 없으면 열지도 않는다 — 하루치 rollout 이 수십 개가 되면
            // 2초마다 그만큼 파일을 여는 셈이 된다
            if size > entry.offset {
                let (lines, consumed) = crate::live::read_from(&entry.path, entry.offset);
                entry.offset += consumed;
                if consumed > 0 {
                    entry.last_activity = now;
                }
                let mut finished_cleanly = false;
                for line in lines {
                    match turn_boundary(&line) {
                        Some(Boundary::Started) => {
                            entry.running = true;
                            finished_cleanly = false;
                        }
                        // 돌고 있던 턴에 온 완료만 센다 — 중복·유실로 들어온 완료가
                        // 이미 끝난 턴을 한 번 더 알리면 안 된다
                        Some(Boundary::Completed) => {
                            finished_cleanly = entry.running;
                            entry.running = false;
                        }
                        Some(Boundary::Aborted) => {
                            entry.running = false;
                            finished_cleanly = false;
                        }
                        None => {}
                    }
                }
                if finished_cleanly {
                    completed.push(id.clone());
                }
            }
            // 완료 이벤트가 영영 안 오는 경우(크래시)의 안전망.
            // **완료 판정 뒤에 와야 한다** — 앞에 두면 타임아웃이 완료로 샌다.
            // 새 데이터가 없어도 돌아야 하므로 위 `if` 바깥이다.
            if entry.running && (now - entry.last_activity).num_milliseconds() > TURN_STALE_MS {
                entry.running = false;
            }
        }

        self.seeded = true;
        // 날짜 창을 벗어난 세션은 잊는다. 완료로는 안 센다 — 사라지는 것은 완료가 아니다.
        self.sessions.retain(|id, _| seen.contains(id));

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

/// 감시할 rollout 들 — 첫 값이 `covered`, 둘째가 `(session_id, 경로)` 목록.
///
/// 홈이 여럿이면 같은 rollout 이 여러 경로로 잡힌다(하드링크 미러). `session_id` 로
/// 접어 **먼저 본 경로 하나만** 쓴다 — 하드링크라 어느 쪽을 읽어도 같은 내용이다.
fn discover(homes: &[PathBuf], now: DateTime<Utc>) -> (bool, Vec<(String, PathBuf)>) {
    let mut covered = false;
    let mut seen = HashSet::new();
    let mut out = vec![];
    for home in homes {
        if home.join("sessions").is_dir() {
            covered = true;
        }
        for dir in day_dirs(home, now) {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.filter_map(|e| e.ok()) {
                let Some(id) = e.file_name().to_str().and_then(session_id_of) else { continue };
                if seen.insert(id.clone()) {
                    out.push((id, e.path()));
                }
            }
        }
    }
    (covered, out)
}

/// 오늘·어제의 rollout 폴더.
///
/// **어제까지 보는 이유**는 자정을 넘겨 도는 턴이다. 오늘 폴더만 보면 23:59 에 시작해
/// 00:01 까지 도는 세션이 목록에서 통째로 사라진다.
///
/// 날짜는 **로컬 시각**이다 — 파일명이 `rollout-2026-08-25T16-37-33-...` 처럼 로컬 시각이고
/// 폴더도 그 기준으로 나뉜다. UTC 로 재면 시차만큼 폴더를 헛짚는다.
fn day_dirs(home: &Path, now: DateTime<Utc>) -> [PathBuf; 2] {
    let today = now.with_timezone(&chrono::Local).date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);
    let dir = |d: chrono::NaiveDate| home.join("sessions").join(d.format("%Y/%m/%d").to_string());
    [dir(today), dir(yesterday)]
}

/// `rollout-<로컬시각>-<uuid>.jsonl` 에서 세션 id 를 뽑는다. 아니면 `None`.
///
/// 시각 부분의 길이를 세지 않고 **끝에서 36자**를 떼는 이유: uuid 길이는 고정이지만
/// 시각 표기는 포맷이 바뀔 수 있다. 모양까지 확인해 엉뚱한 파일을 세션으로 읽지 않는다.
fn session_id_of(file_name: &str) -> Option<String> {
    let rest = file_name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let id = rest.get(rest.len().checked_sub(36)?..)?;
    is_uuid(id).then(|| id.to_string())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
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
                // rollout 이 준 값을 그대로 쓴다 — Codex 는 캐시가 아니라 이벤트라 안 굳는다
                resets_computed: false,
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
        reset_credits: None,
        fetched_at: at,
    })
}

type Rollout = (
    Vec<ParsedEvent>,
    RawContext,
    Option<(DateTime<Utc>, PlanUsage)>,
    Option<SessionRow>,
    ParseDiagnostics,
);

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
    let Ok(content) = std::fs::read_to_string(path) else {
        return (
            vec![],
            ctx,
            limits,
            None,
            ParseDiagnostics { files_failed: 1, ..Default::default() },
        );
    };
    let mut diagnostics = ParseDiagnostics { files_read: 1, ..Default::default() };
    let mut out = vec![];
    let mut prev_total = Counters::default();
    let mut model: Option<String> = None;

    let trailing_partial = !content.ends_with('\n');
    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            if !(trailing_partial && lines.peek().is_none()) {
                diagnostics.checked += 1;
            }
            continue;
        };
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
        if title.is_empty() {
            let prompt = match p_ty {
                // 이전 rollout 형식
                "user_message" => payload.get("message").and_then(Value::as_str),
                // 현재 rollout 형식: response_item/message 안의 input_text 블록
                "message" if payload.get("role").and_then(Value::as_str) == Some("user") => payload
                    .get("content")
                    .and_then(Value::as_array)
                    .and_then(|blocks| {
                        blocks.iter().find_map(|block| {
                            (block.get("type").and_then(Value::as_str) == Some("input_text"))
                                .then(|| block.get("text").and_then(Value::as_str))
                                .flatten()
                        })
                    }),
                _ => None,
            };
            if let Some(m) = prompt.filter(|m| is_human_prompt(m)) {
                title = first_line(m);
            }
        }

        if p_ty != "token_count" {
            continue;
        }
        diagnostics.checked += 1;
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
        let has_direct_counters = [
            "input_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
        ]
        .iter()
        .any(|key| info.get(key).is_some());
        if last.is_none() && total.is_none() && !has_direct_counters {
            continue;
        }
        diagnostics.parsed += 1;

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
                // OpenAI 는 캐시 TTL 을 고를 수 없다 — 단가가 하나뿐이라 몫을 가를 일이 없다
                cache_write_1h: 0,
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
    (out, ctx, limits, row, diagnostics)
}

#[cfg(test)]
mod turn_tests {
    use super::*;
    use std::fs;

    const SID: &str = "019ff450-7640-7c82-91a1-707b3469216b";

    fn ev(kind: &str) -> String {
        format!(r#"{{"timestamp":"2026-08-12T04:54:24.000Z","type":"event_msg","payload":{{"type":"{kind}"}}}}"#)
    }

    /// `<홈>/sessions/<오늘>/rollout-…-<sid>.jsonl` 을 만든다.
    ///
    /// 날짜 폴더를 **로컬 오늘**로 짓는 이유: 감시 범위가 오늘·어제 두 폴더라
    /// (`day_dirs`) 고정 날짜로 지으면 시간이 지나 테스트가 조용히 아무것도 안 보게 된다.
    fn rollout_path(home: &Path, sid: &str) -> PathBuf {
        let today = chrono::Local::now().date_naive();
        home.join("sessions")
            .join(today.format("%Y/%m/%d").to_string())
            .join(format!("rollout-{}T13-52-21-{sid}.jsonl", today.format("%Y-%m-%d")))
    }

    fn write_rollout(home: &Path, sid: &str, lines: &[String]) -> PathBuf {
        let p = rollout_path(home, sid);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let body = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
        fs::write(&p, body).unwrap();
        p
    }

    fn home_with_session(rollout_lines: &[String]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write_rollout(dir.path(), SID, rollout_lines);
        dir
    }

    fn append(path: &Path, line: &str) {
        let mut s = fs::read_to_string(path).unwrap();
        s.push_str(line);
        s.push('\n');
        fs::write(path, s).unwrap();
    }

    fn rollout_of(home: &Path) -> PathBuf {
        rollout_path(home, SID)
    }

    /// 턴 시작은 rollout 의 `task_started` 다. 예전에는 `history.jsonl` 에 줄이 붙는
    /// 것으로 알았는데, 그 파일은 TUI 프롬프트만 남겨서 Obsidian·`exec` 세션이 통째로
    /// 빠졌다 (`TurnWatcher` 주석의 실측표).
    #[test]
    fn task_started_starts_a_turn() {
        let home = home_with_session(&[]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];

        let first = w.poll(&homes, now);
        assert!(first.covered);
        assert!(first.running.is_empty());

        append(&rollout_of(home.path()), &ev("task_started"));
        assert_eq!(w.poll(&homes, now).running, vec![SID.to_string()]);
    }

    #[test]
    fn task_complete_ends_the_turn_immediately() {
        let home = home_with_session(&[]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        append(&rollout_of(home.path()), &ev("task_started"));
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
        let home = home_with_session(&[]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        append(&rollout_of(home.path()), &ev("task_started"));
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
        let home = home_with_session(&[]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        append(&rollout_of(home.path()), &ev("task_started"));
        assert!(!w.poll(&homes, now).running.is_empty());

        append(&rollout_of(home.path()), &ev("turn_aborted"));
        let p = w.poll(&homes, now);
        assert!(p.running.is_empty(), "턴은 끝난다");
        assert!(p.completed.is_empty(), "그러나 완료는 아니다");
    }

    /// **첫 회차는 이미 쌓인 rollout 을 되짚지 않는다** — 안 그러면 오늘 돌았던 세션이
    /// 전부 방금 시작한 것처럼 살아난다.
    #[test]
    fn the_first_poll_does_not_replay_todays_rollouts() {
        let home = home_with_session(&[ev("task_started")]);
        let mut w = TurnWatcher::default();
        let p = w.poll(&[home.path().to_path_buf()], Utc::now());
        assert!(p.running.is_empty());
        assert!(p.completed.is_empty());
    }

    /// 반대로 **첫 회차 뒤에 생긴 파일은 새 세션**이라 처음부터 읽어야 한다.
    /// 끝에서 시작하면 그 세션의 `task_started` 를 놓쳐 첫 턴이 통째로 안 잡힌다.
    #[test]
    fn a_rollout_that_appears_later_is_read_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sessions")).unwrap();
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![dir.path().to_path_buf()];
        assert!(w.poll(&homes, now).running.is_empty());

        write_rollout(dir.path(), SID, &[ev("task_started")]);
        assert_eq!(w.poll(&homes, now).running, vec![SID.to_string()], "새 세션의 첫 턴");
    }

    /// 같은 rollout 이 두 홈에 하드링크로 미러링되는 배치가 실재한다
    /// (Orca 가 주입한 `CODEX_HOME` + `~/.codex`). **한 세션으로 세야 한다** —
    /// 둘로 세면 busy 가 두 배가 되고 완료가 두 번 나간다.
    #[test]
    fn the_same_session_in_two_homes_is_counted_once() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write_rollout(a.path(), SID, &[]);
        write_rollout(b.path(), SID, &[]);
        let homes = vec![a.path().to_path_buf(), b.path().to_path_buf()];

        let mut w = TurnWatcher::default();
        let now = Utc::now();
        w.poll(&homes, now);
        // 미러이므로 양쪽에 같은 내용이 붙는다
        append(&rollout_path(a.path(), SID), &ev("task_started"));
        append(&rollout_path(b.path(), SID), &ev("task_started"));

        let p = w.poll(&homes, now);
        assert_eq!(p.running.len(), 1, "한 세션이다");

        append(&rollout_path(a.path(), SID), &ev("task_complete"));
        append(&rollout_path(b.path(), SID), &ev("task_complete"));
        let p = w.poll(&homes, now);
        assert_eq!(p.completed, vec![SID.to_string()], "완료도 한 번만");
    }

    #[test]
    fn two_sessions_are_tracked_independently() {
        let other = "019ff361-987f-7d32-940b-7ab6a69ed686";
        let home = home_with_session(&[]);
        write_rollout(home.path(), other, &[]);

        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        append(&rollout_path(home.path(), SID), &ev("task_started"));
        append(&rollout_path(home.path(), other), &ev("task_started"));
        let p = w.poll(&homes, now);
        assert_eq!(p.running.len(), 2, "동시 세션은 각각 추적된다");

        // 한쪽만 끝나도 나머지는 그대로
        append(&rollout_path(home.path(), other), &ev("task_complete"));
        let p = w.poll(&homes, now);
        assert_eq!(p.running, vec![SID.to_string()]);
    }

    /// `sessions/` 자체가 없으면 이 방식이 성립하지 않는다.
    /// 예전 기준(`history.jsonl` 존재)은 **파일이 있는데 안 자라는** 경우를 못 걸렀다 —
    /// 이번 버그가 정확히 그 모양이었다.
    #[test]
    fn no_sessions_dir_is_not_covered() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = TurnWatcher::default();
        let p = w.poll(&[dir.path().to_path_buf()], Utc::now());
        assert!(!p.covered);
        assert!(p.running.is_empty());
    }

    /// 파일명에서 세션 id 를 뽑는 규칙 — 엉뚱한 파일을 세션으로 읽으면 안 된다
    #[test]
    fn session_id_comes_from_the_file_name() {
        assert_eq!(
            session_id_of("rollout-2026-08-25T16-37-33-01a037da-611b-7541-ad05-52eb54942282.jsonl")
                .as_deref(),
            Some("01a037da-611b-7541-ad05-52eb54942282")
        );
        // rollout 이 아닌 파일, uuid 가 아닌 꼬리, 확장자 불일치는 전부 거른다
        assert!(session_id_of("history.jsonl").is_none());
        assert!(session_id_of("rollout-2026-08-25T16-37-33-not-a-uuid.jsonl").is_none());
        assert!(session_id_of("rollout-01a037da-611b-7541-ad05-52eb54942282.txt").is_none());
    }

    #[test]
    fn half_written_line_is_not_consumed() {
        let home = home_with_session(&[]);
        let mut w = TurnWatcher::default();
        let now = Utc::now();
        let homes = vec![home.path().to_path_buf()];
        w.poll(&homes, now);
        append(&rollout_of(home.path()), &ev("task_started"));
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

/// 계약 테스트([`crate::adapter`] tests)용 표준 픽스처 — 내용 명세는 그쪽 주석 참고.
/// "같은 요청이 두 번 기록되는" 이 소스의 실제 형태는 **두 홈에 복사된 같은 rollout**
/// 이다 (재배치된 홈 + `~/.codex` 미러 — 실측 사례).
#[cfg(test)]
pub(crate) fn conformance_roots() -> (Vec<tempfile::TempDir>, Vec<PathBuf>) {
    let lines = [
        r#"{"timestamp":"2026-08-13T00:59:58.000Z","type":"session_meta","payload":{"id":"conf-rollout"}}"#,
        r#"{"timestamp":"2026-08-13T00:59:59.000Z","type":"event_msg","payload":{"type":"user_message","message":"계약 테스트 첫 질문"}}"#,
        r#"{"timestamp":"2026-08-13T00:59:59.500Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}"#,
        r#"{"timestamp":"2026-08-13T01:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"total_tokens":110},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"total_tokens":110}}}}"#,
        r#"{"timestamp":"2026-08-13T01:05:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"cached_input_tokens":0,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":200,"cached_input_tokens":0,"output_tokens":20,"total_tokens":220}}}}"#,
    ]
    .join("\n");
    let mut dirs = vec![];
    let mut roots = vec![];
    for _ in 0..2 {
        let d = tempfile::tempdir().unwrap();
        let day = d.path().join("sessions/2026/08/13");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-conf.jsonl"), &lines).unwrap();
        roots.push(d.path().to_path_buf());
        dirs.push(d);
    }
    (dirs, roots)
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
        let lines = [
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
        let out = adapter.scan(DateTime::UNIX_EPOCH);

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
        let lines = [
            r#"{"timestamp":"2026-07-31T02:00:00.000Z","type":"session_meta","payload":{"id":"019fb5e6","cwd":"/x"}}"#.to_string(),
            r#"{"timestamp":"2026-07-31T02:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.6-terra","cwd":"/x"}}"#.to_string(),
            r#"{"timestamp":"2026-07-31T02:03:12.104Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"total_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"cache_write_input_tokens":0,"output_tokens":12,"reasoning_output_tokens":0,"total_tokens":13261},"last_token_usage":{"input_tokens":13249,"cached_input_tokens":9984,"cache_write_input_tokens":0,"output_tokens":12,"reasoning_output_tokens":0,"total_tokens":13261}}}}"#.to_string(),
            // 두 번째 턴: 누적은 커지지만 컨텍스트는 요청 단위 값이어야 한다
            r#"{"timestamp":"2026-07-31T02:05:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"total_token_usage":{"input_tokens":33249,"cached_input_tokens":19984,"output_tokens":112,"reasoning_output_tokens":0,"total_tokens":33361},"last_token_usage":{"input_tokens":20000,"cached_input_tokens":10000,"output_tokens":100,"reasoning_output_tokens":0,"total_tokens":20100}}}}"#.to_string(),
        ];
        fs::write(day.join("rollout-x.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
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
        let lines = [r#"{"timestamp":"2026-07-30T01:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}"#,
            r#"{"timestamp":"2026-07-30T01:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":200,"total_tokens":1200},"last_token_usage":null}}}"#];
        fs::write(day.join("rollout-y.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH);
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
        let out = adapter.scan(DateTime::UNIX_EPOCH);
        assert_eq!(out.events.len(), 1, "같은 rollout 이 두 홈에 있으면 중복 집계된다");

        // 서로 다른 요청은 그대로 남아야 한다 (dedup 이 과하게 먹으면 안 됨)
        let day = b.path().join("sessions/2026/07/31");
        let other = line.replace("02:03:12", "02:09:12");
        fs::write(day.join("rollout-same.jsonl"), format!("{meta}\n{line}\n{other}")).unwrap();
        let mut adapter = CodexAdapter::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
        assert_eq!(adapter.scan(DateTime::UNIX_EPOCH).events.len(), 2);
    }

    #[test]
    fn no_home_reports_no_data() {
        let mut adapter = CodexAdapter::new(vec![]);
        let out = adapter.scan(DateTime::UNIX_EPOCH);
        assert_eq!(out.status, SourceStatus::NoData);
    }

    #[test]
    fn unreadable_rollout_format_reports_degraded() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/09/10");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("rollout-broken.jsonl"), "not-json\nstill-not-json\nbroken-again\n")
            .unwrap();
        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            adapter.scan(DateTime::UNIX_EPOCH).status,
            SourceStatus::Degraded { checked: 3, failed: 3 }
        );
    }

    /// 실파일(rollout-2026-08-11, free 플랜)에서 관측한 `rate_limits` 그대로.
    /// 43,200분 = 30일 창이고 free 플랜에선 secondary 가 null 이다.
    #[test]
    fn official_limits_come_from_the_rollout_itself() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/11");
        fs::create_dir_all(&day).unwrap();
        let lines = [
            r#"{"timestamp":"2026-08-11T01:00:00.000Z","type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#,
            // 이전 값 — 나중 이벤트가 이겨야 한다
            r#"{"timestamp":"2026-08-11T01:20:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":3.0,"window_minutes":43200,"resets_at":1789004769},"secondary":null,"plan_type":"free"}}}"#,
            r#"{"timestamp":"2026-08-11T01:46:50.803Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":26930,"cached_input_tokens":25344,"output_tokens":112,"total_tokens":27042},"model_context_window":258400},"rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":43200,"resets_at":1789004769},"secondary":null,"credits":{"has_credits":false,"unlimited":false,"balance":null},"plan_type":"free"}}}"#,
        ];
        fs::write(day.join("rollout-limits.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
        let p = adapter.plan().unwrap();

        assert_eq!(p.source, Source::Codex);
        assert_eq!(p.detail, "Free", "Claude 계정 카드와 같은 표기 규칙");
        assert_eq!(p.meters.len(), 1, "free 플랜은 secondary 가 null");
        assert_eq!(p.meters[0].label, "월간");
        assert_eq!(p.meters[0].used_pct, 4, "마지막 이벤트 값이어야 함");
        // 문자열 파싱 없이 정확한 시각이 온다 (Claude 와의 차이)
        assert_eq!(p.meters[0].resets_at.unwrap().timestamp(), 1_789_004_769);
        assert!(adapter.last_activity().is_some());
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
        adapter.scan(DateTime::UNIX_EPOCH);
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
        let lines = [
            r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4c7","cwd":"C:\\Users\\u\\projects\\token-chan","git":{"branch":"main","commit_hash":"d6728af"}}}"#,
            // 실측 그대로 — 첫 줄 뒤에 붙여넣은 JSON 이 이어진다
            r#"{"timestamp":"2026-08-12T01:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"상태 정보를 가져오는데\n{\n  \"a\": 1\n}"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:02.000Z","type":"event_msg","payload":{"type":"user_message","message":"두 번째 메시지는 제목이 아니다"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#,
        ];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4c7.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
        let rows = adapter.sessions();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "상태 정보를 가져오는데", "첫 줄만, 첫 메시지만");
        assert_eq!(rows[0].branch, "main");
    }

    #[test]
    fn session_row_reads_current_response_item_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sessions");
        std::fs::create_dir_all(&root).unwrap();
        let lines = [
            r#"{"type":"session_meta","payload":{"id":"current-shape","cwd":"/home/u/projects/api"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>...</environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"현재 형식의 Codex 제목"}]}}"#,
            r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2,"cached_input_tokens":0}}}}"#,
        ];
        std::fs::write(root.join("rollout-current.jsonl"), lines.join("\n")).unwrap();
        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
        assert_eq!(adapter.sessions()[0].label, "현재 형식의 Codex 제목");
    }

    /// 다른 도구가 Claude 대화를 그대로 입력으로 넣은 세션이 실측됐다
    /// (`originator: "Codex Desktop"`, 50건). 그 첫 메시지는 Claude Code 의 명령 래퍼라
    /// 제목이 되면 안 된다 — 걸러서 폴더명으로 떨어져야 한다.
    #[test]
    fn session_row_skips_injected_wrappers() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/12");
        fs::create_dir_all(&day).unwrap();
        let lines = [r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4e0","cwd":"C:\\Users\\u\\projects\\token-chan\\src-tauri"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"<command-name>/usage</command-name>\n<command-message>usage</command-message>"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4e0.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
        assert_eq!(adapter.sessions()[0].label, "src-tauri", "래퍼는 제목이 아니다");
    }

    /// 첫 사용자 메시지가 없으면(도구로만 돈 세션) 예전처럼 폴더명으로 떨어진다
    #[test]
    fn session_row_falls_back_to_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/08/12");
        fs::create_dir_all(&day).unwrap();
        let lines = [r#"{"timestamp":"2026-08-12T01:00:00.000Z","type":"session_meta","payload":{"id":"019ff4c8","cwd":"/home/u/projects/api"}}"#,
            r#"{"timestamp":"2026-08-12T01:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}}"#];
        fs::write(day.join("rollout-2026-08-12T01-00-00-019ff4c8.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = CodexAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH);
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
        adapter.scan(DateTime::UNIX_EPOCH);
        assert!(adapter.plan().is_none());
    }
}

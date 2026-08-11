//! Codex CLI 어댑터.
//!
//! ✅ **실데이터 검증됨** (2026-07-31, codex-tui 0.146.0 rollout): `turn_context`
//! top-level 이벤트의 `payload.model`, `event_msg`/`token_count` 의
//! `info.last_token_usage`(델타)·`total_token_usage`(누적) 구조 확인.
//! 실파일엔 `cache_write_input_tokens` 필드도 존재(관측값 0) → cache_write 로 전달.
//! 검증 방법: `cargo run -p usage-core --example scan_codex`
//!
//! 알려진 포맷:
//! - 세션 파일: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` + `archived_sessions/`
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
use crate::session::{dir_label, SessionRow};

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
/// 같은 rollout 이 여러 루트에 존재할 수 있어 — 예: `CODEX_HOME` 과 `~/.codex` 를 함께 볼 때 —
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

    /// 작업 중 여부를 판정할 때 mtime 을 볼 파일 (가장 최근에 쓰인 rollout).
    /// 사용량 스캔은 10초 주기라 그 값으로 판정하면 작업 시작이 최대 10초 늦게 보인다.
    /// 이 경로 하나만 넘겨 두면 라이브 스레드(2초)가 직접 stat 해서 바로 알아챈다.
    pub fn watch_path(&self) -> Option<PathBuf> {
        self.cache
            .iter()
            .max_by_key(|(_, fc)| fc.written_at)
            .map(|(p, _)| p.clone())
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
                resets: String::new(),
                resets_at,
            },
        ));
    }
    if meters.is_empty() {
        return None;
    }
    meters.sort_by_key(|(m, _)| *m);

    let detail = v
        .get("plan_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s} 플랜"))
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
        id: ctx.session.clone(),
        label: if cwd.is_empty() { ctx.session.chars().take(8).collect() } else { dir_label(&cwd) },
        cwd,
        model: model.unwrap_or_default(),
        // Codex 는 브랜치를 안 남긴다
        branch: String::new(),
        at,
        tokens,
    });
    (out, ctx, limits, row)
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

    /// 같은 rollout 이 두 홈에 있을 때 (예: `CODEX_HOME` + `~/.codex`) 한 번만 세어야 한다.
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
        assert_eq!(p.detail, "free 플랜");
        assert_eq!(p.meters.len(), 1, "free 플랜은 secondary 가 null");
        assert_eq!(p.meters[0].label, "월간");
        assert_eq!(p.meters[0].used_pct, 4, "마지막 이벤트 값이어야 함");
        // 문자열 파싱 없이 정확한 시각이 온다 (Claude 와의 차이)
        assert_eq!(p.meters[0].resets_at.unwrap().timestamp(), 1_789_004_769);
        assert_eq!(adapter.last_activity().is_some(), true);
    }

    /// 유료 플랜은 5시간 + 주간 두 창이 온다. 짧은 창이 먼저여야 첫 미터가
    /// "지금 당장 걸리는 한도"가 된다.
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

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

use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};

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

struct FileCache {
    mtime: SystemTime,
    size: u64,
    events: Vec<UsageEvent>,
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
                        let events = parse_rollout(path);
                        self.cache.insert(path.to_path_buf(), FileCache { mtime, size, events });
                    }
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

        let mut events: Vec<UsageEvent> = self
            .cache
            .values()
            .flat_map(|c| c.events.iter().filter(|e| e.ts >= since).cloned())
            .collect();
        events.sort_by_key(|e| e.ts);

        let status = if any_file { SourceStatus::Ok } else { SourceStatus::NoData };
        ScanOutcome { events, status }
    }
}

fn parse_rollout(path: &Path) -> Vec<UsageEvent> {
    let Ok(content) = std::fs::read_to_string(path) else { return vec![] };
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

        let info = payload.get("info").filter(|i| i.is_object()).unwrap_or(&payload);

        let last = info.get("last_token_usage").filter(|l| l.is_object());
        let total = info.get("total_token_usage").filter(|t| t.is_object());

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
        out.push(UsageEvent {
            source: Source::Codex,
            model: model.clone().unwrap_or_else(|| "codex-unknown".into()),
            ts,
            input: delta.input.saturating_sub(delta.cached),
            output,
            // 실데이터에서 관측값은 아직 0 — input_tokens 포함 여부가 미확인이라 그대로 전달만
            cache_write: delta.cache_write,
            cache_read: delta.cached,
            sidechain: false,
        });
    }
    out
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

    #[test]
    fn no_home_reports_no_data() {
        let mut adapter = CodexAdapter::new(vec![]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.status, SourceStatus::NoData);
    }
}

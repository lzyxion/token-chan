//! Gemini CLI 어댑터.
//!
//! ⚠️ **미검증(unverified)**: Gemini CLI는 기본 설정에서 토큰 사용량을 로컬에 기록하지
//! 않는다. 사용자가 `~/.gemini/settings.json` 에 OTel 텔레메트리를 활성화해야 하며,
//! 그 outfile 을 파싱한다. 파일 구조는 버전에 따라 다를 수 있어 방어적으로 파싱한다.
//!
//! 필요 설정:
//! ```json
//! { "telemetry": { "enabled": true, "target": "local", "outfile": "~/.gemini/telemetry.log" } }
//! ```
//!
//! outfile 포맷: pretty-print 된 JSON 객체가 **연접**된 형태(JSONL 아님) →
//! 문자열/이스케이프를 인지하는 brace-balance 스플리터로 분리.
//! `gemini_cli.api_response` 이벤트에서
//! `input_token_count / output_token_count / cached_content_token_count / thoughts_token_count / model / event.timestamp`
//! 를 추출한다. OTLP kv-list(`{"key":..,"value":{..}}`) 와 평면 객체 둘 다 지원.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};

pub const SETUP_GUIDE: &str = r#"Gemini CLI는 기본적으로 토큰 사용량을 로컬에 기록하지 않습니다. ~/.gemini/settings.json 에 다음을 추가하세요:
{ "telemetry": { "enabled": true, "target": "local", "outfile": "~/.gemini/telemetry.log" } }"#;

struct FileCache {
    mtime: SystemTime,
    size: u64,
    events: Vec<UsageEvent>,
}

pub struct GeminiAdapter {
    homes: Vec<PathBuf>,
    cache: HashMap<PathBuf, FileCache>,
}

impl GeminiAdapter {
    pub fn new(homes: Vec<PathBuf>) -> Self {
        Self { homes, cache: HashMap::new() }
    }

    pub fn with_default_roots() -> Self {
        Self::new(crate::roots::gemini_homes())
    }

    /// settings.json 의 telemetry.outfile + 관례적 위치를 프로브해 존재하는 파일만 반환
    fn telemetry_files(&self) -> Vec<PathBuf> {
        let mut candidates: Vec<PathBuf> = vec![];
        for home in &self.homes {
            candidates.push(home.join("telemetry.log"));
            let settings = home.join("settings.json");
            if let Ok(s) = std::fs::read_to_string(&settings) {
                if let Ok(v) = serde_json::from_str::<Value>(&s) {
                    if let Some(outfile) = v
                        .get("telemetry")
                        .and_then(|t| t.get("outfile"))
                        .and_then(Value::as_str)
                    {
                        candidates.push(resolve_outfile(outfile, home));
                    }
                }
            }
        }
        candidates.retain(|p| p.is_file());
        candidates.dedup();
        candidates
    }

    pub fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        if self.homes.is_empty() {
            return ScanOutcome { events: vec![], status: SourceStatus::NoData };
        }
        let files = self.telemetry_files();
        if files.is_empty() {
            return ScanOutcome {
                events: vec![],
                status: SourceStatus::NeedsSetup { guide: SETUP_GUIDE.to_string() },
            };
        }

        let mut seen = std::collections::HashSet::new();
        for path in &files {
            let Ok(meta) = std::fs::metadata(path) else { continue };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size = meta.len();
            seen.insert(path.clone());
            let needs = match self.cache.get(path) {
                Some(c) => c.mtime != mtime || c.size != size,
                None => true,
            };
            if needs {
                let events = parse_telemetry(path);
                self.cache.insert(path.clone(), FileCache { mtime, size, events });
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

        let mut events: Vec<UsageEvent> = self
            .cache
            .values()
            .flat_map(|c| c.events.iter().filter(|e| e.ts >= since).cloned())
            .collect();
        events.sort_by_key(|e| e.ts);

        ScanOutcome { events, status: SourceStatus::Ok }
    }
}

fn resolve_outfile(outfile: &str, home: &Path) -> PathBuf {
    if let Some(rest) = outfile.strip_prefix("~/") {
        if let Some(h) = crate::roots::home_dir() {
            return h.join(rest);
        }
    }
    let p = PathBuf::from(outfile);
    if p.is_absolute() {
        p
    } else {
        // 상대 경로는 gemini 홈 기준으로 해석 (".gemini/telemetry.log" 관례 포함)
        if let Ok(stripped) = p.strip_prefix(".gemini") {
            home.join(stripped)
        } else {
            home.join(p)
        }
    }
}

fn parse_telemetry(path: &Path) -> Vec<UsageEvent> {
    let Ok(content) = std::fs::read_to_string(path) else { return vec![] };
    let mut out = vec![];
    for chunk in split_json_objects(&content) {
        let Ok(v) = serde_json::from_str::<Value>(chunk) else { continue };
        let mut map: HashMap<String, Value> = HashMap::new();
        flatten(&v, &mut map);

        let is_api_response = map
            .get("event.name")
            .and_then(Value::as_str)
            .map(|n| n.contains("api_response"))
            .unwrap_or(false);
        if !is_api_response {
            continue;
        }

        let Some(ts) = map
            .get("event.timestamp")
            .or_else(|| map.get("timestamp"))
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            continue;
        };

        let g = |k: &str| get_u64(map.get(k));
        let input = g("input_token_count");
        let output = g("output_token_count");
        let cached = g("cached_content_token_count");
        let thoughts = g("thoughts_token_count");
        if input + output + cached + thoughts == 0 {
            continue;
        }
        let model = map
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("gemini-unknown")
            .to_string();

        out.push(UsageEvent {
            source: Source::Gemini,
            model,
            ts,
            // Gemini의 input_token_count(prompt)는 캐시분을 포함 → 순수 입력 = input - cached
            input: input.saturating_sub(cached),
            // thoughts 는 output 과 별도 계상 → 합산
            output: output + thoughts,
            cache_write: 0,
            cache_read: cached,
            sidechain: false,
        });
    }
    out
}

/// 숫자 또는 OTLP 스타일 문자열 숫자("123") 모두 지원
fn get_u64(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// 연접된 pretty-print JSON 객체들을 문자열/이스케이프 인지하며 분리
fn split_json_objects(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut depth = 0usize;
    let mut start = None;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in s.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(st) = start {
                            out.push(&s[st..=i]);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// JSON 트리를 key→scalar 맵으로 평탄화.
/// OTLP kv-list 항목 `{"key": "...", "value": {"intValue": ...}}` 은 key→내부값으로 변환.
fn flatten(v: &Value, map: &mut HashMap<String, Value>) {
    match v {
        Value::Object(o) => {
            if let (Some(Value::String(k)), Some(val)) = (o.get("key"), o.get("value")) {
                map.entry(k.clone()).or_insert_with(|| unwrap_otel_value(val));
                return;
            }
            for (k, val) in o {
                match val {
                    Value::Object(_) | Value::Array(_) => flatten(val, map),
                    _ => {
                        map.entry(k.clone()).or_insert_with(|| val.clone());
                    }
                }
            }
        }
        Value::Array(a) => {
            for item in a {
                flatten(item, map);
            }
        }
        _ => {}
    }
}

fn unwrap_otel_value(v: &Value) -> Value {
    if let Some(o) = v.as_object() {
        for k in ["stringValue", "intValue", "doubleValue", "boolValue"] {
            if let Some(inner) = o.get(k) {
                return inner.clone();
            }
        }
    }
    v.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_concatenated_objects_both_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".gemini");
        fs::create_dir_all(&home).unwrap();
        // 형태 1: 평면 attributes 객체 / 형태 2: OTLP kv-list / + api_request(무시 대상)
        let telemetry = r#"{
  "body": "API response from Gemini.",
  "attributes": {
    "event.name": "gemini_cli.api_response",
    "event.timestamp": "2026-07-30T01:00:00.000Z",
    "model": "gemini-2.5-pro",
    "input_token_count": 1000,
    "output_token_count": 200,
    "cached_content_token_count": 300,
    "thoughts_token_count": 50,
    "total_token_count": 1550,
    "session.id": "abc"
  }
}
{
  "attributes": [
    { "key": "event.name", "value": { "stringValue": "gemini_cli.api_response" } },
    { "key": "event.timestamp", "value": { "stringValue": "2026-07-30T02:00:00.000Z" } },
    { "key": "model", "value": { "stringValue": "gemini-2.5-flash" } },
    { "key": "input_token_count", "value": { "intValue": "500" } },
    { "key": "output_token_count", "value": { "intValue": "100" } }
  ]
}
{
  "attributes": { "event.name": "gemini_cli.api_request", "event.timestamp": "2026-07-30T03:00:00.000Z" }
}"#;
        fs::write(home.join("telemetry.log"), telemetry).unwrap();

        let mut adapter = GeminiAdapter::new(vec![home]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());

        assert_eq!(out.status, SourceStatus::Ok);
        assert_eq!(out.events.len(), 2);

        let e1 = &out.events[0];
        assert_eq!(e1.model, "gemini-2.5-pro");
        assert_eq!(e1.input, 700); // 1000 - 300(cached)
        assert_eq!(e1.cache_read, 300);
        assert_eq!(e1.output, 250); // 200 + 50(thoughts)

        let e2 = &out.events[1];
        assert_eq!(e2.model, "gemini-2.5-flash");
        assert_eq!(e2.input, 500);
        assert_eq!(e2.output, 100);
    }

    #[test]
    fn missing_telemetry_reports_needs_setup() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".gemini");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("settings.json"), r#"{"theme":"Default"}"#).unwrap();

        let mut adapter = GeminiAdapter::new(vec![home]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert!(matches!(out.status, SourceStatus::NeedsSetup { .. }));
    }

    #[test]
    fn settings_outfile_is_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".gemini");
        fs::create_dir_all(&home).unwrap();
        // 절대경로 outfile 지정 — Windows 경로 역슬래시가 JSON 이스케이프되도록 json!으로 직렬화
        let custom = dir.path().join("custom-telemetry.log");
        let settings = serde_json::json!({
            "telemetry": { "enabled": true, "target": "local", "outfile": custom.to_string_lossy() }
        });
        fs::write(home.join("settings.json"), settings.to_string()).unwrap();
        fs::write(&custom, r#"{"attributes":{"event.name":"gemini_cli.api_response","event.timestamp":"2026-07-30T01:00:00.000Z","model":"gemini-2.5-pro","input_token_count":10,"output_token_count":5}}"#).unwrap();

        let mut adapter = GeminiAdapter::new(vec![home]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.status, SourceStatus::Ok);
        assert_eq!(out.events.len(), 1);
    }
}

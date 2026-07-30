//! Claude Code 어댑터.
//!
//! `~/.claude/projects/**/*.jsonl` (서브에이전트 중첩 포함) 를 재귀 탐색해
//! `type:"assistant"` 행의 usage를 추출한다.
//!
//! 핵심 규칙 (이 머신의 실데이터로 검증됨):
//! - 응답 1건이 콘텐츠 블록 수만큼 여러 줄로 반복 기록됨 → `message.id`로 dedup 필수
//! - `costUSD` 필드는 존재하지 않음 → 비용은 단가표로 별도 계산
//! - `message.model == "<synthetic>"` 은 에러 행 → 제외
//! - 서브에이전트 트랜스크립트는 `<sessionId>/subagents/agent-*.jsonl` 로 한 단계 더 깊음

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<Msg>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
}

#[derive(Deserialize)]
struct Msg {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

/// dedup 키와 함께 캐시되는 파싱 결과
struct ParsedEvent {
    dedup_key: String,
    ev: UsageEvent,
}

struct FileCache {
    mtime: SystemTime,
    size: u64,
    events: Vec<ParsedEvent>,
}

pub struct ClaudeAdapter {
    roots: Vec<PathBuf>,
    cache: HashMap<PathBuf, FileCache>,
}

impl ClaudeAdapter {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots, cache: HashMap::new() }
    }

    pub fn with_default_roots() -> Self {
        Self::new(crate::roots::claude_project_roots())
    }

    /// `since` 이후의 이벤트 스냅샷을 반환. 내부 파일 캐시로 변경분만 재파싱.
    pub fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        if self.roots.is_empty() {
            return ScanOutcome { events: vec![], status: SourceStatus::NoData };
        }

        let mut seen_files: HashSet<PathBuf> = HashSet::new();
        let mut any_file = false;

        for root in &self.roots {
            for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                if !entry.file_type().is_file() {
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                any_file = true;

                let Ok(meta) = entry.metadata() else { continue };
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let size = meta.len();

                // mtime이 since보다 오래됐으면 그 파일의 모든 이벤트도 since 이전 → 스킵
                let mtime_dt: DateTime<Utc> = mtime.into();
                if mtime_dt < since {
                    continue;
                }

                seen_files.insert(path.to_path_buf());
                let needs_parse = match self.cache.get(path) {
                    Some(c) => c.mtime != mtime || c.size != size,
                    None => true,
                };
                if needs_parse {
                    let events = parse_transcript(path);
                    self.cache.insert(path.to_path_buf(), FileCache { mtime, size, events });
                }
            }
        }

        // 사라진/오래된 파일의 캐시 제거
        self.cache.retain(|p, _| seen_files.contains(p));

        // 전역 dedup 후 병합
        let mut dedup: HashSet<&str> = HashSet::new();
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

        let status = if !any_file { SourceStatus::NoData } else { SourceStatus::Ok };
        ScanOutcome { events, status }
    }
}

fn parse_transcript(path: &Path) -> Vec<ParsedEvent> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let mut out = vec![];
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        if row.kind.as_deref() != Some("assistant") {
            continue;
        }
        let Some(msg) = row.message else { continue };
        let Some(usage) = msg.usage else { continue };
        let model = msg.model.unwrap_or_default();
        if model.is_empty() || model == "<synthetic>" {
            continue;
        }
        let Some(ts) = row
            .timestamp
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            continue;
        };
        // dedup 키: message.id 우선 (전역 고유), 없으면 requestId
        let Some(dedup_key) = msg.id.or(row.request_id) else { continue };

        out.push(ParsedEvent {
            dedup_key,
            ev: UsageEvent {
                source: Source::Claude,
                model,
                ts,
                input: usage.input_tokens,
                output: usage.output_tokens,
                cache_write: usage.cache_creation_input_tokens,
                cache_read: usage.cache_read_input_tokens,
                sidechain: row.is_sidechain.unwrap_or(false),
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn assistant_line(msg_id: &str, req_id: &str, model: &str, ts: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{req_id}","timestamp":"{ts}","message":{{"id":"{msg_id}","model":"{model}","role":"assistant","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":100,"cache_read_input_tokens":2000}}}}}}"#
        )
    }

    #[test]
    fn dedups_repeated_lines_per_response() {
        // 응답 1건이 여러 줄로 반복 기록되는 실제 패턴 재현:
        // 같은 message.id 를 가진 3줄 + 다른 응답 1줄 → 이벤트 2건이어야 함
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-home-user-proj");
        fs::create_dir_all(&proj).unwrap();
        let mut lines = vec![];
        for _ in 0..3 {
            lines.push(assistant_line("msg_A", "req_A", "claude-opus-4-8", "2026-07-30T01:00:00.000Z", 10, 500));
        }
        lines.push(assistant_line("msg_B", "req_B", "claude-opus-4-8", "2026-07-30T01:05:00.000Z", 20, 700));
        lines.push(r#"{"type":"user","message":{"role":"user"}}"#.to_string());
        fs::write(proj.join("session1.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());

        assert_eq!(out.events.len(), 2, "중복 라인이 dedup 되지 않으면 과대집계");
        let total_output: u64 = out.events.iter().map(|e| e.output).sum();
        assert_eq!(total_output, 1200); // 500 + 700 (중복 합산 시 2200이 됨)
        assert_eq!(out.status, SourceStatus::Ok);
    }

    #[test]
    fn finds_nested_subagent_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("-proj").join("session-uuid").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("agent-xyz.jsonl"),
            assistant_line("msg_S", "req_S", "claude-fable-5", "2026-07-30T02:00:00.000Z", 5, 50),
        )
        .unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 1, "서브에이전트 중첩 파일을 재귀 탐색해야 함");
        assert_eq!(out.events[0].model, "claude-fable-5");
    }

    #[test]
    fn filters_synthetic_and_respects_since() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-proj");
        fs::create_dir_all(&proj).unwrap();
        let lines = vec![
            assistant_line("msg_1", "req_1", "<synthetic>", "2026-07-30T01:00:00.000Z", 0, 0),
            assistant_line("msg_2", "req_2", "claude-opus-4-8", "2026-07-01T01:00:00.000Z", 10, 10),
            assistant_line("msg_3", "req_3", "claude-opus-4-8", "2026-07-30T01:00:00.000Z", 10, 10),
        ];
        fs::write(proj.join("s.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        let since = DateTime::parse_from_rfc3339("2026-07-20T00:00:00Z").unwrap().with_timezone(&Utc);
        let out = adapter.scan(since);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].ts.to_rfc3339(), "2026-07-30T01:00:00+00:00");
    }

    #[test]
    fn empty_root_reports_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.status, SourceStatus::NoData);
    }
}

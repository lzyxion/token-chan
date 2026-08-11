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

use crate::context::{ContextState, RawContext};
use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};
use crate::pricing::PriceTable;
use crate::session::{dir_label, SessionRow};

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
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    /// 작업 디렉토리 — user 행마다 들어 있어 경로를 되짚어 추측할 필요가 없다
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    /// compact 실행 시 `type:"system"` 행에 붙는 실측 메타데이터
    #[serde(rename = "compactMetadata")]
    compact: Option<CompactMeta>,
}

#[derive(Deserialize)]
struct CompactMeta {
    trigger: Option<String>,
    #[serde(rename = "preTokens", default)]
    pre_tokens: u64,
    #[serde(rename = "postTokens", default)]
    post_tokens: u64,
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
    /// 이 트랜스크립트의 컨텍스트 상태 (서브에이전트 파일은 비어 있음)
    ctx: RawContext,
    /// 최근 세션 목록용 (서브에이전트 파일은 None — 사용자가 연 세션이 아니다)
    session: Option<SessionRow>,
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

    /// 최근 세션 목록. 정렬·합치기는 여러 소스를 모으는 호출부(`session::merge`)가 한다.
    pub fn sessions(&self) -> Vec<SessionRow> {
        self.cache.values().filter_map(|fc| fc.session.clone()).collect()
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
                    let (events, ctx, session) = parse_transcript(path);
                    self.cache
                        .insert(path.to_path_buf(), FileCache { mtime, size, events, ctx, session });
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

    /// 지금 작업 중인 세션의 컨텍스트 창 사용량.
    /// 캐시된 트랜스크립트 중 **가장 최근에 움직인 메인 세션** 하나를 고른다
    /// (서브에이전트 트랜스크립트는 별도 컨텍스트라 후보에서 빠져 있다).
    pub fn context(&self, pricing: &PriceTable) -> Option<ContextState> {
        let best = self
            .cache
            .values()
            .map(|fc| &fc.ctx)
            .filter(|c| !c.is_empty())
            .max_by_key(|c| c.last_activity())?;
        Some(crate::context::resolve(
            Source::Claude,
            best,
            pricing.context_window(&best.model),
        ))
    }
}

fn parse_transcript(path: &Path) -> (Vec<ParsedEvent>, RawContext, Option<SessionRow>) {
    let mut ctx = RawContext::default();
    let Ok(content) = std::fs::read_to_string(path) else {
        return (vec![], ctx, None);
    };
    let (mut cwd, mut branch) = (String::new(), String::new());
    let (mut last_at, mut last_model, mut tokens) = (None, String::new(), 0u64);
    // 서브에이전트 트랜스크립트는 자기만의 컨텍스트를 쓴다 — 세션 게이지에 섞으면 안 된다.
    let track_ctx = !path.components().any(|c| c.as_os_str() == "subagents");
    let mut min_read: Option<u64> = None;

    let mut out = vec![];
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        let ts = row
            .timestamp
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));
        let sidechain = row.is_sidechain.unwrap_or(false);
        let main_chain = track_ctx && !sidechain;

        // 작업 위치는 user 행에 붙는다 (세션 내내 같은 값이라 처음 본 것으로 충분)
        if cwd.is_empty() {
            if let Some(c) = row.cwd.as_deref().filter(|c| !c.is_empty()) {
                cwd = c.to_string();
                branch = row.git_branch.clone().unwrap_or_default();
            }
        }

        // compact 이벤트는 `type:"system"` 행에 실측값으로 남는다
        if let Some(cm) = row.compact {
            if main_chain {
                ctx.compactions += 1;
                // `cumulativeDroppedTokens` 필드도 있지만 compact 를 여러 번 한 세션이
                // 실데이터에 없어 누적 여부를 확인하지 못했다. pre-post 를 직접 더하면
                // 누적이든 아니든 맞는다 (단일 compact 에서 두 값이 일치함은 확인).
                ctx.dropped += cm.pre_tokens.saturating_sub(cm.post_tokens);
                ctx.last_compact_post = cm.post_tokens;
                ctx.last_compact_trigger = cm.trigger;
                ctx.last_compact_at = ts;
                ctx.peak = ctx.peak.max(cm.pre_tokens);
                if let Some(sid) = row.session_id.clone() {
                    ctx.session = sid;
                }
            }
            continue;
        }

        if row.kind.as_deref() != Some("assistant") {
            continue;
        }
        let Some(msg) = row.message else { continue };
        let Some(usage) = msg.usage else { continue };
        let model = msg.model.unwrap_or_default();
        if model.is_empty() || model == "<synthetic>" {
            continue;
        }
        let Some(ts) = ts else { continue };

        // 컨텍스트 = 보낸 것(input + cache write + cache read) + 생성한 것(output).
        // compactMetadata.preTokens 와 대조해 검증한 공식 (context.rs 참고).
        if main_chain {
            let total = usage.input_tokens
                + usage.cache_creation_input_tokens
                + usage.cache_read_input_tokens
                + usage.output_tokens;
            ctx.peak = ctx.peak.max(total);
            // 응답 1건이 여러 줄로 반복되지만 값이 같으므로 마지막 줄이 이겨도 무방
            if ctx.at.map(|prev| ts >= prev).unwrap_or(true) {
                ctx.tokens = total;
                ctx.at = Some(ts);
                ctx.model = model.clone();
            }
            // 시스템 프롬프트 + 툴 정의 바닥 — 세션 내 최소 cache_read 로 추정
            if usage.cache_read_input_tokens > 0 {
                min_read = Some(
                    min_read.map_or(usage.cache_read_input_tokens, |m: u64| {
                        m.min(usage.cache_read_input_tokens)
                    }),
                );
            }
            if let Some(sid) = row.session_id.clone() {
                ctx.session = sid;
            }
        }

        // dedup 키: message.id 우선 (전역 고유), 없으면 requestId
        let Some(dedup_key) = msg.id.or(row.request_id) else { continue };

        // 세션 목록용 집계 — 메인체인 기준 (서브에이전트 모델이 대표로 뜨면 안 된다)
        if !sidechain {
            if last_at.map(|prev| ts >= prev).unwrap_or(true) {
                last_at = Some(ts);
                last_model = model.clone();
            }
        }
        tokens += usage.input_tokens
            + usage.output_tokens
            + usage.cache_creation_input_tokens
            + usage.cache_read_input_tokens;

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
                sidechain,
            },
        });
    }

    ctx.baseline = min_read.unwrap_or(0);
    if ctx.session.is_empty() {
        ctx.session = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
    }

    // 서브에이전트 파일은 사용자가 연 세션이 아니라 목록에 넣지 않는다
    let session = (track_ctx && last_at.is_some()).then(|| SessionRow {
        source: Source::Claude,
        id: ctx.session.clone(),
        label: if cwd.is_empty() { ctx.session.chars().take(8).collect() } else { dir_label(&cwd) },
        cwd,
        model: last_model,
        branch,
        at: last_at.unwrap(),
        tokens,
    });
    (out, ctx, session)
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

    fn ctx_line(msg_id: &str, model: &str, ts: &str, input: u64, cw: u64, cr: u64, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"sess-1","requestId":"req_{msg_id}","timestamp":"{ts}","message":{{"id":"{msg_id}","model":"{model}","role":"assistant","usage":{{"input_tokens":{input},"output_tokens":{out},"cache_creation_input_tokens":{cw},"cache_read_input_tokens":{cr}}}}}}}"#
        )
    }

    fn compact_line(ts: &str, trigger: &str, pre: u64, post: u64) -> String {
        format!(
            r#"{{"type":"system","subtype":"compact_boundary","sessionId":"sess-1","isSidechain":false,"timestamp":"{ts}","compactMetadata":{{"trigger":"{trigger}","preTokens":{pre},"postTokens":{post},"cumulativeDroppedTokens":{},"durationMs":1000}}}}"#,
            pre - post
        )
    }

    #[test]
    fn context_tracks_latest_turn() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-proj");
        fs::create_dir_all(&proj).unwrap();
        let lines = vec![
            ctx_line("m1", "claude-opus-5", "2026-08-10T01:00:00.000Z", 2, 10_000, 26_000, 500),
            ctx_line("m2", "claude-opus-5", "2026-08-10T01:05:00.000Z", 2, 1_000, 36_500, 700),
        ];
        fs::write(proj.join("sess-1.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let c = adapter.context(&PriceTable::builtin()).unwrap();

        assert_eq!(c.tokens, 2 + 1_000 + 36_500 + 700, "마지막 턴의 in+cw+cr+out");
        assert_eq!(c.window, 1_000_000, "opus-5 는 단가표의 ctx 를 써야 함");
        assert!(!c.interim);
        assert_eq!(c.compactions, 0);
        assert_eq!(c.session, "sess-1");
    }

    #[test]
    fn compact_event_yields_interim_value_and_dropped_total() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-proj");
        fs::create_dir_all(&proj).unwrap();
        let lines = vec![
            ctx_line("m1", "claude-opus-5", "2026-08-10T01:00:00.000Z", 2, 3_000, 100_000, 600),
            compact_line("2026-08-10T01:10:00.000Z", "manual", 103_602, 9_000),
        ];
        fs::write(proj.join("sess-1.jsonl"), lines.join("\n")).unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let c = adapter.context(&PriceTable::builtin()).unwrap();

        assert!(c.interim, "compact 뒤 턴이 없으면 잠정값이어야 함");
        assert_eq!(c.compactions, 1);
        assert_eq!(c.dropped, 103_602 - 9_000);
        // postTokens 를 그대로 쓰면 안 되고 baseline(최소 cache_read)이 더해져야 한다
        assert_eq!(c.tokens, 9_000 + 100_000);
        assert_eq!(c.last_compact_trigger.as_deref(), Some("manual"));
    }

    #[test]
    fn subagent_transcript_excluded_from_context() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-proj");
        let sub = proj.join("sess-1").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        // 서브에이전트가 더 최근이지만 자기만의 컨텍스트라 세션 게이지에 잡히면 안 된다
        fs::write(
            sub.join("agent-x.jsonl"),
            ctx_line("s1", "claude-opus-5", "2026-08-10T09:00:00.000Z", 2, 0, 5_000, 100),
        )
        .unwrap();
        fs::write(
            proj.join("sess-1.jsonl"),
            ctx_line("m1", "claude-opus-5", "2026-08-10T01:00:00.000Z", 2, 1_000, 50_000, 300),
        )
        .unwrap();

        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        adapter.scan(DateTime::UNIX_EPOCH.into());
        let c = adapter.context(&PriceTable::builtin()).unwrap();
        assert_eq!(c.tokens, 2 + 1_000 + 50_000 + 300);
    }

    /// 같은 트랜스크립트가 두 루트에 있어도 (설정으로 추가한 홈이 자동 탐지분과 겹치는 등)
    /// `message.id` 전역 dedup 이 한 번만 세도록 보장해야 한다.
    #[test]
    fn same_transcript_in_two_roots_counted_once() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let line = assistant_line("msg_A", "req_A", "claude-opus-5", "2026-08-10T01:00:00.000Z", 10, 500);
        for root in [a.path(), b.path()] {
            let proj = root.join("-proj");
            fs::create_dir_all(&proj).unwrap();
            fs::write(proj.join("s.jsonl"), &line).unwrap();
        }

        let mut adapter = ClaudeAdapter::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].output, 500);
    }

    #[test]
    fn empty_root_reports_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = ClaudeAdapter::new(vec![dir.path().to_path_buf()]);
        let out = adapter.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.status, SourceStatus::NoData);
    }
}

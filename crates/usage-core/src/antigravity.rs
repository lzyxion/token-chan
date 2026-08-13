//! Antigravity CLI (`agy`) 어댑터.
//!
//! ✅ **실데이터 검증됨** (2026-08-10, `~/.gemini/antigravity-cli`, gemini-3.6-flash):
//! 대화 2개·요청 19건에서 아래 필드 매핑을 전부 대조했다.
//!
//! Gemini CLI 를 대체한 도구라 예전 OTel 텔레메트리 어댑터를 이걸로 갈아치웠다.
//! 텔레메트리를 켜야만 기록되던 예전과 달리 **설정 없이 항상** 남는다.
//!
//! ## 저장 위치와 포맷
//!
//! `~/.gemini/antigravity-cli/conversations/<uuid>.db` — 대화 하나가 **SQLite 파일 하나**다.
//! JSONL 이 아니고, `brain/<uuid>/.system_generated/logs/transcript.jsonl` 에는 토큰이
//! 아예 없다(의미 단위 로그). **대신 거기에 턴 경계가 있다** — [`TurnWatcher`] 참고.
//!
//! 쓰는 테이블은 `gen_metadata` 하나뿐 — **요청 1건 = 1행**이고 `data` 가 protobuf blob 이다.
//! 3턴 대화에 14행이 남았다: 에이전트가 한 턴에 툴 호출로 여러 번 요청하기 때문이다.
//!
//! ## 필드 번호 (`.proto` 미배포 → 번호로 직접 읽는다)
//!
//! ```text
//! 1        생성 기록
//!   1.4    사용량
//!     1.4.2   캐시 미스 프롬프트 토큰
//!     1.4.3   출력 토큰  (= 1.4.9 + 1.4.10, 19행 전부 일치)
//!     1.4.5   캐시 히트 프롬프트 토큰 (첫 요청엔 필드 자체가 없음 = 캐시 없음)
//!     1.4.11  요청 id — 19행 모두 고유해 dedup 키로 쓴다
//!   1.9.4.1  unix 초
//!   1.9.10   컨텍스트
//!     1.9.10.1  현재 컨텍스트 토큰
//!     1.9.10.3  구성 내역 (System Prompt / Tools / Chat Messages)
//!     1.9.10.4  컨텍스트 창
//!   1.19   모델명
//! ```
//!
//! ## 컨텍스트 — 직접 제공된다
//!
//! Claude 처럼 유도할 필요가 없고 Codex 보다도 낫다. 창 크기(`1.9.10.4` = 256,000)뿐 아니라
//! 구성 내역까지 주는데, 실측에서 내역 합이 총계와 **정확히 일치**했다
//! (System Prompt 9,870 + Tools 14,992 + Chat Messages 12,967 = 37,829).
//!
//! ⚠️ **대화의 첫 행(idx 0)은 컨텍스트가 엉터리다** — 실측 164 / 161. 그 시점엔 내역에
//! `USER_INPUT` 하나뿐이고 시스템 프롬프트·툴 정의(고정 약 24.9k)가 아직 안 잡혀 있다.
//! 그래서 첫 행은 컨텍스트 판정에서 제외한다. 사용량 집계에서는 멀쩡하므로 그대로 쓴다.
//!
//! 참고: 이 컨텍스트 총계는 서버가 보고한 프롬프트 합(`1.4.2 + 1.4.5`)보다 7천 토큰쯤 크다
//! (실측 37,829 vs 30,705). 전자는 agy 가 자기 화면에 쓰는 값이라 게이지에는 이쪽이 맞고,
//! 후자는 과금 기준이라 사용량 집계에는 그쪽을 쓴다. 서로 다른 질문에 대한 답이다.
//!
//! ## 계정
//!
//! agy 는 자격증명·계정 파일을 **디스크에 남기지 않는다**. `~/.gemini/oauth_creds.json` 과
//! `google_account_id` 는 구 Gemini CLI 가 남긴 것이고(실측 mtime 2025-07-03, agy 실행에
//! 안 건드려짐), 그 계정 id 는 agy DB 어디에도 없다. DB 전체에 이메일 문자열도 0건이다.
//! 그래서 계정이 아니라 **설치본 단위**로만 묶인다 — [`crate::accounts`] 참고.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};

use crate::context::{ContextState, RawContext};
use crate::model::{ScanOutcome, Source, SourceStatus, UsageEvent};
use crate::pricing::PriceTable;
use crate::protobuf::Message;
use crate::session::{dir_label, from_file_uri, SessionRow};

/// 잠긴 DB 를 기다리는 한도. agy 가 쓰는 중이면 잠깐 뒤 풀리고, 안 풀려도
/// 다음 스캔에서 다시 시도하므로 길게 잡을 이유가 없다.
const BUSY_TIMEOUT: Duration = Duration::from_millis(200);

/// dedup 키와 함께 캐시되는 파싱 결과. 같은 대화 DB 가 여러 홈에 복사돼 있어도
/// 요청 id 로 걸러 두 번 집계되지 않는다.
struct ParsedEvent {
    dedup_key: String,
    ev: UsageEvent,
}

struct FileCache {
    /// `.db` 와 `-wal` 을 합친 서명 — WAL 모드면 본체 mtime 이 안 움직일 수 있다
    stamp: (SystemTime, u64, SystemTime, u64),
    events: Vec<ParsedEvent>,
    ctx: RawContext,
    /// DB·WAL 중 마지막으로 쓰인 시각 — agy 에는 세션 레지스트리가 없어
    /// 작업 중 판정을 이 신선도로 유도한다
    written_at: DateTime<Utc>,
    /// 최근 세션 목록용 (제목·작업 위치 포함)
    session: Option<SessionRow>,
}

pub struct AntigravityAdapter {
    /// 각 홈 = `antigravity-cli` 디렉토리 (그 아래 `conversations/`)
    homes: Vec<PathBuf>,
    cache: HashMap<PathBuf, FileCache>,
}

impl AntigravityAdapter {
    pub fn new(homes: Vec<PathBuf>) -> Self {
        Self { homes, cache: HashMap::new() }
    }

    pub fn with_default_roots() -> Self {
        Self::new(crate::roots::antigravity_homes())
    }

    pub fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        if self.homes.is_empty() {
            return ScanOutcome { events: vec![], status: SourceStatus::NoData };
        }

        let mut seen = HashSet::new();
        let mut any_file = false;

        for home in &self.homes {
            let dir = home.join("conversations");
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("db") {
                    continue;
                }
                any_file = true;
                let Some(stamp) = db_stamp(&path) else { continue };
                seen.insert(path.clone());
                // WAL 이 갱신되면 본체가 그대로여도 작업이 있었다는 뜻이다
                let written_at: DateTime<Utc> = stamp.0.max(stamp.2).into();
                let needs = self.cache.get(&path).map(|c| c.stamp != stamp).unwrap_or(true);
                if needs {
                    // 잠겨서 못 읽으면 이전 캐시를 유지한다 — 지워 버리면 agy 가 도는 동안
                    // 사용량이 통째로 사라져 보인다
                    if let Some((events, ctx, session)) = parse_conversation(&path) {
                        self.cache.insert(
                            path.clone(),
                            FileCache { stamp, events, ctx, written_at, session },
                        );
                    }
                } else if let Some(fc) = self.cache.get_mut(&path) {
                    fc.written_at = written_at;
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

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

        let status = if any_file { SourceStatus::Ok } else { SourceStatus::NoData };
        ScanOutcome { events, status }
    }

    /// 가장 최근에 움직인 대화의 컨텍스트 창 사용량.
    /// 창 크기를 로그가 직접 알려주므로 단가표 힌트는 예비용이다.
    pub fn context(&self, pricing: &PriceTable) -> Option<ContextState> {
        let best = self
            .cache
            .values()
            .map(|fc| &fc.ctx)
            .filter(|c| !c.is_empty())
            .max_by_key(|c| c.last_activity())?;
        Some(crate::context::resolve(
            Source::Antigravity,
            best,
            pricing.context_window(&best.model),
        ))
    }

    /// 최근 대화 목록. 제목·작업 위치까지 `parse_conversation` 이 채워 둔다.
    pub fn sessions(&self) -> Vec<SessionRow> {
        self.cache.values().filter_map(|fc| fc.session.clone()).collect()
    }

    /// 스캔한 대화 DB 중 가장 최근에 쓰인 시각 — 작업 중 판정용.
    ///
    /// `presence/<uuid>.lock` 이 있어서 그쪽이 나아 보이지만 쓸 수 없다. 실측에서 agy
    /// 프로세스가 하나도 없는데 lock 파일 4개가 그대로 남아 있었다 — 종료 시 정리를
    /// 안 한다. mtime 신선도가 유일하게 믿을 수 있는 신호다.
    pub fn last_activity(&self) -> Option<DateTime<Utc>> {
        self.cache.values().map(|fc| fc.written_at).max()
    }

}

/// 계약 가입 ([`crate::adapter`]) — 인헌트 메서드에 위임만 한다.
/// agy 는 공식 한도도, 리셋 계산도 없다 — 기본 `None` 이 곧 사실이다 (plan.rs 표 참고).
impl crate::adapter::SourceAdapter for AntigravityAdapter {
    fn source(&self) -> Source {
        Source::Antigravity
    }
    fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome {
        AntigravityAdapter::scan(self, since)
    }
    fn context(&self, pricing: &PriceTable) -> Option<ContextState> {
        AntigravityAdapter::context(self, pricing)
    }
    fn sessions(&self) -> Vec<SessionRow> {
        AntigravityAdapter::sessions(self)
    }
}

/// 대화의 제목과 작업 위치 — **첫 스텝(첫 사용자 입력)** 에서 뽑는다.
///
/// `conversation_summaries.db` 에도 `preview` 가 있지만 실측에서 대화 5건 중 1건만
/// 들어 있었다 (요약 DB 갱신이 늦다). 첫 스텝은 5건 **전부**에 있어 이쪽이 맞다.
///
/// ```text
/// 19.2       첫 사용자 메시지  ← 제목
/// 19.12.12   작업 위치 (file:// URI)
/// ```
fn first_step_meta(conn: &Connection) -> (String, String) {
    let Ok(mut stmt) = conn.prepare("select step_payload from steps order by idx limit 1") else {
        return (String::new(), String::new());
    };
    let blob: Option<Vec<u8>> = stmt.query_row([], |r| r.get(0)).ok();
    let Some(blob) = blob else { return (String::new(), String::new()) };
    let step = Message::new(&blob);
    let Some(input) = step.msg(19) else { return (String::new(), String::new()) };
    // 세 소스 공통 규칙 — 첫 메시지에 줄바꿈·코드블록이 섞여 오고, 도구가 주입한
    // 래퍼가 들어올 수 있다. agy 에서 래퍼를 관측한 적은 없지만 규칙을 갈라 둘 이유가 없다.
    let raw = input.str(2).unwrap_or_default();
    let title =
        if crate::session::is_human_prompt(raw) { crate::session::first_line(raw) } else { String::new() };
    let workspace = input
        .msg(12)
        .and_then(|m| m.str(12))
        .map(from_file_uri)
        .unwrap_or_default();
    (title, workspace)
}

/// `.db` + `-wal` 의 (mtime, size) 를 합친 변경 감지용 서명
fn db_stamp(path: &Path) -> Option<(SystemTime, u64, SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let wal = path.with_extension("db-wal");
    let (wt, ws) = match std::fs::metadata(&wal) {
        Ok(w) => (w.modified().unwrap_or(SystemTime::UNIX_EPOCH), w.len()),
        Err(_) => (SystemTime::UNIX_EPOCH, 0),
    };
    Some((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), wt, ws))
}

/// 대화 DB 하나를 읽어 이벤트와 컨텍스트를 뽑는다.
/// 열지 못하거나(잠김·손상) 스키마가 다르면 `None` — 호출부가 이전 값을 유지한다.
fn parse_conversation(path: &Path) -> Option<(Vec<ParsedEvent>, RawContext, Option<SessionRow>)> {
    // 읽기 전용으로 연다. agy 가 쓰는 중이어도 읽기는 대개 통과한다.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(BUSY_TIMEOUT);

    let mut stmt = conn.prepare("select idx, data from gen_metadata order by idx").ok()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .ok()?;

    // 파일명이 곧 대화 uuid — 복사본끼리도 같은 값이라 세션 식별에 그대로 쓴다
    let session = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let mut ctx = RawContext { session: session.clone(), ..Default::default() };
    let (title, workspace) = first_step_meta(&conn);

    let mut out = vec![];
    let (mut last_at, mut last_model, mut tokens) = (None, String::new(), 0u64);
    // 대화 끝에 **모델도 사용량도 없이 컨텍스트만 있는 행**이 붙는다 (실측 idx 4:
    // 1.19 없음 / 1.4 전부 0 / 1.9.10.1 = 28,980). 그 행이 컨텍스트 대표가 되므로
    // 모델을 그때그때 읽으면 "미상"으로 떨어진다 → 마지막으로 본 모델을 물려준다.
    let mut known_model: Option<String> = None;

    for row in rows {
        let Ok((idx, blob)) = row else { continue };
        let root = Message::new(&blob);
        let Some(rec) = root.msg(1) else { continue };
        let Some(usage) = rec.msg(4) else { continue };

        let Some(ts) = rec.path(&[9, 4]).and_then(|t| {
            let secs = t.varint(1)?;
            let nanos = t.u64(2) as u32;
            Utc.timestamp_opt(secs as i64, nanos).single()
        }) else {
            continue;
        };
        if let Some(m) = rec.str(19).filter(|m| !m.is_empty()) {
            known_model = Some(m.to_string());
        }
        let model = known_model.clone().unwrap_or_else(|| "gemini-unknown".into());

        // ── 컨텍스트 ──
        // 첫 행은 시스템 프롬프트·툴 정의가 아직 안 잡혀 있어 값이 엉터리다 (실측 164).
        if idx > 0 {
            if let Some(c) = rec.path(&[9, 10]) {
                let tokens = c.u64(1);
                if tokens > 0 {
                    ctx.peak = ctx.peak.max(tokens);
                    if ctx.at.map(|prev| ts >= prev).unwrap_or(true) {
                        ctx.tokens = tokens;
                        ctx.at = Some(ts);
                        ctx.model = model.clone();
                        // 0 이면 미보고 — 단가표 힌트로 넘긴다
                        ctx.window = Some(c.u64(4)).filter(|w| *w > 0);
                    }
                }
            }
        }

        // ── 사용량 ──
        // 요청 단위 값이다 (누적 아님 — 값이 턴마다 오르내리는 것으로 확인).
        let input = usage.u64(2);
        let cache_read = usage.u64(5);
        let output = usage.u64(3);
        if input == 0 && cache_read == 0 && output == 0 {
            continue;
        }
        let dedup_key = usage
            .str(11)
            .map(String::from)
            .unwrap_or_else(|| format!("{session}#{idx}"));

        if last_at.map(|prev| ts >= prev).unwrap_or(true) {
            last_at = Some(ts);
            last_model = model.clone();
        }
        tokens += input + output + cache_read;

        out.push(ParsedEvent {
            dedup_key,
            ev: UsageEvent {
                source: Source::Antigravity,
                model,
                ts,
                // 1.4.2 는 캐시분을 **제외한** 값이다 (첫 요청 17,298 = 캐시 없음,
                // 이후 2,436 + 캐시 16,284 로 갈림) — 빼면 안 된다
                input,
                output,
                // 캐시 생성 비용을 따로 알려주는 필드는 없다
                cache_write: 0,
                cache_read,
                sidechain: false,
            },
        });
    }

    let row = last_at.map(|at| SessionRow {
        source: Source::Antigravity,
        id: session.clone(),
        label: if !title.is_empty() {
            title
        } else if !workspace.is_empty() {
            dir_label(&workspace)
        } else {
            session.chars().take(8).collect()
        },
        cwd: workspace,
        model: last_model,
        branch: String::new(),
        at,
        tokens,
    });
    Some((out, ctx, row))
}

/// 계약 테스트([`crate::adapter`] tests)용 표준 픽스처 — 내용 명세는 그쪽 주석 참고.
/// "같은 요청이 두 번 기록되는" 이 소스의 실제 형태는 **두 홈에 복사된 같은 대화 DB**
/// 다 (요청 id `1.4.11` 이 dedup 키 — 모듈 주석).
#[cfg(test)]
pub(crate) fn conformance_roots() -> (Vec<tempfile::TempDir>, Vec<PathBuf>) {
    use crate::protobuf::build::{field_bytes, field_varint};
    fn blob(input: u64, output: u64, req: &str, secs: u64) -> Vec<u8> {
        let usage =
            [field_varint(2, input), field_varint(3, output), field_bytes(11, req.as_bytes())]
                .concat();
        let gen9 = [
            field_bytes(4, &field_varint(1, secs)),
            field_bytes(10, &[field_varint(1, 100), field_varint(4, 256_000)].concat()),
        ]
        .concat();
        let rec =
            [field_bytes(4, &usage), field_bytes(9, &gen9), field_bytes(19, b"gemini-3.6-flash")]
                .concat();
        field_bytes(1, &rec)
    }
    let t0 = chrono::DateTime::parse_from_rfc3339("2026-08-13T01:00:00Z").unwrap().timestamp()
        as u64;
    // 제목은 steps 첫 행의 19.2 (첫 사용자 메시지)
    let step = field_bytes(19, &field_bytes(2, "계약 테스트 첫 질문".as_bytes()));
    let mut dirs = vec![];
    let mut roots = vec![];
    for _ in 0..2 {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("antigravity-cli");
        std::fs::create_dir_all(home.join("conversations")).unwrap();
        let conn =
            Connection::open(home.join("conversations/0f232cbb-b515-46e8-a6ca-f60532dca6d7.db"))
                .unwrap();
        conn.execute(
            "create table gen_metadata (idx integer primary key, data blob, size integer default 0)",
            [],
        )
        .unwrap();
        conn.execute("create table steps (idx integer primary key, step_payload blob)", [])
            .unwrap();
        conn.execute(
            "insert into steps (idx, step_payload) values (0, ?1)",
            rusqlite::params![step.clone()],
        )
        .unwrap();
        for (i, (input, output, req, secs)) in
            [(100u64, 10u64, "req-A", t0), (200, 20, "req-B", t0 + 300)].iter().enumerate()
        {
            conn.execute(
                "insert into gen_metadata (idx, data) values (?1, ?2)",
                rusqlite::params![i as i64, blob(*input, *output, req, *secs)],
            )
            .unwrap();
        }
        drop(conn);
        roots.push(home);
        dirs.push(d);
    }
    (dirs, roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protobuf::build::{field_bytes, field_varint};

    // ── 턴 추적 ──

    /// 실측 줄 모양 그대로 (필요 필드만)
    fn step(kind: &str, tools: bool) -> String {
        let tc = if tools { r#","tool_calls":[{"name":"view_file"}]"# } else { "" };
        format!(r#"{{"step_index":"0","source":"MODEL","type":"{kind}","status":"DONE"{tc}}}"#)
    }

    fn conv_home(id: &str, lines: &[String]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let logs = d.path().join("brain").join(id).join(".system_generated").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        std::fs::write(logs.join("transcript.jsonl"), body).unwrap();
        d
    }

    fn append(home: &Path, id: &str, lines: &[String]) {
        let p = home
            .join("brain").join(id).join(".system_generated").join("logs").join("transcript.jsonl");
        let mut s = std::fs::read_to_string(&p).unwrap_or_default();
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        std::fs::write(p, s).unwrap();
    }

    const CONV: &str = "0f232cbb-b515-46e8-a6ca-f60532dca6d7";

    /// 실측 흐름 그대로: 입력 → 도구 왕복 → 최종 답변
    #[test]
    fn turn_starts_on_input_and_ends_on_the_final_answer() {
        let now = Utc::now();
        let home = conv_home(CONV, &[step("USER_INPUT", false)]);
        let homes = vec![home.path().to_path_buf()];
        let mut w = TurnWatcher::default();

        // 첫 관측은 과거를 되짚지 않는다 — 이미 있던 줄로 시작했다고 보면 안 된다
        let first = w.poll(&homes, now);
        assert!(first.covered);
        assert!(first.running.is_empty(), "첫 회차는 기준점만 잡는다");

        append(home.path(), CONV, &[step("USER_INPUT", false)]);
        assert_eq!(w.poll(&homes, now).running, vec![CONV.to_string()]);

        // 도구를 부르는 동안은 계속 작업 중
        append(home.path(), CONV, &[step("PLANNER_RESPONSE", true), step("VIEW_FILE", false)]);
        assert_eq!(w.poll(&homes, now).running, vec![CONV.to_string()], "도구 왕복 중");

        // tool_calls 없는 PLANNER_RESPONSE = 최종 답변
        append(home.path(), CONV, &[step("PLANNER_RESPONSE", false)]);
        assert!(w.poll(&homes, now).running.is_empty(), "최종 답변에 풀려야 한다");
    }

    /// 종료 줄이 영영 안 와도(크래시) 언젠가 풀린다
    #[test]
    fn a_turn_that_never_ends_is_released() {
        let now = Utc::now();
        let home = conv_home(CONV, &[]);
        let homes = vec![home.path().to_path_buf()];
        let mut w = TurnWatcher::default();
        w.poll(&homes, now);
        append(home.path(), CONV, &[step("USER_INPUT", false)]);
        assert!(!w.poll(&homes, now).running.is_empty());

        // **크래시를 완료로 보고하면 안 된다.** running 에서 빠지는 건 정상 완료와
        // 같지만, 그건 종료 신호를 못 받았다는 뜻이지 끝났다는 뜻이 아니다.
        let later = now + chrono::Duration::milliseconds(TURN_STALE_MS + 1);
        let p = w.poll(&homes, later);
        assert!(p.running.is_empty(), "안전망이 풀어 준다");
        assert!(p.completed.is_empty(), "그러나 완료는 아니다");
    }

    /// `tool_calls` 없는 최종 답변만 완료다 — 도구를 부르는 중간 응답은 아니다.
    #[test]
    fn only_the_final_response_completes_the_turn() {
        let now = Utc::now();
        let home = conv_home(CONV, &[]);
        let homes = vec![home.path().to_path_buf()];
        let mut w = TurnWatcher::default();
        w.poll(&homes, now);

        append(home.path(), CONV, &[step("USER_INPUT", false)]);
        assert!(w.poll(&homes, now).completed.is_empty(), "이제 막 시작했다");

        // 도구를 부르는 응답 — 턴은 계속된다
        append(home.path(), CONV, &[step("PLANNER_RESPONSE", true)]);
        let mid = w.poll(&homes, now);
        assert!(!mid.running.is_empty(), "도구 호출 중이면 아직 돈다");
        assert!(mid.completed.is_empty());

        append(home.path(), CONV, &[step("PLANNER_RESPONSE", false)]);
        let p = w.poll(&homes, now);
        assert!(p.running.is_empty());
        assert_eq!(p.completed, vec![CONV.to_string()]);
    }

    /// 대화마다 파일이 따로라 **동시 대화가 각각** 추적된다 (크기 변화 방식이 못 하던 것)
    #[test]
    fn concurrent_conversations_are_tracked_separately() {
        let now = Utc::now();
        let other = "8bdd6789-2bd0-4b51-801c-8791b3064b41";
        let home = conv_home(CONV, &[]);
        let logs = home.path().join("brain").join(other).join(".system_generated").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("transcript.jsonl"), "").unwrap();
        let homes = vec![home.path().to_path_buf()];
        let mut w = TurnWatcher::default();
        w.poll(&homes, now);

        append(home.path(), CONV, &[step("USER_INPUT", false)]);
        append(home.path(), other, &[step("USER_INPUT", false)]);
        let mut running = w.poll(&homes, now).running;
        running.sort();
        assert_eq!(running.len(), 2, "둘 다 돌고 있어야 한다");

        // 하나만 끝나면 다른 하나는 그대로
        append(home.path(), CONV, &[step("PLANNER_RESPONSE", false)]);
        assert_eq!(w.poll(&homes, now).running, vec![other.to_string()]);
    }

    /// transcript 가 없는 옛 버전이면 폴백하라고 알려야 한다
    #[test]
    fn no_transcript_means_not_covered() {
        let d = tempfile::tempdir().unwrap();
        let mut w = TurnWatcher::default();
        let poll = w.poll(&[d.path().to_path_buf()], Utc::now());
        assert!(!poll.covered);
        assert!(poll.running.is_empty());
    }

    /// 실데이터와 같은 배치로 gen_metadata 행 하나를 만든다.
    struct Row {
        idx: i64,
        input: u64,
        output: u64,
        cached: u64,
        req_id: &'static str,
        secs: u64,
        ctx_tokens: u64,
        window: u64,
    }

    fn blob(r: &Row) -> Vec<u8> {
        blob_with_model(r, "gemini-3.6-flash")
    }

    fn blob_with_model(r: &Row, model: &str) -> Vec<u8> {
        let mut usage = [
            field_varint(2, r.input),
            field_varint(3, r.output),
        ]
        .concat();
        if r.cached > 0 {
            usage.extend(field_varint(5, r.cached));
        }
        usage.extend(field_bytes(11, r.req_id.as_bytes()));

        let time = field_varint(1, r.secs);
        let ctx = [field_varint(1, r.ctx_tokens), field_varint(4, r.window)].concat();
        let gen9 = [field_bytes(4, &time), field_bytes(10, &ctx)].concat();

        let mut rec = [field_bytes(4, &usage), field_bytes(9, &gen9)].concat();
        // 빈 문자열이면 1.19 필드를 아예 안 넣는다 (실데이터의 꼬리 행과 같은 모양)
        if !model.is_empty() {
            rec.extend(field_bytes(19, model.as_bytes()));
        }
        field_bytes(1, &rec)
    }

    fn write_db(path: &Path, rows: &[Row]) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "create table gen_metadata (idx integer primary key, data blob, size integer default 0)",
            [],
        )
        .unwrap();
        for r in rows {
            conn.execute(
                "insert into gen_metadata (idx, data) values (?1, ?2)",
                rusqlite::params![r.idx, blob(r)],
            )
            .unwrap();
        }
    }

    /// 실측한 대화 0f232cbb 의 앞부분·끝부분을 그대로 옮긴 것
    fn sample() -> Vec<Row> {
        vec![
            Row { idx: 0, input: 9167, output: 232, cached: 8143, req_id: "CdB5at", secs: 1_786_368_009, ctx_tokens: 164, window: 256_000 },
            Row { idx: 1, input: 2436, output: 139, cached: 16284, req_id: "C9B5av", secs: 1_786_368_011, ctx_tokens: 26_506, window: 256_000 },
            Row { idx: 2, input: 2187, output: 994, cached: 28518, req_id: "jNB5aq", secs: 1_786_368_139, ctx_tokens: 37_829, window: 256_000 },
        ]
    }

    /// 행마다 모델명을 달리 줄 수 있는 변형 (빈 문자열 = 1.19 필드 자체가 없음)
    fn home_with_models(rows: &[Row], models: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("antigravity-cli");
        std::fs::create_dir_all(home.join("conversations")).unwrap();
        let db = home.join("conversations/0f232cbb-b515-46e8-a6ca-f60532dca6d7.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "create table gen_metadata (idx integer primary key, data blob, size integer default 0)",
            [],
        )
        .unwrap();
        for (i, r) in rows.iter().enumerate() {
            let m = models.get(i).copied().unwrap_or("gemini-3.6-flash");
            conn.execute(
                "insert into gen_metadata (idx, data) values (?1, ?2)",
                rusqlite::params![r.idx, blob_with_model(r, m)],
            )
            .unwrap();
        }
        drop(conn);
        (dir, home)
    }

    fn home_with(rows: &[Row]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("antigravity-cli");
        std::fs::create_dir_all(home.join("conversations")).unwrap();
        write_db(&home.join("conversations/0f232cbb-b515-46e8-a6ca-f60532dca6d7.db"), rows);
        (dir, home)
    }

    #[test]
    fn parses_per_request_tokens() {
        let (_d, home) = home_with(&sample());
        let mut a = AntigravityAdapter::new(vec![home]);
        let out = a.scan(DateTime::UNIX_EPOCH.into());

        assert_eq!(out.status, SourceStatus::Ok);
        assert_eq!(out.events.len(), 3);

        let e0 = &out.events[0];
        assert_eq!(e0.model, "gemini-3.6-flash");
        assert_eq!(e0.source, Source::Antigravity);
        // 1.4.2 는 이미 캐시분을 뺀 값이라 그대로 들어가야 한다
        assert_eq!(e0.input, 9167);
        assert_eq!(e0.cache_read, 8143);
        assert_eq!(e0.output, 232);
        assert_eq!(e0.cache_write, 0);

        // 요청 단위(델타)라 값이 누적되지 않는다
        assert_eq!(out.events[1].input, 2436);
    }

    #[test]
    fn context_uses_reported_window_and_skips_the_bogus_first_row() {
        let (_d, home) = home_with(&sample());
        let mut a = AntigravityAdapter::new(vec![home]);
        a.scan(DateTime::UNIX_EPOCH.into());
        let c = a.context(&PriceTable::builtin()).unwrap();

        assert_eq!(c.source, Source::Antigravity);
        assert_eq!(c.session, "0f232cbb-b515-46e8-a6ca-f60532dca6d7");
        assert_eq!(c.model, "gemini-3.6-flash");
        assert_eq!(c.tokens, 37_829, "마지막 행의 총계여야 함");
        assert_eq!(c.window, 256_000, "로그가 알려준 창을 그대로 써야 함");
        assert!(!c.window_inferred);
        assert!(!c.interim);
    }

    /// 대화 끝에는 **모델도 사용량도 없이 컨텍스트만 있는 행**이 붙는다 (실측 idx 4).
    /// 그 행이 컨텍스트 대표가 되므로, 모델을 물려받지 않으면 게이지·카드에 "미상"이 뜬다.
    #[test]
    fn trailing_context_only_row_keeps_the_last_known_model() {
        let mut rows = sample();
        rows.push(Row {
            idx: 3,
            input: 0,
            output: 0,
            cached: 0,
            req_id: "TAIL",
            secs: 1_786_368_200,
            ctx_tokens: 40_000,
            window: 256_000,
        });
        let (_d, home) = home_with_models(&rows, &["gemini-3.6-flash", "gemini-3.6-flash", "gemini-3.6-flash", ""]);

        let mut a = AntigravityAdapter::new(vec![home]);
        let out = a.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 3, "사용량 0인 꼬리 행은 이벤트가 아니다");

        let c = a.context(&PriceTable::builtin()).unwrap();
        assert_eq!(c.tokens, 40_000, "꼬리 행이 가장 최신 컨텍스트다");
        assert_eq!(c.model, "gemini-3.6-flash", "모델 필드가 없어도 미상으로 떨어지면 안 됨");
    }

    /// 첫 행 하나뿐인 대화는 컨텍스트를 만들어내면 안 된다 — 164/256,000 = 0.06% 라는
    /// 새빨간 거짓말이 게이지에 뜬다.
    #[test]
    fn single_row_conversation_has_no_context() {
        let (_d, home) = home_with(&sample()[..1]);
        let mut a = AntigravityAdapter::new(vec![home]);
        let out = a.scan(DateTime::UNIX_EPOCH.into());
        assert_eq!(out.events.len(), 1, "사용량 집계는 그대로 되어야 함");
        assert!(a.context(&PriceTable::builtin()).is_none());
    }

    /// 같은 대화 DB 가 두 홈에 있을 때 (홈을 직접 추가하면 실제로 생긴다)
    #[test]
    fn same_conversation_in_two_homes_counted_once() {
        let (_a, home_a) = home_with(&sample());
        let (_b, home_b) = home_with(&sample());
        let mut a = AntigravityAdapter::new(vec![home_a, home_b.clone()]);
        assert_eq!(a.scan(DateTime::UNIX_EPOCH.into()).events.len(), 3);

        // 서로 다른 요청은 그대로 남아야 한다 (dedup 이 과하게 먹으면 안 됨)
        let mut extra = sample();
        extra.push(Row { idx: 3, input: 100, output: 10, cached: 0, req_id: "NEW", secs: 1_786_368_200, ctx_tokens: 40_000, window: 256_000 });
        write_db(
            &home_b.join("conversations/other.db"),
            &extra[3..],
        );
        let mut a = AntigravityAdapter::new(vec![home_b]);
        assert_eq!(a.scan(DateTime::UNIX_EPOCH.into()).events.len(), 4);
    }

    #[test]
    fn no_home_reports_no_data() {
        let mut a = AntigravityAdapter::new(vec![]);
        assert_eq!(a.scan(DateTime::UNIX_EPOCH.into()).status, SourceStatus::NoData);
    }

    #[test]
    fn home_without_conversations_reports_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("antigravity-cli");
        std::fs::create_dir_all(&home).unwrap();
        let mut a = AntigravityAdapter::new(vec![home]);
        assert_eq!(a.scan(DateTime::UNIX_EPOCH.into()).status, SourceStatus::NoData);
    }

    /// 손상된 blob 이 섞여도 나머지 행은 살아야 한다
    #[test]
    fn corrupt_blob_does_not_kill_the_conversation() {
        let (_d, home) = home_with(&sample());
        let db = home.join("conversations/0f232cbb-b515-46e8-a6ca-f60532dca6d7.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "insert into gen_metadata (idx, data) values (9, ?1)",
            rusqlite::params![vec![0xff_u8, 0xff, 0xff]],
        )
        .unwrap();
        drop(conn);

        let mut a = AntigravityAdapter::new(vec![home]);
        assert_eq!(a.scan(DateTime::UNIX_EPOCH.into()).events.len(), 3);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 턴 추적 — 크기 변화 유도 대신 **파일에 적힌 턴 경계**를 읽는다
// ─────────────────────────────────────────────────────────────────────────────

/// 턴이 영영 안 끝날 때 풀어 주는 안전망 (크래시·강제 종료).
/// 스텝 줄이 몇 초 간격으로 흐르므로(실측: 9초 턴에 10줄) 5분이면 넉넉하다.
pub const TURN_STALE_MS: i64 = 5 * 60 * 1000;

/// `transcript.jsonl` 한 줄. 쓰는 필드만 받는다.
#[derive(serde::Deserialize)]
struct TranscriptLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    /// **있으면 도구를 호출한 것** = 턴이 계속된다. 최종 답변에는 이 키가 아예 없다.
    tool_calls: Option<serde_json::Value>,
}

struct ConvTurn {
    /// transcript 에서 읽은 지점 — 매 회차 새로 늘어난 부분만 본다
    offset: u64,
    running: bool,
    /// 마지막으로 줄을 본 시각 (안전망 기준)
    last_activity: DateTime<Utc>,
}

/// [`TurnWatcher::poll`] 결과 — Codex 와 같은 모양이라 계약 모듈이 정의를 갖는다.
/// 여기서 `covered` 는 "transcript 를 읽을 수 있었는가"(옛 버전엔 파일이 없다),
/// `completed` 는 "최종 답변(`tool_calls` 없는 `PLANNER_RESPONSE`)으로 끝났는가"다.
pub use crate::adapter::TurnPoll;

/// agy 턴 추적.
///
/// **왜 이렇게 하는가** — 예전의 크기 변화 방식(이후 제거됨)은 세 가지를 잃고 있었다.
/// 종료를 45초 창으로 때려 맞춰야 했고, 감시 파일이 **가장 최근 대화 하나**뿐이라 동시
/// 대화를 못 봤으며, DB 크기는 턴과 무관하게도 출렁였다. 실측(2026-08-13, 감시 30분):
///
/// ```text
/// 12:25:19  DB  +28672B     ← 아무도 입력하지 않음
/// 12:25:27  DB  -840512B    ← 크기가 **줄어듦** (WAL 체크포인트)
/// 12:25:41  STEP idx=34 USER_INPUT        ← 진짜 턴 시작은 여기
/// ```
///
/// `<홈>/brain/<대화 uuid>/.system_generated/logs/transcript.jsonl` 이 답이었다.
/// **대화마다 파일이 따로**이고 uuid 가 `conversations/<uuid>.db` 와 정확히 같으며,
/// 줄이 **턴 도중에 흘러나온다**(같은 실측, 9초 턴에 10줄):
///
/// ```text
/// 12:26:03  idx=40 USER_INPUT                  ← 제출한 그 초에 붙는다
/// 12:26:04  idx=41 PLANNER_RESPONSE  tool_calls 있음
/// 12:26:04  idx=42 LIST_DIRECTORY
/// …
/// 12:26:12  idx=49 PLANNER_RESPONSE  tool_calls 없음  ← 최종 답변 = 턴 종료
/// ```
///
/// 그래서 `USER_INPUT` 으로 시작을, **`tool_calls` 없는 `PLANNER_RESPONSE`** 로 종료를
/// 읽는다. 실측 61건이 깔끔하게 갈렸다 — 도구 호출 45건은 전부 `content` 가 없고,
/// 최종 답변 16건은 `tool_calls` 키가 아예 없다.
#[derive(Default)]
pub struct TurnWatcher {
    /// transcript 경로별 상태
    convs: HashMap<PathBuf, ConvTurn>,
}

/// `<홈>/brain/*/.system_generated/logs/transcript.jsonl` 을 훑는다.
/// 대화 폴더만 한 겹 읽으므로 재귀 탐색이 아니다.
fn transcripts(home: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(home.join("brain")) else { return vec![] };
    let mut out = vec![];
    for e in entries.filter_map(|e| e.ok()) {
        if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(id) = e.file_name().to_str().map(str::to_string) else { continue };
        let p = e.path().join(".system_generated").join("logs").join("transcript.jsonl");
        if p.is_file() {
            out.push((id, p));
        }
    }
    out
}

/// 계약 가입 ([`crate::adapter::TurnWatch`]) — 인헌트 `poll` 에 위임만 한다.
impl crate::adapter::TurnWatch for TurnWatcher {
    fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll {
        TurnWatcher::poll(self, homes, now)
    }
}

impl TurnWatcher {
    pub fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll {
        let mut covered = false;
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut running = vec![];
        let mut completed = vec![];

        for home in homes {
            for (id, path) in transcripts(home) {
                covered = true;
                seen.insert(path.clone());
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

                let start = match self.convs.get(&path) {
                    // **첫 관측은 과거 이력을 되짚지 않는다.** 그러면 지난 대화가 전부
                    // 방금 시작한 것처럼 보인다 (Codex 와 같은 규칙).
                    None => size,
                    Some(c) if c.offset > size => 0,
                    Some(c) => c.offset,
                };
                let (lines, consumed) = crate::live::read_from(&path, start);
                let turn = self.convs.entry(path.clone()).or_insert(ConvTurn {
                    offset: start,
                    running: false,
                    last_activity: now,
                });
                turn.offset = start + consumed;

                let mut finished_cleanly = false;
                for line in &lines {
                    let Ok(v) = serde_json::from_str::<TranscriptLine>(line) else { continue };
                    turn.last_activity = now;
                    match v.kind.as_deref() {
                        Some("USER_INPUT") => {
                            turn.running = true;
                            finished_cleanly = false;
                        }
                        // 도구를 부르는 중이면 아직 끝이 아니다.
                        // 돌고 있던 턴에 온 최종 답변만 완료로 센다.
                        Some("PLANNER_RESPONSE") if !has_tools(&v) => {
                            finished_cleanly = turn.running;
                            turn.running = false;
                        }
                        _ => {}
                    }
                }
                if finished_cleanly {
                    completed.push(id.clone());
                }

                // 종료 이벤트가 영영 안 오는 경우(크래시) 풀어 준다.
                // **완료 판정 뒤에 와야 한다** — 앞에 두면 타임아웃이 완료로 샌다.
                if turn.running && (now - turn.last_activity).num_milliseconds() > TURN_STALE_MS {
                    turn.running = false;
                }
                if turn.running {
                    running.push(id);
                }
            }
        }
        // 지워진 대화는 들고 있을 이유가 없다
        self.convs.retain(|p, _| seen.contains(p));
        TurnPoll { covered, running, completed }
    }
}

fn has_tools(v: &TranscriptLine) -> bool {
    match &v.tool_calls {
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Null) | None => false,
        // 배열이 아닌 모양으로 오면 뜻을 모르니 "도구 있음"으로 본다 —
        // 턴을 일찍 끝내는 것보다 안전망(TURN_STALE_MS)에 맡기는 쪽이 낫다
        Some(_) => true,
    }
}

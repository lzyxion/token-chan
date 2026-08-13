//! 어댑터 계약 — 세 소스가 지켜야 하는 공통 모양을 **코드로** 못박는다.
//!
//! 이 trait 이 생기기 전에는 세 어댑터가 같은 메서드 이름을 관례로만 지켰다. 관례는
//! 컴파일러가 안 지켜 준다 — 네 번째 소스를 붙이거나 어댑터 하나를 고칠 때 "뭘 지켜야
//! 하는지"가 코드 밖(문화)에 있었다. 이제 소스 추가 = [`SourceAdapter`] 구현 하나이고,
//! 오케스트레이터(모니터 스레드)는 소스를 구분하지 않고 trait 객체 목록을 돈다.
//!
//! **파싱이 제각각인 건 그대로 둔다 — 그게 본질이다.** 세 CLI 가 데이터를 다르게
//! 저장하므로(JSONL / rollout / SQLite+protobuf) 어댑터 내부는 통일할 수 없고, 통일할
//! 것은 **경계**다: 무엇을 내놓아야 하는가, 없는 능력은 어떻게 말하는가.
//!
//! # 계약
//!
//! | 메서드 | 의미 | 규칙 |
//! |---|---|---|
//! | `scan` | 트랜스크립트 → [`UsageEvent`] 스트림 | **요청 1건 = 이벤트 1건** (파일·줄 단위가 아니라). 루트가 없으면 `NoData` — "0 사용"과 "데이터 없음"은 다른 사실이다 |
//! | `sessions` | 최근 세션 목록 | 제목은 **첫 사람 메시지** ([`crate::session`] 공통 규칙) |
//! | `context` | 가장 최근 세션의 컨텍스트 | 모르면 `None` — 지어내지 않는다 |
//! | `plan` | 스캔한 파일에 실려 온 공식 한도 | 주는 소스만 (Codex rollout). 기본 `None` |
//! | `session_reset` | 5시간 창 리셋 자체 계산 | 하는 소스만 (Claude [`crate::blocks`]). 기본 `None` |
//!
//! 능력 편차는 `Option` 반환 기본 메서드로 표현한다 — 없는 능력을 빈 값·0 으로
//! 흉내내면 화면이 거짓말을 하게 된다 (README "가능한 것과 아닌 것이 다릅니다").
//!
//! 작업 중 감지는 어댑터가 아니라 [`TurnWatch`] 로 따로 묶는다 — 폴링 주기가 다르고
//! (2초 vs 10초), 상태(파일 오프셋)를 어댑터와 공유하지 않으며, Claude 는 이 방식이
//! 아니라 세션 레지스트리를 읽는다 ([`crate::live`]).

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::context::ContextState;
use crate::model::{ScanOutcome, Source};
use crate::plan::PlanUsage;
use crate::pricing::PriceTable;
use crate::session::SessionRow;

/// 사용량 소스 하나의 공통 인터페이스. 구현은 [`crate::claude`]·[`crate::codex`]·
/// [`crate::antigravity`] — 각 모듈의 인헌트 메서드에 위임만 한다 (예제·테스트는
/// 구체 타입을 그대로 쓰므로 인헌트가 원본이고 trait 은 계약이다).
pub trait SourceAdapter {
    /// 이 어댑터가 다루는 소스 — trait 객체 목록에서 자기소개용
    fn source(&self) -> Source;
    /// 루트를 훑어 `since` 이후의 사용량 이벤트를 정규화해 내놓는다
    fn scan(&mut self, since: DateTime<Utc>) -> ScanOutcome;
    /// 가장 최근에 움직인 세션의 컨텍스트 게이지 (트랜스크립트 파일 단위 정보)
    fn context(&self, pricing: &PriceTable) -> Option<ContextState>;
    /// 최근 세션 목록 (마지막 스캔 기준)
    fn sessions(&self) -> Vec<SessionRow>;
    /// 스캔한 파일에 실려 온 공식 한도 — 주는 소스(Codex)만 구현한다
    fn plan(&self) -> Option<PlanUsage> {
        None
    }
    /// 5시간 창 종료를 트랜스크립트에서 계산 — 하는 소스(Claude)만 구현한다
    fn session_reset(&self, _now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        None
    }
}

/// [`TurnWatch::poll`] 결과. Codex·agy 가 같은 모양을 내놓는다 — 파일은 달라도
/// (history+rollout vs brain transcript) 사실의 종류가 같아서다.
pub struct TurnPoll {
    /// 턴 파일을 하나라도 읽을 수 있었는지 — **진단용**이다. `false` 면 이 방식이
    /// 성립하지 않으므로(설정으로 껐거나 옛 버전) 그 홈의 세션은 작업 중으로 잡히지
    /// 않는다. 예전엔 여기서 크기 변화로 폴백했지만 그 신호로는 완료·크래시가
    /// 구분되지 않아 제거했다 ([`crate::live`] 모듈 주석).
    pub covered: bool,
    /// 지금 턴이 돌고 있는 세션/대화 id 들
    pub running: Vec<String>,
    /// 이번 회차에 **종료 신호로** 끝난 id 들. 안전망 타임아웃으로 풀린 것은 여기
    /// 없다 — 크래시·취소와 완료를 가르는 신호다. 신호의 결은 소스마다 다르다:
    /// Codex 는 `task_complete`/`turn_aborted` 로 사유까지 갈리고, agy 는
    /// `tool_calls` 없는 `PLANNER_RESPONSE` 하나뿐이라 셋 중 가장 약한 고리다.
    pub completed: Vec<String>,
}

/// 턴 경계 추적기의 공통 인터페이스. 어댑터와 분리된 이유는 모듈 주석 참고.
pub trait TurnWatch {
    /// 홈들을 훑어 턴 시작/종료를 읽는다. 파일 오프셋을 내부에 들고 증분만 본다.
    fn poll(&mut self, homes: &[PathBuf], now: DateTime<Utc>) -> TurnPoll;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceStatus;

    /// 세 어댑터 전부 — 계약 테스트는 이 목록을 돌며 **같은 시험지**를 치른다.
    /// 소스를 추가하면 여기 한 줄을 늘리는 것이 계약 가입이다.
    fn all() -> Vec<Box<dyn SourceAdapter>> {
        vec![
            Box::new(crate::claude::ClaudeAdapter::new(vec![])),
            Box::new(crate::codex::CodexAdapter::new(vec![])),
            Box::new(crate::antigravity::AntigravityAdapter::new(vec![])),
        ]
    }

    #[test]
    fn every_adapter_names_its_source() {
        let seen: Vec<Source> = all().iter().map(|a| a.source()).collect();
        assert_eq!(seen, vec![Source::Claude, Source::Codex, Source::Antigravity]);
    }

    /// 루트가 없으면(설치본 없음) `NoData` 다 — "0 사용"(Ok + 빈 이벤트)과 "데이터
    /// 없음"은 화면에서 다르게 그려야 하는 다른 사실이다. 부가 정보도 지어내지 않는다.
    #[test]
    fn empty_roots_mean_nodata_not_zero_usage() {
        let pricing = PriceTable::with_overrides(None);
        for mut a in all() {
            let out = a.scan(Utc::now() - chrono::Duration::days(1));
            assert!(out.events.is_empty(), "{:?}", a.source());
            assert_eq!(out.status, SourceStatus::NoData, "{:?}", a.source());
            assert!(a.sessions().is_empty(), "{:?}", a.source());
            assert!(a.context(&pricing).is_none(), "{:?}", a.source());
            assert!(a.plan().is_none(), "{:?}", a.source());
            assert!(a.session_reset(Utc::now()).is_none(), "{:?}", a.source());
        }
    }

    // ── 표준 픽스처 계약 ──
    //
    // 각 소스의 `conformance_roots()` 는 **같은 내용**을 자기 포맷으로 만든다:
    //
    //   - 첫 사람 메시지: "계약 테스트 첫 질문"
    //   - 요청 A: 입력 100 / 출력 10 (2026-08-13T01:00Z) — **두 번 기록됨**.
    //     중복의 형태는 그 소스가 실제로 겪는 그대로다: Claude 는 같은 파일의 반복 줄,
    //     Codex·agy 는 두 홈에 복사된 같은 파일.
    //   - 요청 B: 입력 200 / 출력 20 (2026-08-13T01:05Z)
    //
    // 아래 시험지는 이 내용이 소스와 무관하게 **같은 사실**로 나오는지 본다.
    // 파서가 제각각인 건 본질이라 그대로 두고, 경계의 행동만 못박는 것이다.

    const TITLE: &str = "계약 테스트 첫 질문";

    /// (픽스처 수명 가드, 어댑터) — 가드가 드롭되면 임시 홈이 지워진다
    fn subjects() -> Vec<(Vec<tempfile::TempDir>, Box<dyn SourceAdapter>)> {
        let (g1, r1) = crate::claude::conformance_roots();
        let (g2, r2) = crate::codex::conformance_roots();
        let (g3, r3) = crate::antigravity::conformance_roots();
        vec![
            (g1, Box::new(crate::claude::ClaudeAdapter::new(r1))),
            (g2, Box::new(crate::codex::CodexAdapter::new(r2))),
            (g3, Box::new(crate::antigravity::AntigravityAdapter::new(r3))),
        ]
    }

    /// 요청 1건 = 이벤트 1건. 두 번 기록된 요청 A 가 두 번 집계되면 과대집계다.
    #[test]
    fn the_standard_fixture_reads_the_same_in_every_source() {
        for (_guard, mut a) in subjects() {
            let src = a.source();
            let out = a.scan(chrono::DateTime::UNIX_EPOCH);
            assert_eq!(out.status, SourceStatus::Ok, "{src:?}");
            assert_eq!(out.events.len(), 2, "{src:?}: 두 번 기록된 요청 A 는 한 번이어야");
            assert!(out.events.iter().all(|e| e.source == src), "{src:?}: 소스 표기");
            assert_eq!(out.events.iter().map(|e| e.input).sum::<u64>(), 300, "{src:?}");
            assert_eq!(out.events.iter().map(|e| e.output).sum::<u64>(), 30, "{src:?}");
        }
    }

    /// 두 번째 스캔은 파일 캐시 경로다 — 같은 사실이 나와야 한다 (0 도, 두 배도 아님).
    #[test]
    fn a_rescan_returns_the_same_facts() {
        for (_guard, mut a) in subjects() {
            a.scan(chrono::DateTime::UNIX_EPOCH);
            let again = a.scan(chrono::DateTime::UNIX_EPOCH);
            assert_eq!(again.events.len(), 2, "{:?}", a.source());
            assert_eq!(again.status, SourceStatus::Ok, "{:?}", a.source());
        }
    }

    /// `since` 뒤로 다 걸러져도 파일이 있으면 `Ok` 다 — "보존기간 안에 사용 없음"과
    /// "미설치"(NoData)는 다른 사실이다. (픽스처 시각이 2026-08-13 이라 그 뒤 시각으로 거른다.)
    #[test]
    fn filtered_out_means_ok_not_nodata() {
        let since = DateTime::parse_from_rfc3339("2026-08-13T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (_guard, mut a) in subjects() {
            let out = a.scan(since);
            assert!(out.events.is_empty(), "{:?}", a.source());
            assert_eq!(out.status, SourceStatus::Ok, "{:?}", a.source());
        }
    }

    /// 세션 제목은 **첫 사람 메시지**다 ([`crate::session`] 공통 규칙). 같은 파일이 두 홈에
    /// 있으면 행이 여러 개일 수 있지만(합치기는 `session::merge` 몫) 제목 규칙은 같아야 한다.
    #[test]
    fn session_titles_come_from_the_first_human_prompt() {
        for (_guard, mut a) in subjects() {
            a.scan(chrono::DateTime::UNIX_EPOCH);
            let rows = a.sessions();
            assert!(!rows.is_empty(), "{:?}", a.source());
            let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
            assert!(labels.iter().all(|l| *l == TITLE), "{:?}: {labels:?}", a.source());
        }
    }
}

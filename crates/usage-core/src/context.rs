//! 세션 컨텍스트 창 사용량.
//!
//! Claude Code 는 컨텍스트 잔량을 별도 필드로 기록하지 않지만, assistant 메시지의
//! usage 로 정확히 복원된다:
//!
//! ```text
//! context = input_tokens + cache_creation_input_tokens + cache_read_input_tokens + output_tokens
//! ```
//!
//! 이 공식은 `compactMetadata.preTokens`(compact 직전에 Claude Code 가 남기는 실측치)와
//! 대조해 검증했다 — 한 세션에서는 정확히 일치했고(105,528+159 = 105,687), 다른 세션에서는
//! 137 토큰 차이가 났다(361,919 vs 362,056). 그 차이는 마지막 응답 뒤에 사용자가 친
//! 메시지분이다. 즉 **직전 턴 기준으로 정확하고, 최대 한 턴 지연**된다. 비용처럼 단가표에
//! 기반한 추정치가 아니므로 "추정" 표기가 필요 없다.
//!
//! compact 처리에서 주의할 점:
//! - `postTokens` 는 게이지 값이 **아니다**. 남긴 대화량만 센 값이라 시스템 프롬프트 +
//!   툴 정의(수만 토큰)가 빠져 있다. 실측에서 postTokens 9,667 vs 실제 다음 요청 49,876.
//!   그래서 compact 직후 잠정값은 `postTokens + baseline` 으로 잡고, 다음 assistant
//!   메시지가 오면 위 공식으로 덮어쓴다.
//! - 그 baseline(시스템 프롬프트 + 툴 정의) 때문에 게이지는 compact 후에도 0 으로
//!   떨어지지 않는다. 버그가 아니라 실제 바닥이다.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::Source;

/// 단가표에 `ctx` 가 없는 모델의 시작 가정치. 관측치가 넘으면 아래 구간으로 승격된다.
pub const DEFAULT_WINDOW: u64 = 200_000;

/// 알려진 컨텍스트 창 구간 (오름차순). 미지의 모델이 기본값을 넘겼을 때 이 중
/// 관측치를 담는 가장 작은 값으로 올라간다.
const TIERS: &[u64] = &[200_000, 500_000, 1_000_000, 2_000_000];

/// 트랜스크립트 1개에서 뽑은 원본 상태 (창 크기는 아직 해석하지 않음).
#[derive(Clone, Debug, Default)]
pub struct RawContext {
    /// 트랜스크립트의 sessionId
    pub session: String,
    /// 마지막 메인체인 assistant 메시지의 모델
    pub model: String,
    /// 그 메시지 기준 컨텍스트 크기
    pub tokens: u64,
    /// 그 메시지의 시각
    pub at: Option<DateTime<Utc>>,
    /// 세션에서 관측된 최대 컨텍스트 — 창 크기 승격의 근거
    pub peak: u64,
    /// 소스가 창 크기를 **직접 알려준** 경우 (Codex 의 `model_context_window`).
    /// 이건 권위 있는 값이라 단가표 힌트나 관측 승격보다 우선하고, 승격도 하지 않는다.
    pub window: Option<u64>,
    /// 시스템 프롬프트 + 툴 정의로 항상 깔리는 바닥.
    /// 세션 내 최소 cache_read 로 추정한다 (첫 요청의 cache_read 가 곧 그 프리픽스).
    pub baseline: u64,
    pub compactions: u32,
    /// 지금까지 compact 로 버려진 총량
    pub dropped: u64,
    pub last_compact_at: Option<DateTime<Utc>>,
    pub last_compact_trigger: Option<String>,
    /// 마지막 compact 의 postTokens — compact 가 마지막 이벤트일 때 잠정값 계산용
    pub last_compact_post: u64,
}

impl RawContext {
    /// 컨텍스트를 한 번도 못 읽었으면 표시할 게 없다.
    pub fn is_empty(&self) -> bool {
        self.tokens == 0 && self.compactions == 0
    }

    /// 이 세션이 마지막으로 움직인 시각 — 여러 트랜스크립트 중 활성 세션 선택 기준.
    pub fn last_activity(&self) -> Option<DateTime<Utc>> {
        match (self.at, self.last_compact_at) {
            (Some(a), Some(c)) => Some(a.max(c)),
            (a, c) => a.or(c),
        }
    }
}

/// 프론트로 나가는 컨텍스트 상태
#[derive(Clone, Debug, Serialize)]
pub struct ContextState {
    pub source: Source,
    pub session: String,
    pub model: String,
    /// 현재 컨텍스트 크기 (토큰)
    pub tokens: u64,
    /// 분모로 쓴 컨텍스트 창
    pub window: u64,
    /// 0..100
    pub used_pct: f64,
    /// 이 값을 읽은 시각
    pub at: Option<DateTime<Utc>>,
    /// compact 직후 다음 턴이 아직 없어 잠정값을 쓴 상태
    pub interim: bool,
    /// 창 크기가 소스가 알려준 값도, 단가표 값도 아니라 관측치로 추정된 것인지
    pub window_inferred: bool,
    pub compactions: u32,
    pub dropped: u64,
    /// 버려진 분량까지 합친 이 세션의 실제 대화 총량
    pub total: u64,
    pub last_compact_at: Option<DateTime<Utc>>,
    pub last_compact_trigger: Option<String>,
}

/// 단가표 힌트와 관측 최대치로 실효 창 크기를 정한다.
/// 관측치가 힌트를 넘으면 그 자체가 창이 더 크다는 증거이므로 승격한다.
fn effective_window(hint: Option<u64>, peak: u64) -> (u64, bool) {
    let base = hint.unwrap_or(DEFAULT_WINDOW);
    if peak <= base {
        return (base, hint.is_none());
    }
    let promoted = TIERS.iter().copied().find(|t| *t >= peak).unwrap_or(peak);
    (promoted.max(base), true)
}

/// 원본 상태 + 단가표의 `ctx` 힌트 → 표시용 상태.
///
/// 창 크기 우선순위: 소스가 직접 알려준 값(`raw.window`) → 단가표 `ctx` → 관측 승격.
/// 소스가 알려준 값은 승격하지 않는다 — 그 값이 곧 정답이고, 넘겼다면 100%(꽉 참)가 맞다.
pub fn resolve(source: Source, raw: &RawContext, window_hint: Option<u64>) -> ContextState {
    // compact 가 마지막 assistant 메시지보다 뒤면 tokens 가 compact 이전 값이라 낡았다.
    // 다음 턴이 올 때까지 postTokens + baseline 으로 잠정 표시한다.
    let stale = match (raw.last_compact_at, raw.at) {
        (Some(c), Some(a)) => c > a,
        (Some(_), None) => true,
        _ => false,
    };
    let (tokens, at, interim) = if stale {
        (
            raw.last_compact_post + raw.baseline,
            raw.last_compact_at,
            true,
        )
    } else {
        (raw.tokens, raw.at, false)
    };

    let (window, window_inferred) = match raw.window {
        Some(w) if w > 0 => (w, false),
        _ => effective_window(window_hint, raw.peak.max(tokens)),
    };
    let used_pct = if window == 0 {
        0.0
    } else {
        (tokens as f64 / window as f64 * 100.0).clamp(0.0, 100.0)
    };

    ContextState {
        source,
        session: raw.session.clone(),
        model: raw.model.clone(),
        tokens,
        window,
        used_pct,
        at,
        interim,
        window_inferred,
        compactions: raw.compactions,
        dropped: raw.dropped,
        total: tokens + raw.dropped,
        last_compact_at: raw.last_compact_at,
        last_compact_trigger: raw.last_compact_trigger.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn raw() -> RawContext {
        RawContext {
            session: "s".into(),
            model: "claude-opus-5".into(),
            tokens: 100_000,
            at: Some(ts("2026-08-10T01:00:00Z")),
            peak: 100_000,
            baseline: 26_000,
            ..Default::default()
        }
    }

    #[test]
    fn pct_uses_table_window() {
        let c = resolve(Source::Claude, &raw(), Some(1_000_000));
        assert_eq!(c.window, 1_000_000);
        assert!(!c.window_inferred);
        assert!((c.used_pct - 10.0).abs() < 1e-9);
        assert!(!c.interim);
    }

    #[test]
    fn unknown_model_defaults_then_promotes_on_evidence() {
        // 힌트 없음 → 200k 가정
        let mut r = raw();
        r.tokens = 150_000;
        r.peak = 150_000;
        let c = resolve(Source::Claude, &r, None);
        assert_eq!(c.window, DEFAULT_WINDOW);
        assert!(c.window_inferred);

        // 관측치가 200k 를 넘으면 그 자체가 증거 → 다음 구간으로 승격
        r.tokens = 360_000;
        r.peak = 360_000;
        let c = resolve(Source::Claude, &r, None);
        assert_eq!(c.window, 500_000);
        assert!(c.used_pct < 100.0, "승격 후에는 100% 로 터지지 않아야 함");
    }

    #[test]
    fn table_window_never_shrinks_below_observation() {
        // 표에 200k 로 적혀 있어도 실제로 그걸 넘겼다면 표가 틀린 것
        let mut r = raw();
        r.tokens = 900_000;
        r.peak = 900_000;
        let c = resolve(Source::Claude, &r, Some(200_000));
        assert_eq!(c.window, 1_000_000);
        assert!(c.window_inferred);
    }

    #[test]
    fn source_reported_window_wins_and_never_promotes() {
        // Codex 는 `model_context_window` 로 실효 창을 직접 알려준다 (272k 의 95%).
        let mut r = raw();
        r.model = "gpt-5.6-terra".into();
        r.tokens = 129_200;
        r.peak = 129_200;
        r.window = Some(258_400);

        // 단가표 힌트가 있어도 소스가 알려준 값이 이긴다
        let c = resolve(Source::Codex, &r, Some(1_000_000));
        assert_eq!(c.window, 258_400);
        assert!(!c.window_inferred);
        assert!((c.used_pct - 50.0).abs() < 1e-9);

        // 알려준 창을 넘겼다면 승격이 아니라 100%(꽉 참)가 맞다
        r.tokens = 300_000;
        r.peak = 300_000;
        let c = resolve(Source::Codex, &r, None);
        assert_eq!(c.window, 258_400, "권위 있는 창은 관측치로 승격하지 않는다");
        assert!((c.used_pct - 100.0).abs() < 1e-9);
    }

    #[test]
    fn post_compact_uses_interim_until_next_turn() {
        let mut r = raw();
        r.tokens = 361_919; // compact 이전 값 — 그대로 두면 낡은 값이 남는다
        r.peak = 361_919;
        r.compactions = 1;
        r.dropped = 351_116;
        r.last_compact_post = 10_940;
        r.last_compact_at = Some(ts("2026-08-10T02:00:00Z")); // 마지막 응답보다 뒤
        r.last_compact_trigger = Some("manual".into());

        let c = resolve(Source::Claude, &r, Some(1_000_000));
        assert!(c.interim);
        // postTokens 를 그대로 쓰면 안 되고 baseline 이 더해져야 한다
        assert_eq!(c.tokens, 10_940 + 26_000);
        // 버려진 분량까지 합친 총량은 보존된다
        assert_eq!(c.total, 10_940 + 26_000 + 351_116);
        // 창 크기는 compact 이전 peak 을 근거로 유지된다
        assert_eq!(c.window, 1_000_000);
    }

    #[test]
    fn next_turn_after_compact_wins_over_interim() {
        let mut r = raw();
        r.tokens = 49_876;
        r.at = Some(ts("2026-08-10T03:00:00Z")); // compact 보다 뒤
        r.compactions = 1;
        r.last_compact_post = 9_667;
        r.last_compact_at = Some(ts("2026-08-10T02:00:00Z"));

        let c = resolve(Source::Claude, &r, Some(1_000_000));
        assert!(!c.interim);
        assert_eq!(c.tokens, 49_876, "실측 턴이 있으면 postTokens 잠정값을 쓰면 안 됨");
    }
}

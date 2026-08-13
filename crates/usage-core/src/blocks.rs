//! Claude 5시간 창 — 리셋 시각을 **캐시 없이 직접 계산한다.**
//!
//! 사슬의 마지막 단이다: ① 사용량 API 실시간 → ② `.claude.json` 캐시 → ③ 이 계산
//! ([`crate::plan`] 모듈 주석). API 가 물러나고(오프라인·토큰 만료·macOS 키체인) 캐시마저
//! 굳었을 때 온다 — 실측(2026-08-13): 세션이 계속 도는데도 `fetchedAtMs` 가 6시간 전이었고
//! 5시간 창의 `resets_at` 은 이미 2시간 전으로 지나가 있었다. 그때도 리셋 시각은 보여줘야
//! 하므로, 우리가 늘 읽고 있는 트랜스크립트 타임스탬프에서 같은 값을 만든다.
//!
//! # 규칙 (실측으로 확정)
//!
//! ```text
//! 블록 시작 = 그 블록의 첫 활동을 30분 경계로 **내림**
//! 블록 종료 = 시작 + 5시간
//! 새 블록   = 직전 블록이 끝난 뒤 처음 나타난 활동
//! ```
//!
//! 실측 대조 — 공식 캐시가 신선했던 시점(2026-08-13T00:52:27Z)의 값과 맞춰 봤다:
//!
//! | 내림 단위 | 계산된 종료 | 공식 `04:30:00Z` 와의 차이 |
//! |---|---|---|
//! | 없음 | 04:36:33Z | +6.5분 |
//! | 15분 | 04:30:00Z | **0.0분** |
//! | 30분 | 04:30:00Z | **0.0분** |
//! | 60분 | 04:00:00Z | −30.0분 |
//!
//! 첫 활동이 `23:36:33Z` 라 15분·30분이 같은 답을 낸다 — 이 표본으로는 **둘을 못 가른다.**
//! 30분을 택한 이유는 같은 응답의 주간 창이 `06:00:00Z` 로 떨어졌고(30분 격자와 일관),
//! 제품 표기가 보통 30분 단위이기 때문이다. 틀렸다면 신선한 공식 값과 **15분** 어긋나
//! 바로 드러나므로, 그때 [`ANCHOR_MINUTES`] 만 고치면 된다.
//!
//! ⚠️ **첫 활동은 사용자 메시지지 assistant 응답이 아니다.** 처음엔 사용량 이벤트(=응답)만
//! 세어 47분 어긋났다 — 위 사례에서 첫 메시지는 `23:36`, 첫 응답은 `00:16` 이었다.
//! 그래서 [`active_block_end`] 에 **타임스탬프가 있는 모든 행**을 넘긴다.

use chrono::{DateTime, Duration, Timelike, Utc};

/// 창 길이
pub const BLOCK_HOURS: i64 = 5;
/// 블록 시작을 내림할 격자 (모듈 주석의 실측 표 참고)
pub const ANCHOR_MINUTES: u32 = 30;

/// 블록 시작 격자로 내림
fn anchor(t: DateTime<Utc>) -> DateTime<Utc> {
    let total = t.hour() * 60 + t.minute();
    let floored = total - (total % ANCHOR_MINUTES);
    t.with_hour(floored / 60)
        .and_then(|t| t.with_minute(floored % 60))
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// 지금 열려 있는 5시간 창의 **종료 시각**. 열린 창이 없으면 `None`.
///
/// `stamps` 는 정렬돼 있지 않아도 되고 중복이 있어도 된다 — 여기서 정렬한다.
/// 창이 이미 닫혔으면(마지막 활동 뒤 5시간이 지났으면) `None` 이다. 그때는 다음 창이
/// 언제 열릴지 알 수 없다 — **다음 메시지를 보내는 순간** 열리기 때문이다.
pub fn active_block_end(
    stamps: &mut Vec<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    stamps.sort_unstable();
    let mut end: Option<DateTime<Utc>> = None;
    for &ts in stamps.iter() {
        // 열린 창 안이면 그 창에 속한다. 아니면 이 활동이 새 창을 연다.
        if end.map(|e| ts >= e).unwrap_or(true) {
            end = Some(anchor(ts) + Duration::hours(BLOCK_HOURS));
        }
    }
    end.filter(|e| now < *e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// 실측 재현 — 공식 값이 신선했을 때의 그 블록을 그대로 맞혀야 한다.
    /// 첫 활동 `23:36:33Z` → 내림 `23:30` → +5h = `04:30:00Z` (공식과 일치).
    #[test]
    fn reproduces_the_observed_official_reset() {
        let mut stamps = vec![
            t("2026-08-12T23:36:33.083Z"),
            t("2026-08-12T23:38:48Z"),
            t("2026-08-13T00:16:46.436Z"),
            t("2026-08-13T04:28:00Z"),
        ];
        let now = t("2026-08-13T00:52:27Z");
        assert_eq!(active_block_end(&mut stamps, now), Some(t("2026-08-13T04:30:00Z")));
    }

    /// 창이 닫힌 뒤의 첫 활동이 새 창을 연다 — 활동이 끊긴 만큼 창도 밀린다.
    #[test]
    fn a_new_window_opens_on_the_first_activity_after_the_old_one_closed() {
        let mut stamps = vec![t("2026-08-12T23:36:00Z"), t("2026-08-13T05:23:41Z")];
        let now = t("2026-08-13T07:00:00Z");
        // 첫 창은 04:30 에 닫혔고, 05:23 이 새 창을 연다 → 내림 05:00 + 5h
        assert_eq!(active_block_end(&mut stamps, now), Some(t("2026-08-13T10:00:00Z")));
    }

    /// 활동이 5시간 넘게 없으면 **열린 창이 없다.** 다음 창은 다음 메시지가 연다 —
    /// 그 시각을 우리가 알 수 없으므로 0 이나 과거 시각을 지어내지 않는다.
    #[test]
    fn no_open_window_when_everything_expired() {
        let mut stamps = vec![t("2026-08-13T00:00:00Z")];
        assert_eq!(active_block_end(&mut stamps, t("2026-08-13T12:00:00Z")), None);
    }

    #[test]
    fn empty_input_has_no_window() {
        assert_eq!(active_block_end(&mut vec![], t("2026-08-13T12:00:00Z")), None);
    }

    /// 정렬·중복은 호출자가 신경 쓰지 않아도 된다 (파일별로 모아 붙이므로 뒤섞여 온다).
    #[test]
    fn unsorted_and_duplicated_input_is_fine() {
        let a = t("2026-08-13T05:23:41Z");
        let b = t("2026-08-12T23:36:00Z");
        let mut stamps = vec![a, b, a, b];
        let now = t("2026-08-13T07:00:00Z");
        assert_eq!(active_block_end(&mut stamps, now), Some(t("2026-08-13T10:00:00Z")));
    }

    /// 내림은 30분 격자 — 경계값을 못박아 둔다.
    #[test]
    fn anchor_floors_to_the_half_hour() {
        assert_eq!(anchor(t("2026-08-13T09:00:00Z")), t("2026-08-13T09:00:00Z"));
        assert_eq!(anchor(t("2026-08-13T09:29:59.999Z")), t("2026-08-13T09:00:00Z"));
        assert_eq!(anchor(t("2026-08-13T09:30:00Z")), t("2026-08-13T09:30:00Z"));
        assert_eq!(anchor(t("2026-08-13T09:59:59Z")), t("2026-08-13T09:30:00Z"));
    }
}

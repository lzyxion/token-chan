//! Claude 5시간 과금 블록 계산.
//!
//! - 블록 시작 = 블록의 첫 이벤트 타임스탬프 (내림 없음)
//!   (ccusage는 정시로 내림하지만, 공식 `/usage`의 리셋 시각이 정시가 아닌 것으로 보아
//!    Anthropic의 실제 윈도우는 첫 메시지 + 5h — 내림하면 최대 59분 빨라짐)
//! - 블록 길이 = 5시간
//! - 이벤트가 현재 블록 종료를 넘거나, 직전 이벤트와 간격이 5시간을 넘으면 새 블록
//! - 활성 블록 = now < 블록 종료 && 마지막 이벤트가 5시간 이내

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::aggregate::Totals;
use crate::model::UsageEvent;
use crate::pricing::PriceTable;

pub const BLOCK_HOURS: i64 = 5;

#[derive(Clone, Debug, Serialize)]
pub struct Block {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub last_event: DateTime<Utc>,
    pub totals: Totals,
    /// 단가가 알려진 이벤트들의 비용 합
    pub cost: f64,
    /// 단가 미등록 모델이 포함되어 비용이 부분합인지
    pub cost_partial: bool,
    pub events: usize,
}

/// `events`는 ts 오름차순 정렬 가정.
pub fn compute_blocks(events: &[UsageEvent], pricing: &PriceTable) -> Vec<Block> {
    let mut blocks: Vec<Block> = vec![];
    for ev in events {
        let need_new = match blocks.last() {
            None => true,
            Some(b) => ev.ts >= b.end || ev.ts - b.last_event > Duration::hours(BLOCK_HOURS),
        };
        if need_new {
            let start = ev.ts;
            blocks.push(Block {
                start,
                end: start + Duration::hours(BLOCK_HOURS),
                last_event: ev.ts,
                totals: Totals::default(),
                cost: 0.0,
                cost_partial: false,
                events: 0,
            });
        }
        let b = blocks.last_mut().unwrap();
        b.last_event = ev.ts;
        b.totals.add_event(ev);
        b.events += 1;
        match pricing.cost(ev) {
            Some(c) => b.cost += c,
            None => b.cost_partial = true,
        }
    }
    blocks
}

/// 현재 활성 블록 (진행 중인 5시간 윈도우)
pub fn active_block<'a>(blocks: &'a [Block], now: DateTime<Utc>) -> Option<&'a Block> {
    blocks
        .last()
        .filter(|b| now < b.end && now - b.last_event < Duration::hours(BLOCK_HOURS))
}

/// 과거 완료 블록 대비 현재 블록의 토큰 소진율 (자동 베이스라인).
/// 완료된 블록이 2개 미만이면 None.
pub fn token_ratio_vs_history(blocks: &[Block], active: &Block) -> Option<f64> {
    let history_max = blocks
        .iter()
        .filter(|b| b.start != active.start)
        .map(|b| b.totals.total())
        .max()?;
    if blocks.len() < 3 || history_max == 0 {
        return None;
    }
    Some(active.totals.total() as f64 / history_max as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn ev(ts: &str, output: u64) -> UsageEvent {
        UsageEvent {
            source: Source::Claude,
            model: "claude-opus-4-8".into(),
            ts: DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Utc),
            input: 10,
            output,
            cache_write: 0,
            cache_read: 0,
            sidechain: false,
        }
    }

    #[test]
    fn splits_blocks_by_window_and_gap() {
        let pricing = PriceTable::builtin();
        let events = vec![
            ev("2026-07-30T01:20:00Z", 100), // 블록1 시작 01:20, 종료 06:20 (내림 없음)
            ev("2026-07-30T02:00:00Z", 200), // 같은 블록
            ev("2026-07-30T08:00:00Z", 300), // 06:20 넘음 → 블록2 (08:00~13:00)
            ev("2026-07-31T00:00:00Z", 400), // 간격 > 5h → 블록3
        ];
        let blocks = compute_blocks(&events, &pricing);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].start.to_rfc3339(), "2026-07-30T01:20:00+00:00");
        assert_eq!(blocks[0].end.to_rfc3339(), "2026-07-30T06:20:00+00:00");
        assert_eq!(blocks[0].totals.output, 300);
        assert_eq!(blocks[1].totals.output, 300);
        assert_eq!(blocks[2].totals.output, 400);
    }

    #[test]
    fn active_block_detection() {
        let pricing = PriceTable::builtin();
        let events = vec![ev("2026-07-30T01:20:00Z", 100)];
        let blocks = compute_blocks(&events, &pricing);

        // 블록 내 + 마지막 이벤트 5h 이내 → 활성
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        assert!(active_block(&blocks, now).is_some());

        // 블록 종료(06:20) 후 → 비활성
        let later = DateTime::parse_from_rfc3339("2026-07-30T07:00:00Z").unwrap().with_timezone(&Utc);
        assert!(active_block(&blocks, later).is_none());
    }
}

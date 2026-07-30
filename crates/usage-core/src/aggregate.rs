//! 이벤트 → 요약(Summary) 집계. "오늘"은 로컬 타임존 기준 (테스트를 위해 offset 주입).

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

use crate::blocks::{active_block, compute_blocks, token_ratio_vs_history, BLOCK_HOURS};
use crate::model::{Source, SourceStatus, UsageEvent};
use crate::pricing::PriceTable;

#[derive(Default, Clone, Copy, Debug, Serialize)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Totals {
    pub fn add_event(&mut self, ev: &UsageEvent) {
        self.input += ev.input;
        self.output += ev.output;
        self.cache_write += ev.cache_write;
        self.cache_read += ev.cache_read;
    }
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceSummary {
    pub source: Source,
    pub label: String,
    pub status: SourceStatus,
    pub today: Totals,
    pub today_cost: f64,
    pub cost_partial: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelRow {
    pub model: String,
    pub source: Source,
    pub totals: Totals,
    pub cost: f64,
    pub cost_known: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DailyRow {
    /// ISO 날짜 (로컬 타임존 기준)
    pub date: String,
    pub totals: Totals,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockSummary {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub totals: Totals,
    pub cost: f64,
    pub cost_partial: bool,
    pub remaining_minutes: i64,
    /// 블록 시간 경과율 0..1
    pub time_ratio: f64,
    /// 과거 최대 블록 대비 토큰 소진율 (히스토리 부족 시 None)
    pub token_ratio: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub generated_at: DateTime<Utc>,
    pub today_date: String,
    pub today: Totals,
    pub today_cost: f64,
    pub cost_partial: bool,
    pub sources: Vec<SourceSummary>,
    pub models_today: Vec<ModelRow>,
    pub daily: Vec<DailyRow>,
    pub block: Option<BlockSummary>,
    pub last_event_ts: Option<DateTime<Utc>>,
    /// 가장 최근 메인체인(서브에이전트 제외) 이벤트의 모델 — "활성 모델" 캐릭터 매핑용
    pub last_model: Option<String>,
    /// 스캔 범위에서 관측된 모든 모델명 (정렬) — 규칙 편집 도우미용
    pub observed_models: Vec<String>,
}

fn local_date(ts: DateTime<Utc>, offset: FixedOffset) -> NaiveDate {
    ts.with_timezone(&offset).date_naive()
}

/// `events`는 ts 오름차순 정렬 가정.
pub fn build_summary(
    events: &[UsageEvent],
    statuses: &[(Source, SourceStatus)],
    pricing: &PriceTable,
    days: usize,
    now: DateTime<Utc>,
    offset: FixedOffset,
) -> Summary {
    let today = local_date(now, offset);

    let mut today_totals = Totals::default();
    let mut today_cost = 0.0;
    let mut cost_partial = false;

    let mut per_source: std::collections::BTreeMap<Source, (Totals, f64, bool)> = Default::default();
    let mut per_model: std::collections::BTreeMap<(Source, String), (Totals, f64, bool)> = Default::default();
    let mut per_day: std::collections::BTreeMap<NaiveDate, (Totals, f64)> = Default::default();

    for ev in events {
        let d = local_date(ev.ts, offset);
        let cost = pricing.cost(ev);

        let day = per_day.entry(d).or_default();
        day.0.add_event(ev);
        day.1 += cost.unwrap_or(0.0);

        if d == today {
            today_totals.add_event(ev);
            match cost {
                Some(c) => today_cost += c,
                None => cost_partial = true,
            }

            let s = per_source.entry(ev.source).or_default();
            s.0.add_event(ev);
            match cost {
                Some(c) => s.1 += c,
                None => s.2 = true,
            }

            let m = per_model.entry((ev.source, ev.model.clone())).or_default();
            m.0.add_event(ev);
            match cost {
                Some(c) => m.1 += c,
                None => m.2 = true,
            }
        }
    }

    // 소스 요약: 상태 목록 순서대로, 데이터 없으면 0으로
    let sources = statuses
        .iter()
        .map(|(src, status)| {
            let (totals, cost, partial) = per_source.get(src).copied().unwrap_or_default();
            SourceSummary {
                source: *src,
                label: src.label().to_string(),
                status: status.clone(),
                today: totals,
                today_cost: cost,
                cost_partial: partial,
            }
        })
        .collect();

    let mut models_today: Vec<ModelRow> = per_model
        .into_iter()
        .map(|((source, model), (totals, cost, partial))| ModelRow {
            model,
            source,
            totals,
            cost,
            cost_known: !partial,
        })
        .collect();
    models_today.sort_by(|a, b| b.totals.total().cmp(&a.totals.total()));

    // 최근 N일 (빈 날 포함, 오름차순)
    let mut daily = vec![];
    for i in (0..days).rev() {
        let d = today - Duration::days(i as i64);
        let (totals, cost) = per_day.get(&d).copied().unwrap_or_default();
        daily.push(DailyRow { date: d.to_string(), totals, cost });
    }

    // 5시간 블록: Claude 이벤트만
    let claude_events: Vec<UsageEvent> =
        events.iter().filter(|e| e.source == Source::Claude).cloned().collect();
    let blocks = compute_blocks(&claude_events, pricing);
    let block = active_block(&blocks, now).map(|b| {
        let elapsed = (now - b.start).num_seconds() as f64;
        let len = (Duration::hours(BLOCK_HOURS)).num_seconds() as f64;
        BlockSummary {
            start: b.start,
            end: b.end,
            totals: b.totals,
            cost: b.cost,
            cost_partial: b.cost_partial,
            remaining_minutes: (b.end - now).num_minutes().max(0),
            time_ratio: (elapsed / len).clamp(0.0, 1.0),
            token_ratio: token_ratio_vs_history(&blocks, b),
        }
    });

    let last_model = events.iter().rev().find(|e| !e.sidechain).map(|e| e.model.clone());
    let observed: std::collections::BTreeSet<String> =
        events.iter().map(|e| e.model.clone()).collect();

    Summary {
        generated_at: now,
        today_date: today.to_string(),
        today: today_totals,
        today_cost,
        cost_partial,
        sources,
        models_today,
        daily,
        block,
        last_event_ts: events.last().map(|e| e.ts),
        last_model,
        observed_models: observed.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(source: Source, model: &str, ts: &str, output: u64) -> UsageEvent {
        UsageEvent {
            source,
            model: model.into(),
            ts: DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Utc),
            input: 100,
            output,
            cache_write: 0,
            cache_read: 1000,
            sidechain: false,
        }
    }

    #[test]
    fn summary_today_boundary_uses_local_offset() {
        // KST(+9) 기준: UTC 07-29 16:00 = KST 07-30 01:00 → "오늘"
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        let events = vec![
            ev(Source::Claude, "claude-opus-4-8", "2026-07-29T10:00:00Z", 100), // KST 07-29 → 어제
            ev(Source::Claude, "claude-opus-4-8", "2026-07-29T16:30:00Z", 200), // KST 07-30 → 오늘
            ev(Source::Codex, "gpt-5-codex", "2026-07-30T01:00:00Z", 300),      // 오늘
        ];
        let statuses = vec![
            (Source::Claude, SourceStatus::Ok),
            (Source::Codex, SourceStatus::Ok),
            (Source::Gemini, SourceStatus::NoData),
        ];
        let s = build_summary(&events, &statuses, &PriceTable::builtin(), 7, now, kst);

        assert_eq!(s.today_date, "2026-07-30");
        assert_eq!(s.today.output, 500);
        assert_eq!(s.sources.len(), 3);
        let claude = s.sources.iter().find(|x| x.source == Source::Claude).unwrap();
        assert_eq!(claude.today.output, 200);
        assert_eq!(s.daily.len(), 7);
        assert_eq!(s.daily.last().unwrap().date, "2026-07-30");
        assert!(s.today_cost > 0.0);
        assert!(!s.cost_partial);
        // 활성 블록: 마지막 Claude 이벤트(16:30Z) 기준 5h 이내 아님 → now 03:00Z 는 블록(16:00~21:00) 밖
        assert!(s.block.is_none());
        assert_eq!(s.models_today.len(), 2);
    }

    #[test]
    fn unknown_model_marks_cost_partial() {
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        let events = vec![ev(Source::Gemini, "gemini-99-ultra", "2026-07-30T01:00:00Z", 10)];
        let statuses = vec![(Source::Gemini, SourceStatus::Ok)];
        let s = build_summary(&events, &statuses, &PriceTable::builtin(), 3, now, kst);
        assert!(s.cost_partial);
        assert!(!s.models_today[0].cost_known);
    }
}

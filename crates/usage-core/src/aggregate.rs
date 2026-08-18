//! 이벤트 → 요약(Summary) 집계. "오늘"은 로컬 타임존 기준 (테스트를 위해 offset 주입).

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

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

/// 하루에 모델 하나가 쓴 양. 비용은 담지 않는다 — 막대는 토큰 비중만 그린다.
#[derive(Clone, Debug, Serialize)]
pub struct DayModel {
    pub model: String,
    pub source: Source,
    pub tokens: u64,
}

/// 하루치 모델 내역
#[derive(Clone, Debug, Serialize)]
pub struct DailyModels {
    pub date: String,
    /// 토큰 많은 순. **상위 N 추리기는 프론트가 한다** — 주 전체를 놓고 골라야
    /// 막대마다 범례가 달라지지 않아서, 하루씩 자르는 여기서는 정할 수 없다.
    pub models: Vec<DayModel>,
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
    /// 최근 [`WEEK_DAYS`]일의 **날짜별 모델 내역** — 주간 막대를 모델로 쌓기 위한 것.
    ///
    /// `daily` 에 붙이지 않는다. `daily` 는 보존기간만큼 길어(최대 수개월) 10초마다
    /// 통째로 직렬화되는데, 모델로 쌓아 보여주는 건 7일뿐이라 나머지 날의 모델 배열은
    /// 아무도 읽지 않고 payload 만 불린다.
    #[serde(default)]
    pub week_models: Vec<DailyModels>,
    /// 스캔 범위에서 **가장 오래된** 이벤트 시각.
    /// 잔디에서 "그날 안 씀"과 "그때는 기록 자체가 없음"을 가르는 경계다 —
    /// 둘 다 0 으로 그리면 CLI 를 설치하기 전 날짜까지 "안 씀"으로 보인다.
    pub first_event_ts: Option<DateTime<Utc>>,
    pub last_event_ts: Option<DateTime<Utc>>,
    /// 가장 최근 메인체인(서브에이전트 제외) 이벤트의 모델 — "활성 모델" 캐릭터 매핑용
    pub last_model: Option<String>,
    /// 스캔 범위에서 관측된 모든 모델명 (정렬) — 규칙 편집 도우미용
    pub observed_models: Vec<String>,
    /// 소스별 활성 세션의 컨텍스트 창 사용량 (해당 소스에 최근 세션이 있는 것만).
    /// 트랜스크립트 파일 단위 정보라 이벤트 집계로는 만들 수 없어, 스캔 뒤
    /// 각 어댑터의 `context()` 결과를 채워 넣는다 (monitor.rs).
    ///
    /// 게이지는 이 중 하나(활성 벤더)만 쓰고 패널은 전부 쓴다 — 어느 쪽을 고를지는
    /// 프론트가 정하므로 여기서는 고르지 않고 그대로 내보낸다.
    #[serde(default)]
    pub contexts: Vec<crate::context::ContextState>,
    /// 최근 세션 (소스 합쳐 최근순). 어느 프로젝트에서 태웠는지는 이것만 답한다.
    #[serde(default)]
    pub sessions: Vec<crate::session::SessionRow>,
}

impl Summary {
    /// 가장 최근에 움직인 세션의 컨텍스트 (활성 벤더 자동 선택의 기본 근거)
    pub fn latest_context(&self) -> Option<&crate::context::ContextState> {
        self.contexts.iter().max_by_key(|c| c.at)
    }
}

/// 주간 막대(`WeekBars`)가 덮는 일수. 잔디와 달리 고정이다 — 요일별 크기를 읽는
/// 그래프라 한 주가 곧 단위다. 프론트가 `daily` 꼬리를 자르는 길이와 같아야
/// [`Summary::week_models`] 가 날짜별로 맞물린다.
pub const WEEK_DAYS: usize = 7;

/// 잔디 격자가 덮을 일수 — **보존기간에서 유도**한다.
///
/// 예전엔 91일 고정이었는데, 보존기간을 30일로 줄이면 격자 61칸이 데이터가 존재할 수
/// 없는 날로 남고 늘려도 91일까지만 보였다. 보존기간이 곧 "우리가 아는 범위"이므로
/// 그걸 따라가는 게 맞다.
///
/// 주 단위로 떨어뜨리는 이유는 격자가 7행이라서다 — 어중간하면 마지막 열이 잘린다.
/// 하한 4주: 그보다 짧으면 흐름이 안 보인다. 상한 26주: 패널 폭(약 274px)에서
/// 그 이상은 칸이 실오라기가 된다.
pub fn daily_window(retention_days: u32) -> usize {
    const MIN_WEEKS: u32 = 4;
    const MAX_WEEKS: u32 = 26;
    ((retention_days / 7).clamp(MIN_WEEKS, MAX_WEEKS) * 7) as usize
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
    // 주간 막대용 (날짜, 소스, 모델) → 토큰. 보존기간이 7일보다 짧으면 그만큼만 본다.
    let week_len = days.min(WEEK_DAYS);
    let week_start = today - Duration::days(week_len.max(1) as i64 - 1);
    let mut per_day_model: std::collections::BTreeMap<(NaiveDate, Source, String), u64> =
        Default::default();

    for ev in events {
        let d = local_date(ev.ts, offset);
        let cost = pricing.cost(ev);

        let day = per_day.entry(d).or_default();
        day.0.add_event(ev);
        day.1 += cost.unwrap_or(0.0);

        if week_len > 0 && d >= week_start && d <= today {
            *per_day_model.entry((d, ev.source, ev.model.clone())).or_default() += ev.total();
        }

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
    models_today.sort_by_key(|m| std::cmp::Reverse(m.totals.total()));

    // 최근 N일 (빈 날 포함, 오름차순)
    let mut daily = vec![];
    for i in (0..days).rev() {
        let d = today - Duration::days(i as i64);
        let (totals, cost) = per_day.get(&d).copied().unwrap_or_default();
        daily.push(DailyRow { date: d.to_string(), totals, cost });
    }

    // 주간 막대용 모델 내역 — `daily` 꼬리와 같은 날짜·같은 순서여야 프론트가 붙일 수 있다
    let mut by_date: std::collections::BTreeMap<NaiveDate, Vec<DayModel>> = Default::default();
    for ((d, source, model), tokens) in per_day_model {
        by_date.entry(d).or_default().push(DayModel { model, source, tokens });
    }
    let mut week_models = vec![];
    for i in (0..week_len).rev() {
        let d = today - Duration::days(i as i64);
        let mut models = by_date.remove(&d).unwrap_or_default();
        models.sort_by_key(|m| std::cmp::Reverse(m.tokens));
        week_models.push(DailyModels { date: d.to_string(), models });
    }

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
        week_models,
        first_event_ts: events.first().map(|e| e.ts),
        last_event_ts: events.last().map(|e| e.ts),
        last_model,
        observed_models: observed.into_iter().collect(),
        contexts: vec![],
        sessions: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_window_follows_retention_in_whole_weeks() {
        // 기본값 90일 → 12주 (예전 고정값 91일과 거의 같다)
        assert_eq!(daily_window(90), 84);
        // 짧게 줄이면 격자도 줄어든다 — 데이터가 있을 수 없는 칸을 남기지 않는다
        assert_eq!(daily_window(30), 28);
        assert_eq!(daily_window(7), 28, "하한 4주");
        assert_eq!(daily_window(1), 28, "하한 4주");
        // 길게 늘려도 패널 폭에 들어가는 만큼만
        assert_eq!(daily_window(365), 182, "상한 26주");
        // 항상 주 단위로 떨어진다 (격자가 7행이라 어중간하면 마지막 열이 잘린다)
        for d in [1u32, 13, 45, 88, 200, 1000] {
            assert_eq!(daily_window(d) % 7, 0, "retention={d}");
        }
    }

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
            (Source::Antigravity, SourceStatus::NoData),
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
        assert_eq!(s.models_today.len(), 2);
    }

    /// 주간 막대가 모델로 쌓이려면 날짜별 내역이 `daily` 꼬리와 맞물려야 한다.
    #[test]
    fn week_models_line_up_with_the_daily_tail() {
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        let events = vec![
            // KST 07-28 — 한 날에 모델 둘
            ev(Source::Claude, "claude-opus-4-8", "2026-07-27T16:00:00Z", 100),
            ev(Source::Codex, "gpt-5-codex", "2026-07-27T17:00:00Z", 900),
            // KST 07-30 (오늘)
            ev(Source::Claude, "claude-opus-4-8", "2026-07-30T01:00:00Z", 300),
            // 주 밖 (KST 07-20) — 격자엔 있어도 주간 막대엔 없어야 한다
            ev(Source::Claude, "claude-sonnet-4-5", "2026-07-20T01:00:00Z", 50),
        ];
        let statuses = vec![(Source::Claude, SourceStatus::Ok), (Source::Codex, SourceStatus::Ok)];
        let s = build_summary(&events, &statuses, &PriceTable::builtin(), 28, now, kst);

        // 날짜가 `daily` 의 마지막 7개와 같아야 프론트가 붙일 수 있다
        assert_eq!(s.week_models.len(), WEEK_DAYS);
        let tail: Vec<&str> = s.daily[s.daily.len() - WEEK_DAYS..].iter().map(|d| d.date.as_str()).collect();
        let wk: Vec<&str> = s.week_models.iter().map(|d| d.date.as_str()).collect();
        assert_eq!(wk, tail);

        // 07-28: 토큰 많은 순 — codex(900+100+1000) 가 claude(100+100+1000) 앞
        let d28 = s.week_models.iter().find(|d| d.date == "2026-07-28").unwrap();
        assert_eq!(d28.models.len(), 2);
        assert_eq!(d28.models[0].source, Source::Codex);
        assert_eq!(d28.models[0].tokens, 2000);
        assert_eq!(d28.models[1].tokens, 1200);

        // 하루 합이 같은 날 `daily` 총량과 어긋나면 막대 비율이 거짓말이 된다
        for row in &s.week_models {
            let day = s.daily.iter().find(|d| d.date == row.date).unwrap();
            let sum: u64 = row.models.iter().map(|m| m.tokens).sum();
            assert_eq!(sum, day.totals.total(), "{}", row.date);
        }

        // 주 밖의 모델은 안 실린다 (격자에는 남아 있다)
        assert!(!s.week_models.iter().any(|d| d.models.iter().any(|m| m.model.contains("sonnet"))));
        assert!(s.observed_models.iter().any(|m| m.contains("sonnet")));
    }

    /// 보존기간이 한 주보다 짧으면 있는 만큼만 — 없는 날을 지어내지 않는다.
    #[test]
    fn week_models_shrink_with_a_short_window() {
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        let events = vec![ev(Source::Claude, "claude-opus-4-8", "2026-07-30T01:00:00Z", 10)];
        let statuses = vec![(Source::Claude, SourceStatus::Ok)];
        let s = build_summary(&events, &statuses, &PriceTable::builtin(), 3, now, kst);
        assert_eq!(s.week_models.len(), 3);
        assert_eq!(s.week_models.last().unwrap().date, "2026-07-30");
    }

    #[test]
    fn unknown_model_marks_cost_partial() {
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-30T03:00:00Z").unwrap().with_timezone(&Utc);
        let events = vec![ev(Source::Antigravity, "gemini-99-ultra", "2026-07-30T01:00:00Z", 10)];
        let statuses = vec![(Source::Antigravity, SourceStatus::Ok)];
        let s = build_summary(&events, &statuses, &PriceTable::builtin(), 3, now, kst);
        assert!(s.cost_partial);
        assert!(!s.models_today[0].cost_known);
    }
}

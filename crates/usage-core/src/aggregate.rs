//! 이벤트 → 요약(Summary) 집계. "오늘"은 로컬 타임존 기준 (테스트를 위해 offset 주입).

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

use crate::model::{Source, SourceStatus, UsageEvent};
use crate::pricing::{CostParts, PriceTable};

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
    /// 오늘 비용의 종류별 내역
    #[serde(default)]
    pub today_parts: CostParts,
    /// **격자 기간**(`daily` 와 같은 창)의 벤더별 합계.
    ///
    /// `daily` 를 더해서는 못 만든다 — 거기엔 소스 구분이 없다. "어느 벤더가 돈을
    /// 먹었나"와 벤더 상세의 구성 분해가 이 값을 쓴다.
    #[serde(default)]
    pub period: Totals,
    #[serde(default)]
    pub period_cost: f64,
    #[serde(default)]
    pub period_parts: CostParts,
    /// 기간 안에 단가 미등록 모델이 섞였는지 (§ cost_partial 과 같은 뜻, 기간 범위)
    #[serde(default)]
    pub period_partial: bool,
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

/// 선택한 날짜의 벤더별 사용량.
#[derive(Clone, Debug, Serialize)]
pub struct DailySource {
    pub source: Source,
    pub totals: Totals,
    pub cost: f64,
    pub cost_known: bool,
}

/// 사용 기록에서 날짜를 눌렀을 때 보여줄 상세. `daily`와 같은 기간만 담는다.
#[derive(Clone, Debug, Serialize)]
pub struct DailyDetail {
    pub date: String,
    pub sources: Vec<DailySource>,
    pub models: Vec<ModelRow>,
}

/// 하루에 모델 하나가 쓴 양.
///
/// 토큰과 비용을 **둘 다** 담는다. 막대의 기준을 토글로 바꾸는데, 높이만 비용으로
/// 바꾸고 조각은 토큰으로 쌓으면 한 그래프 안에서 기준이 섞여 더 나쁘다.
#[derive(Clone, Debug, Serialize)]
pub struct DayModel {
    pub model: String,
    pub source: Source,
    pub tokens: u64,
    #[serde(default)]
    pub cost: f64,
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
    /// 오늘 비용의 종류별 내역 (전 소스 합)
    #[serde(default)]
    pub today_parts: CostParts,
    pub sources: Vec<SourceSummary>,
    pub models_today: Vec<ModelRow>,
    /// **격자 기간**(`daily` 와 같은 창)의 모델별 합계. `models_today` 와 같은 모양이라
    /// 화면이 같은 그래프를 기간으로 돌릴 수 있다.
    #[serde(default)]
    pub models_period: Vec<ModelRow>,
    pub daily: Vec<DailyRow>,
    #[serde(default)]
    pub daily_details: Vec<DailyDetail>,
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
    /// 조회 기간 안의 프로젝트 합계와 프로젝트별 최근 세션.
    #[serde(default)]
    pub projects: Vec<crate::session::ProjectSessions>,
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
///
/// `0` 은 **기간 제한 없음**이라 상한을 준다 — "전부"라도 격자가 담을 수 있는 건
/// 26주까지고, 그보다 오래된 사용량은 격자가 아니라 합계에만 실린다.
pub fn daily_window(retention_days: u32) -> usize {
    const MIN_WEEKS: u32 = 4;
    const MAX_WEEKS: u32 = 26;
    if retention_days == 0 {
        return (MAX_WEEKS * 7) as usize;
    }
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
    let mut today_parts = CostParts::default();

    let mut per_source: std::collections::BTreeMap<Source, (Totals, f64, bool)> = Default::default();
    let mut per_source_parts: std::collections::BTreeMap<Source, CostParts> = Default::default();
    // 격자 기간(= `daily` 가 덮는 창)의 소스별 합계. 창 밖 이벤트는 넣지 않는다 —
    // 화면의 기간 합계는 `daily` 를 더해 만들므로 범위가 어긋나면 숫자가 안 맞는다.
    let period_start = today - Duration::days(days.max(1) as i64 - 1);
    let mut per_source_period: std::collections::BTreeMap<Source, (Totals, f64, bool, CostParts)> =
        Default::default();
    // 모델 구성도 같은 창으로 낸다. 오늘치만으로는 구성비가 안 나온다 — 실측에서 오늘은
    // 2종(한 모델이 98%)인데 84일로 보면 10종에 70/29 로 갈렸다. 페이지의 다른 값이
    // 전부 기간인데 모델만 오늘이면 무엇과 비교하는 그래프인지도 어긋난다.
    let mut per_model_period: std::collections::BTreeMap<(Source, String), (Totals, f64, bool)> =
        Default::default();
    let mut per_model: std::collections::BTreeMap<(Source, String), (Totals, f64, bool)> = Default::default();
    let mut per_day: std::collections::BTreeMap<NaiveDate, (Totals, f64)> = Default::default();
    let mut per_day_source: std::collections::BTreeMap<(NaiveDate, Source), (Totals, f64, bool)> =
        Default::default();
    let mut per_day_detail_model: std::collections::BTreeMap<(NaiveDate, Source, String), (Totals, f64, bool)> =
        Default::default();
    let week_len = days.min(WEEK_DAYS);

    for ev in events {
        let d = local_date(ev.ts, offset);
        let parts = pricing.cost_parts(ev);
        let cost = parts.map(|p| p.total());

        if days > 0 && d >= period_start && d <= today {
            let e = per_source_period.entry(ev.source).or_default();
            e.0.add_event(ev);
            match parts {
                Some(p) => {
                    e.1 += p.total();
                    e.3.add(&p);
                }
                None => e.2 = true,
            }

            let m = per_model_period.entry((ev.source, ev.model.clone())).or_default();
            m.0.add_event(ev);
            match cost {
                Some(c) => m.1 += c,
                None => m.2 = true,
            }

            let source = per_day_source.entry((d, ev.source)).or_default();
            source.0.add_event(ev);
            match cost {
                Some(c) => source.1 += c,
                None => source.2 = true,
            }
            let model = per_day_detail_model.entry((d, ev.source, ev.model.clone())).or_default();
            model.0.add_event(ev);
            match cost {
                Some(c) => model.1 += c,
                None => model.2 = true,
            }

        }

        let day = per_day.entry(d).or_default();
        day.0.add_event(ev);
        day.1 += cost.unwrap_or(0.0);
        if d == today {
            today_totals.add_event(ev);
            match parts {
                Some(p) => {
                    today_cost += p.total();
                    today_parts.add(&p);
                }
                None => cost_partial = true,
            }

            let s = per_source.entry(ev.source).or_default();
            s.0.add_event(ev);
            match cost {
                Some(c) => s.1 += c,
                None => s.2 = true,
            }
            if let Some(p) = parts {
                per_source_parts.entry(ev.source).or_default().add(&p);
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
            let (p_tot, p_cost, p_partial, p_parts) =
                per_source_period.get(src).copied().unwrap_or_default();
            SourceSummary {
                source: *src,
                label: src.label().to_string(),
                status: status.clone(),
                today: totals,
                today_cost: cost,
                cost_partial: partial,
                today_parts: per_source_parts.get(src).copied().unwrap_or_default(),
                period: p_tot,
                period_cost: p_cost,
                period_parts: p_parts,
                period_partial: p_partial,
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

    let mut models_period: Vec<ModelRow> = per_model_period
        .into_iter()
        .map(|((source, model), (totals, cost, partial))| ModelRow {
            model,
            source,
            totals,
            cost,
            cost_known: !partial,
        })
        .collect();
    models_period.sort_by_key(|m| std::cmp::Reverse(m.totals.total()));

    // 최근 N일 (빈 날 포함, 오름차순)
    let mut daily = vec![];
    let mut daily_details = vec![];
    for i in (0..days).rev() {
        let d = today - Duration::days(i as i64);
        let (totals, cost) = per_day.get(&d).copied().unwrap_or_default();
        daily.push(DailyRow { date: d.to_string(), totals, cost });
        let mut sources: Vec<DailySource> = per_day_source
            .iter()
            .filter(|((day, _), _)| *day == d)
            .map(|((_, source), (totals, cost, partial))| DailySource {
                source: *source,
                totals: *totals,
                cost: *cost,
                cost_known: !*partial,
            })
            .collect();
        sources.sort_by_key(|s| std::cmp::Reverse(s.totals.total()));
        let mut models: Vec<ModelRow> = per_day_detail_model
            .iter()
            .filter(|((day, _, _), _)| *day == d)
            .map(|((_, source, model), (totals, cost, partial))| ModelRow {
                source: *source,
                model: model.clone(),
                totals: *totals,
                cost: *cost,
                cost_known: !*partial,
            })
            .collect();
        models.sort_by_key(|m| std::cmp::Reverse(m.totals.total()));
        daily_details.push(DailyDetail { date: d.to_string(), sources, models });
    }

    // 주간 막대는 일별 상세의 최근 7일을 다시 쓴다. 같은 모델 집계를 두 번 만들면
    // 날짜 상세와 막대가 어긋날 여지가 생긴다.
    let week_models = daily_details
        .iter()
        .rev()
        .take(week_len)
        .rev()
        .map(|detail| DailyModels {
            date: detail.date.clone(),
            models: detail
                .models
                .iter()
                .map(|m| DayModel {
                    model: m.model.clone(),
                    source: m.source,
                    tokens: m.totals.total(),
                    cost: m.cost,
                })
                .collect(),
        })
        .collect();

    let last_model = events.iter().rev().find(|e| !e.sidechain).map(|e| e.model.clone());
    let observed: std::collections::BTreeSet<String> =
        events.iter().map(|e| e.model.clone()).collect();

    Summary {
        generated_at: now,
        today_date: today.to_string(),
        today: today_totals,
        today_cost,
        cost_partial,
        today_parts,
        sources,
        models_today,
        models_period,
        daily,
        daily_details,
        week_models,
        first_event_ts: events.first().map(|e| e.ts),
        last_event_ts: events.last().map(|e| e.ts),
        last_model,
        observed_models: observed.into_iter().collect(),
        contexts: vec![],
        sessions: vec![],
        projects: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 벤더별 기간 합계는 **`daily` 와 같은 창**을 덮어야 한다 — 화면의 기간 총합은
    /// `daily` 를 더해 만들므로, 범위가 어긋나면 "벤더별 합 ≠ 전체"가 된다.
    #[test]
    fn per_source_period_matches_the_daily_window() {
        let pricing = PriceTable::builtin();
        let off = FixedOffset::east_opt(0).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:00Z").unwrap().with_timezone(&Utc);
        let days = 28;
        let evs = vec![
            ev(Source::Claude, "claude-opus-5", "2026-08-23T01:00:00Z", 10), // 오늘
            ev(Source::Claude, "claude-opus-5", "2026-08-10T01:00:00Z", 10), // 창 안
            ev(Source::Claude, "claude-opus-5", "2026-01-01T01:00:00Z", 10), // 창 밖
        ];
        let s = build_summary(&evs, &[(Source::Claude, SourceStatus::Ok)], &pricing, days, now, off);
        let src = &s.sources[0];
        let from_daily: u64 = s.daily.iter().map(|d| d.totals.total()).sum();
        assert_eq!(src.period.total(), from_daily, "벤더별 기간 합 = daily 합");
        let cost_from_daily: f64 = s.daily.iter().map(|d| d.cost).sum();
        assert!((src.period_cost - cost_from_daily).abs() < 1e-9);
        // 창 밖 이벤트는 빠졌다 (이벤트 3개 중 2개만)
        assert_eq!(src.period.output, 20);
    }

    /// 기간 모델은 **`daily` 와 같은 창**을 덮어야 한다 — 벤더별 기간과 같은 조건이다.
    /// 어긋나면 화면에서 "모델 합 ≠ 기간 총액" 이 되어 어느 쪽이 참인지 알 수 없다.
    #[test]
    fn period_models_match_the_daily_window() {
        let pricing = PriceTable::builtin();
        let off = FixedOffset::east_opt(0).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:00Z").unwrap().with_timezone(&Utc);
        let days = 28;
        let evs = vec![
            ev(Source::Claude, "claude-opus-5", "2026-08-23T01:00:00Z", 10), // 오늘
            ev(Source::Claude, "claude-sonnet-5", "2026-08-10T01:00:00Z", 20), // 창 안, 다른 모델
            ev(Source::Claude, "claude-opus-5", "2026-01-01T01:00:00Z", 40), // 창 밖
        ];
        let s = build_summary(&evs, &[(Source::Claude, SourceStatus::Ok)], &pricing, days, now, off);

        // 창 밖 이벤트는 빠진다 — 모델은 둘, 출력은 10+20
        assert_eq!(s.models_period.len(), 2, "창 안의 모델만");
        let tok: u64 = s.models_period.iter().map(|m| m.totals.total()).sum();
        let from_daily: u64 = s.daily.iter().map(|d| d.totals.total()).sum();
        assert_eq!(tok, from_daily, "기간 모델 합 = daily 합");
        let cost: f64 = s.models_period.iter().map(|m| m.cost).sum();
        let cost_daily: f64 = s.daily.iter().map(|d| d.cost).sum();
        assert!((cost - cost_daily).abs() < 1e-9, "비용도 같아야 한다");

        // 오늘치는 그대로 오늘만 — 기간이 오늘을 덮어쓰지 않는다
        assert_eq!(s.models_today.len(), 1);
        assert_eq!(s.models_today[0].totals.output, 10);
    }

    /// 오늘 구성의 합은 오늘 총액과 같다.
    #[test]
    fn today_parts_sum_to_today_cost() {
        let pricing = PriceTable::builtin();
        let off = FixedOffset::east_opt(0).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:00Z").unwrap().with_timezone(&Utc);
        let evs = vec![
            ev(Source::Claude, "claude-opus-5", "2026-08-23T01:00:00Z", 100),
            ev(Source::Codex, "gpt-5.6-terra", "2026-08-23T02:00:00Z", 50),
        ];
        let statuses = [(Source::Claude, SourceStatus::Ok), (Source::Codex, SourceStatus::Ok)];
        let s = build_summary(&evs, &statuses, &pricing, 28, now, off);
        assert!((s.today_parts.total() - s.today_cost).abs() < 1e-9);
        let per_src: f64 = s.sources.iter().map(|x| x.today_parts.total()).sum();
        assert!((per_src - s.today_cost).abs() < 1e-9, "벤더별 구성 합 = 전체 오늘 비용");
    }

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
        // 0 = 기간 제한 없음. 하한(4주)으로 떨어지면 "전체"를 골랐는데 격자가
        // 가장 짧아지는 정반대 결과가 된다
        assert_eq!(daily_window(0), 182, "제한 없음도 상한 26주");
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
            cache_write_1h: 0,
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

        // 날짜 상세도 `daily`와 같은 합계를 가리켜야 선택 카드가 기간 총합과 어긋나지 않는다.
        for detail in &s.daily_details {
            let day = s.daily.iter().find(|d| d.date == detail.date).unwrap();
            assert_eq!(
                detail.models.iter().map(|m| m.totals.total()).sum::<u64>(),
                day.totals.total(),
                "모델 합: {}",
                detail.date,
            );
            assert_eq!(
                detail.sources.iter().map(|s| s.totals.total()).sum::<u64>(),
                day.totals.total(),
                "벤더 합: {}",
                detail.date,
            );
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

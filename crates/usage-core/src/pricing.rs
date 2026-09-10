//! 모델 단가표. 내장 스냅샷(pricing/prices.json) + 사용자 오버라이드 병합.
//! 모델 ID는 날짜 접미사가 붙을 수 있으므로(예: claude-sonnet-4-5-20250929)
//! **최장 접두사 매칭**으로 조회한다.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::UsageEvent;

/// 비용을 **토큰 종류별로** 쪼갠 것.
///
/// 토큰 수([`crate::aggregate::Totals`])와는 다른 축이다 — 종류마다 단가가 20배까지
/// 차이나서, 토큰 비중과 비용 비중이 서로를 예측하지 못한다 (실측: 캐시 읽기가 토큰의
/// 98.6% 인데 비용은 73.5%, 출력은 토큰 0.3% 인데 비용 11.6%). 그 괴리를 화면이
/// 보여주려면 비용도 갈라서 와야 하고, 단가는 모델마다 달라서 프론트가 나눌 수 없다.
#[derive(Default, Clone, Copy, Debug, Serialize)]
pub struct CostParts {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    /// **캐시가 없었다면** 들었을 비용 — 캐시로 오간 입력을 전부 정가 입력으로 친 값.
    ///
    /// 네 항목의 형제가 아니라 **비교용 반사실**이라 [`CostParts::total`] 에 들어가지
    /// 않는다 (`total_excludes_uncached` 가 지킨다). `cache_write_1h` 와 같은 규칙이다.
    pub uncached: f64,
}

impl CostParts {
    /// 실제 비용. `uncached` 는 반사실이라 빠진다.
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    pub fn add(&mut self, o: &CostParts) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.uncached += o.uncached;
    }
}

const BUILTIN: &str = include_str!("../pricing/prices.json");

/// USD / 1M tokens
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Price {
    #[serde(rename = "in")]
    pub input: f64,
    #[serde(rename = "out")]
    pub output: f64,
    /// cache write (5분 TTL 기준)
    pub cw: f64,
    /// cache write (1시간 TTL). 없으면 [`Price::cw`] 로 물러난다 — 사용자 오버라이드
    /// 파일이 이 키를 모르던 시절 그대로여도 깨지지 않아야 한다.
    #[serde(default)]
    pub cw1h: Option<f64>,
    /// cache read
    pub cr: f64,
    /// 컨텍스트 창 크기(토큰). 확인된 모델에만 채워져 있고, 없으면 `context` 모듈이
    /// 기본값에서 시작해 실제 관측치로 승격한다.
    #[serde(default)]
    pub ctx: Option<u64>,
}

#[derive(Deserialize)]
struct PriceFile {
    #[serde(default)]
    models: HashMap<String, Price>,
}

pub struct PriceTable {
    /// key 길이 내림차순 정렬 (최장 접두사 우선)
    entries: Vec<(String, Price)>,
}

impl PriceTable {
    pub fn from_maps(maps: &[HashMap<String, Price>]) -> Self {
        let mut merged: HashMap<String, Price> = HashMap::new();
        for m in maps {
            for (k, v) in m {
                merged.insert(k.clone(), *v); // 뒤의 맵(오버라이드)이 우선
            }
        }
        let mut entries: Vec<(String, Price)> = merged.into_iter().collect();
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Self { entries }
    }

    pub fn builtin() -> Self {
        let file: PriceFile = serde_json::from_str(BUILTIN).expect("내장 prices.json 파싱 실패");
        Self::from_maps(&[file.models])
    }

    /// 내장 단가 + 오버라이드 파일(JSON, 같은 스키마) 병합. 파일 오류는 무시하고 내장만 사용.
    pub fn with_overrides(path: Option<&Path>) -> Self {
        let builtin: PriceFile = serde_json::from_str(BUILTIN).expect("내장 prices.json 파싱 실패");
        let mut maps = vec![builtin.models];
        if let Some(p) = path {
            if let Ok(s) = std::fs::read_to_string(p) {
                if let Ok(f) = serde_json::from_str::<PriceFile>(&s) {
                    maps.push(f.models);
                }
            }
        }
        Self::from_maps(&maps)
    }

    pub fn lookup(&self, model: &str) -> Option<Price> {
        self.entries
            .iter()
            .find(|(prefix, _)| model.starts_with(prefix.as_str()))
            .map(|(_, p)| *p)
    }

    /// 모델의 컨텍스트 창(토큰). 단가표에 `ctx` 가 없으면 None.
    pub fn context_window(&self, model: &str) -> Option<u64> {
        self.lookup(model).and_then(|p| p.ctx)
    }

    /// 이벤트 비용(USD). 단가 미등록 모델은 None.
    pub fn cost(&self, ev: &UsageEvent) -> Option<f64> {
        self.cost_parts(ev).map(|p| p.total())
    }

    /// 이벤트 비용을 토큰 종류별로 쪼개 준다. 단가 미등록 모델은 None.
    pub fn cost_parts(&self, ev: &UsageEvent) -> Option<CostParts> {
        let p = self.lookup(&ev.model)?;
        // cache_write_1h 는 cache_write 의 **부분집합**이다 (model.rs 불변식).
        // 나머지가 5분 몫이고, 1h 단가가 없는 표는 전부 5분으로 친다.
        let cw_1h = ev.cache_write_1h.min(ev.cache_write);
        let cw_5m = ev.cache_write - cw_1h;
        const M: f64 = 1_000_000.0;
        Some(CostParts {
            input: ev.input as f64 * p.input / M,
            output: ev.output as f64 * p.output / M,
            cache_write: (cw_5m as f64 * p.cw + cw_1h as f64 * p.cw1h.unwrap_or(p.cw)) / M,
            cache_read: ev.cache_read as f64 * p.cr / M,
            // 캐시가 없었다면 캐시로 오간 입력도 전부 정가 입력이다.
            uncached: ((ev.input + ev.cache_write + ev.cache_read) as f64 * p.input
                + ev.output as f64 * p.output)
                / M,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use chrono::Utc;

    fn ev(model: &str, input: u64, output: u64, cw: u64, cr: u64) -> UsageEvent {
        ev_ttl(model, input, output, cw, 0, cr)
    }

    fn ev_ttl(model: &str, input: u64, output: u64, cw: u64, cw1h: u64, cr: u64) -> UsageEvent {
        UsageEvent {
            source: Source::Claude,
            model: model.into(),
            ts: Utc::now(),
            input,
            output,
            cache_write: cw,
            cache_write_1h: cw1h,
            cache_read: cr,
            sidechain: false,
        }
    }

    #[test]
    fn longest_prefix_matches_dated_model_ids() {
        let t = PriceTable::builtin();
        // 날짜 접미사가 붙은 풀 ID도 매칭되어야 함
        assert!(t.lookup("claude-sonnet-4-5-20250929").is_some());
        assert!(t.lookup("claude-haiku-4-5-20251001").is_some());
        // gpt-5-codex 는 gpt-5 보다 긴 접두사가 우선
        let codex = t.lookup("gpt-5-codex").unwrap();
        assert_eq!(codex.input, 1.25);
        let mini = t.lookup("gpt-5-mini-2025-08-07").unwrap();
        assert_eq!(mini.input, 0.25);
        // 미지의 모델은 None
        assert!(t.lookup("totally-unknown-model").is_none());
    }

    /// 2026-09-10 공식 페이지 대조값. 셋 다 2026-07-30 인하가 반영돼 있다.
    #[test]
    fn gpt_5_6_prices_and_alias_match_the_official_tiers() {
        let t = PriceTable::builtin();
        // 공식 표(developers.openai.com/api/docs/pricing) standard·short 구간.
        let cases = [
            ("gpt-5.6-sol", 4.0, 20.0, 5.0, 0.4),
            ("gpt-5.6-terra", 2.0, 12.0, 2.5, 0.2),
            ("gpt-5.6-luna", 0.2, 1.2, 0.25, 0.02),
            // 공식 표에 bare 행이 없다 — 알 수 없는 5.6 변종의 폴백이라 상위(Sol)에 맞춘다.
            ("gpt-5.6", 4.0, 20.0, 5.0, 0.4),
        ];
        for (model, input, output, cache_write, cache_read) in cases {
            let p = t.lookup(model).unwrap();
            assert_eq!((p.input, p.output, p.cw, p.cr), (input, output, cache_write, cache_read));
            assert_eq!(p.ctx, Some(1_050_000));
        }
    }

    #[test]
    fn gpt_6_astra_price_and_context_match_the_official_standard_tier() {
        let p = PriceTable::builtin().lookup("gpt-6-astra").unwrap();
        assert_eq!((p.input, p.output, p.cw, p.cr), (10.0, 50.0, 12.5, 1.0));
        assert_eq!(p.ctx, Some(1_050_000));
    }

    /// OpenAI 의 cache writes 요금은 Astra와 5.6 계열에만 있다 (공식 표 실측).
    /// 구세대에 값을 넣으면 없는 요금을 물리게 된다.
    #[test]
    fn openai_cache_writes_match_the_official_model_families() {
        let t = PriceTable::builtin();
        for m in [
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ] {
            assert!(t.lookup(m).unwrap().cw > 0.0, "{m} 는 cache writes 요금이 있다");
        }
        for m in [
            "gpt-5.3-codex",
            "gpt-5.1",
            "gpt-5",
            "gpt-5-mini",
            "gpt-5-nano",
            "gpt-4o",
        ] {
            assert_eq!(t.lookup(m).unwrap().cw, 0.0, "{m} 에는 cache writes 요금이 없다");
        }
        // TTL 선택이 없으므로 1시간 단가도 없다 → cost() 가 cw 로 물러난다
        assert!(t.lookup("gpt-6-astra").unwrap().cw1h.is_none());
        assert!(t.lookup("gpt-5.6-terra").unwrap().cw1h.is_none());
    }

    #[test]
    fn claude_5_1_models_use_the_reduced_cache_read_price() {
        let t = PriceTable::builtin();
        for m in ["claude-fable-5-1", "claude-mythos-5-1"] {
            let p = t.lookup(m).unwrap();
            assert_eq!(
                (p.input, p.output, p.cw, p.cw1h, p.cr),
                (10.0, 50.0, 12.5, Some(20.0), 0.25)
            );
            assert_eq!(p.ctx, Some(1_000_000));
        }
        // `-5`도 접두사이므로 최장 접두사 매칭이 5.1 가격을 제대로 선택해야 한다.
        assert_eq!(t.lookup("claude-fable-5").unwrap().cr, 1.0);
    }

    /// 지원 대상 모델은 표에 있어야 한다 — 없으면 토큰만 세고 비용은 0 이 된다.
    #[test]
    fn supported_models_have_prices() {
        let t = PriceTable::builtin();
        for m in [
            "gemini-3.8-flash",
            "gemini-3.7-flash",
            "gemini-3.6-flash",
            "gemini-3.5-flash",
            "gemini-3.5-flash-lite",
            "gpt-6-astra",
            "gpt-5.6-terra",
            "gpt-5.3-codex",
            "claude-fable-5-1",
            "claude-mythos-5-1",
        ] {
            assert!(t.lookup(m).is_some(), "{m} 단가 없음");
        }
        // 3.6~3.8은 같은 도입 단가다. 예전엔 3.6에 2.5-flash 값이
        // 들어가 있어 agy 비용이 2.5배 낮게 잡혔다.
        for m in [
            "gemini-3.6-flash",
            "gemini-3.7-flash",
            "gemini-3.8-flash",
        ] {
            let p = t.lookup(m).unwrap();
            assert_eq!((p.input, p.output, p.cr), (0.75, 3.75, 0.075));
            assert_eq!(p.ctx, Some(1_048_576));
        }
        assert_eq!(
            {
                let p = t.lookup("gemini-3.5-flash").unwrap();
                (p.input, p.output, p.cr)
            },
            (1.5, 9.0, 0.15)
        );
        assert_eq!(
            {
                let p = t.lookup("gemini-3.5-flash-lite").unwrap();
                (p.input, p.output, p.cr)
            },
            (0.3, 2.5, 0.03)
        );
    }

    /// 구성 비용의 합은 총 비용과 같아야 한다 — 화면이 둘을 나란히 놓기 때문이다.
    /// 그리고 `uncached` 는 반사실이라 합에 들어가면 안 된다.
    #[test]
    fn parts_sum_to_cost_and_total_excludes_uncached() {
        let t = PriceTable::builtin();
        let e = ev_ttl("claude-opus-5", 1_000, 2_000, 3_000, 1_000, 4_000);
        let p = t.cost_parts(&e).unwrap();
        let c = t.cost(&e).unwrap();
        assert!((p.total() - c).abs() < 1e-12, "구성 합 {} vs 총액 {c}", p.total());
        assert!(p.uncached > p.total(), "캐시가 없었다면 더 비싸야 한다");
        // uncached 를 합에 넣으면 이 등식이 깨진다
        assert!((p.input + p.output + p.cache_write + p.cache_read - p.total()).abs() < 1e-12);
    }

    /// 캐시 읽기가 많을수록 `uncached` 와의 격차가 벌어진다 — 효율 지표의 근거다.
    #[test]
    fn uncached_reflects_cache_savings() {
        let t = PriceTable::builtin();
        // 캐시 읽기 100만: 실제는 cr 단가(0.5), 캐시가 없었다면 입력 단가(5.0)
        let p = t.cost_parts(&ev_ttl("claude-opus-5", 0, 0, 0, 0, 1_000_000)).unwrap();
        assert!((p.total() - 0.5).abs() < 1e-9);
        assert!((p.uncached - 5.0).abs() < 1e-9, "10배");
    }

    /// 1시간 TTL 몫은 `cw1h` 단가로, 나머지는 `cw` 로 — 부분집합이지 별도 종류가 아니다.
    #[test]
    fn cache_write_splits_by_ttl() {
        let t = PriceTable::builtin();
        // opus-5: in 5 / cw 6.25 / cw1h 10 — 1M 캐시쓰기가 전부 1시간이면 $10
        let all_1h = t.cost(&ev_ttl("claude-opus-5", 0, 0, 1_000_000, 1_000_000, 0)).unwrap();
        assert!((all_1h - 10.0).abs() < 1e-9, "1시간 전액: {all_1h}");
        // 전부 5분이면 $6.25
        let all_5m = t.cost(&ev_ttl("claude-opus-5", 0, 0, 1_000_000, 0, 0)).unwrap();
        assert!((all_5m - 6.25).abs() < 1e-9, "5분 전액: {all_5m}");
        // 반반이면 그 중간
        let half = t.cost(&ev_ttl("claude-opus-5", 0, 0, 1_000_000, 500_000, 0)).unwrap();
        assert!((half - 8.125).abs() < 1e-9, "반반: {half}");
        // 1시간 몫이 총량을 넘어와도 총량을 넘겨 과금하지 않는다
        let over = t.cost(&ev_ttl("claude-opus-5", 0, 0, 1_000_000, 9_000_000, 0)).unwrap();
        assert!((over - 10.0).abs() < 1e-9, "총량 상한: {over}");
    }

    /// `cw1h` 가 없는 표(옛 사용자 오버라이드)는 전부 5분 단가로 물러난다.
    #[test]
    fn missing_cw1h_falls_back_to_cw() {
        let mut m = std::collections::HashMap::new();
        m.insert("x".to_string(), Price { input: 1.0, output: 1.0, cw: 2.0, cw1h: None, cr: 1.0, ctx: None });
        let t = PriceTable::from_maps(&[m]);
        let c = t.cost(&ev_ttl("x", 0, 0, 1_000_000, 1_000_000, 0)).unwrap();
        assert!((c - 2.0).abs() < 1e-9, "폴백: {c}");
    }

    /// **이중 계산 방지** — 1시간 몫은 부분집합이라 총 토큰 수를 늘리지 않는다.
    #[test]
    fn total_ignores_cache_write_1h() {
        let a = ev_ttl("claude-opus-5", 1, 2, 100, 0, 4);
        let b = ev_ttl("claude-opus-5", 1, 2, 100, 100, 4);
        assert_eq!(a.total(), b.total(), "1시간 몫이 total() 을 바꾸면 안 된다");
        assert_eq!(a.total(), 1 + 2 + 100 + 4);
    }

    #[test]
    fn cost_math() {
        let t = PriceTable::builtin();
        // opus-4-8: in 5, out 25, cw 6.25, cr 0.5 (USD/MTok)
        let e = ev("claude-opus-4-8", 1_000_000, 1_000_000, 1_000_000, 1_000_000);
        let c = t.cost(&e).unwrap();
        assert!((c - (5.0 + 25.0 + 6.25 + 0.5)).abs() < 1e-9);
        assert!(t.cost(&ev("mystery", 10, 10, 0, 0)).is_none());
    }

    #[test]
    fn overrides_take_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("override.json");
        std::fs::write(&p, r#"{"models":{"claude-opus-4-8":{"in":1.0,"out":2.0,"cw":0.0,"cr":0.0}}}"#).unwrap();
        let t = PriceTable::with_overrides(Some(&p));
        assert_eq!(t.lookup("claude-opus-4-8").unwrap().input, 1.0);
        // 오버라이드에 없는 모델은 내장 유지
        assert_eq!(t.lookup("claude-fable-5").unwrap().input, 10.0);
    }
}

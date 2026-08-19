//! 모델 단가표. 내장 스냅샷(pricing/prices.json) + 사용자 오버라이드 병합.
//! 모델 ID는 날짜 접미사가 붙을 수 있으므로(예: claude-sonnet-4-5-20250929)
//! **최장 접두사 매칭**으로 조회한다.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::model::UsageEvent;

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
        let p = self.lookup(&ev.model)?;
        Some(
            (ev.input as f64 * p.input
                + ev.output as f64 * p.output
                + ev.cache_write as f64 * p.cw
                + ev.cache_read as f64 * p.cr)
                / 1_000_000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use chrono::Utc;

    fn ev(model: &str, input: u64, output: u64, cw: u64, cr: u64) -> UsageEvent {
        UsageEvent {
            source: Source::Claude,
            model: model.into(),
            ts: Utc::now(),
            input,
            output,
            cache_write: cw,
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

    /// 2026-08-19 공식 페이지 대조값. 셋 다 2026-07-30 인하가 반영돼 있고
    /// (Luna −80%, Terra −20%, Sol 동결) 캐시 **쓰기**는 과금하지 않아 cw 가 0 이다 —
    /// 예전엔 Anthropic 의 1.25배 규칙을 그대로 적어 두어 없는 요금을 만들어 냈다.
    #[test]
    fn gpt_5_6_prices_and_alias_match_the_official_tiers() {
        let t = PriceTable::builtin();
        let cases = [
            ("gpt-5.6-sol", 5.0, 30.0, 0.0, 0.5),
            ("gpt-5.6-terra", 2.0, 12.0, 0.0, 0.2),
            ("gpt-5.6-luna", 0.2, 1.2, 0.0, 0.02),
            // 공식 별칭 `gpt-5.6` 은 Sol 로 라우팅된다.
            ("gpt-5.6", 5.0, 30.0, 0.0, 0.5),
        ];
        for (model, input, output, cache_write, cache_read) in cases {
            let p = t.lookup(model).unwrap();
            assert_eq!((p.input, p.output, p.cw, p.cr), (input, output, cache_write, cache_read));
            assert_eq!(p.ctx, Some(1_050_000));
        }
    }

    /// 실제로 관측되는 모델은 표에 있어야 한다 — 없으면 토큰만 세고 비용은 0 이 된다.
    /// (agy 가 gemini-3.7-flash 로 넘어갔을 때 실제로 겪었다)
    #[test]
    fn models_seen_in_the_wild_have_prices() {
        let t = PriceTable::builtin();
        for m in ["gemini-3.7-flash", "gemini-3.6-flash", "gpt-5.6-terra", "gpt-5.3-codex"] {
            assert!(t.lookup(m).is_some(), "{m} 단가 없음");
        }
        // 3.6 과 3.7 은 같은 단가다 (공식 표에서 같은 줄). 예전엔 3.6 에 2.5-flash 값이
        // 들어가 있어 agy 비용이 2.5배 낮게 잡혔다.
        let (a, b) = (t.lookup("gemini-3.6-flash").unwrap(), t.lookup("gemini-3.7-flash").unwrap());
        assert_eq!((a.input, a.output, a.cr), (b.input, b.output, b.cr));
        assert_eq!((a.input, a.output), (0.75, 3.75));
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

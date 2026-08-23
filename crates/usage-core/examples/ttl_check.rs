//! 실데이터 검증용: 캐시 쓰기의 5분/1시간 몫이 실제로 갈려 단가에 반영되는지 확인.
//! 사용: cargo run -p usage-core --example ttl_check
//!
//! 원본 JSONL 을 직접 세면 안 된다 — `message.id` dedup 을 거치지 않아 약 2배로 부푼다.
//! 반드시 어댑터를 통과한 이벤트로 대조한다.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use usage_core::claude::ClaudeAdapter;
use usage_core::pricing::PriceTable;

fn main() {
    let mut adapter = ClaudeAdapter::with_default_roots();
    let out = adapter.scan(DateTime::<Utc>::UNIX_EPOCH);
    let t = PriceTable::builtin();

    // 모델별 (cw 총량, 그중 1h, 실제 비용, 전부 5분으로 쳤을 때의 비용)
    let mut per: BTreeMap<String, (u64, u64, f64, f64)> = BTreeMap::new();
    for e in &out.events {
        let x = per.entry(e.model.clone()).or_default();
        x.0 += e.cache_write;
        x.1 += e.cache_write_1h;
        if let Some(c) = t.cost(e) {
            x.2 += c;
            let mut flat = e.clone();
            flat.cache_write_1h = 0; // 비교용: TTL 내역을 모르던 시절
            x.3 += t.cost(&flat).unwrap_or(0.0);
        }
    }

    println!("{:<30} {:>13} {:>13} {:>7} {:>11} {:>11}", "model", "cache_write", "of which 1h", "1h%", "cost", "as-if-5m");
    let (mut a, mut b) = (0.0, 0.0);
    for (m, (cw, cw1h, real, flat)) in &per {
        if *cw == 0 {
            continue;
        }
        println!("{m:<30} {cw:>13} {cw1h:>13} {:>6.1}% {real:>11.2} {flat:>11.2}", 100.0 * *cw1h as f64 / *cw as f64);
        a += real;
        b += flat;
    }

    let no_ttl = out.events.iter().filter(|e| e.cache_write > 0 && e.cache_write_1h == 0).count();
    println!("\nevents={} (deduped), with cache_write={}, of which no TTL breakdown={no_ttl}",
        out.events.len(), out.events.iter().filter(|e| e.cache_write > 0).count());
    println!("total ${a:.2} vs as-if-5m ${b:.2}  ->  {:+.2}%", 100.0 * (a - b) / b);
}

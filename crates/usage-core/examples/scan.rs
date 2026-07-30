//! 실데이터 검증용: 실제 ~/.claude 를 스캔해 모델별 합계를 출력.
//! 사용: cargo run -p usage-core --example scan [days]

use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use usage_core::claude::ClaudeAdapter;
use usage_core::pricing::PriceTable;

fn main() {
    let days: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3650);
    let since = Utc::now() - Duration::days(days);

    let mut adapter = ClaudeAdapter::with_default_roots();
    let t0 = std::time::Instant::now();
    let out = adapter.scan(since);
    eprintln!("[1st scan] {:?}, events={}", t0.elapsed(), out.events.len());

    let mut per: BTreeMap<String, (u64, u64, u64, u64, u64)> = BTreeMap::new();
    for e in &out.events {
        let x = per.entry(e.model.clone()).or_default();
        x.0 += e.input;
        x.1 += e.output;
        x.2 += e.cache_write;
        x.3 += e.cache_read;
        x.4 += 1;
    }
    for (m, (i, o, cw, cr, n)) in &per {
        println!("{m} in={i} out={o} cw={cw} cr={cr} n={n}");
    }

    let pricing = PriceTable::builtin();
    let cost: f64 = out.events.iter().filter_map(|e| pricing.cost(e)).sum();
    println!("est_total_cost_usd={cost:.2}");

    let t1 = std::time::Instant::now();
    let out2 = adapter.scan(since);
    eprintln!("[cached rescan] {:?}, events={}", t1.elapsed(), out2.events.len());
}

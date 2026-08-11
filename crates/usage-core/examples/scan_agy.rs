//! 실데이터 검증용: 실제 `~/.gemini/antigravity-cli` 를 스캔해 요청별 사용량과
//! 컨텍스트를 출력한다.
//! 사용: cargo run -p usage-core --example scan_agy [days]

use chrono::{Duration, Utc};
use usage_core::antigravity::AntigravityAdapter;
use usage_core::pricing::PriceTable;

fn main() {
    let days: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3650);
    let since = Utc::now() - Duration::days(days);

    let homes = usage_core::roots::antigravity_homes();
    println!("homes = {homes:?}");

    let mut adapter = AntigravityAdapter::new(homes);
    let t0 = std::time::Instant::now();
    let out = adapter.scan(since);
    println!("[1st scan] {:?}, status={:?}, events={}", t0.elapsed(), out.status, out.events.len());

    let t1 = std::time::Instant::now();
    let again = adapter.scan(since);
    println!("[2nd scan] {:?} (캐시 적중), events={}", t1.elapsed(), again.events.len());

    println!("\n{:<20} {:<18} {:>8} {:>8} {:>9} {:>9}", "시각", "모델", "입력", "출력", "캐시읽기", "합계");
    let mut sum = (0u64, 0u64, 0u64);
    for e in &out.events {
        println!(
            "{:<20} {:<18} {:>8} {:>8} {:>9} {:>9}",
            e.ts.format("%Y-%m-%d %H:%M:%S").to_string(),
            e.model,
            e.input,
            e.output,
            e.cache_read,
            e.total()
        );
        sum = (sum.0 + e.input, sum.1 + e.output, sum.2 + e.cache_read);
    }
    println!("{:<39} {:>8} {:>8} {:>9}", "합계", sum.0, sum.1, sum.2);

    let pricing = PriceTable::builtin();
    match adapter.context(&pricing) {
        Some(c) => println!(
            "\n컨텍스트: {}/{} ({:.1}%) · 모델 {} · 세션 {} · 창 추정={} · 시각 {:?}",
            c.tokens, c.window, c.used_pct, c.model, c.session, c.window_inferred, c.at
        ),
        None => println!("\n컨텍스트: 없음"),
    }
}

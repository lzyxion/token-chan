//! 화면이 그릴 값을 실데이터로 확인 (설계 검증용).
//! 사용: cargo run -p usage-core --example efficiency

use chrono::{DateTime, Local, Utc};
use usage_core::aggregate::{build_summary, daily_window};
use usage_core::antigravity::AntigravityAdapter;
use usage_core::claude::ClaudeAdapter;
use usage_core::codex::CodexAdapter;
use usage_core::model::{Source, SourceStatus};
use usage_core::pricing::PriceTable;

fn main() {
    let since = DateTime::<Utc>::UNIX_EPOCH;
    let mut evs = vec![];
    evs.extend(ClaudeAdapter::with_default_roots().scan(since).events);
    evs.extend(CodexAdapter::with_default_roots().scan(since).events);
    evs.extend(AntigravityAdapter::with_default_roots().scan(since).events);
    evs.sort_by_key(|e| e.ts);

    let statuses: Vec<_> = Source::ALL.iter().map(|s| (*s, SourceStatus::Ok)).collect();
    let days = daily_window(90);
    let s = build_summary(&evs, &statuses, &PriceTable::builtin(), days, Utc::now(), *Local::now().offset());

    let period_cost: f64 = s.daily.iter().map(|d| d.cost).sum();
    println!("=== 개요 ({days}일) ===");
    println!("오늘  {} tok  ${:.2}", s.today.total(), s.today_cost);
    println!("기간  {} tok  ${:.2}", s.daily.iter().map(|d| d.totals.total()).sum::<u64>(), period_cost);
    println!("\n벤더별 비중 (기간)");
    for v in &s.sources {
        if v.period_cost == 0.0 {
            continue;
        }
        println!("  {:<18} ${:>9.2}  {:>5.1}%", v.label, v.period_cost, 100.0 * v.period_cost / period_cost);
    }

    for v in &s.sources {
        let p = &v.period_parts;
        if p.total() == 0.0 {
            continue;
        }
        let (t, c) = (&v.period, p.total());
        println!("\n=== {} 상세 ===", v.label);
        let rows = [
            ("캐시읽기", t.cache_read, p.cache_read),
            ("캐시쓰기", t.cache_write, p.cache_write),
            ("출력", t.output, p.output),
            ("입력", t.input, p.input),
        ];
        for (name, tok, cost) in rows {
            println!("  {name:<10} 비용 {:>5.1}%  ${:>9.2}   (토큰 {:>5.1}%)",
                100.0 * cost / c, cost, 100.0 * tok as f64 / t.total() as f64);
        }
        println!("  ── 효율 ──");
        if t.cache_write > 0 {
            println!("  캐시 재사용  {:.1}배", t.cache_read as f64 / t.cache_write as f64);
        } else {
            println!("  캐시 재사용  (표시 안 함 — 이 벤더는 캐시 쓰기를 기록하지 않는다)");
        }
        println!("  캐시 절감    ${:.2} → ${:.2}  ({:.1}배)", p.uncached, c, p.uncached / c);
    }
}

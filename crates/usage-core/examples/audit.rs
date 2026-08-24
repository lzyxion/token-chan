//! 화면 요소가 실제로 정보를 주는지 점검 (설계 검토용).
//! 사용: cargo run -p usage-core --example audit

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

    println!("== 잔디 ({days}일) ==");
    let nonzero = s.daily.iter().filter(|d| d.totals.total() > 0).count();
    let costs: Vec<f64> = s.daily.iter().map(|d| d.cost).collect();
    let mx = costs.iter().cloned().fold(0.0f64, f64::max);
    println!("  기록 있는 날 {nonzero}/{days}  ·  최대 일비용 ${mx:.2}");
    // 잔디는 5단계로 나눈다 — 값이 한 단계에 몰리면 색 차이가 정보를 안 준다
    let mut lv = [0usize; 5];
    for c in &costs {
        let i = if *c <= 0.0 { 0 } else { (((c / mx) * 4.0).ceil() as usize).min(4) };
        lv[i] += 1;
    }
    println!("  단계 분포 (0=빈칸): {lv:?}");

    println!("\n== 최근 7일 ==");
    for d in s.daily.iter().rev().take(7).rev() {
        println!("  {} tok {:>12} ${:>7.2}", d.date, d.totals.total(), d.cost);
    }

    println!("\n== 오늘 모델 ==");
    for m in &s.models_today {
        println!("  {:<34} {:>12} tok  ${:>8.4}  단가앎={}", m.model, m.totals.total(), m.cost, m.cost_known);
    }

    println!("\n== 최근 세션 {}개 ==", s.sessions.len());
    for r in s.sessions.iter().take(6) {
        println!("  [{:?}] {:<28} {:>10} tok  {}", r.source, r.label.chars().take(28).collect::<String>(), r.tokens, r.cwd);
    }

    let t = PriceTable::builtin();
    let start = Utc::now() - chrono::Duration::days(days as i64);
    let mut pm: std::collections::BTreeMap<(Source, String), (u64, f64)> = Default::default();
    for e in evs.iter().filter(|e| e.ts >= start) {
        let x = pm.entry((e.source, e.model.clone())).or_default();
        x.0 += e.total();
        x.1 += t.cost(e).unwrap_or(0.0);
    }
    let mut v: Vec<_> = pm.into_iter().collect();
    v.sort_by(|a, b| b.1 .1.partial_cmp(&a.1 .1).unwrap());
    let tot: f64 = v.iter().map(|x| x.1 .1).sum();
    // 날짜별 구성비·효율 — 일별로 보여줄 값어치가 있는지 판단하려면 변동폭을 봐야 한다
    let tz = *Local::now().offset();
    let mut byday: std::collections::BTreeMap<String, (u64, u64, u64, u64, f64, f64)> =
        Default::default();
    let tt = PriceTable::builtin();
    for e in &evs {
        let d = e.ts.with_timezone(&tz).date_naive().to_string();
        let x = byday.entry(d).or_default();
        x.0 += e.input;
        x.1 += e.output;
        x.2 += e.cache_write;
        x.3 += e.cache_read;
        if let Some(pp) = tt.cost_parts(e) {
            x.4 += pp.total();
            x.5 += pp.uncached;
        }
    }
    for d in s.daily.iter().filter(|d| d.totals.total() > 0) {
    // 날짜별 모델 집중도 — "오늘 구성" 그래프가 날마다 정보를 주는지
    let tz2 = *Local::now().offset();
    let mut dm: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> =
        Default::default();
    for e in &evs {
        let d = e.ts.with_timezone(&tz2).date_naive().to_string();
        *dm.entry(d).or_default().entry(e.model.clone()).or_default() += e.total();
    }
    for (d, ms) in dm.iter().rev().take(20) {
        let tot: u64 = ms.values().sum();
        if tot == 0 { continue; }
        let mut v: Vec<u64> = ms.values().cloned().collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        let top = 100.0 * v[0] as f64 / tot as f64;
        let second = v.get(1).map(|x| 100.0 * *x as f64 / tot as f64).unwrap_or(0.0);
        println!("CONC {d} 모델수={:<2} 1위={top:>5.1}% 2위={second:>5.1}%", ms.len());
    }

        println!("ALLDAY {} {} {:.6}", d.date, d.totals.total(), d.cost);
    }

    println!("[daily mix] date        cr%   cw%  out%   in%  | hit%  reuse  save  $cost");
    let keys: Vec<_> = byday.keys().cloned().collect();
    for d in keys.iter().rev().take(14).rev() {
        let (i2, o, cw, cr, cost, unc) = byday[d];
        if cost <= 0.0 { continue; }
        let pc = |x: f64| 100.0 * x / cost;
        let (ci, co, ccw, ccr) = {
            let p2 = tt.lookup("claude-opus-5").unwrap();
            let _ = p2;
            // 비용 구성은 이벤트 단위로 이미 더했으니 토큰 대신 비율만 다시 계산
            (0.0, 0.0, 0.0, 0.0)
        };
        let _ = (ci, co, ccw, ccr);
        let readable = i2 + cw + cr;
        let hit = if readable > 0 { 100.0 * cr as f64 / readable as f64 } else { 0.0 };
        let reuse = if cw > 0 { cr as f64 / cw as f64 } else { 0.0 };
        println!(
            "            {d}  tok cr {:>4.1}% cw {:>4.1}% out {:>4.1}% | hit {:>5.1}%  x{:>5.1}  x{:>4.1}  ${:>7.2}",
            100.0 * cr as f64 / (i2 + o + cw + cr) as f64,
            100.0 * cw as f64 / (i2 + o + cw + cr) as f64,
            100.0 * o as f64 / (i2 + o + cw + cr) as f64,
            hit, reuse, unc / cost, cost
        );
        let _ = pc;
    }

    println!("[period models] kinds={}", v.len());
    for ((src, m), (tok, c)) in v.iter().take(10) {
        println!("  [{src:?}] {m:<28} {tok:>13} tok  ${c:>9.2}  {:>5.1}%", 100.0 * c / tot);
    }
    let mut cs: Vec<f64> = s.daily.iter().map(|d| d.cost).filter(|c| *c > 0.0).collect();
    cs.sort_by(|a, b| b.partial_cmp(a).unwrap());
    println!("[daily cost desc] {}", cs.iter().map(|c| format!("{c:.0}")).collect::<Vec<_>>().join(" "));

    println!("\n== 컨텍스트 ==");
    for c in &s.contexts {
        println!("  [{:?}] {}/{} ({:.0}%) compact={} model={}", c.source, c.tokens, c.window, c.used_pct, c.compactions, c.model);
    }
}

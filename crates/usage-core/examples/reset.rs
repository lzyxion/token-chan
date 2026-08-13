//! 실데이터 검증: 5시간 창 리셋 시각을 **공식 캐시와 우리 계산** 둘 다 내서 대조한다.
//! 사용: cargo run -p usage-core --example reset
//!
//! 공식 값이 신선하면 둘이 같아야 한다. 어긋나면 `blocks::ANCHOR_MINUTES` 를 의심한다
//! (15분/30분을 가르는 표본을 아직 못 봤다 — blocks.rs 모듈 주석 참고).

use chrono::Utc;

fn main() {
    let now = Utc::now();
    let mut claude = usage_core::claude::ClaudeAdapter::with_default_roots();
    // 캐시를 채우려면 한 번 스캔해야 한다 (타임스탬프는 파싱 중에 모인다)
    let since = now - chrono::Duration::days(7);
    let outcome = claude.scan(since);
    println!("스캔: 이벤트 {}건, 상태 {:?}\n", outcome.events.len(), outcome.status);

    let computed = claude.session_reset(now);
    println!("우리 계산 (트랜스크립트 → 30분 내림 + 5h)");
    match computed {
        Some(end) => {
            let rem = (end - now).num_minutes();
            println!("  종료 {end}  ({}h {}m 남음)", rem / 60, rem % 60);
            println!("  로컬 {}", end.with_timezone(&chrono::Local).format("%m-%d %H:%M"));
        }
        None => println!("  열린 창 없음 (마지막 활동 뒤 5시간 경과)"),
    }

    println!("\n공식 캐시 (.claude.json)");
    let accounts =
        usage_core::accounts::discover(&usage_core::accounts::ExtraHomes::default(), true);
    let mut best: Option<usage_core::plan::PlanUsage> = None;
    for a in accounts.iter().filter(|a| a.source == usage_core::model::Source::Claude) {
        for i in &a.installs {
            if let Some(p) = usage_core::plan::read_utilization(&i.home) {
                if best.as_ref().map(|b| p.fetched_at > b.fetched_at).unwrap_or(true) {
                    best = Some(p);
                }
            }
        }
    }
    match best {
        None => println!("  못 읽음"),
        Some(p) => {
            let age = (now - p.fetched_at).num_minutes();
            println!("  받아온 시각 {} ({age}분 전){}", p.fetched_at, if age > 30 { "  ← 낡음" } else { "" });
            for m in &p.meters {
                match m.resets_at {
                    Some(r) => {
                        let d = (r - now).num_minutes();
                        println!("  {:8} {:>3}%  리셋 {r} ({d:+}분)", m.label, m.used_pct);
                    }
                    None => println!("  {:8} {:>3}%  리셋 없음", m.label, m.used_pct),
                }
            }
            // 첫 미터(가장 짧은 창)와 우리 계산을 맞춰 본다
            if let (Some(off), Some(mine)) = (p.meters.first().and_then(|m| m.resets_at), computed) {
                let diff = (mine - off).num_minutes();
                println!("\n대조: 계산 − 공식 = {diff:+}분{}", match diff.abs() {
                    0 => "  (일치)",
                    1..=20 => "  (같은 창, 오차 범위)",
                    _ => "  ← 어긋남: 창이 갈렸거나 내림 단위가 틀렸다",
                });
            }
        }
    }
}

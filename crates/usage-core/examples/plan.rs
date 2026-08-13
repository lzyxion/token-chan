//! 실데이터 검증용: 공식 플랜 한도를 계정별로 출력.
//! 사용: cargo run -p usage-core --example plan
//!
//! 두 소스가 같은 모양으로 나오는지 보는 게 목적이다 — Claude 는 홈의 `.claude.json`,
//! Codex 는 rollout 의 `rate_limits` 로 경로만 다르고 결과는 같은 [`PlanUsage`] 다.

use chrono::{Duration, Utc};
use usage_core::accounts::{discover, ExtraHomes};
use usage_core::codex::CodexAdapter;
use usage_core::model::Source;
use usage_core::plan::{read_utilization, PlanUsage};

fn show(who: &str, p: &PlanUsage) {
    let age = Utc::now() - p.fetched_at;
    println!(
        "  {who}{}  (받아온 지 {}분)",
        if p.detail.is_empty() { String::new() } else { format!(" · {}", p.detail) },
        age.num_minutes()
    );
    for m in &p.meters {
        let resets = match m.resets_at {
            Some(t) => {
                let left = t - Utc::now();
                format!("{}h{}m 뒤", left.num_hours(), left.num_minutes() % 60)
            }
            None => "리셋 없음".into(),
        };
        println!("      {:<14} {:>3}%   {resets}", m.label, m.used_pct);
    }
}

fn main() {
    let accounts = discover(&ExtraHomes::default(), true);

    println!("=== Claude — 홈마다 .claude.json 직독 ===");
    let mut any = false;
    for a in accounts.iter().filter(|a| a.source == Source::Claude) {
        for i in &a.installs {
            match read_utilization(&i.home) {
                Some(p) => {
                    any = true;
                    show(&a.label, &p);
                    println!("      ↳ {}", i.home.display());
                }
                None => println!("  {} · 캐시 없음 — {}", a.label, i.home.display()),
            }
        }
    }
    if !any {
        println!("  (없음)");
    }

    println!("\n=== Codex — rollout 의 rate_limits ===");
    let mut codex = CodexAdapter::with_default_roots();
    codex.scan(Utc::now() - Duration::days(7));
    match codex.plan() {
        // 아직 홈별로 나누지 않는다 — 전체에서 가장 최근 것 하나다
        Some(p) => show("(전체 홈 중 최신)", &p),
        None => println!("  (없음)"),
    }
}

//! 실데이터 검증용: 공식 플랜 한도를 계정별로 출력.
//! 사용: cargo run -p usage-core --example plan
//!
//! 네 경로가 같은 모양으로 나오는지 보는 게 목적이다 — Claude 와 Codex 각각 ① 사용량
//! API 와 ② 파일 폴백(`.claude.json` 캐시 / rollout `rate_limits`)이 있고, 전부 같은
//! [`PlanUsage`] 다. ①②를 나란히 찍어 낡은 ②(굳은 캐시, 마지막 턴 이후의 rollout)를
//! 눈으로 확인할 수 있게 한다.

use chrono::{Duration, Utc};
use usage_core::accounts::{discover, ExtraHomes};
use usage_core::codex::CodexAdapter;
use usage_core::model::Source;
use usage_core::plan::{fetch_claude_usage, fetch_codex_usage, read_utilization, PlanUsage};

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

    println!("=== Claude — ① 사용량 API / ② .claude.json 캐시 ===");
    let mut any = false;
    for a in accounts.iter().filter(|a| a.source == Source::Claude) {
        for i in &a.installs {
            match fetch_claude_usage(&i.home, Utc::now()) {
                Some(p) => {
                    any = true;
                    show(&format!("{} · ① API", a.label), &p);
                }
                None => println!(
                    "  {} · ① API 불가 (자격증명 없음·토큰 만료·오프라인) — {}",
                    a.label,
                    i.home.display()
                ),
            }
            match read_utilization(&i.home) {
                Some(p) => {
                    any = true;
                    show(&format!("{} · ② 캐시", a.label), &p);
                    println!("      ↳ {}", i.home.display());
                }
                None => println!("  {} · ② 캐시 없음 — {}", a.label, i.home.display()),
            }
        }
    }
    if !any {
        println!("  (없음)");
    }

    println!("\n=== Codex — ① 사용량 API / ② rollout 의 rate_limits ===");
    for a in accounts.iter().filter(|a| a.source == Source::Codex) {
        for i in &a.installs {
            match fetch_codex_usage(&i.home, Utc::now()) {
                Some(p) => show(&format!("{} · ① API", a.label), &p),
                None => println!(
                    "  {} · ① API 불가 (자격증명 없음·토큰 만료·오프라인) — {}",
                    a.label,
                    i.home.display()
                ),
            }
        }
    }
    let mut codex = CodexAdapter::with_default_roots();
    codex.scan(Utc::now() - Duration::days(7));
    match codex.plan() {
        // ② 는 아직 홈별로 나누지 않는다 — 전체에서 가장 최근 것 하나다
        Some(p) => show("(전체 홈 중 최신) · ② rollout", &p),
        None => println!("  · ② rollout 없음"),
    }
}

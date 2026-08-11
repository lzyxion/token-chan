//! 실데이터 검증용: 세 어댑터에서 최근 세션 목록을 뽑아 출력.
//! 사용: cargo run -p usage-core --example sessions [days]

use chrono::{Duration, Utc};
use usage_core::antigravity::AntigravityAdapter;
use usage_core::claude::ClaudeAdapter;
use usage_core::codex::CodexAdapter;

fn main() {
    let days: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(14);
    let since = Utc::now() - Duration::days(days);

    let mut claude = ClaudeAdapter::with_default_roots();
    let mut codex = CodexAdapter::with_default_roots();
    let mut agy = AntigravityAdapter::with_default_roots();
    claude.scan(since);
    codex.scan(since);
    agy.scan(since);

    let rows = usage_core::session::merge(
        [claude.sessions(), codex.sessions(), agy.sessions()].concat(),
        15,
    );
    println!("{:<12} {:<28} {:<18} {:>10}  {}", "소스", "이름", "모델", "토큰", "시각");
    for r in &rows {
        let name: String = r.label.chars().take(26).collect();
        println!(
            "{:<12} {:<28} {:<18} {:>10}  {}  {}",
            format!("{:?}", r.source),
            name,
            r.model,
            r.tokens,
            r.at.format("%m-%d %H:%M"),
            if r.branch.is_empty() { r.cwd.clone() } else { format!("{} ({})", r.cwd, r.branch) }
        );
    }
    println!("\n총 {}개", rows.len());
}

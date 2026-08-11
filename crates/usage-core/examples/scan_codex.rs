//! Codex 어댑터 실데이터 검증용: 이 머신의 기본 루트(~/.codex 등)를 실제로 스캔해 출력한다.
//! 사용: cargo run -p usage-core --example scan_codex

use chrono::{DateTime, Utc};
use usage_core::codex::CodexAdapter;

fn main() {
    let mut adapter = CodexAdapter::with_default_roots();
    let out = adapter.scan(DateTime::<Utc>::UNIX_EPOCH);
    println!("status: {:?}", out.status);
    for e in &out.events {
        println!(
            "{}  {}  input={} output={} cache_read={} cache_write={} (total={})",
            e.ts,
            e.model,
            e.input,
            e.output,
            e.cache_read,
            e.cache_write,
            e.total()
        );
    }
    println!("events: {}", out.events.len());

    // 공식 한도는 rollout 안에 실려 온다 — Claude 처럼 CLI 를 띄우지 않는다
    match adapter.plan() {
        Some(p) => {
            println!("\n공식 한도 ({}):", if p.detail.is_empty() { "플랜 미상" } else { &p.detail });
            for m in &p.meters {
                println!("  {:<8} {:>3}%  리셋 {:?}", m.label, m.used_pct, m.resets_at);
            }
        }
        None => println!("\n공식 한도: 없음 (rate_limits 미기록)"),
    }
    println!("마지막 쓰기: {:?}", adapter.last_activity());
    println!("감시 파일: {:?}", adapter.watch_path());
}

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
}

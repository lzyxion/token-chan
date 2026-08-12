//! 작업 중 유도(`live::add_inferred`)가 보는 값을 그대로 찍어 보는 진단용.
//!
//! Codex·agy 는 세션 레지스트리가 없어 감시 파일의 mtime 으로 작업 중을 유도한다.
//! 그 전제(응답 중에 mtime 이 계속 움직인다)가 이 머신에서 실제로 성립하는지 본다.
//! CLI 에 프롬프트를 하나 날려 두고 실행하면 된다.

use std::time::{Duration, SystemTime};

use usage_core::antigravity::AntigravityAdapter;
use usage_core::codex::CodexAdapter;

fn main() {
    let since = chrono::Utc::now() - chrono::Duration::days(30);
    let mut codex = CodexAdapter::with_default_roots();
    let mut agy = AntigravityAdapter::with_default_roots();
    let _ = codex.scan(since);
    let _ = agy.scan(since);

    let mut watch = vec![];
    if let Some(p) = codex.watch_path() {
        watch.push(("codex", p));
    }
    if let Some(p) = agy.watch_path() {
        watch.push(("agy", p));
    }
    if watch.is_empty() {
        println!("감시할 파일 없음 (스캔된 세션이 없다)");
        return;
    }
    for (name, p) in &watch {
        println!("{name}: {}", p.display());
    }
    println!("\n{:<8} {:<6} {:>10} {:>10}  {}", "시각", "소스", "크기", "mtime나이", "판정");

    for _ in 0..40 {
        let now = chrono::Utc::now();
        for (name, path) in &watch {
            let meta = std::fs::metadata(path).ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta.as_ref().and_then(|m| m.modified().ok());
            let age = mtime
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(-1);
            let busy = usage_core::live::infer_busy(mtime.map(Into::into), now);
            println!(
                "{:<8} {:<6} {:>10} {:>9}s  {}",
                now.with_timezone(&chrono::Local).format("%H:%M:%S"),
                name,
                size,
                age,
                if busy { "작업 중" } else { "-" }
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

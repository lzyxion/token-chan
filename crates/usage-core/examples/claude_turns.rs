//! 실데이터 검증: 레지스트리가 상태를 안 주는 Claude 세션(TUI 없이 뜬 것)을
//! 트랜스크립트로 따라가는지 이 머신의 진짜 홈에 물려 돌린다.
//! 사용: cargo run -p usage-core --example claude_turns
//!
//! Obsidian 플러그인 같은 SDK 진입점 세션에 말을 걸면 BUSY 로 뜨고,
//! 답변이 끝나면 DONE 이 찍혀야 한다.

use std::collections::HashSet;

fn main() {
    let dirs: Vec<_> = usage_core::roots::claude_session_dirs();
    println!("세션 레지스트리 {}곳: {dirs:?}\n", dirs.len());

    let now = chrono::Utc::now();
    let live = usage_core::live::read_live_state(&dirs, now.timestamp_millis());
    println!("레지스트리에 보이는 세션 {}개:", live.sessions.len());
    for s in &live.sessions {
        let status = if s.status.is_empty() { "(상태 없음)" } else { s.status.as_str() };
        println!("  {:<10} {:<24} {}", status, s.name, s.cwd);
    }
    println!("\n감시 대상(상태를 안 주는 세션) {}개:", live.headless.len());
    for h in &live.headless {
        println!("  {}  <- {}", h.id, h.projects_root.display());
    }
    if live.headless.is_empty() {
        println!("  없음 — TUI 없이 띄운 Claude 세션이 있어야 확인됩니다.");
        return;
    }

    let mut w = usage_core::claude::TurnWatcher::default();
    let first = w.poll(&live.headless, now);
    println!(
        "\n첫 회차 — covered={} running={:?}  (첫 관측은 기준점만 잡으므로 비어야 정상)",
        first.covered, first.running
    );

    println!("\n이제 그 세션에 말을 걸어 보세요. 변화가 있을 때만 찍습니다 (Ctrl-C 로 종료)\n");
    let mut prev: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let now = chrono::Utc::now();
        let live = usage_core::live::read_live_state(&dirs, now.timestamp_millis());
        let poll = w.poll(&live.headless, now);
        for id in &poll.completed {
            println!("{}  DONE  {}", now.format("%H:%M:%S"), &id[..8]);
        }
        let cur: HashSet<String> = poll.running.into_iter().collect();
        if cur == prev {
            continue;
        }
        for id in cur.difference(&prev) {
            println!("{}  BUSY  {}", now.format("%H:%M:%S"), &id[..8]);
        }
        for id in prev.difference(&cur) {
            println!("{}  IDLE  {}", now.format("%H:%M:%S"), &id[..8]);
        }
        prev = cur;
    }
}

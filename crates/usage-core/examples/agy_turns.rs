//! 실데이터 검증: agy 턴 추적기를 이 머신의 진짜 홈에 물려 돌린다.
//! 사용: cargo run -p usage-core --example agy_turns
//!
//! agy 를 옆에서 돌리면 `USER_INPUT` 직후 running 에 뜨고, 최종 답변에 빠져야 한다.

use std::collections::HashSet;

fn main() {
    let homes = usage_core::roots::antigravity_homes();
    println!("홈 {}곳: {homes:?}\n", homes.len());

    let mut w = usage_core::antigravity::TurnWatcher::default();
    let first = w.poll(&homes, chrono::Utc::now());
    println!(
        "첫 회차 — covered={} running={:?}  (첫 관측은 기준점만 잡으므로 비어야 정상)",
        first.covered, first.running
    );
    if !first.covered {
        println!("transcript.jsonl 을 못 찾았습니다. 폴백(크기 변화)으로 돕니다.");
        return;
    }

    println!("\n이제 agy 를 돌려 보세요. 상태가 바뀔 때만 찍습니다 (Ctrl-C 로 종료)\n");
    let mut prev: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let now = chrono::Utc::now();
        let cur: HashSet<String> = w.poll(&homes, now).running.into_iter().collect();
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

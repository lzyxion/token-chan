//! 실데이터 검증: Codex 턴 추적기를 **앱과 같은 루트 선택**으로 돌린다.
//! 사용: cargo run -p usage-core --example codex_turns
//!
//! `roots::codex_homes()` 가 아니라 `accounts::discover` 를 쓰는 게 핵심이다 —
//! 마커 스캔으로만 발견되는 홈(재배치된 CODEX_HOME 등)은 저쪽에만 나온다.

use std::collections::HashSet;

fn main() {
    let extra = usage_core::accounts::ExtraHomes::default();
    let accounts = usage_core::accounts::discover(&extra, true);

    // monitor.rs `account_enabled` 의 기본 규칙과 같다 (사용자 설정은 안 읽는다)
    let mut homes = vec![];
    for a in accounts.iter().filter(|a| a.standard && a.wsl_distro().is_none()) {
        for i in a.installs.iter().filter(|i| i.source == usage_core::Source::Codex) {
            homes.push(i.transcript_root());
        }
    }
    println!("Codex 홈 {}곳:", homes.len());
    for h in &homes {
        let hist = h.join("history.jsonl");
        let meta = std::fs::metadata(&hist).ok();
        println!(
            "  {}\n    history.jsonl: {}",
            h.display(),
            match meta {
                Some(m) => format!("{} bytes", m.len()),
                None => "없음".into(),
            }
        );
    }

    let mut w = usage_core::codex::TurnWatcher::default();
    let first = w.poll(&homes, chrono::Utc::now());
    println!(
        "\n첫 회차 — covered={} running={:?}  (첫 관측은 기준점만 잡으므로 비어야 정상)",
        first.covered, first.running
    );

    println!("\n이제 Codex 를 돌려 보세요. 변화가 있을 때만 찍습니다 (Ctrl-C 로 종료)\n");
    let mut prev: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let now = chrono::Utc::now();
        let poll = w.poll(&homes, now);
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

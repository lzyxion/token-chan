//! 진단용: 이 머신에서 발견되는 CLI 설치본과 계정 묶음을 출력.
//! 사용: cargo run -p usage-core --example accounts

use usage_core::accounts::{discover, ExtraHomes};

fn main() {
    let t0 = std::time::Instant::now();
    let accounts = discover(&ExtraHomes::default(), true);
    println!("마커 스캔 포함 {:?}\n", t0.elapsed());

    for a in &accounts {
        println!(
            "{:?} · {} ({}) · 표준={} · 설치 {}곳",
            a.source,
            a.label,
            a.detail,
            a.standard,
            a.installs.len()
        );
        for i in &a.installs {
            println!(
                "    {}{}",
                i.transcript_root().display(),
                if i.discovered { "  [마커 스캔]" } else { "" }
            );
        }
    }
}

//! 라이브 세션 감지 진단용: 실제 ~/.claude/sessions 를 읽어 상태 출력.

fn main() {
    let dirs = usage_core::roots::claude_session_dirs();
    println!("dirs = {dirs:?}");
    let now = chrono::Utc::now().timestamp_millis();
    let state = usage_core::live::read_live_state(&dirs, now);
    println!("{}", serde_json::to_string_pretty(&state).unwrap());
}

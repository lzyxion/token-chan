//! 사용량 읽기 — 모니터 스레드가 채워 둔 상태를 그대로 돌려준다.

use tauri::State;

use crate::AppState;

#[tauri::command]
pub fn get_summary(state: State<'_, AppState>) -> Option<usage_core::Summary> {
    state.summary.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_live(state: State<'_, AppState>) -> usage_core::live::LiveState {
    state.live.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_plan(state: State<'_, AppState>) -> Vec<usage_core::plan::PlanUsage> {
    state.plan.lock().unwrap().clone()
}

//! 프론트엔드 ↔ 백엔드 IPC 커맨드.

use tauri::{AppHandle, Manager, PhysicalPosition, State};
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{self, Settings};
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
pub fn get_plan(state: State<'_, AppState>) -> Option<usage_core::plan::PlanUsage> {
    state.plan.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// 설정 일괄 갱신: 변경된 항목의 side effect(autostart/클릭 통과/크기)를 적용하고
/// `settings-changed` 이벤트로 모든 창에 알림. 설정 패널의 단일 진입점.
#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, mut new_settings: Settings) {
    let old = state.settings.lock().unwrap().clone();

    // petPos는 패널이 편집하지 않음 — 드래그 저장과의 경합 방지를 위해 서버 상태 유지
    new_settings.pet_pos = old.pet_pos;
    new_settings.pet_scale = new_settings.pet_scale.clamp(0.5, 2.5);
    new_settings.alert_threshold = new_settings.alert_threshold.clamp(0.1, 1.0);
    new_settings.weekly_alert_threshold = new_settings.weekly_alert_threshold.clamp(0.1, 1.0);
    new_settings.hover_delay_ms = new_settings.hover_delay_ms.min(3000);
    new_settings.reset_notify_minutes = new_settings.reset_notify_minutes.min(120);
    new_settings.sleep_after_minutes = new_settings.sleep_after_minutes.clamp(1, 480);

    if old.autostart != new_settings.autostart {
        let autolaunch = app.autolaunch();
        if new_settings.autostart {
            let _ = autolaunch.enable();
        } else {
            let _ = autolaunch.disable();
        }
    }
    if old.click_through != new_settings.click_through {
        apply_click_through(&app, new_settings.click_through);
    }
    if (old.pet_scale - new_settings.pet_scale).abs() > f64::EPSILON {
        resize_pet(&app, new_settings.pet_scale);
        use tauri::Emitter;
        let _ = app.emit("pet-scale", new_settings.pet_scale);
    }

    settings::save(&new_settings);
    *state.settings.lock().unwrap() = new_settings.clone();
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &new_settings);
}

/// 클릭 통과 모드 적용. Linux(GTK)에서 숨김(미realize) 창에 호출하면 패닉하므로
/// 보이는 상태일 때만 적용 — 숨겨진 경우 표시 시점(트레이 토글)에 재적용된다.
pub fn apply_click_through(app: &AppHandle, on: bool) {
    if let Some(pet) = app.get_webview_window("pet") {
        if pet.is_visible().unwrap_or(false) {
            let _ = pet.set_ignore_cursor_events(on);
        }
    }
    if on {
        if let Some(bubble) = app.get_webview_window("bubble") {
            let _ = bubble.hide();
        }
    }
}

#[tauri::command]
pub fn save_pet_position(state: State<'_, AppState>, x: i32, y: i32) {
    let mut s = state.settings.lock().unwrap();
    s.pet_pos = Some((x, y));
    settings::save(&s);
}

/// 펫 창 위치 기준으로 말풍선을 배치하고 표시.
/// 기본은 펫 **위쪽 중앙** (진짜 말풍선처럼), 화면 상단에 닿으면 아래쪽으로 반전.
/// `headroom`: 펫 웹뷰가 측정한 캐릭터 머리 위 여백(논리 px) — 캐릭터 팩 이미지의
/// 크기·비율이 제각각이므로 고정 비율 대신 실측값으로 겹침량을 계산한다.
#[tauri::command]
pub fn show_bubble(app: AppHandle, headroom: Option<f64>) {
    let (Some(pet), Some(bubble)) = (app.get_webview_window("pet"), app.get_webview_window("bubble"))
    else {
        return;
    };
    let (Ok(pos), Ok(size), Ok(bsize)) = (pet.outer_position(), pet.outer_size(), bubble.outer_size())
    else {
        return;
    };

    // 머리 위 여백만큼 겹치되 8px(논리)은 남겨 꼬리가 머리에 닿지 않게
    let sf = pet.scale_factor().unwrap_or(1.0);
    let overlap = (((headroom.unwrap_or(0.0) - 8.0).max(0.0)) * sf) as i32;
    let overlap = overlap.min(size.height as i32 * 6 / 10); // 안전 상한
    let mut x = pos.x + (size.width as i32) / 2 - (bsize.width as i32) / 2;
    let mut y = pos.y - bsize.height as i32 + overlap;
    let mut tail = "bottom"; // 말풍선이 위에 → 꼬리는 아래로 펫을 가리킴

    if let Ok(Some(mon)) = pet.current_monitor() {
        let mpos = mon.position();
        let msize = mon.size();
        let max_x = mpos.x + msize.width as i32 - bsize.width as i32 - 4;
        x = x.clamp(mpos.x + 4, max_x.max(mpos.x + 4));
        if y < mpos.y + 4 {
            // 상단 공간 부족 → 펫 아래에 표시, 꼬리는 위로
            y = pos.y + size.height as i32 + 6;
            tail = "top";
        }
    }

    let _ = bubble.set_position(PhysicalPosition::new(x, y));
    let _ = bubble.show();

    use tauri::Emitter;
    let _ = app.emit("bubble-tail", tail);
}

/// 펫/말풍선 호버 상태 갱신 — 둘 다 벗어났을 때만 hide_bubble 이 실제로 숨긴다
#[tauri::command]
pub fn set_hover(state: State<'_, AppState>, zone: String, hovering: bool) {
    let mut h = state.hover.lock().unwrap();
    match zone.as_str() {
        "pet" => h.0 = hovering,
        "bubble" => h.1 = hovering,
        _ => {}
    }
}

#[tauri::command]
pub fn hide_bubble(app: AppHandle, state: State<'_, AppState>) {
    if *state.bubble_pinned.lock().unwrap() {
        return;
    }
    // 펫 또는 말풍선 위에 마우스가 있으면 유지 (펫 ↔ 말풍선 이동 허용)
    let (pet_h, bubble_h) = *state.hover.lock().unwrap();
    if pet_h || bubble_h {
        return;
    }
    if let Some(bubble) = app.get_webview_window("bubble") {
        let _ = bubble.hide();
    }
}

/// 말풍선 고정 토글 (우클릭/트레이). 반환값: 토글 후 고정 여부.
#[tauri::command]
pub fn toggle_bubble_pin(app: AppHandle, state: State<'_, AppState>) -> bool {
    let pinned = {
        let mut p = state.bubble_pinned.lock().unwrap();
        *p = !*p;
        *p
    };
    if pinned {
        show_bubble(app.clone(), None);
    } else if let Some(bubble) = app.get_webview_window("bubble") {
        let _ = bubble.hide();
    }
    use tauri::Emitter;
    let _ = app.emit("bubble-pinned", pinned);
    pinned
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// 펫 창 리사이즈 (하단 중앙 = 발 위치 고정: 크기 변화만큼 위/좌로 보정 이동)
fn resize_pet(app: &AppHandle, scale: f64) {
    if let Some(pet) = app.get_webview_window("pet") {
        let sf = pet.scale_factor().unwrap_or(1.0);
        let old_pos = pet.outer_position().ok();
        let old_size = pet.outer_size().ok();
        let new_w = (settings::PET_BASE_W * scale * sf).round() as i32;
        let new_h = (settings::PET_BASE_H * scale * sf).round() as i32;
        let _ = pet.set_size(tauri::PhysicalSize::new(new_w.max(1) as u32, new_h.max(1) as u32));
        if let (Some(p), Some(s)) = (old_pos, old_size) {
            let x = p.x - (new_w - s.width as i32) / 2;
            let y = p.y - (new_h - s.height as i32);
            let _ = pet.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

/// 캐릭터 크기 배율 적용: 설정 저장 + 펫 창 리사이즈 + 프론트(pet-scale 이벤트) 알림.
/// 설정 패널 슬라이더가 드래그 중 연속 호출하는 빠른 경로.
pub fn apply_pet_scale(app: &AppHandle, scale: f64) {
    let scale = scale.clamp(0.5, 2.5);
    {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.pet_scale = scale;
        settings::save(&s);
    }
    resize_pet(app, scale);
    use tauri::Emitter;
    let _ = app.emit("pet-scale", scale);
}

#[tauri::command]
pub fn set_pet_scale(app: AppHandle, scale: f64) {
    apply_pet_scale(&app, scale);
}

/// 지원하는 캐릭터 이미지 확장자 (탐색 우선순위)
const CHAR_EXTS: [&str; 4] = ["gif", "webp", "apng", "png"];
const CHAR_STATES: [&str; 6] = ["idle", "working", "alert", "sleep", "exhausted", "refreshed"];

fn find_state_file(pack_dir: &std::path::Path, state: &str) -> Option<std::path::PathBuf> {
    CHAR_EXTS
        .iter()
        .map(|ext| pack_dir.join(format!("{state}.{ext}")))
        .find(|p| p.is_file())
}

/// characters 디렉토리에서 유효한 팩(idle 이미지가 있는 폴더) 목록
#[tauri::command]
pub fn list_character_packs() -> Vec<String> {
    let Some(root) = settings::characters_dir() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(&root) else { return vec![] };
    let mut packs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| find_state_file(&e.path(), "idle").is_some())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    packs.sort();
    packs
}

/// 지정한(또는 기본 선택된) 팩의 상태별 이미지를 data URL 로 반환.
/// 없는 상태는 idle 로 폴백. 팩 미지정/무효 시 None (기본 CSS 고양이 사용).
#[tauri::command]
pub fn get_character_images(
    state: State<'_, AppState>,
    pack: Option<String>,
) -> Option<std::collections::HashMap<String, String>> {
    use base64::Engine;

    let pack = pack.or_else(|| state.settings.lock().unwrap().character_pack.clone())?;
    let dir = settings::characters_dir()?.join(&pack);
    let idle = find_state_file(&dir, "idle")?; // idle 필수

    let to_data_url = |p: &std::path::Path| -> Option<String> {
        let mime = match p.extension()?.to_str()? {
            "gif" => "image/gif",
            "webp" => "image/webp",
            "apng" => "image/apng",
            _ => "image/png",
        };
        let bytes = std::fs::read(p).ok()?;
        // 데스크톱 펫 이미지로는 과대한 크기 방지 (20MB)
        if bytes.len() > 20 * 1024 * 1024 {
            return None;
        }
        Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    };

    let idle_url = to_data_url(&idle)?;
    let mut map = std::collections::HashMap::new();
    for st in CHAR_STATES {
        let url = find_state_file(&dir, st)
            .and_then(|p| to_data_url(&p))
            .unwrap_or_else(|| idle_url.clone());
        map.insert(st.to_string(), url);
    }
    Some(map)
}

/// characters 폴더를 만들고 OS 파일 탐색기로 열기
#[tauri::command]
pub fn open_characters_dir() {
    let Some(root) = settings::characters_dir() else { return };
    let _ = std::fs::create_dir_all(&root);

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(&root).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&root).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(&root).spawn();
}

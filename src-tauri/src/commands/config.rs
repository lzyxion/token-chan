//! 설정 읽기·쓰기와 저장 실패 추적. 모든 저장 지점이 [`save_settings`] 를 거친다.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{self, Settings};
use crate::window;
use crate::AppState;

use super::pet::resize_pet;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// 설정을 저장하고 결과를 추적한다 — 모든 저장 지점이 이 함수를 거친다.
/// 성공↔실패가 **전환될 때만** `settings-save-error` 를 보내(payload: 오류 문자열
/// 또는 해제를 뜻하는 null), 드래그처럼 연속 저장되는 경로에서 알림이 쏟아지지
/// 않게 한다. 설정 창이 이 이벤트를 상단 배너로 표시한다.
pub(crate) fn save_settings(app: &AppHandle, s: &Settings) {
    let err = settings::save(s).err();
    let state = app.state::<AppState>();
    let mut last = state.save_error.lock().unwrap();
    if *last != err {
        *last = err.clone();
        use tauri::Emitter;
        let _ = app.emit("settings-save-error", &err);
    }
}

/// 설정 창이 열릴 때 현재 배너 상태를 물어본다 — 실패는 창이 닫혀 있는 동안
/// (드래그·트레이 토글) 일어날 수 있어, 이벤트만으로는 놓친다.
#[tauri::command]
pub fn get_save_error(state: State<'_, AppState>) -> Option<String> {
    state.save_error.lock().unwrap().clone()
}

/// 설정 일괄 갱신: 변경된 항목의 side effect(autostart/클릭 통과/크기)를 적용하고
/// `settings-changed` 이벤트로 모든 창에 알림. 설정 패널의 단일 진입점.
#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, mut new_settings: Settings) {
    let mut old = state.settings.lock().unwrap().clone();

    // petPos는 패널이 편집하지 않음 — 드래그 저장과의 경합 방지를 위해 서버 상태 유지
    new_settings.pet_pos = old.pet_pos;
    new_settings.pet_scale = new_settings.pet_scale.clamp(0.5, 2.5);
    new_settings.alert_threshold = new_settings.alert_threshold.clamp(0.1, 1.0);
    new_settings.context_alert_threshold = new_settings.context_alert_threshold.clamp(0.1, 1.0);
    new_settings.speech_duration_ms = new_settings.speech_duration_ms.clamp(1000, 15000);
    // 창 위치·크기는 설정 화면이 편집하지 않음 — 드래그/리사이즈 저장과의 경합 방지.
    // 대상 목록은 `window::WINDOWS` 표가 안다 (창이 늘어도 여기는 그대로).
    window::keep_geometry(&mut old, &mut new_settings);
    new_settings.reset_notify_minutes = new_settings.reset_notify_minutes.min(120);
    // 통화는 아는 값만 받는다 — 설정 파일은 사람이 고치는 JSON 이라 오타가 들어올 수 있고,
    // 모르는 값이면 곱하지 않은 달러로 떨어지는 게 안전하다.
    if new_settings.currency != "krw" {
        new_settings.currency = "usd".into();
    }
    // 통화와 같은 이유 — 사람이 고치는 JSON 이라 모르는 값이면 기본 모양으로
    if !settings::GAUGE_STYLES.contains(&new_settings.gauge_style.as_str()) {
        new_settings.gauge_style = settings::GAUGE_STYLES[0].into();
    }
    // 0 이나 음수면 비용이 전부 0/음수로 보인다. 상한은 자릿수 실수(1400 → 1400000) 방지.
    if !new_settings.usd_to_krw.is_finite() || new_settings.usd_to_krw <= 0.0 {
        new_settings.usd_to_krw = old.usd_to_krw;
    }
    new_settings.usd_to_krw = new_settings.usd_to_krw.clamp(1.0, 100_000.0);
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
        crate::tray::sync_click_through(&app, new_settings.click_through);
    }
    if (old.pet_scale - new_settings.pet_scale).abs() > f64::EPSILON {
        resize_pet(&app, new_settings.pet_scale);
        use tauri::Emitter;
        let _ = app.emit("pet-scale", new_settings.pet_scale);
    }

    save_settings(&app, &new_settings);
    *state.settings.lock().unwrap() = new_settings.clone();
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &new_settings);
}

/// 게이지 벤더 전환 — 펫의 로고 클릭용 빠른 경로 (설정 창을 안 거친다)
#[tauri::command]
pub fn set_gauge_vendor(app: AppHandle, vendor: String) {
    // 아는 값만 받는다 — 목록은 `Source::ALL` 이 갖는다
    if vendor != "auto" && !usage_core::Source::ALL.iter().any(|s| s.id() == vendor) {
        return;
    }
    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.gauge_vendor = vendor;
        save_settings(&app, &s);
        s.clone()
    };
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
}

/// 클릭 통과 모드 켜기/끄기 (트레이·펫 우클릭 메뉴 공용 진입점).
/// 설정 저장 + 창 적용 + 트레이 체크 표시·설정 패널 동기화까지 한 번에 처리한다.
pub fn set_click_through(app: &AppHandle, on: bool) {
    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        if s.click_through == on {
            None
        } else {
            s.click_through = on;
            save_settings(app, &s);
            Some(s.clone())
        }
    };
    // 값이 그대로여도 메뉴 체크는 맞춰 둔다 (외부에서 어긋난 경우 대비)
    crate::tray::sync_click_through(app, on);
    let Some(new_settings) = updated else { return };
    apply_click_through(app, on);
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
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

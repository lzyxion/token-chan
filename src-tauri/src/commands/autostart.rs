//! 로그인 자동 시작의 OS 상태가 원본이다. 개발 빌드는 등록을 변경하지 않는다.
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    enabled: bool,
    can_change: bool,
    error: Option<String>,
}

fn can_change() -> bool {
    !cfg!(debug_assertions) && !tauri::is_dev()
}

// 성공 응답만으로 판단하지 않고 OS에서 다시 읽어 검증한다.
fn apply_enabled(
    enabled: bool,
    mut read: impl FnMut() -> Result<bool, String>,
    write: impl FnOnce(bool) -> Result<(), String>,
) -> Result<bool, String> {
    let before = read()?;
    // enable은 활성 상태에서도 호출해 현재 실행 파일 경로로 갱신한다.
    // 이미 없는 항목의 disable은 일부 플랫폼에서 오류이므로 생략한다.
    if enabled || before {
        write(enabled)?;
    }
    let actual = read()?;
    if actual != enabled {
        return Err("The operating system did not apply the requested startup setting.".into());
    }
    Ok(actual)
}

fn apply(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    apply_enabled(
        enabled,
        || manager.is_enabled().map_err(|e| e.to_string()),
        |value| {
            if value { manager.enable() } else { manager.disable() }
                .map_err(|e| e.to_string())
        },
    )
}

fn sync_settings(app: &AppHandle, enabled: bool) {
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    if settings.autostart != enabled {
        settings.autostart = enabled;
        if can_change() {
            super::save_settings(app, &settings);
        }
        use tauri::Emitter;
        let _ = app.emit("settings-changed", &*settings);
    }
}

#[tauri::command]
pub fn get_autostart_status(app: AppHandle) -> Result<AutostartStatus, String> {
    let state = app.state::<AppState>();
    let last_error = state.autostart_error.lock().unwrap();
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    sync_settings(&app, enabled);
    Ok(AutostartStatus { enabled, can_change: can_change(), error: last_error.clone() })
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<AutostartStatus, String> {
    if !can_change() {
        return Err("Launch at login can only be changed in a release build.".into());
    }
    let state = app.state::<AppState>();
    let mut last_error = state.autostart_error.lock().unwrap();
    let result = apply(&app, enabled);
    *last_error = result.as_ref().err().cloned();
    // 등록은 성공했지만 후속 작업이 실패하는 경우도 있어 실제 상태를 재조회한다.
    if let Ok(actual) = app.autolaunch().is_enabled() {
        sync_settings(&app, actual);
    }
    result.map(|actual| AutostartStatus {
        enabled: actual, can_change: true, error: None,
    })
}

pub(crate) fn restore_autostart(app: &AppHandle) {
    if !can_change() {
        return;
    }
    let state = app.state::<AppState>();
    let mut last_error = state.autostart_error.lock().unwrap();
    let result = app.autolaunch().is_enabled().map_err(|e| e.to_string())
        .and_then(|enabled| if enabled { apply(app, true) } else { Ok(false) });
    match result {
        Ok(enabled) => sync_settings(app, enabled),
        Err(error) => {
            eprintln!("[autostart] {error}");
            *last_error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn enabling_refreshes_an_existing_registration() {
        let written = Cell::new(false);
        assert_eq!(apply_enabled(true, || Ok(true), |value| {
            assert!(value);
            written.set(true);
            Ok(())
        }), Ok(true));
        assert!(written.get());
    }

    #[test]
    fn disabling_an_absent_registration_is_idempotent() {
        assert_eq!(apply_enabled(false, || Ok(false), |_| panic!("must not write")), Ok(false));
    }

    #[test]
    fn failed_registration_is_not_reported_as_success() {
        assert_eq!(apply_enabled(true, || Ok(false), |_| Err("denied".into())), Err("denied".into()));
    }

    #[test]
    fn successful_write_with_wrong_os_state_is_an_error() {
        assert!(apply_enabled(true, || Ok(false), |_| Ok(())).is_err());
    }

    #[test]
    fn unreadable_os_state_does_not_change_registration() {
        assert!(apply_enabled(true, || Err("unavailable".into()), |_| panic!("must not write")).is_err());
    }

    #[test]
    fn disabling_is_verified_by_reading_back() {
        let enabled = Cell::new(true);
        assert_eq!(apply_enabled(false, || Ok(enabled.get()), |value| {
            enabled.set(value);
            Ok(())
        }), Ok(false));
    }
}

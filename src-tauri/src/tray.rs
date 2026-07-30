//! 시스템 트레이 아이콘 + 메뉴. 창 닫기 대신 트레이에서 종료한다.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    // 트레이는 즉시 액션만 담는다 — 지속 옵션(클릭 통과/자동 시작 등)은 설정 패널로 일원화
    let show = MenuItem::with_id(app, "show", "펫 보이기/숨기기", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "pin", "말풍선 고정 토글", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "설정…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &pin, &settings_item, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("기본 아이콘 없음").clone())
        .tooltip("Token Pet — AI 토큰 사용량")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                // 토글: 보이는 상태면 숨기고(말풍선 포함), 숨겨져 있으면 표시
                if let Some(pet) = app.get_webview_window("pet") {
                    if pet.is_visible().unwrap_or(false) {
                        let _ = pet.hide();
                        if let Some(bubble) = app.get_webview_window("bubble") {
                            let _ = bubble.hide();
                        }
                    } else {
                        let _ = pet.show();
                        let _ = pet.set_focus();
                        // 숨김 상태에서는 클릭 통과를 적용할 수 없으므로 표시 시점에 재적용
                        let ct = {
                            let state = app.state::<crate::AppState>();
                            let s = state.settings.lock().unwrap();
                            s.click_through
                        };
                        if ct {
                            crate::commands::apply_click_through(app, true);
                        }
                    }
                }
            }
            "pin" => {
                let _ = crate::commands::toggle_bubble_pin(app.clone(), app.state());
            }
            "settings" => {
                // 설정 패널을 트레이 근처(주 모니터 우하단)에 표시
                if let Some(w) = app.get_webview_window("settings") {
                    if let Ok(Some(mon)) = w.primary_monitor() {
                        let mp = mon.position();
                        let ms = mon.size();
                        let sf = mon.scale_factor();
                        let (ww, wh) = ((320.0 * sf) as i32, (650.0 * sf) as i32);
                        let x = mp.x + ms.width as i32 - ww - 16;
                        let y = mp.y + ms.height as i32 - wh - 80;
                        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

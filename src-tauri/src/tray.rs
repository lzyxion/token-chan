//! 시스템 트레이 아이콘 + 메뉴. 창 닫기 대신 트레이에서 종료한다.

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let click_through = {
        let state = app.state::<crate::AppState>();
        let s = state.settings.lock().unwrap();
        s.click_through
    };
    let show = MenuItem::with_id(app, "show", "펫 보이기/숨기기", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "사용량 패널 열기/닫기", true, None::<&str>)?;
    // 클릭 통과를 켜면 펫이 마우스를 전혀 받지 않아 우클릭으로 되돌릴 수 없다
    // → 해제 경로인 이 트레이 항목이 유일한 출구이므로 반드시 여기 있어야 한다.
    let ct = CheckMenuItem::with_id(
        app,
        "clickthrough",
        "클릭 통과 모드",
        true,
        click_through,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(app, "settings", "설정…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &panel, &ct, &settings_item, &quit])?;
    *app.state::<crate::AppState>().tray_click_through.lock().unwrap() = Some(ct);

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("기본 아이콘 없음").clone())
        .tooltip("Token Pet — AI 토큰 사용량")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_action(app, event.id.as_ref()))
        .build(app)?;
    Ok(())
}

/// 트레이 메뉴와 펫 우클릭 메뉴가 공유하는 동작 처리.
/// 두 메뉴가 항상 같게 동작하도록 분기를 한 군데로 모은다.
pub fn handle_action(app: &AppHandle, action: &str) {
    match action {
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
                        // 숨김 상태에서는 클릭 통과를 적용할 수 없으므로 표시 시점에 재적용.
                        // 끄는 쪽도 반드시 적용해야 한다 — 켠 채로 숨겼다가 숨김 중에
                        // 끈 경우, 그냥 두면 설정은 꺼졌는데 창은 계속 통과 상태로 남는다.
                        let ct = {
                            let state = app.state::<crate::AppState>();
                            let s = state.settings.lock().unwrap();
                            s.click_through
                        };
                        crate::commands::apply_click_through(app, ct);
                    }
                }
            }
            "panel" => {
                let _ = crate::commands::toggle_panel(app.clone());
            }
            "settings" => {
                // 설정 패널을 트레이 근처(주 모니터 우하단)에 표시
                if let Some(w) = app.get_webview_window("settings") {
                    // 리사이즈 도중 ✕/Esc 로 닫혔을 수 있다 — toggle_panel 과 같은 정리
                    crate::commands::clear_window_resize(app, "settings");
                    // 사용자가 조절한 크기 복원 — 우하단 정렬 계산보다 먼저 적용해야
                    // 아래 위치 계산이 실제 크기를 쓴다
                    let saved_size = {
                        let state = app.state::<crate::AppState>();
                        let s = state.settings.lock().unwrap();
                        s.settings_size
                    };
                    if let Some((ww, wh)) = saved_size {
                        let _ = w.set_size(tauri::PhysicalSize::new(ww, wh));
                    }
                    if let (Ok(Some(mon)), Ok(size)) = (w.primary_monitor(), w.outer_size()) {
                        let mp = mon.position();
                        let ms = mon.size();
                        let x = mp.x + ms.width as i32 - size.width as i32 - 16;
                        let y = mp.y + ms.height as i32 - size.height as i32 - 80;
                        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "clickthrough" => {
                let on = {
                    let state = app.state::<crate::AppState>();
                    let s = state.settings.lock().unwrap();
                    !s.click_through
                };
                crate::commands::set_click_through(app, on);
            }
            "quit" => app.exit(0),
            _ => {}
    }
}

/// 트레이 메뉴의 클릭 통과 체크 표시를 현재 값에 맞춘다.
/// (설정 패널·펫 우클릭 메뉴에서 바뀌어도 트레이가 어긋나지 않도록)
pub fn sync_click_through(app: &AppHandle, on: bool) {
    let item = app
        .state::<crate::AppState>()
        .tray_click_through
        .lock()
        .unwrap()
        .clone();
    if let Some(item) = item {
        let _ = item.set_checked(on);
    }
}

/// 캐릭터 우클릭 컨텍스트 메뉴 — 트레이와 같은 항목을 펫에서 바로 쓴다.
/// 트레이와 id 가 겹치지 않게 `petmenu:` 접두사를 붙이고, 처리는 위 함수로 넘긴다.
pub fn popup_pet_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{ContextMenu, PredefinedMenuItem};

    let Some(pet) = app.get_webview_window("pet") else {
        return Ok(());
    };
    let click_through = {
        let state = app.state::<crate::AppState>();
        let s = state.settings.lock().unwrap();
        s.click_through
    };
    let hide = MenuItem::with_id(app, "petmenu:show", "펫 숨기기", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "petmenu:panel", "사용량 패널 열기/닫기", true, None::<&str>)?;
    // 켜는 순간 펫이 마우스를 받지 않으므로 이 메뉴로는 다시 끌 수 없다 → 해제는 트레이에서
    let ct = CheckMenuItem::with_id(
        app,
        "petmenu:clickthrough",
        "클릭 통과 모드",
        true,
        click_through,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(app, "petmenu:settings", "설정…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "petmenu:quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&hide, &panel, &ct, &settings_item, &sep, &quit])?;
    menu.popup(pet.as_ref().window())
}

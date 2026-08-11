//! 시스템 트레이 아이콘 + 메뉴. 창 닫기 대신 트레이에서 종료한다.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use usage_core::model::Source;

/// "연결된 계정" 서브메뉴. 같은 계정의 설치본은 한 줄로 묶여 나오고,
/// 체크가 곧 집계 포함 여부다.
///
/// `prefix` 는 펫 우클릭 메뉴에서 트레이와 id 가 겹치지 않게 붙이는 접두사.
fn accounts_submenu(app: &AppHandle, prefix: &str) -> tauri::Result<Submenu<tauri::Wry>> {
    let (accounts, overrides, extras) = {
        let state = app.state::<crate::AppState>();
        // 두 락을 겹쳐 잡지 않는다
        let (ov, ex) = {
            let s = state.settings.lock().unwrap();
            (
                s.accounts_enabled.clone(),
                [
                    ("claude", s.extra_claude_homes.clone()),
                    ("codex", s.extra_codex_homes.clone()),
                    ("antigravity", s.extra_antigravity_homes.clone()),
                ],
            )
        };
        let ac = { state.accounts.lock().unwrap().clone() };
        (ac, ov, ex)
    };

    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![];
    if accounts.is_empty() {
        items.push(Box::new(MenuItem::with_id(
            app,
            format!("{prefix}acctnone"),
            "발견된 계정 없음",
            false,
            None::<&str>,
        )?));
    }
    for a in &accounts {
        let src = match a.source {
            Source::Claude => "Claude",
            Source::Codex => "Codex",
            Source::Antigravity => "AGY",
        };
        // 어떤 계정인지만 알면 되므로 계정명만 표시한다
        items.push(Box::new(CheckMenuItem::with_id(
            app,
            format!("{prefix}acct:{}", a.setting_key()),
            format!("{src} · {}", a.label),
            true,
            crate::monitor::account_enabled(a, &overrides),
            None::<&str>,
        )?));
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, format!("{prefix}acctadd:claude"), "Claude 홈 추가…", true, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, format!("{prefix}acctadd:codex"), "Codex 홈 추가…", true, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, format!("{prefix}acctadd:antigravity"), "AGY 홈 추가…", true, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, format!("{prefix}acctrescan"), "다시 검색", true, None::<&str>)?));

    // 직접 추가한 경로의 삭제 — 설정 창에서 데이터 소스 UI 를 없앴으므로 여기가 유일한 제거 경로
    let mut del_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![];
    for (src, list) in &extras {
        let disp = match *src {
            "codex" => "Codex",
            "antigravity" => "AGY",
            _ => "Claude",
        };
        for p in list.iter().filter(|p| !p.trim().is_empty()) {
            del_items.push(Box::new(MenuItem::with_id(
                app,
                format!("{prefix}acctdel:{src}:{p}"),
                format!("{disp} · {p}"),
                true,
                None::<&str>,
            )?));
        }
    }
    if !del_items.is_empty() {
        let del_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
            del_items.iter().map(|b| b.as_ref()).collect();
        items.push(Box::new(Submenu::with_id_and_items(
            app,
            format!("{prefix}acctdelmenu"),
            "추가한 경로 제거",
            true,
            &del_refs,
        )?));
    }

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Submenu::with_id_and_items(app, format!("{prefix}accounts"), "연결된 계정", true, &refs)
}

/// 계정 목록이 바뀌면 트레이 메뉴를 다시 만든다 (트레이는 시작 시 한 번만 만들어져서
/// 그냥 두면 체크 상태와 목록이 낡는다). 펫 우클릭 메뉴는 열 때마다 새로 만들어 무관.
pub fn refresh_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else { return };
    if let Ok(menu) = build_tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let click_through = {
        let state = app.state::<crate::AppState>();
        let s = state.settings.lock().unwrap();
        s.click_through
    };
    let show = MenuItem::with_id(app, "show", "펫 보이기/숨기기", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "사용량 패널 열기/닫기", true, None::<&str>)?;
    let accounts = accounts_submenu(app, "")?;
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
    let menu = Menu::with_items(app, &[&show, &panel, &accounts, &ct, &settings_item, &quit])?;
    // 메뉴를 다시 만들 때마다 새 체크 항목으로 교체 — 옛 항목을 붙들고 있으면
    // sync_click_through 가 화면에 없는 항목을 건드리게 된다
    *app.state::<crate::AppState>().tray_click_through.lock().unwrap() = Some(ct);
    Ok(menu)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;

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
            // 계정 토글 — id 의 나머지가 곧 Account::setting_key()
            a if a.starts_with("acct:") => toggle_account(app, &a["acct:".len()..]),
            "acctrescan" => {
                crate::monitor::rediscover(app);
                refresh_menu(app);
            }
            a if a.starts_with("acctadd:") => pick_home(app, &a["acctadd:".len()..]),
            a if a.starts_with("acctdel:") => remove_home(app, &a["acctdel:".len()..]),
            _ => {}
    }
}

/// 계정의 집계 포함 여부를 뒤집는다.
fn toggle_account(app: &AppHandle, setting_key: &str) {
    let state = app.state::<crate::AppState>();
    let accounts = { state.accounts.lock().unwrap().clone() };
    let Some(account) = accounts.iter().find(|a| a.setting_key() == setting_key) else {
        return;
    };
    let updated = {
        let mut s = state.settings.lock().unwrap();
        let now = crate::monitor::account_enabled(account, &s.accounts_enabled);
        s.accounts_enabled.insert(setting_key.to_string(), !now);
        crate::settings::save(&s);
        s.clone()
    };
    refresh_menu(app);
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
}

/// 폴더 선택으로 스캔 경로 추가. 다이얼로그는 메인 스레드를 막으면 안 되므로 별도 스레드에서.
fn pick_home(app: &AppHandle, source: &str) {
    use tauri_plugin_dialog::DialogExt;
    let app = app.clone();
    let source = source.to_string();
    std::thread::spawn(move || {
        let Some(picked) = app.dialog().file().blocking_pick_folder() else { return };
        let Ok(path) = picked.into_path() else { return };
        let path = path.display().to_string();

        let updated = {
            let state = app.state::<crate::AppState>();
            let mut s = state.settings.lock().unwrap();
            let list = match source.as_str() {
                "codex" => &mut s.extra_codex_homes,
                "antigravity" => &mut s.extra_antigravity_homes,
                _ => &mut s.extra_claude_homes,
            };
            if list.iter().any(|p| p == &path) {
                return; // 이미 있는 경로 — 중복 등록 방지
            }
            list.push(path);
            crate::settings::save(&s);
            s.clone()
        };
        // 새 경로가 어떤 계정인지 알아야 목록에 뜨므로 다시 발견한다
        crate::monitor::rediscover(&app);
        refresh_menu(&app);
        use tauri::Emitter;
        let _ = app.emit("settings-changed", &updated);
    });
}

/// 직접 추가한 스캔 경로를 지운다. `arg` 는 "<source>:<경로>" —
/// 경로에 든 콜론(드라이브 문자)은 첫 콜론에서만 갈라 문제없다.
fn remove_home(app: &AppHandle, arg: &str) {
    let Some((source, path)) = arg.split_once(':') else { return };
    let updated = {
        let state = app.state::<crate::AppState>();
        let mut s = state.settings.lock().unwrap();
        let list = match source {
            "codex" => &mut s.extra_codex_homes,
            "antigravity" => &mut s.extra_antigravity_homes,
            _ => &mut s.extra_claude_homes,
        };
        let before = list.len();
        list.retain(|p| p != path);
        if list.len() == before {
            return; // 메뉴가 낡아 이미 지워진 경로 — 저장/재탐색 불필요
        }
        crate::settings::save(&s);
        s.clone()
    };
    // 지운 경로의 계정이 목록에서 빠지도록 다시 발견한다
    crate::monitor::rediscover(app);
    refresh_menu(app);
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
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
    let accounts = accounts_submenu(app, "petmenu:")?;
    let settings_item = MenuItem::with_id(app, "petmenu:settings", "설정…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "petmenu:quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&hide, &panel, &accounts, &ct, &settings_item, &sep, &quit])?;
    menu.popup(pet.as_ref().window())
}

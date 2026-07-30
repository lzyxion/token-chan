mod commands;
mod monitor;
mod settings;
mod tray;

use std::sync::Mutex;

use tauri::{Manager, PhysicalPosition};

pub struct AppState {
    pub summary: Mutex<Option<usage_core::Summary>>,
    pub live: Mutex<usage_core::live::LiveState>,
    pub plan: Mutex<Option<usage_core::plan::PlanUsage>>,
    pub settings: Mutex<settings::Settings>,
    pub bubble_pinned: Mutex<bool>,
    /// (펫 호버 중, 말풍선 호버 중) — 둘 다 아닐 때만 말풍선 숨김
    pub hover: Mutex<(bool, bool)>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    eprintln!("[boot] token-pet starting (pid={})", std::process::id());
    let mut builder = tauri::Builder::default();
    // 진단용: TOKENPET_NO_SINGLE_INSTANCE=1 이면 단일 인스턴스 검사를 건너뜀
    if std::env::var_os("TOKENPET_NO_SINGLE_INSTANCE").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            eprintln!("[boot] second-instance signal received (기존 인스턴스가 살아있음)");
            // 두 번째 인스턴스 실행 시 기존 펫 창을 앞으로
            if let Some(pet) = app.get_webview_window("pet") {
                let _ = pet.show();
                let _ = pet.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            summary: Mutex::new(None),
            live: Mutex::new(usage_core::live::LiveState::default()),
            plan: Mutex::new(None),
            settings: Mutex::new(settings::Settings::default()),
            bubble_pinned: Mutex::new(false),
            hover: Mutex::new((false, false)),
        })
        .setup(|app| {
            eprintln!("[boot] setup start");
            let loaded = settings::load();
            *app.state::<AppState>().settings.lock().unwrap() = loaded.clone();

            if let Some(pet) = app.get_webview_window("pet") {
                // 저장된 캐릭터 크기 적용
                if (loaded.pet_scale - 1.0).abs() > f64::EPSILON {
                    let _ = pet.set_size(tauri::LogicalSize::new(
                        settings::PET_BASE_W * loaded.pet_scale,
                        settings::PET_BASE_H * loaded.pet_scale,
                    ));
                }
                match loaded.pet_pos {
                    Some((x, y)) => {
                        let _ = pet.set_position(PhysicalPosition::new(x, y));
                    }
                    None => {
                        // 첫 실행: 주 모니터 우하단 근처에 배치
                        if let Ok(Some(mon)) = pet.primary_monitor() {
                            let msize = mon.size();
                            let mpos = mon.position();
                            let x = mpos.x + msize.width as i32 - 220;
                            let y = mpos.y + msize.height as i32 - 280;
                            let _ = pet.set_position(PhysicalPosition::new(x, y));
                        }
                    }
                }
            }

            // 주의: bubble 의 set_ignore_cursor_events 는 여기서 호출하면 안 된다.
            // Linux(GTK)에서 visible:false 창은 아직 realize 되지 않아 tao 내부의
            // window().unwrap() 이 패닉한다 → show() 직후에 호출 (commands::show_bubble).

            // 시작 시 숨김 / 클릭 통과 적용 (클릭 통과는 보이는 창에만 — 숨김이면 표시 시점에 재적용)
            if let Some(pet) = app.get_webview_window("pet") {
                if loaded.start_hidden {
                    let _ = pet.hide();
                } else if loaded.click_through {
                    commands::apply_click_through(app.handle(), true);
                }
            }

            tray::create(app.handle())?;
            monitor::spawn(app.handle().clone());
            eprintln!("[boot] setup done");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_summary,
            commands::get_live,
            commands::get_plan,
            commands::get_settings,
            commands::set_settings,
            commands::save_pet_position,
            commands::set_pet_scale,
            commands::list_character_packs,
            commands::get_character_images,
            commands::open_characters_dir,
            commands::show_bubble,
            commands::hide_bubble,
            commands::set_hover,
            commands::toggle_bubble_pin,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패")
}

mod commands;
mod monitor;
mod settings;
mod tray;
mod window;

use std::sync::Mutex;

use tauri::Manager;

/// 창 리사이즈 진행 상태 — 시작 시점의 커서와 창 사각형, 그리고 잡은 변.
/// 사용량 패널·설정 창이 같은 흐름을 공유하고 `label` 로 구분한다.
/// 펫 드래그와 같은 이유로 전부 **물리 px**로 들고 있는다 (배율 다른 모니터 대응).
#[derive(Clone, Copy)]
pub struct WindowResize {
    /// 리사이즈 중인 창 라벨 ("panel" | "settings")
    pub label: &'static str,
    /// 잡은 순간의 커서 (물리 px)
    pub cursor: (f64, f64),
    /// 잡은 순간의 창 사각형 (left, top, right, bottom / 물리 px)
    pub rect: (i32, i32, i32, i32),
    /// 움직이는 변 (west, north, east, south)
    pub edges: (bool, bool, bool, bool),
    /// 이 창의 최소 크기 (논리 px)
    pub min: (f64, f64),
}

pub struct AppState {
    pub summary: Mutex<Option<usage_core::Summary>>,
    pub live: Mutex<usage_core::live::LiveState>,
    /// 소스별 공식 한도. Claude 는 CLI 실행으로, Codex 는 rollout 파싱으로 채워지므로
    /// 서로 다른 스레드가 각자 자기 소스만 갱신한다 (`monitor::set_plan`).
    pub plan: Mutex<Vec<usage_core::plan::PlanUsage>>,
    /// Claude 5시간 창의 종료 시각을 **트랜스크립트에서 계산한 값** (`usage_core::blocks`).
    ///
    /// 사용량 스레드(10초)가 채우고 플랜 스레드(30초)가 읽는다 — 어댑터가 저쪽에 있어서다.
    /// 공식 캐시가 굳어 리셋이 과거로 남을 때만 쓰인다.
    pub claude_reset: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    pub settings: Mutex<settings::Settings>,
    /// 발견된 계정 목록 (설치본 포함). 마커 스캔이 수백 ms 들므로 캐시하고,
    /// 시작 시 1회 + 사용자가 "다시 검색" 할 때만 갱신한다.
    pub accounts: Mutex<Vec<usage_core::accounts::Account>>,
    /// 펫 웹뷰가 마지막으로 보고한 캐릭터 머리 위 여백(논리 px).
    /// 대사가 이벤트로 갑자기 떠도 꼬리 위치를 맞추기 위해 캐시한다.
    pub headroom: Mutex<f64>,
    /// 캐릭터 발밑 여백(논리 px) — 그림자·미니 라벨·무대 패딩이 차지하는 부분.
    /// 말풍선이 펫 아래로 뒤집힐 때 이만큼 파고들어야 위쪽과 간격이 같아진다.
    pub footroom: Mutex<f64>,
    /// 캐릭터의 창 안 가로 중심(논리 px). 게이지 열 때문에 캐릭터가 창 중앙에
    /// 있지 않으므로, 말풍선은 창이 아니라 이 지점을 기준으로 맞춘다.
    pub center_x: Mutex<Option<f64>>,
    /// 대사 표시 세대 — 자동 숨김 타이머가 자기 대사만 지우도록 하는 토큰
    pub speech_gen: Mutex<u64>,
    /// 지금 말풍선이 그리고 있는 꼬리 방향("bottom"|"top"). 펫을 따라 옮길 때
    /// 방향이 실제로 뒤집힌 경우에만 웹뷰에 알리기 위한 비교용.
    pub speech_tail: Mutex<&'static str>,
    /// 드래그 중 "커서 − 창 좌표" 오프셋 (물리 px). 모니터마다 배율이 달라도
    /// 물리 좌표로만 계산하면 어긋나지 않는다. None = 드래그 중 아님.
    pub drag_grab: Mutex<Option<(f64, f64)>>,
    /// 창(패널·설정) 리사이즈 중 상태. None = 리사이즈 중 아님.
    /// 커서는 하나뿐이라 동시에 두 창을 리사이즈할 일은 없다.
    pub window_resize: Mutex<Option<WindowResize>>,
    /// 트레이 메뉴의 "클릭 통과" 체크 항목 — 설정이 어디서 바뀌든 체크 표시를
    /// 맞추려면 핸들이 필요하다. 트레이 생성(setup) 전까지는 None.
    pub tray_click_through: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
    /// 마지막 설정 저장의 오류 (None = 성공). 전환 시에만 이벤트를 보내기 위한
    /// 비교용이자, 설정 창이 열릴 때 현재 배너 상태를 물어보는 원본.
    pub save_error: Mutex<Option<String>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    eprintln!("[boot] token-chan starting (pid={})", std::process::id());
    // 웹뷰는 setup 보다 먼저 살아나 get_settings 를 호출한다. setup 에서 읽으면
    // 그 사이 요청이 기본값을 받아가 캐릭터 크기·팩·임계값이 전부 기본으로 굳는다
    // (프론트는 재시도하지 않음) → 반드시 manage() 전에 읽어 둔다.
    let loaded = settings::load();
    let mut builder = tauri::Builder::default();
    // 진단용: TOKENCHAN_NO_SINGLE_INSTANCE=1 이면 단일 인스턴스 검사를 건너뜀
    if std::env::var_os("TOKENCHAN_NO_SINGLE_INSTANCE").is_none() {
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
        // 네이티브 메뉴에는 텍스트 입력이 없으므로, 스캔 경로 추가는 폴더 선택 다이얼로그로
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            summary: Mutex::new(None),
            live: Mutex::new(usage_core::live::LiveState::default()),
            plan: Mutex::new(vec![]),
            claude_reset: Mutex::new(None),
            settings: Mutex::new(loaded.clone()),
            accounts: Mutex::new(vec![]),
            headroom: Mutex::new(0.0),
            footroom: Mutex::new(0.0),
            center_x: Mutex::new(None),
            speech_gen: Mutex::new(0),
            speech_tail: Mutex::new("bottom"),
            drag_grab: Mutex::new(None),
            save_error: Mutex::new(None),
            window_resize: Mutex::new(None),
            tray_click_through: Mutex::new(None),
        })
        .setup(move |app| {
            eprintln!("[boot] setup start");

            if let Some(pet) = app.get_webview_window("pet") {
                // 저장해 둔 자리에 저장해 둔 크기로 — 자리와 크기는 한 몸이라
                // (배율이 다른 화면으로 가면 같은 캐릭터도 물리 px 이 달라진다)
                // 계산은 `window` 가 통째로 한다.
                window::restore_pet(&pet, loaded.pet_pos, loaded.pet_scale);
            }

            // 주의: bubble 의 set_ignore_cursor_events 는 여기서 호출하면 안 된다.
            // Linux(GTK)에서 visible:false 창은 아직 realize 되지 않아 tao 내부의
            // window().unwrap() 이 패닉한다 → show() 직후에 호출 (commands::show_speech).

            // 시작 시 숨김 / 클릭 통과 적용 (클릭 통과는 보이는 창에만 — 숨김이면 표시 시점에 재적용)
            if let Some(pet) = app.get_webview_window("pet") {
                if loaded.start_hidden {
                    let _ = pet.hide();
                } else if loaded.click_through {
                    commands::apply_click_through(app.handle(), true);
                }
            }

            // 떠 있는 말풍선이 펫을 따라다니게 — 드래그로 옮기든, 크기 변경(fit_pet_window)이나
            // 모니터 이동으로 창이 움직이든 Moved/Resized 는 똑같이 오므로 여기 한 곳에서 처리한다.
            if let Some(pet) = app.get_webview_window("pet") {
                let handle = app.handle().clone();
                pet.on_window_event(move |e| {
                    if matches!(
                        e,
                        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                    ) {
                        commands::reposition_bubble(&handle);
                    }
                });
            }

            // 펫 우클릭 팝업 메뉴의 선택은 앱 레벨로 온다 (트레이는 자체 핸들러 사용).
            // id 접두사로 구분해 트레이와 같은 처리로 넘긴다.
            app.on_menu_event(|app, event| {
                if let Some(action) = event.id.as_ref().strip_prefix("petmenu:") {
                    tray::handle_action(app, action);
                }
            });

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
            commands::get_save_error,
            commands::open_settings,
            commands::get_accounts,
            commands::set_account_enabled,
            commands::set_retention_days,
            commands::add_home,
            commands::remove_home,
            commands::rescan_accounts,
            commands::set_gauge_vendor,
            commands::save_pet_position,
            commands::set_pet_scale,
            commands::fit_pet_window,
            commands::start_pet_drag,
            commands::drag_pet,
            commands::end_pet_drag,
            commands::show_pet_menu,
            commands::list_character_packs,
            commands::get_character_images,
            commands::get_state_images,
            commands::get_character_speech,
            commands::set_character_speech,
            commands::get_character_config,
            commands::set_character_config,
            commands::create_character_pack,
            commands::rename_character_pack,
            commands::delete_character_pack,
            commands::list_character_dirs,
            commands::import_state_image,
            commands::import_state_image_from_path,
            commands::remove_state_image,
            commands::open_studio,
            commands::open_characters_dir,
            commands::show_speech,
            commands::set_anchor,
            commands::toggle_panel,
            commands::save_window_position,
            commands::save_window_size,
            commands::start_window_resize,
            commands::resize_window,
            commands::end_window_resize,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패")
}

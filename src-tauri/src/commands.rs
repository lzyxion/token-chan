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
    new_settings.speech_duration_ms = new_settings.speech_duration_ms.clamp(1000, 15000);
    // panelPos·panelSize·settingsSize는 패널이 편집하지 않음 — 드래그/리사이즈 저장과의 경합 방지
    new_settings.panel_pos = old.panel_pos;
    new_settings.panel_size = old.panel_size;
    new_settings.settings_size = old.settings_size;
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
        crate::tray::sync_click_through(&app, new_settings.click_through);
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
            settings::save(&s);
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
pub fn save_pet_position(state: State<'_, AppState>, x: i32, y: i32) {
    let mut s = state.settings.lock().unwrap();
    s.pet_pos = Some((x, y));
    settings::save(&s);
}

/// 펫 웹뷰가 캐릭터의 실측 위치를 미리 보고 (크기·팩·상태·게이지 위치 변경 시).
/// `headroom`은 머리 위 여백, `center_x`는 창 안에서의 캐릭터 가로 중심.
/// 대사가 이벤트로 갑자기 떠도 꼬리가 머리에 정확히 닿게 하기 위해 캐시해 둔다.
#[tauri::command]
pub fn set_anchor(state: State<'_, AppState>, headroom: f64, footroom: f64, center_x: f64) {
    *state.headroom.lock().unwrap() = headroom.max(0.0);
    *state.footroom.lock().unwrap() = footroom.max(0.0);
    *state.center_x.lock().unwrap() = Some(center_x);
}

/// 지금 펫 위치를 기준으로 말풍선이 놓일 좌표와 꼬리 방향을 계산한다.
/// 펫이 숨겨져 있으면(말할 주체가 없다) 좌표를 못 읽으면 None.
///
/// 표시 시점(`show_speech`)과 펫이 움직일 때(`reposition_bubble`)가 같은 계산을 써야
/// 드래그 도중에도 꼬리가 머리에서 떨어지지 않는다.
fn bubble_placement(app: &AppHandle) -> Option<(i32, i32, &'static str)> {
    let (pet, bubble) = (
        app.get_webview_window("pet")?,
        app.get_webview_window("bubble")?,
    );
    if !pet.is_visible().unwrap_or(false) {
        return None;
    }
    let (Ok(pos), Ok(size), Ok(bsize)) =
        (pet.outer_position(), pet.outer_size(), bubble.outer_size())
    else {
        return None;
    };

    // 머리 위 여백만큼 겹치되 8px(논리)은 남겨 꼬리가 머리에 닿지 않게
    let (headroom, footroom, center_x) = {
        let s = app.state::<AppState>();
        let h = *s.headroom.lock().unwrap();
        let f = *s.footroom.lock().unwrap();
        let c = *s.center_x.lock().unwrap();
        (h, f, c)
    };
    let sf = pet.scale_factor().unwrap_or(1.0);
    // 창 안 여백(논리 px)을 물리 px 겹침으로 — 8px은 남기고, 창 높이의 60%를 넘지 않게
    let bite = |room: f64| {
        ((((room - 8.0).max(0.0)) * sf) as i32).min(size.height as i32 * 6 / 10)
    };
    let overlap = bite(headroom);
    // 게이지 열 때문에 캐릭터는 창 중앙이 아니다 → 보고받은 캐릭터 중심에 맞춘다
    let anchor_x = match center_x {
        Some(cx) => pos.x + (cx * sf) as i32,
        None => pos.x + (size.width as i32) / 2,
    };
    // 가로는 화면 안으로 가두지 않는다 — 펫을 화면 밖에 걸쳐 두는 건 의도된 배치이고
    // (start_pet_drag 참고), 말풍선만 경계에서 멈추면 꼬리가 머리에서 떨어져 나간다.
    // 펫과 함께 잘려 나가더라도 항상 붙어 있는 쪽을 택한다.
    let x = anchor_x - (bsize.width as i32) / 2;
    let mut y = pos.y - bsize.height as i32 + overlap;
    let mut tail = "bottom"; // 말풍선이 위에 → 꼬리는 아래로 펫을 가리킴

    // 세로는 다르다: 위가 막히면 아래로 뒤집어도 여전히 머리에 붙어 있으므로 가둠이 아니다
    if let Ok(Some(mon)) = pet.current_monitor() {
        let mpos = mon.position();
        if y < mpos.y + 4 {
            // 상단 공간 부족 → 펫 아래에 표시, 꼬리는 위로.
            // 창 바닥이 아니라 캐릭터 발밑을 기준으로 삼는다 — 그림자·미니 라벨·무대
            // 패딩(footroom)을 빼지 않으면 위로 띄울 때보다 눈에 띄게 멀어진다.
            y = pos.y + size.height as i32 - bite(footroom);
            tail = "top";
        }
    }
    Some((x, y, tail))
}

/// 펫이 움직이거나 크기가 바뀌면 떠 있는 말풍선을 따라 옮긴다.
/// 펫 창의 Moved/Resized 에 걸려 있어 드래그·크기 변경·모니터 이동을 모두 커버한다.
pub fn reposition_bubble(app: &AppHandle) {
    let Some(bubble) = app.get_webview_window("bubble") else {
        return;
    };
    // 말풍선이 떠 있을 때만 — 숨은 창을 옮길 이유가 없다
    if !bubble.is_visible().unwrap_or(false) {
        return;
    }
    let Some((x, y, tail)) = bubble_placement(app) else {
        return;
    };
    let _ = bubble.set_position(PhysicalPosition::new(x, y));

    // 꼬리 방향이 실제로 뒤집힐 때만 알린다 — 드래그 프레임마다 이벤트를 쏘면
    // 말풍선이 쉴 새 없이 다시 그려진다
    use tauri::Emitter;
    let state = app.state::<AppState>();
    let mut last = state.speech_tail.lock().unwrap();
    if *last != tail {
        *last = tail;
        let _ = app.emit("speech-tail", tail);
    }
}

/// 캐릭터 머리 위에 대사 말풍선을 띄운다 (상황 이벤트 전용, 일정 시간 후 자동 사라짐).
/// 화면 상단에 닿으면 펫 아래로 반전한다.
#[tauri::command]
pub fn show_speech(app: AppHandle, text: String) {
    let (enabled, duration) = {
        let s = app.state::<AppState>();
        let s = s.settings.lock().unwrap();
        (s.speech_enabled, s.speech_duration_ms)
    };
    if !enabled || text.trim().is_empty() {
        return;
    }
    let Some(bubble) = app.get_webview_window("bubble") else {
        return;
    };
    let Some((x, y, tail)) = bubble_placement(&app) else {
        return;
    };
    *app.state::<AppState>().speech_tail.lock().unwrap() = tail;

    use tauri::Emitter;
    let _ = app.emit("speech", serde_json::json!({ "text": text, "tail": tail }));
    let _ = bubble.set_position(PhysicalPosition::new(x, y));
    let _ = bubble.show();
    // 대사는 꼬리가 머리에 닿도록 펫 창과 일부러 겹친다. 펫도 always-on-top 이라
    // 그냥 두면 겹친 부분이 펫에 가리므로 표시할 때마다 최상위로 다시 올린다.
    let _ = bubble.set_always_on_top(true);

    // 자동 숨김. 표시할 때마다 세대를 올려, 새 대사가 뜨면 이전 타이머는 무시된다.
    let gen = {
        let s = app.state::<AppState>();
        let mut g = s.speech_gen.lock().unwrap();
        *g = g.wrapping_add(1);
        *g
    };
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(duration));
        let current = *app2.state::<AppState>().speech_gen.lock().unwrap();
        if current != gen {
            return; // 그 사이 새 대사가 떴다 → 이 타이머는 만료 처리
        }
        if let Some(b) = app2.get_webview_window("bubble") {
            let _ = b.hide();
        }
    });
}

/// 사용량 패널 열기/닫기 (펫 우클릭·트레이). 반환값: 토글 후 열림 여부.
#[tauri::command]
pub fn toggle_panel(app: AppHandle) -> bool {
    let Some(panel) = app.get_webview_window("panel") else {
        return false;
    };
    // 리사이즈 도중 닫혔을 수 있다 — 상태를 남기면 다시 열었을 때 첫 커서 이동에 끌려간다
    clear_window_resize(&app, "panel");
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return false;
    }
    place_panel(&app, &panel);
    let _ = panel.show();
    let _ = panel.set_always_on_top(true);
    let _ = panel.set_focus();
    true
}

/// 저장된 크기·위치로 패널 배치. 위치가 없으면 펫이 있는 모니터의 우하단 근처.
/// 크기를 먼저 적용해야 기본 위치 계산(우하단 정렬)이 실제 크기를 쓴다.
fn place_panel(app: &AppHandle, panel: &tauri::WebviewWindow) {
    let (saved, saved_size) = {
        let s = app.state::<AppState>();
        let s = s.settings.lock().unwrap();
        (s.panel_pos, s.panel_size)
    };
    if let Some((w, h)) = saved_size {
        let _ = panel.set_size(tauri::PhysicalSize::new(w, h));
    }
    if let Some((x, y)) = saved {
        let _ = panel.set_position(PhysicalPosition::new(x, y));
        return;
    }
    let Ok(psize) = panel.outer_size() else { return };
    let mon = app
        .get_webview_window("pet")
        .and_then(|p| p.current_monitor().ok().flatten())
        .or_else(|| panel.primary_monitor().ok().flatten());
    if let Some(mon) = mon {
        let mp = mon.position();
        let ms = mon.size();
        let x = mp.x + ms.width as i32 - psize.width as i32 - 24;
        let y = mp.y + ms.height as i32 - psize.height as i32 - 96;
        let _ = panel.set_position(PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
pub fn save_panel_position(state: State<'_, AppState>, x: i32, y: i32) {
    let mut s = state.settings.lock().unwrap();
    s.panel_pos = Some((x, y));
    settings::save(&s);
}

/// 리사이즈를 허용하는 창과 각각의 최소 크기 (논리 px).
/// 패널은 이보다 작으면 헤더와 페이지 내비게이션이 겹친다.
/// 설정 창은 본문이 스크롤되므로 헤더·탭이 남을 만큼만 막는다.
fn resize_target(label: &str) -> Option<(&'static str, f64, f64)> {
    match label {
        "panel" => Some(("panel", 260.0, 180.0)),
        "settings" => Some(("settings", 280.0, 240.0)),
        _ => None,
    }
}

/// 리사이즈 시작 — 어느 창(`label`)의 어느 변(`"n"`·`"se"` 등)을 잡았는지와
/// 그 순간의 창 사각형을 기억한다.
///
/// 펫 드래그와 같은 이유로 직접 구현한다. 네이티브 리사이즈(`startResizeDragging`)를
/// 쓰려면 창이 `resizable` 이어야 하는데, 그러면 Windows 가 WS_THICKFRAME 을 붙여
/// 투명 창 둘레에 흐릿한 프레임 그림자를 그리고 모서리 히트테스트도 OS 손에 넘어간다.
/// 커서 물리 좌표로 직접 옮기면 창을 장식 없이 둔 채 여덟 방향 모두 잡을 수 있다.
#[tauri::command]
pub fn start_window_resize(app: AppHandle, label: String, dir: String) {
    let Some((label, min_w, min_h)) = resize_target(&label) else {
        return;
    };
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let (Ok(cursor), Ok(pos), Ok(size)) = (
        app.cursor_position(),
        win.outer_position(),
        win.outer_size(),
    ) else {
        return;
    };
    // "ne"·"sw" 처럼 두 글자면 두 변을 동시에 잡는다 (대각선)
    let edges = (
        dir.contains('w'),
        dir.contains('n'),
        dir.contains('e'),
        dir.contains('s'),
    );
    if edges == (false, false, false, false) {
        return;
    }
    *app.state::<AppState>().window_resize.lock().unwrap() = Some(crate::WindowResize {
        label,
        cursor: (cursor.x, cursor.y),
        rect: (
            pos.x,
            pos.y,
            pos.x + size.width as i32,
            pos.y + size.height as i32,
        ),
        edges,
        min: (min_w, min_h),
    });
}

/// 리사이즈 한 스텝 — 시작 사각형에 커서 이동량을 더해 잡은 변만 민다.
#[tauri::command]
pub fn resize_window(app: AppHandle) {
    let Some(r) = *app.state::<AppState>().window_resize.lock().unwrap() else {
        return;
    };
    let Some(win) = app.get_webview_window(r.label) else {
        return;
    };
    let Ok(cursor) = app.cursor_position() else {
        return;
    };
    let sf = win.scale_factor().unwrap_or(1.0);
    let (min_w, min_h) = ((r.min.0 * sf) as i32, (r.min.1 * sf) as i32);

    let (dx, dy) = (cursor.x - r.cursor.0, cursor.y - r.cursor.1);
    let (l0, t0, r0, b0) = r.rect;
    let (west, north, east, south) = r.edges;
    let mut left = if west { (l0 as f64 + dx).round() as i32 } else { l0 };
    let mut top = if north { (t0 as f64 + dy).round() as i32 } else { t0 };
    let mut right = if east { (r0 as f64 + dx).round() as i32 } else { r0 };
    let mut bottom = if south { (b0 as f64 + dy).round() as i32 } else { b0 };

    // 최소 크기에 걸리면 잡고 있는 변만 되민다 — 반대편은 제자리에 있어야 한다
    if right - left < min_w {
        if west {
            left = right - min_w;
        } else {
            right = left + min_w;
        }
    }
    if bottom - top < min_h {
        if north {
            top = bottom - min_h;
        } else {
            bottom = top + min_h;
        }
    }

    let _ = win.set_size(tauri::PhysicalSize::new(
        (right - left) as u32,
        (bottom - top) as u32,
    ));
    // 서/북쪽을 잡으면 좌상단도 함께 움직인다. 매 스텝을 누적이 아니라 시작 사각형
    // 기준으로 다시 계산하므로 두 호출로 나뉘어도 좌표가 밀리지 않는다.
    let _ = win.set_position(PhysicalPosition::new(left, top));
}

#[tauri::command]
pub fn end_window_resize(app: AppHandle) {
    *app.state::<AppState>().window_resize.lock().unwrap() = None;
}

/// 리사이즈 도중 창이 닫혔을 수 있을 때의 정리 — 상태가 남으면 다시 열었을 때
/// 손잡이에 커서만 스쳐도 창이 끌려간다. 다른 창의 진행 중 리사이즈는 건드리지 않는다.
pub fn clear_window_resize(app: &AppHandle, label: &str) {
    let state = app.state::<AppState>();
    let mut r = state.window_resize.lock().unwrap();
    if (*r).is_some_and(|v| v.label == label) {
        *r = None;
    }
}

#[tauri::command]
pub fn save_window_size(state: State<'_, AppState>, label: String, width: u32, height: u32) {
    // 최소화 등으로 0이 오면 저장하지 않는다 — 다음 실행 때 창이 사라져 보인다
    if width == 0 || height == 0 {
        return;
    }
    let mut s = state.settings.lock().unwrap();
    match label.as_str() {
        "panel" => s.panel_size = Some((width, height)),
        "settings" => s.settings_size = Some((width, height)),
        _ => return,
    }
    settings::save(&s);
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

/// 드래그 시작 — 잡은 지점을 **창 크기 대비 비율(0..1)** 로 기억한다.
///
/// OS 드래그(`start_dragging`)는 창 상단이 작업 영역 위로 못 올라가게 막혀서
/// 화면 밖 배치가 불가능하다. 그래서 직접 옮긴다. 이때 간격을 물리 px로 고정하면
/// 배율이 다른 모니터로 넘어갈 때(예: 150% → 100%) Windows 가 창을 리사이즈하면서
/// 잡은 지점이 캐릭터 위에서 밀려나 커서와 벌어진다 → 비율로 들고 있으면
/// 창이 커지든 작아지든 같은 지점을 계속 잡고 있게 된다.
#[tauri::command]
pub fn start_pet_drag(app: AppHandle) {
    let Some(pet) = app.get_webview_window("pet") else {
        return;
    };
    let (Ok(cursor), Ok(pos), Ok(size)) =
        (app.cursor_position(), pet.outer_position(), pet.outer_size())
    else {
        return;
    };
    let (w, h) = (size.width.max(1) as f64, size.height.max(1) as f64);
    *app.state::<AppState>().drag_grab.lock().unwrap() = Some((
        ((cursor.x - pos.x as f64) / w).clamp(0.0, 1.0),
        ((cursor.y - pos.y as f64) / h).clamp(0.0, 1.0),
    ));
}

/// 드래그 한 스텝 — 지금 커서 위치에서, 잡은 비율만큼 되짚어 창 좌상단을 정한다.
#[tauri::command]
pub fn drag_pet(app: AppHandle) {
    let Some((rx, ry)) = *app.state::<AppState>().drag_grab.lock().unwrap() else {
        return;
    };
    let Some(pet) = app.get_webview_window("pet") else {
        return;
    };
    let (Ok(cursor), Ok(size)) = (app.cursor_position(), pet.outer_size()) else {
        return;
    };
    let _ = pet.set_position(PhysicalPosition::new(
        (cursor.x - rx * size.width as f64).round() as i32,
        (cursor.y - ry * size.height as f64).round() as i32,
    ));
}

/// 캐릭터 우클릭 시 트레이와 같은 메뉴를 띄운다.
#[tauri::command]
pub fn show_pet_menu(app: AppHandle) {
    let _ = crate::tray::popup_pet_menu(&app);
}

#[tauri::command]
pub fn end_pet_drag(app: AppHandle) {
    *app.state::<AppState>().drag_grab.lock().unwrap() = None;
}

/// 펫 웹뷰가 실측한 콘텐츠 크기에 창을 딱 맞춘다 (논리 px).
/// 투명한 여백이 남으면 화면 가장자리에 붙여도 캐릭터가 떠 보이고, 그 여백이
/// 클릭을 삼키기까지 한다 → 콘텐츠만큼만 남긴다.
/// 발 위치(하단 중앙)를 기준으로 보정 이동해 캐릭터는 제자리에 머문다.
#[tauri::command]
pub fn fit_pet_window(app: AppHandle, width: f64, height: f64) {
    let Some(pet) = app.get_webview_window("pet") else {
        return;
    };
    let (Ok(old_pos), Ok(old_size), Ok(sf)) =
        (pet.outer_position(), pet.outer_size(), pet.scale_factor())
    else {
        return;
    };
    let new_w = (width.max(40.0) * sf).round() as i32;
    let new_h = (height.max(40.0) * sf).round() as i32;
    // 1px 떨림으로 리사이즈가 반복되지 않게 여유를 둔다
    if (old_size.width as i32 - new_w).abs() <= 2 && (old_size.height as i32 - new_h).abs() <= 2 {
        return;
    }
    let _ = pet.set_size(tauri::PhysicalSize::new(new_w.max(1) as u32, new_h.max(1) as u32));
    let x = old_pos.x - (new_w - old_size.width as i32) / 2;
    let y = old_pos.y - (new_h - old_size.height as i32);
    let _ = pet.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 지원하는 캐릭터 이미지 확장자 (탐색 우선순위)
const CHAR_EXTS: [&str; 4] = ["gif", "webp", "apng", "png"];
const CHAR_STATES: [&str; 7] =
    ["idle", "working", "alert", "sleep", "exhausted", "refreshed", "poke"];

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

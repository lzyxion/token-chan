//! 프론트엔드 ↔ 백엔드 IPC 커맨드.

use tauri::{AppHandle, Manager, PhysicalPosition, State};
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{self, Settings};
use crate::window;
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
    new_settings.weekly_alert_threshold = new_settings.weekly_alert_threshold.clamp(0.1, 1.0);
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
    if new_settings.gauge_style != "bar" {
        new_settings.gauge_style = "ring".into();
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
    if !["auto", "claude", "codex", "antigravity"].contains(&vendor.as_str()) {
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
pub fn save_pet_position(app: AppHandle, state: State<'_, AppState>, x: i32, y: i32) {
    let mut s = state.settings.lock().unwrap();
    s.pet_pos = Some((x, y));
    save_settings(&app, &s);
}

/// 펫 웹뷰가 캐릭터의 실측 위치를 미리 보고 (크기·팩·상태·게이지 위치 변경 시).
/// `headroom`은 머리 위 여백, `center_x`는 창 안에서의 캐릭터 가로 중심.
/// 대사가 이벤트로 갑자기 떠도 꼬리가 머리에 정확히 닿게 하기 위해 캐시해 둔다.
#[tauri::command]
pub fn set_anchor(app: AppHandle, headroom: f64, footroom: f64, center_x: f64) {
    {
        let state = app.state::<AppState>();
        *state.headroom.lock().unwrap() = headroom.max(0.0);
        *state.footroom.lock().unwrap() = footroom.max(0.0);
        *state.center_x.lock().unwrap() = Some(center_x);
    }
    // 상태 전이 대사는 새 포즈의 앵커가 보고되기 **전에** 뜬다(rAF 순서상 show_speech 가
    // 먼저다). 창 크기가 바뀌면 Resized 가 reposition_bubble 을 불러 바로잡히지만,
    // 크기가 같은 포즈로 바뀌면 이 보고가 마지막 신호다 — 여기서 안 옮기면 말풍선이
    // 이전 포즈 기준 자리에 대사 내내 남는다.
    reposition_bubble(&app);
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
    if let Some(sp) = window::spec("panel") {
        window::restore(&app, sp, &panel);
    }
    let _ = panel.show();
    let _ = panel.set_always_on_top(true);
    let _ = panel.set_focus();
    true
}

/// 사용자가 옮긴 자리를 기억한다. 어느 창인지는 [`window::WINDOWS`] 표가 안다 —
/// 창마다 커맨드를 따로 두면 새 창이 생길 때마다 둘씩 늘어난다.
#[tauri::command]
pub fn save_window_position(app: AppHandle, label: String, x: i32, y: i32) {
    if let Some(sp) = window::spec(&label) {
        sp.save_pos(&app, x, y);
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
    // 표에 없는 라벨은 리사이즈를 허용하지 않는다 (펫·말풍선)
    let Some(sp) = window::spec(&label) else {
        return;
    };
    let Some(win) = app.get_webview_window(sp.label) else {
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
        label: sp.label,
        cursor: (cursor.x, cursor.y),
        rect: (
            pos.x,
            pos.y,
            pos.x + size.width as i32,
            pos.y + size.height as i32,
        ),
        edges,
        min: sp.min,
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
pub fn save_window_size(app: AppHandle, label: String, width: u32, height: u32) {
    if let Some(sp) = window::spec(&label) {
        sp.save_size(&app, width, height);
    }
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
        save_settings(app, &s);
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

/// 지원하는 캐릭터 이미지 확장자 (탐색 우선순위).
/// svg 는 `<img>` 로 그려지므로 바깥 CSS 가 내부에 닿지 않는다 —
/// 애니메이션이 필요하면 SVG 파일 안에 직접 넣어야 한다.
const CHAR_EXTS: [&str; 5] = ["gif", "webp", "apng", "png", "svg"];
/// 상태 이미지 크기 상한. 렌더링(`image_data_url`)과 교체(`copy_state_image`)가
/// 같은 값을 봐야 한다 — 복사만 성공하고 화면에는 안 나오는 반쪽 상태를 막는다.
const CHAR_MAX_BYTES: u64 = 20 * 1024 * 1024;
const CHAR_STATES: [&str; 8] =
    ["idle", "working", "alert", "sleep", "exhausted", "refreshed", "done", "poke"];

fn find_state_file(pack_dir: &std::path::Path, state: &str) -> Option<std::path::PathBuf> {
    CHAR_EXTS
        .iter()
        .map(|ext| pack_dir.join(format!("{state}.{ext}")))
        .find(|p| p.is_file())
}

/// 팩 폴더의 대사 파일 (`characters/<팩>/speech.json`). 없으면 None — 기본 문구 폴백.
/// 대사는 캐릭터의 속성이라 설정이 아닌 팩 폴더에서 이미지와 함께 관리한다.
#[tauri::command]
pub fn get_character_speech(
    pack: String,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    settings::load_pack_speech(&pack)
}

/// 팩 대사 저장 — 설정 창 편집기의 쓰기 경로. 실질 문구가 있는 키만 남기고,
/// 전부 비면 파일을 지워 팩 폴더를 깨끗하게 유지한다. 대사 파일은 설정 파일 밖이라
/// settings-changed 로는 전파되지 않으므로 전용 이벤트로 펫에게 알린다.
#[tauri::command]
pub fn set_character_speech(
    app: AppHandle,
    pack: String,
    lines: std::collections::HashMap<String, Vec<String>>,
) {
    let Some(path) = settings::pack_speech_path(&pack) else { return };
    let lines: std::collections::HashMap<String, Vec<String>> = lines
        .into_iter()
        .filter(|(_, v)| v.iter().any(|l| !l.trim().is_empty()))
        .collect();
    if lines.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else if let Ok(json) = serde_json::to_string_pretty(&lines) {
        let _ = std::fs::write(&path, json);
    }
    use tauri::Emitter;
    let _ = app.emit("character-speech-changed", &pack);
}

/// 팩별 동작 설정 (`characters/<팩>/pack.json`). 없으면 기본값(모든 상태 사용).
#[tauri::command]
pub fn get_character_config(pack: String) -> settings::PackConfig {
    settings::load_pack_config(&pack)
}

/// 팩 설정 저장 — 기본값(끈 상태 없음)이면 파일을 지워 폴더를 깨끗하게 유지한다.
#[tauri::command]
pub fn set_character_config(app: AppHandle, pack: String, config: settings::PackConfig) {
    let Some(path) = settings::pack_config_path(&pack) else { return };
    if config.disabled_states.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else if let Ok(json) = serde_json::to_string_pretty(&config) {
        let _ = std::fs::write(&path, json);
    }
    use tauri::Emitter;
    let _ = app.emit("character-config-changed", &pack);
}

/// 새 팩 폴더 생성. idle 이미지를 넣기 전까지 펫에서는 선택 불가(목록 필터)지만,
/// 스튜디오에서는 `list_character_dirs` 로 보여 이어서 채울 수 있다.
#[tauri::command]
pub fn create_character_pack(name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    let Some(root) = settings::characters_dir() else {
        return Err("설정 폴더를 찾을 수 없습니다".into());
    };
    let Some(dir) = settings::pack_dir(&name) else {
        return Err("팩 이름에 쓸 수 없는 문자가 있습니다".into());
    };
    if dir.exists() {
        return Err("이미 있는 팩 이름입니다".into());
    }
    let _ = std::fs::create_dir_all(root);
    std::fs::create_dir(&dir).map_err(|e| e.to_string())
}

/// 팩 이름 변경. 폴더만 바꾸면 설정이 옛 이름을 가리켜 낡으므로,
/// 선택된 팩(characterPack)과 모델별 규칙(characterRules)의 참조도 함께 고친다.
#[tauri::command]
pub fn rename_character_pack(app: AppHandle, old: String, new: String) -> Result<(), String> {
    let new = new.trim().to_string();
    let (Some(old_dir), Some(new_dir)) = (settings::pack_dir(&old), settings::pack_dir(&new))
    else {
        return Err("팩 이름에 쓸 수 없는 문자가 있습니다".into());
    };
    if old == new {
        return Ok(());
    }
    if !old_dir.is_dir() {
        return Err("이미 없는 팩입니다".into());
    }
    if new_dir.exists() {
        return Err("이미 있는 팩 이름입니다".into());
    }
    std::fs::rename(&old_dir, &new_dir).map_err(|e| e.to_string())?;

    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        if s.character_pack.as_deref() == Some(old.as_str()) {
            s.character_pack = Some(new.clone());
        }
        for r in &mut s.character_rules {
            if r.pack == old {
                r.pack = new.clone();
            }
        }
        save_settings(&app, &s);
        s.clone()
    };
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
    let _ = app.emit("character-images-changed", &new);
    Ok(())
}

/// 팩 폴더 삭제 — 영구 삭제가 아니라 **휴지통**으로 보낸다. 이미지·대사·설정이
/// 통째로 사라지는 작업이라, 확인창 대신 되돌릴 수 있는 경로를 택했다.
#[tauri::command]
pub fn delete_character_pack(app: AppHandle, pack: String) -> Result<(), String> {
    let Some(dir) = settings::pack_dir(&pack) else {
        return Err("잘못된 팩 이름입니다".into());
    };
    if !dir.is_dir() {
        return Err("이미 없는 팩입니다".into());
    }
    trash::delete(&dir).map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("character-images-changed", &pack);
    Ok(())
}

/// 스튜디오 좌측 목록용 — idle 이 아직 없는(미완성) 팩 폴더까지 전부
#[tauri::command]
pub fn list_character_dirs() -> Vec<String> {
    let Some(root) = settings::characters_dir() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(&root) else { return vec![] };
    let mut dirs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    dirs.sort();
    dirs
}

/// 원본 이미지를 팩 폴더에 `<상태>.<확장자>` 로 **원자적으로** 복사 — 임시 파일에
/// 다 받은 뒤 `rename` 으로 갈아끼운다(settings.rs `save_to` 와 같은 패턴). 대상에
/// 직접 복사하면 도중에 앱이 죽었을 때 잘린 이미지가 남는다.
///
/// 같은 상태의 다른 확장자 파일은 **교체가 성공한 뒤에** 지운다 — 탐색 우선순위
/// (CHAR_EXTS)가 옛 파일을 계속 집는 걸 막되, 복사가 실패하면 기존 이미지를 지킨다.
fn copy_state_image(dir: &std::path::Path, state: &str, src: &std::path::Path) -> bool {
    let Some(ext) = src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .filter(|e| CHAR_EXTS.contains(&e.as_str()))
    else {
        return false;
    };
    // 크기 선검사 — 렌더링이 거부할 파일이면 손대기 전에 거른다. 여기서 안 거르면
    // 교체는 성공했는데 화면에는 안 나오고, 멀쩡하던 기존 이미지만 잃는다.
    match std::fs::metadata(src) {
        Ok(m) if m.len() <= CHAR_MAX_BYTES => {}
        _ => return false,
    }
    // 고정 이름이라 중간에 죽어 남더라도 다음 교체가 덮어쓴다. 확장자가 tmp 라
    // CHAR_EXTS 탐색에는 절대 걸리지 않는다.
    let tmp = dir.join(format!("{state}.{ext}.tmp"));
    // sync_all(FlushFileBuffers)은 Windows 에서 쓰기 핸들을 요구한다 — write 로 연다.
    let copied = std::fs::copy(src, &tmp).is_ok()
        && std::fs::OpenOptions::new()
            .write(true)
            .open(&tmp)
            .and_then(|f| f.sync_all())
            .is_ok()
        && std::fs::rename(&tmp, dir.join(format!("{state}.{ext}"))).is_ok();
    if !copied {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    for other in CHAR_EXTS.iter().filter(|e| **e != ext) {
        let _ = std::fs::remove_file(dir.join(format!("{state}.{other}")));
    }
    true
}

/// 상태 이미지 첨부 — 파일 선택 다이얼로그를 별도 스레드에서 띄운다
/// (메인 스레드를 막으면 안 된다).
#[tauri::command]
pub fn import_state_image(app: AppHandle, pack: String, state: String) {
    if !CHAR_STATES.contains(&state.as_str()) {
        return;
    }
    let Some(dir) = settings::pack_dir(&pack) else { return };
    use tauri_plugin_dialog::DialogExt;
    std::thread::spawn(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("이미지", &CHAR_EXTS)
            .blocking_pick_file();
        let Some(picked) = picked else { return };
        let Ok(src) = picked.into_path() else { return };
        if copy_state_image(&dir, &state, &src) {
            use tauri::Emitter;
            let _ = app.emit("character-images-changed", &pack);
        }
    });
}

/// 드래그&드롭용 — 다이얼로그 없이 주어진 경로의 이미지를 상태 슬롯에 등록
#[tauri::command]
pub fn import_state_image_from_path(
    app: AppHandle,
    pack: String,
    state: String,
    path: String,
) -> Result<(), String> {
    if !CHAR_STATES.contains(&state.as_str()) {
        return Err("알 수 없는 상태입니다".into());
    }
    let Some(dir) = settings::pack_dir(&pack) else {
        return Err("잘못된 팩 이름입니다".into());
    };
    if !copy_state_image(&dir, &state, std::path::Path::new(&path)) {
        return Err(
            "이미지 파일이 아니거나 20MB 를 넘거나 복사에 실패했습니다 (gif·webp·apng·png·svg)"
                .into(),
        );
    }
    use tauri::Emitter;
    let _ = app.emit("character-images-changed", &pack);
    Ok(())
}

/// 상태 이미지 제거 (모든 확장자) — 그 상태는 idle 폴백으로 돌아간다.
/// 확인 없이 바로 지운다 — 스튜디오 썸네일이 즉시 폴백으로 바뀌어 결과가 눈에 보인다.
#[tauri::command]
pub fn remove_state_image(app: AppHandle, pack: String, state: String) {
    if !CHAR_STATES.contains(&state.as_str()) {
        return;
    }
    let Some(dir) = settings::pack_dir(&pack) else { return };
    for ext in CHAR_EXTS {
        let _ = std::fs::remove_file(dir.join(format!("{state}.{ext}")));
    }
    use tauri::Emitter;
    let _ = app.emit("character-images-changed", &pack);
}

/// 캐릭터 스튜디오 창 — 저장된 크기로 주 모니터 중앙에 표시
#[tauri::command]
pub fn open_studio(app: AppHandle) {
    let Some(w) = app.get_webview_window("studio") else { return };
    clear_window_resize(&app, "studio");
    if let Some(sp) = window::spec("studio") {
        window::restore(&app, sp, &w);
    }
    let _ = w.show();
    let _ = w.set_focus();
}

/// 설정 창을 트레이 근처(주 모니터 우하단)에 띄운다. `tab` 을 주면 그 탭으로 연다.
///
/// 트레이·펫 메뉴·프론트가 모두 이 한 곳을 쓴다 — 위치 계산과 크기 복원이 갈라지면
/// 어느 경로로 열었는지에 따라 창이 다른 자리에 뜬다.
#[tauri::command]
pub fn open_settings(app: AppHandle, tab: Option<String>) {
    let Some(w) = app.get_webview_window("settings") else { return };
    // 리사이즈 도중 ✕/Esc 로 닫혔을 수 있다 — toggle_panel 과 같은 정리
    clear_window_resize(&app, "settings");
    // 크기·자리 복원은 세 창이 같은 규칙을 쓴다 (`window::restore`)
    if let Some(sp) = window::spec("settings") {
        window::restore(&app, sp, &w);
    }
    let _ = w.show();
    let _ = w.set_focus();
    if let Some(tab) = tab {
        use tauri::Emitter;
        // 창이 이미 떠 있던 경우에도 탭은 바꿔야 하므로 show 뒤에 항상 보낸다
        let _ = app.emit("settings-tab", tab);
    }
}

// ─────────────────────────── 계정·홈 ───────────────────────────
//
// 발견 결과(`AppState::accounts`)는 여태 트레이 메뉴만 읽었고, 거기서는 `label` 한 줄밖에
// 못 그린다. 계정 탭이 나머지(플랜·인증 방식·설치본 경로·발견 방식)를 쓰므로 여기서 내보낸다.

/// 계정 한 줄 + "지금 집계에 포함되는가".
///
/// 포함 여부 규칙(`account_enabled`: 사용자 지정이 있으면 그 값, 없으면 `standard`)은
/// 백엔드에만 있다. 프론트가 같은 규칙을 다시 구현하면 두 곳이 어긋나므로 풀어서 보낸다.
#[derive(serde::Serialize)]
pub struct AccountView {
    #[serde(flatten)]
    account: usage_core::accounts::Account,
    /// 토글할 때 그대로 돌려보내는 키
    setting_key: String,
    enabled: bool,
}

#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Vec<AccountView> {
    // 두 락을 겹쳐 잡지 않는다 (monitor::enabled_roots 와 같은 이유)
    let overrides = { state.settings.lock().unwrap().accounts_enabled.clone() };
    let accounts = { state.accounts.lock().unwrap().clone() };
    accounts
        .into_iter()
        .map(|a| AccountView {
            setting_key: a.setting_key(),
            enabled: crate::monitor::account_enabled(&a, &overrides),
            account: a,
        })
        .collect()
}

/// 계정의 집계 포함 여부를 지정한다. 트레이 체크 항목도 이 함수를 거쳐 한 규칙만 남긴다.
#[tauri::command]
pub fn set_account_enabled(app: AppHandle, setting_key: String, enabled: bool) {
    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.accounts_enabled.insert(setting_key, enabled);
        save_settings(&app, &s);
        s.clone()
    };
    crate::tray::refresh_menu(&app);
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
    // 계정 목록 자체는 안 바뀌었지만 체크 상태가 바뀌었으므로 탭도 다시 그린다
    let _ = app.emit("accounts-changed", ());
}

/// 설정에 저장된 소스별 추가 홈 목록을 고른다. 세 군데(추가·제거·표시)가 같은 분기를
/// 쓰므로 한 곳에 둔다 — 알 수 없는 값은 Claude 로 떨어뜨리던 기존 동작을 유지한다.
fn extra_list<'a>(s: &'a mut Settings, source: &str) -> &'a mut Vec<String> {
    match source {
        "codex" => &mut s.extra_codex_homes,
        "antigravity" => &mut s.extra_antigravity_homes,
        _ => &mut s.extra_claude_homes,
    }
}

/// 계정/홈이 바뀐 뒤의 공통 뒤처리 — 다시 발견하고 메뉴와 창을 모두 갱신한다.
fn after_home_change(app: &AppHandle, updated: &Settings) {
    // 새 경로가 어떤 계정인지는 다시 발견해야 알 수 있다
    crate::monitor::rediscover(app);
    crate::tray::refresh_menu(app);
    use tauri::Emitter;
    let _ = app.emit("settings-changed", updated);
    let _ = app.emit("accounts-changed", ());
}

/// 폴더 선택으로 스캔 홈을 추가한다.
/// 다이얼로그가 메인 스레드를 막으면 안 되므로 별도 스레드에서 연다.
#[tauri::command]
pub fn add_home(app: AppHandle, source: String) {
    use tauri_plugin_dialog::DialogExt;
    std::thread::spawn(move || {
        let Some(picked) = app.dialog().file().blocking_pick_folder() else { return };
        let Ok(path) = picked.into_path() else { return };
        let path = path.display().to_string();

        let updated = {
            let state = app.state::<AppState>();
            let mut s = state.settings.lock().unwrap();
            let list = extra_list(&mut s, &source);
            if list.iter().any(|p| p == &path) {
                return; // 이미 있는 경로 — 중복 등록 방지
            }
            list.push(path);
            save_settings(&app, &s);
            s.clone()
        };
        after_home_change(&app, &updated);
    });
}

/// 직접 추가한 스캔 홈을 지운다. 자동 발견된 홈은 대상이 아니다(다시 발견되므로).
#[tauri::command]
pub fn remove_home(app: AppHandle, source: String, path: String) {
    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        let list = extra_list(&mut s, &source);
        let before = list.len();
        list.retain(|p| p != &path);
        if list.len() == before {
            return; // 화면이 낡아 이미 지워진 경로 — 저장/재탐색 불필요
        }
        save_settings(&app, &s);
        s.clone()
    };
    after_home_change(&app, &updated);
}

/// 계정을 처음부터 다시 찾는다 (마커 스캔 + WSL 재조회).
///
/// 수백 ms ~ 수 초가 걸리므로 별도 스레드로 돌리고, 끝나면 `accounts-changed` 로 알린다.
/// 커맨드가 바로 반환하니 프론트는 그 이벤트를 기다려야 한다.
#[tauri::command]
pub fn rescan_accounts(app: AppHandle) {
    std::thread::spawn(move || {
        crate::monitor::rescan(&app);
        crate::tray::refresh_menu(&app);
        use tauri::Emitter;
        let _ = app.emit("accounts-changed", ());
    });
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

/// 이미지 파일 → data URL (CHAR_MAX_BYTES 상한 — 데스크톱 펫 이미지로는 과대한 크기 방지)
fn image_data_url(p: &std::path::Path) -> Option<String> {
    use base64::Engine;
    let mime = match p.extension()?.to_str()? {
        "gif" => "image/gif",
        "webp" => "image/webp",
        "apng" => "image/apng",
        "svg" => "image/svg+xml",
        _ => "image/png",
    };
    // 읽기 전에 거른다 — 거부할 파일을 메모리에 통째로 올릴 이유가 없다
    if std::fs::metadata(p).ok()?.len() > CHAR_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// 지정한(또는 기본 선택된) 팩의 상태별 이미지를 data URL 로 반환.
/// 없는 상태는 idle 로 폴백. 팩 미지정/무효 시 None (기본 CSS 고양이 사용).
#[tauri::command]
pub fn get_character_images(
    state: State<'_, AppState>,
    pack: Option<String>,
) -> Option<std::collections::HashMap<String, String>> {
    let pack = pack.or_else(|| state.settings.lock().unwrap().character_pack.clone())?;
    let dir = settings::pack_dir(&pack)?;
    let idle = find_state_file(&dir, "idle")?; // idle 필수

    let idle_url = image_data_url(&idle)?;
    let mut map = std::collections::HashMap::new();
    for st in CHAR_STATES {
        let url = find_state_file(&dir, st)
            .and_then(|p| image_data_url(&p))
            .unwrap_or_else(|| idle_url.clone());
        map.insert(st.to_string(), url);
    }
    Some(map)
}

/// 스튜디오용 — 상태별 **자기 파일**만 (idle 폴백 없음, 없는 상태는 None).
/// 펫 렌더링용 `get_character_images` 는 폴백을 채워 주므로 "이 상태에 진짜
/// 이미지가 있나"를 구분할 수 없다 — 편집기는 그 구분이 본질이다.
/// idle 이 없는 미완성 팩도 있는 그대로 보여준다.
#[tauri::command]
pub fn get_state_images(
    pack: String,
) -> std::collections::HashMap<String, Option<String>> {
    let mut map = std::collections::HashMap::new();
    let dir = settings::pack_dir(&pack);
    for st in CHAR_STATES {
        let url = dir
            .as_deref()
            .and_then(|d| find_state_file(d, st))
            .and_then(|p| image_data_url(&p));
        map.insert(st.to_string(), url);
    }
    map
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

#[cfg(test)]
mod tests {
    use super::*;

    fn files_in(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// 확장자가 바뀌는 교체 — 새 파일이 자리잡고, 옛 확장자와 임시 파일은 안 남는다
    #[test]
    fn replacing_across_extensions_leaves_only_the_new_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("idle.png"), b"old").unwrap();
        let src = dir.path().join("src.gif");
        std::fs::write(&src, b"new").unwrap();

        assert!(copy_state_image(dir.path(), "idle", &src));
        assert_eq!(files_in(dir.path()), ["idle.gif", "src.gif"]);
        assert_eq!(std::fs::read(dir.path().join("idle.gif")).unwrap(), b"new");
    }

    /// 원자성의 핵심 — 원본을 못 읽으면 기존 이미지가 그대로 살아 있어야 한다
    #[test]
    fn failed_copy_keeps_the_existing_image() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("idle.png"), b"old").unwrap();

        assert!(!copy_state_image(dir.path(), "idle", &dir.path().join("missing.gif")));
        assert_eq!(files_in(dir.path()), ["idle.png"]);
        assert_eq!(std::fs::read(dir.path().join("idle.png")).unwrap(), b"old");
    }

    /// 중간에 죽어 남은 임시 파일이 있어도 다음 교체가 덮어쓴다
    #[test]
    fn stale_temp_file_does_not_block_replacement() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("idle.png.tmp"), b"stale").unwrap();
        let src = dir.path().join("src.png");
        std::fs::write(&src, b"new").unwrap();

        assert!(copy_state_image(dir.path(), "idle", &src));
        assert_eq!(files_in(dir.path()), ["idle.png", "src.png"]);
        assert_eq!(std::fs::read(dir.path().join("idle.png")).unwrap(), b"new");
    }

    /// 같은 확장자 교체 — Windows 에서 rename 이 기존 파일을 대체하는 경로
    #[test]
    fn replacing_same_extension_overwrites_in_place() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("idle.png"), b"old").unwrap();
        let src = dir.path().join("src.png");
        std::fs::write(&src, b"new").unwrap();

        assert!(copy_state_image(dir.path(), "idle", &src));
        assert_eq!(files_in(dir.path()), ["idle.png", "src.png"]);
        assert_eq!(std::fs::read(dir.path().join("idle.png")).unwrap(), b"new");
    }

    /// 렌더링이 거부할 크기는 교체 전에 거른다 — 기존 이미지가 그대로 살아 있어야 한다
    #[test]
    fn oversized_source_is_rejected_before_touching_anything() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("idle.png"), b"old").unwrap();
        let src = dir.path().join("src.gif");
        let big = std::fs::File::create(&src).unwrap();
        big.set_len(CHAR_MAX_BYTES + 1).unwrap();

        assert!(!copy_state_image(dir.path(), "idle", &src));
        assert_eq!(files_in(dir.path()), ["idle.png", "src.gif"]);
        assert_eq!(std::fs::read(dir.path().join("idle.png")).unwrap(), b"old");
    }

    /// 허용 목록 밖 확장자는 손대기 전에 거른다
    #[test]
    fn unknown_extension_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bmp");
        std::fs::write(&src, b"x").unwrap();

        assert!(!copy_state_image(dir.path(), "idle", &src));
        assert_eq!(files_in(dir.path()), ["src.bmp"]);
    }
}

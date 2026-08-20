//! 펫 창과 말풍선 — 위치·크기·드래그·대사 표시.

use tauri::{AppHandle, Manager, PhysicalPosition, State};

use crate::settings;
use crate::AppState;

use super::config::save_settings;

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

/// 말풍선을 놓을 자리 — 좌표·꼬리 방향과 **갈 화면 배율로 잰 창 크기**.
struct BubbleSpot {
    /// 창 좌상단 (물리 px)
    x: i32,
    y: i32,
    /// 꼬리 방향 ("bottom" = 말풍선이 위, 꼬리가 아래로 펫을 가리킴)
    tail: &'static str,
    /// 창 크기 (물리 px) — 갈 화면 배율로 잰 값
    size: (u32, u32),
}

/// 지금 펫 위치를 기준으로 말풍선이 놓일 자리를 계산한다.
/// 펫이 숨겨져 있으면(말할 주체가 없다) 좌표를 못 읽으면 None.
///
/// 표시 시점(`show_speech`)과 펫이 움직일 때(`reposition_bubble`)가 같은 계산을 써야
/// 드래그 도중에도 꼬리가 머리에서 떨어지지 않는다.
///
/// 말풍선 크기를 **지금 창에서 읽지 않고 다시 재는** 이유: 말풍선은 펫을 따라 화면을
/// 넘나드는데, 창을 옮기면 Windows 가 크기를 배율만큼 다시 잡는다. 옮기기 전 크기로
/// 자리를 계산하면 꼬리가 머리에서 그 차이만큼 어긋난다.
fn bubble_placement(app: &AppHandle) -> Option<BubbleSpot> {
    let pet = app.get_webview_window("pet")?;
    if !pet.is_visible().unwrap_or(false) {
        return None;
    }
    let (Ok(pos), Ok(size)) = (pet.outer_position(), pet.outer_size()) else {
        return None;
    };
    let sf = crate::window::scale_factor(&pet);
    let bsize = crate::window::scaled((settings::BUBBLE_BASE_W, settings::BUBBLE_BASE_H), sf);

    // 머리 위 여백만큼 겹치되 8px(논리)은 남겨 꼬리가 머리에 닿지 않게
    let (headroom, footroom, center_x) = {
        let s = app.state::<AppState>();
        let h = *s.headroom.lock().unwrap();
        let f = *s.footroom.lock().unwrap();
        let c = *s.center_x.lock().unwrap();
        (h, f, c)
    };
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
    let x = anchor_x - (bsize.0 as i32) / 2;
    let mut y = pos.y - bsize.1 as i32 + overlap;
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
    Some(BubbleSpot { x, y, tail, size: bsize })
}

/// 말풍선을 그 자리에 앉힌다 — **자리 → 크기** 순서 (`crate::window` 모듈 주석).
fn place_bubble(bubble: &tauri::WebviewWindow, spot: &BubbleSpot) {
    let _ = bubble.set_position(PhysicalPosition::new(spot.x, spot.y));
    let _ = bubble.set_size(tauri::PhysicalSize::new(spot.size.0, spot.size.1));
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
    let Some(spot) = bubble_placement(app) else {
        return;
    };
    place_bubble(&bubble, &spot);
    let tail = spot.tail;

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
    let Some(spot) = bubble_placement(&app) else {
        return;
    };
    let tail = spot.tail;
    *app.state::<AppState>().speech_tail.lock().unwrap() = tail;

    use tauri::Emitter;
    let _ = app.emit("speech", serde_json::json!({ "text": text, "tail": tail }));
    place_bubble(&bubble, &spot);
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

/// 펫 창 리사이즈 (하단 중앙 = 발 위치 고정: 크기 변화만큼 위/좌로 보정 이동)
pub(crate) fn resize_pet(app: &AppHandle, scale: f64) {
    if let Some(pet) = app.get_webview_window("pet") {
        let sf = crate::window::scale_factor(&pet);
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

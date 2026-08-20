//! 창 열고 닫기와 리사이즈. 기하 규칙 자체는 [`crate::window`] 표가 갖는다.

use tauri::{AppHandle, Manager, PhysicalPosition};

use crate::window;
use crate::AppState;

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
    let sf = window::scale_factor(&win);
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

//! 창 기하(위치·크기) 규칙 한 곳.
//!
//! 패널·설정·스튜디오는 셋 다 "저장된 크기를 먼저 적용하고, 저장된 자리가 화면 안이면
//! 거기로, 아니면 기본 자리로" 라는 **같은 절차**를 따른다. 예전에는 그 절차가 세 벌로
//! 복사돼 있었고 라벨→설정 필드 매핑은 또 따로 세 군데(위치 저장·크기 저장·리사이즈
//! 하한)에 있었다 — 창 하나를 고치려면 여섯 곳을 뒤져야 했고, 실제로 기본 크기를 바꿀
//! 때마다 한 곳씩 빠뜨렸다.
//!
//! 그래서 **라벨이 아는 모든 것**을 [`WINDOWS`] 표 한 줄로 모은다: 저장 슬롯, 리사이즈
//! 하한, 저장값이 없을 때의 기본 자리. 창을 추가하는 일은 이제 표에 한 줄 넣는 것이다.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

use crate::settings::Settings;
use crate::AppState;

/// 저장된 창 위치 (물리 px) — 없으면 아직 사용자가 옮긴 적이 없다는 뜻
pub type SavedPos = Option<(i32, i32)>;
/// 저장된 창 크기 (물리 px) — 없으면 `tauri.conf.json` 의 기본값을 쓴다
pub type SavedSize = Option<(u32, u32)>;

/// 저장된 위치가 없을 때(첫 실행·모니터 해제 등) 어디에 놓을지.
#[derive(Clone, Copy)]
pub enum Place {
    /// 우하단에서 (dx, dy) 만큼 안쪽.
    ///
    /// `follow_pet` 이면 **펫이 떠 있는 모니터**를 기준으로 한다 — 패널은 펫 옆에
    /// 딸려 나오는 창이라 펫과 다른 화면에 뜨면 못 찾는다. 설정 창은 트레이에서
    /// 여는 일이 많아 주 모니터를 기준으로 한다.
    Corner { dx: i32, dy: i32, follow_pet: bool },
    /// 주 모니터 중앙
    Center,
}

/// 창 하나가 지키는 규칙. 필드가 곧 "이 창에 대해 알아야 할 전부"다.
pub struct WinSpec {
    /// Tauri 창 라벨 — 프론트가 보내오는 문자열과 같아야 한다
    pub label: &'static str,
    /// 리사이즈 하한 (논리 px). 이보다 작아지면 레이아웃이 깨진다
    pub min: (f64, f64),
    /// 저장값이 없을 때의 기본 자리
    pub place: Place,
    /// 설정에서 이 창의 위치 슬롯을 꺼낸다
    pos: fn(&mut Settings) -> &mut SavedPos,
    /// 설정에서 이 창의 크기 슬롯을 꺼낸다 (물리 px)
    size: fn(&mut Settings) -> &mut SavedSize,
}

/// 기하를 기억하는 창 목록. 여기 없는 라벨은 리사이즈도 저장도 하지 않는다
/// (펫·말풍선은 크기를 사용자가 정하지 않는다 — 펫은 배율 설정, 말풍선은 내용 맞춤).
pub const WINDOWS: &[WinSpec] = &[
    WinSpec {
        label: "panel",
        // 이보다 작으면 헤더와 페이지 내비게이션이 겹친다
        min: (260.0, 180.0),
        place: Place::Corner { dx: 24, dy: 96, follow_pet: true },
        pos: |s| &mut s.panel_pos,
        size: |s| &mut s.panel_size,
    },
    WinSpec {
        label: "settings",
        // 본문이 스크롤되므로 헤더·탭이 남을 만큼만 막는다
        min: (280.0, 240.0),
        place: Place::Corner { dx: 16, dy: 80, follow_pet: false },
        pos: |s| &mut s.settings_pos,
        size: |s| &mut s.settings_size,
    },
    WinSpec {
        label: "studio",
        // 2열 레이아웃이라 이보다 좁으면 우측 열이 못 산다
        min: (460.0, 340.0),
        place: Place::Center,
        pos: |s| &mut s.studio_pos,
        size: |s| &mut s.studio_size,
    },
];

/// 라벨 → 규칙. 모르는 라벨이면 `None` — 호출부는 조용히 무시하면 된다.
pub fn spec(label: &str) -> Option<&'static WinSpec> {
    WINDOWS.iter().find(|w| w.label == label)
}

impl WinSpec {
    /// 사용자가 옮긴 자리를 기억한다.
    pub fn save_pos(&self, app: &AppHandle, x: i32, y: i32) {
        self.store(app, |s| *(self.pos)(s) = Some((x, y)));
    }

    /// 사용자가 조절한 크기를 기억한다 (물리 px).
    pub fn save_size(&self, app: &AppHandle, width: u32, height: u32) {
        // 최소화 등으로 0이 오면 저장하지 않는다 — 다음 실행 때 창이 사라져 보인다
        if width == 0 || height == 0 {
            return;
        }
        self.store(app, |s| *(self.size)(s) = Some((width, height)));
    }

    fn store(&self, app: &AppHandle, edit: impl FnOnce(&mut Settings)) {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        edit(&mut s);
        crate::commands::save_settings(app, &s);
    }

    fn saved(&self, app: &AppHandle) -> (SavedPos, SavedSize) {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        (*(self.pos)(&mut s), *(self.size)(&mut s))
    }
}

/// 설정 저장(`set_settings`)이 창 기하를 덮어쓰지 못하게 옛 값으로 되돌린다.
///
/// 설정 화면은 창 위치·크기를 편집하지 않지만, 자기가 열릴 때 읽어 둔 **낡은 스냅샷**을
/// 통째로 보내온다. 그 사이 창을 옮기거나 크기를 조절했다면 방금 저장한 값이 되감긴다.
/// 예전엔 이 방어가 필드 6줄을 손으로 나열한 것이라, 창을 추가하면서 빠뜨리기 쉬웠다.
pub fn keep_geometry(old: &mut Settings, new: &mut Settings) {
    for w in WINDOWS {
        *(w.pos)(new) = *(w.pos)(old);
        *(w.size)(new) = *(w.size)(old);
    }
}

/// 저장된 크기·위치를 창에 되돌린다. 표시(`show`)는 하지 않는다 — 호출부마다 그 뒤에
/// 할 일이 다르다(패널은 always-on-top 재적용, 설정은 탭 이벤트).
///
/// **크기를 먼저** 적용해야 기본 자리 계산(우하단 정렬)이 실제 크기를 쓴다.
pub fn restore(app: &AppHandle, sp: &WinSpec, win: &WebviewWindow) {
    let (saved_pos, saved_size) = sp.saved(app);
    if let Some((w, h)) = saved_size {
        let _ = win.set_size(tauri::PhysicalSize::new(w, h));
    }
    // 옮겨 둔 자리가 있으면 그대로 — 창이 뒤로 갔다가 다시 불려 올 때마다 기본 자리로
    // 튀면 옮겨 둔 의미가 없다. 화면 밖을 가리키는 옛 좌표만 걸러 낸다.
    if let Some((x, y)) = saved_pos.filter(|&(x, y)| position_on_screen(win, x, y)) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
        return;
    }
    place_default(app, sp, win);
}

fn place_default(app: &AppHandle, sp: &WinSpec, win: &WebviewWindow) {
    let (dx, dy, follow_pet) = match sp.place {
        Place::Center => {
            let _ = win.center();
            return;
        }
        Place::Corner { dx, dy, follow_pet } => (dx, dy, follow_pet),
    };
    let mon = follow_pet
        .then(|| {
            app.get_webview_window("pet")
                .and_then(|p| p.current_monitor().ok().flatten())
        })
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let (Some(mon), Ok(size)) = (mon, win.outer_size()) else { return };
    let (mp, ms) = (mon.position(), mon.size());
    let _ = win.set_position(PhysicalPosition::new(
        mp.x + ms.width as i32 - size.width as i32 - dx,
        mp.y + ms.height as i32 - size.height as i32 - dy,
    ));
}

/// 저장된 좌표가 현재 모니터 어딘가에 걸치는지. 멀티 모니터를 해제하면 예전 좌표가
/// 화면 밖을 가리켜 창을 영영 못 찾으므로, 복원 전에 이걸로 걸러 기본 배치로 돌린다.
/// 일부만 걸쳐 있는 건 의도된 배치일 수 있어 통과시킨다 (펫과 같은 규칙).
pub fn position_on_screen(win: &WebviewWindow, x: i32, y: i32) -> bool {
    let (Ok(size), Ok(monitors)) = (win.outer_size(), win.available_monitors()) else {
        // 판정 자체가 안 되면 저장값을 믿는 쪽이 덜 놀랍다
        return true;
    };
    let (w, h) = (size.width as i32, size.height as i32);
    monitors.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        x + w > mp.x && x < mp.x + ms.width as i32 && y + h > mp.y && y < mp.y + ms.height as i32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 표에 있는 라벨은 모두 찾을 수 있고, 없는 라벨은 못 찾는다.
    /// (`spec` 이 `None` 을 주면 호출부가 저장·리사이즈를 통째로 건너뛴다)
    #[test]
    fn every_listed_label_resolves() {
        for w in WINDOWS {
            assert_eq!(spec(w.label).map(|s| s.label), Some(w.label));
        }
        assert!(spec("pet").is_none());
        assert!(spec("bubble").is_none());
    }

    /// 설정 저장이 창 기하를 되감지 못하는지 — 프론트가 낡은 값(여기선 None)을
    /// 보내와도 방금 드래그로 저장된 값이 살아남아야 한다.
    #[test]
    fn geometry_survives_a_stale_settings_save() {
        let mut saved = Settings::default();
        for (i, w) in WINDOWS.iter().enumerate() {
            *(w.pos)(&mut saved) = Some((i as i32 + 10, 20));
            *(w.size)(&mut saved) = Some((i as u32 + 300, 400));
        }
        let mut incoming = Settings::default(); // 기하가 비어 있는 낡은 스냅샷
        keep_geometry(&mut saved, &mut incoming);
        for (i, w) in WINDOWS.iter().enumerate() {
            assert_eq!(*(w.pos)(&mut incoming), Some((i as i32 + 10, 20)), "{}", w.label);
            assert_eq!(*(w.size)(&mut incoming), Some((i as u32 + 300, 400)), "{}", w.label);
        }
    }

    /// 슬롯 접근자가 서로 다른 필드를 가리키는지 — 복붙 실수로 두 창이 같은 슬롯을
    /// 쓰면 한쪽 크기를 바꿀 때 다른 쪽이 따라 움직인다.
    #[test]
    fn slots_are_distinct() {
        let mut s = Settings::default();
        for (i, w) in WINDOWS.iter().enumerate() {
            *(w.pos)(&mut s) = Some((i as i32, 0));
            *(w.size)(&mut s) = Some((i as u32 + 1, 1));
        }
        for (i, w) in WINDOWS.iter().enumerate() {
            assert_eq!(*(w.pos)(&mut s), Some((i as i32, 0)), "{}", w.label);
            assert_eq!(*(w.size)(&mut s), Some((i as u32 + 1, 1)), "{}", w.label);
        }
    }
}

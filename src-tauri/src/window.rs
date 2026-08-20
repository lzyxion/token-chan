//! 창 기하(위치·크기) 규칙 한 곳.
//!
//! 패널·설정·스튜디오는 셋 다 "저장된 자리로 옮기고, 그 화면에 맞는 크기를 준다" 라는
//! **같은 절차**를 따른다. 예전에는 그 절차가 세 벌로 복사돼 있었고 라벨→설정 필드
//! 매핑은 또 따로 세 군데(위치 저장·크기 저장·리사이즈 하한)에 있었다 — 창 하나를
//! 고치려면 여섯 곳을 뒤져야 했고, 실제로 기본 크기를 바꿀 때마다 한 곳씩 빠뜨렸다.
//!
//! 그래서 **라벨이 아는 모든 것**을 [`WINDOWS`] 표 한 줄로 모은다: 저장 슬롯, 리사이즈
//! 하한, 저장값이 없을 때의 크기와 자리. 창을 추가하는 일은 이제 표에 한 줄 넣는 것이다.
//!
//! ## 배율이 다른 화면 둘
//!
//! 창 크기를 **논리 px 로, 옮기기 전에** 정하면 창이 지금 떠 있는 화면의 배율로 물리
//! px 이 굳는다. 그 뒤 배율이 다른 화면(저장해 둔 자리)으로 옮겨도 그 크기는 따라오지
//! 않아 — 2번 화면 배율로 잰 창이 1번 화면에 뜬다. 반대로 물리 크기를 먼저 넣으면
//! 이번엔 Windows 가 화면을 넘는 순간 배율만큼 다시 늘려(WM_DPICHANGED) 방금 넣은
//! 저장값을 덮는다.
//!
//! 그래서 여기서는 언제나 **① 갈 자리를 정하고 ② 그 자리가 놓인 화면의 배율로 크기를
//! 환산하고 ③ 자리 → 크기 순으로 적용**한다.

use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, WebviewWindow};

use crate::settings::{self, Settings};
use crate::AppState;

/// 저장된 창 위치 (물리 px) — 없으면 아직 사용자가 옮긴 적이 없다는 뜻
pub type SavedPos = Option<(i32, i32)>;
/// 저장된 창 크기 (물리 px) — 없으면 표의 `base` 를 그 화면 배율로 환산해 쓴다
pub type SavedSize = Option<(u32, u32)>;

/// 물리 px 사각형 `(x, y, w, h)` — 화면이든 창이든 겹침 계산은 같다.
type Rect = (i32, i32, i32, i32);

/// 저장된 위치가 없을 때(첫 실행·모니터 해제 등) 어디에 놓을지.
#[derive(Clone, Copy)]
pub enum Place {
    /// 우하단에서 (dx, dy) 만큼 안쪽.
    ///
    /// `follow_pet` 이면 **펫이 떠 있는 화면**을 기준으로 한다 — 패널은 펫 옆에
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
    /// 저장된 크기가 없을 때의 크기 (논리 px) — `tauri.conf.json` 의 같은 창 값이다
    /// (`table_matches_conf` 테스트가 어긋남을 잡는다). 창이 **처음 뜬 물리 크기**는
    /// 그 화면 배율에 물들어 있어 못 쓴다 → 갈 화면 배율로 다시 환산할 원본이 필요하다.
    pub base: (f64, f64),
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
        base: (280.0, 400.0),
        place: Place::Corner { dx: 24, dy: 96, follow_pet: true },
        pos: |s| &mut s.panel_pos,
        size: |s| &mut s.panel_size,
    },
    WinSpec {
        label: "settings",
        // 본문이 스크롤되므로 헤더·탭이 남을 만큼만 막는다
        min: (280.0, 240.0),
        base: (280.0, 560.0),
        place: Place::Corner { dx: 16, dy: 80, follow_pet: false },
        pos: |s| &mut s.settings_pos,
        size: |s| &mut s.settings_size,
    },
    WinSpec {
        label: "studio",
        // 2열 레이아웃이라 이보다 좁으면 우측 열이 못 산다
        min: (460.0, 340.0),
        base: (680.0, 560.0),
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

/// 저장된 자리·크기를 창에 되돌린다. 표시(`show`)는 하지 않는다 — 호출부마다 그 뒤에
/// 할 일이 다르다(패널은 always-on-top 재적용, 설정은 탭 이벤트).
pub fn restore(app: &AppHandle, sp: &WinSpec, win: &WebviewWindow) {
    let (saved_pos, saved_size) = sp.saved(app);
    // 지금 크기는 창이 처음 뜬 화면 배율에 물들어 있다 → "화면에 걸치나" 판정에만 쓴다
    let cur = outer_size(win);
    // 옮겨 둔 자리가 있으면 그대로 — 창이 뒤로 갔다가 다시 불려 올 때마다 기본 자리로
    // 튀면 옮겨 둔 의미가 없다. 화면 밖을 가리키는 옛 좌표만 걸러 낸다.
    let kept = saved_pos.filter(|&(x, y)| on_screen(win, (x, y, cur.0, cur.1)));
    let (x, y, (w, h)) = match kept {
        // 저장된 자리 → 저장된 크기 그대로(이미 그 화면에서 잰 물리 px). 크기를 아직
        // 저장한 적이 없으면 그 화면 배율로 기본 크기를 환산한다.
        Some((x, y)) => {
            let sf = scale_factor_at(win, (x, y, cur.0, cur.1));
            (x, y, saved_size.unwrap_or_else(|| scaled(sp.base, sf)))
        }
        // 첫 실행이거나 그 화면이 사라졌다 → 기준 화면의 기본 자리. 자리 계산이 실제
        // 크기를 쓰므로(우하단·중앙 정렬) 크기를 먼저 정한다.
        None => {
            let Some((screen, sf)) = default_screen(app, sp, win) else {
                return;
            };
            let size = saved_size.unwrap_or_else(|| scaled(sp.base, sf));
            let (x, y) = place_in(screen, sp.place, size);
            (x, y, size)
        }
    };
    // 자리 → 크기 순서 (모듈 주석 "배율이 다른 화면 둘" 참고)
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.set_size(tauri::PhysicalSize::new(w, h));
}

/// 시작할 때 펫을 저장해 둔 자리에 앉힌다.
///
/// 펫은 [`WINDOWS`] 표에 없다 — 크기를 사용자가 정하지 않기 때문이다(배율 설정 ×
/// 콘텐츠 실측). 하지만 "갈 자리를 먼저, 크기는 그 화면 배율로" 라는 규칙은 똑같이
/// 필요하므로 계산은 여기 둔다.
pub fn restore_pet(pet: &WebviewWindow, saved: SavedPos, scale: f64) {
    let cur = outer_size(pet);
    // 화면 밖으로 걸쳐 두는 건 의도된 배치일 수 있으므로 되돌리지 않는다. 다만 모니터
    // 구성이 바뀌어 어느 화면에도 안 걸리면 창을 영영 못 찾으므로, 그때만 되돌린다.
    let kept = saved.filter(|&(x, y)| on_screen(pet, (x, y, cur.0, cur.1)));
    let base = (settings::PET_BASE_W * scale, settings::PET_BASE_H * scale);
    let (x, y, (w, h)) = match kept {
        Some((x, y)) => (x, y, scaled(base, scale_factor_at(pet, (x, y, cur.0, cur.1)))),
        None => {
            // 자리를 잃었으면(모니터 해제) 남은 화면 안쪽으로, 첫 실행이면 주 화면으로
            let lost = saved.is_some();
            let Some(((sx, sy, sw, sh), sf)) = pet_screen(pet, lost) else {
                return;
            };
            let size = scaled(base, sf);
            let (w, h) = (size.0 as i32, size.1 as i32);
            let (x, y) = if lost {
                (sx + 40, sy + 40)
            } else {
                // 오른쪽 끝에 붙이고 바닥에서 캐릭터 키만큼 띄운다
                (sx + sw - w, sy + sh - 2 * h)
            };
            (x, y, size)
        }
    };
    let _ = pet.set_position(PhysicalPosition::new(x, y));
    let _ = pet.set_size(tauri::PhysicalSize::new(w, h));
}

/// 논리 크기 → 그 화면의 물리 px. 0은 만들지 않는다 — 창이 사라져 보인다.
pub fn scaled((w, h): (f64, f64), sf: f64) -> (u32, u32) {
    (
        (w * sf).round().max(1.0) as u32,
        (h * sf).round().max(1.0) as u32,
    )
}

fn outer_size(win: &WebviewWindow) -> (i32, i32) {
    win.outer_size()
        .map_or((0, 0), |s| (s.width as i32, s.height as i32))
}

fn as_screen(m: &Monitor) -> (Rect, f64) {
    let (p, s) = (m.position(), m.size());
    ((p.x, p.y, s.width as i32, s.height as i32), m.scale_factor())
}

/// 지금 붙어 있는 화면들 — 못 읽으면 `None` (호출부가 "판정 불가"로 갈라진다).
fn screens(win: &WebviewWindow) -> Option<Vec<(Rect, f64)>> {
    Some(win.available_monitors().ok()?.iter().map(as_screen).collect())
}

/// 창 사각형이 **가장 많이** 걸치는 화면. 어디에도 안 걸리면 `None`.
/// 걸치기만 하면 되는 게 아니라 넓이로 고르는 이유: 경계에 걸친 창은 몸통이 놓인
/// 쪽 배율을 따라야 눈에 보이는 크기가 맞는다.
fn best_screen(screens: &[(Rect, f64)], win: Rect) -> Option<usize> {
    let (wx, wy, ww, wh) = win;
    screens
        .iter()
        .enumerate()
        .filter_map(|(i, &((sx, sy, sw, sh), _))| {
            let ox = (wx + ww).min(sx + sw) - wx.max(sx);
            let oy = (wy + wh).min(sy + sh) - wy.max(sy);
            (ox > 0 && oy > 0).then_some((ox as i64 * oy as i64, i))
        })
        .max_by_key(|&(area, _)| area)
        .map(|(_, i)| i)
}

/// 저장된 사각형이 지금 화면 어딘가에 걸치는지. 멀티 모니터를 해제하면 예전 좌표가
/// 화면 밖을 가리켜 창을 영영 못 찾으므로, 복원 전에 이걸로 걸러 기본 배치로 돌린다.
/// 일부만 걸쳐 있는 건 의도된 배치일 수 있어 통과시킨다.
fn on_screen(win: &WebviewWindow, rect: Rect) -> bool {
    // 판정 자체가 안 되면 저장값을 믿는 쪽이 덜 놀랍다
    let Some(screens) = screens(win) else {
        return true;
    };
    best_screen(&screens, rect).is_some()
}

/// 창이 **지금 실제로 놓인** 화면의 배율.
///
/// `WebviewWindow::scale_factor()` 를 쓰지 않는 이유: 그건 창을 만들 때 정해지고
/// Windows 가 배율 변경(WM_DPICHANGED)을 알려 줄 때만 갱신되는 캐시다. 프로그램이
/// 창을 다른 배율의 화면으로 옮겼을 때 그 알림이 없으면 옛 배율이 그대로 남아, 논리
/// 크기를 물리 px 로 바꿀 때마다 계속 틀린 값을 낸다. 화면 목록에서 직접 고르면
/// 알림에 기대지 않는다.
pub fn scale_factor(win: &WebviewWindow) -> f64 {
    let (x, y) = win.outer_position().map_or((0, 0), |p| (p.x, p.y));
    let (w, h) = outer_size(win);
    scale_factor_at(win, (x, y, w, h))
}

/// 그 사각형이 놓일 화면의 배율. 창을 옮기기 **전에** 크기를 정할 때 쓴다 —
/// 창이 지금 있는 화면의 배율로 환산하면 옮겨 간 화면에서 크기가 어긋난다.
fn scale_factor_at(win: &WebviewWindow, rect: Rect) -> f64 {
    screens(win)
        .and_then(|s| best_screen(&s, rect).map(|i| s[i].1))
        .or_else(|| win.scale_factor().ok())
        .unwrap_or(1.0)
}

/// 저장된 자리가 없을 때 기준이 되는 화면 ([`Place`] 의 설명 참고).
fn default_screen(app: &AppHandle, sp: &WinSpec, win: &WebviewWindow) -> Option<(Rect, f64)> {
    let follow_pet = matches!(sp.place, Place::Corner { follow_pet: true, .. });
    let mon = follow_pet
        .then(|| {
            app.get_webview_window("pet")
                .and_then(|p| p.current_monitor().ok().flatten())
        })
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten())?;
    Some(as_screen(&mon))
}

/// 펫의 기준 화면 — 자리를 잃었으면(`lost`) 남은 화면 아무거나, 첫 실행이면 주 화면.
fn pet_screen(pet: &WebviewWindow, lost: bool) -> Option<(Rect, f64)> {
    let mon = if lost {
        pet.available_monitors().ok()?.into_iter().next()
    } else {
        pet.primary_monitor().ok().flatten()
    }?;
    Some(as_screen(&mon))
}

/// 화면 안에서의 기본 자리 (물리 px).
fn place_in((sx, sy, sw, sh): Rect, place: Place, size: (u32, u32)) -> (i32, i32) {
    let (w, h) = (size.0 as i32, size.1 as i32);
    match place {
        Place::Center => (sx + (sw - w) / 2, sy + (sh - h) / 2),
        Place::Corner { dx, dy, .. } => (sx + sw - w - dx, sy + sh - h - dy),
    }
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

    /// 코드가 아는 기본 크기는 `tauri.conf.json` 의 창 크기와 같아야 한다 — 어긋나면
    /// 창이 첫 표시 때 conf 크기에서 코드 크기로 한 번 튄다. 펫·말풍선은 표에 없지만
    /// (크기를 저장하지 않는다) 같은 값을 상수로 들고 있으므로 함께 지킨다.
    #[test]
    fn table_matches_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let wins = conf["app"]["windows"].as_array().unwrap();
        let size_of = |label: &str| {
            let c = wins
                .iter()
                .find(|c| c["label"] == label)
                .unwrap_or_else(|| panic!("{label} 창이 tauri.conf.json 에 없다"));
            (c["width"].as_f64().unwrap(), c["height"].as_f64().unwrap())
        };
        for w in WINDOWS {
            assert_eq!(size_of(w.label), w.base, "{}", w.label);
        }
        assert_eq!(size_of("pet"), (settings::PET_BASE_W, settings::PET_BASE_H));
        assert_eq!(
            size_of("bubble"),
            (settings::BUBBLE_BASE_W, settings::BUBBLE_BASE_H)
        );
    }

    /// 두 화면이 나란히 있을 때 창은 **몸통이 놓인** 화면을 따른다.
    /// (경계에 조금 걸쳤다고 반대편 배율을 쓰면 크기가 눈에 띄게 튄다)
    #[test]
    fn window_belongs_to_the_screen_it_sits_on() {
        // 왼쪽: 175% 노트북, 오른쪽: 100% 외부 모니터
        let screens = [((0, 0, 2880, 1800), 1.75), ((2880, 0, 1920, 1080), 1.0)];
        assert_eq!(best_screen(&screens, (100, 100, 400, 300)), Some(0));
        assert_eq!(best_screen(&screens, (3000, 100, 400, 300)), Some(1));
        // 경계에 걸친 창 — 오른쪽에 300px, 왼쪽에 100px 이면 오른쪽
        assert_eq!(best_screen(&screens, (2780, 100, 400, 300)), Some(1));
        // 어느 화면에도 안 걸치면 없음 (모니터를 뺐다)
        assert_eq!(best_screen(&screens, (-900, 100, 400, 300)), None);
    }

    /// 배율이 다른 화면으로 가면 같은 논리 크기가 다른 물리 px 이 된다 —
    /// 이 환산이 빠지면 100% 화면에서 잰 크기가 175% 화면에 그대로 뜬다.
    #[test]
    fn logical_size_follows_the_screen_scale() {
        assert_eq!(scaled((220.0, 140.0), 1.0), (220, 140));
        assert_eq!(scaled((220.0, 140.0), 1.75), (385, 245));
        // 아무리 작아도 0은 안 된다
        assert_eq!(scaled((0.0, 0.0), 1.0), (1, 1));
    }

    /// 기본 자리는 화면 원점을 더해야 한다 — 빼먹으면 두 번째 화면에 열어도
    /// 창이 첫 화면 구석으로 간다.
    #[test]
    fn default_place_is_relative_to_its_screen() {
        let screen = (2880, 0, 1920, 1080);
        let corner = Place::Corner { dx: 24, dy: 96, follow_pet: false };
        assert_eq!(
            place_in(screen, corner, (280, 400)),
            (2880 + 1920 - 280 - 24, 1080 - 400 - 96)
        );
        assert_eq!(
            place_in(screen, Place::Center, (680, 560)),
            (2880 + (1920 - 680) / 2, (1080 - 560) / 2)
        );
    }
}

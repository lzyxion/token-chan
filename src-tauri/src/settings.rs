//! 앱 설정 (JSON 파일 영속화). 스캔 상태는 저장하지 않는다 — 설정/창 위치만.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 모델 → 캐릭터 팩 매핑 규칙.
/// `prefixes`: 콤마 구분 접두사 목록 (예: "gpt, o3, codex"), 최장 접두사 매칭이 우선.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CharacterRule {
    pub prefixes: String,
    pub pack: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// 펫 창 위치 (물리 픽셀)
    pub pet_pos: Option<(i32, i32)>,
    /// 사용량 보존/스캔 기간 (일)
    pub retention_days: u32,
    /// 5시간 블록 소진율 경고 임계값 (0..1)
    pub alert_threshold: f64,
    /// 단가 오버라이드 JSON 경로
    pub price_override_path: Option<String>,
    /// 로그인 시 자동 시작
    pub autostart: bool,
    /// 캐릭터 크기 배율 (0.5 ~ 2.5)
    pub pet_scale: f64,
    /// 주간 한도 경고 임계값 (0..1) — 공식 주간 % 기준
    pub weekly_alert_threshold: f64,
    /// 블록 리셋 임박 알림 (분 전, 0 = 끔)
    pub reset_notify_minutes: u32,
    /// 클릭 통과 모드 — 펫이 마우스에 전혀 반응하지 않음 (트레이에서만 해제)
    pub click_through: bool,
    /// 말풍선 숨김 지연 (ms)
    pub hover_delay_ms: u64,
    /// 시작 시 펫 숨김 (트레이로만 시작)
    pub start_hidden: bool,
    /// 선택된 캐릭터 팩 이름 (None = 기본 CSS 고양이)
    pub character_pack: Option<String>,
    /// 잠자기 진입 대기 시간 (분) — 마지막 활동 후 이 시간이 지나면 sleep 상태
    pub sleep_after_minutes: u32,
    /// 모델별 캐릭터 규칙 (최장 접두사 매칭, 미매칭 시 character_pack 폴백)
    pub character_rules: Vec<CharacterRule>,
    /// 비활성화한 펫 상태 목록 (working/alert/sleep/exhausted/refreshed)
    pub disabled_states: Vec<String>,
    /// 발밑 미니 라벨 (소진율·남은시간) 상시 표시
    pub show_mini_label: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pet_pos: None,
            retention_days: 90,
            alert_threshold: 0.8,
            price_override_path: None,
            autostart: false,
            pet_scale: 1.0,
            weekly_alert_threshold: 0.9,
            reset_notify_minutes: 15,
            click_through: false,
            hover_delay_ms: 250,
            start_hidden: false,
            character_pack: None,
            sleep_after_minutes: 30,
            character_rules: vec![],
            disabled_states: vec![],
            show_mini_label: false,
        }
    }
}

/// 사용자 캐릭터 팩 루트 (`<config>/token-pet/characters/<팩이름>/idle.gif ...`)
pub fn characters_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("token-pet").join("characters"))
}

/// 펫 창 기본 크기 (논리 px) — 스케일 적용의 기준
pub const PET_BASE_W: f64 = 150.0;
pub const PET_BASE_H: f64 = 160.0;

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("token-pet").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = config_path() else { return Settings::default() };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let Some(path) = config_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

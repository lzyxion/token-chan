//! 앱 설정 (JSON 파일 영속화). 스캔 상태는 저장하지 않는다 — 설정/창 위치만.

use std::path::{Path, PathBuf};

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
    /// 컨텍스트 경고 임계값 (0..1) — 활성 벤더의 컨텍스트 사용률 기준 (compact 임박)
    pub context_alert_threshold: f64,
    /// 블록 리셋 임박 대사 (분 전, 0 = 끔) — OS 알림이 아니라 말풍선으로 나간다
    pub reset_notify_minutes: u32,
    /// 클릭 통과 모드 — 펫이 마우스에 전혀 반응하지 않음 (트레이에서만 해제)
    pub click_through: bool,
    /// 사용량 패널 창 위치 (물리 픽셀) — 사용자가 옮긴 자리를 기억
    pub panel_pos: Option<(i32, i32)>,
    /// 사용량 패널 창 크기 (물리 픽셀) — 사용자가 조절한 크기를 기억
    pub panel_size: Option<(u32, u32)>,
    /// 설정 창 크기 (물리 픽셀) — 사용자가 조절한 크기를 기억
    pub settings_size: Option<(u32, u32)>,
    /// 설정 창 위치 (물리 픽셀). **최상단이 아니라서** 자리를 기억해야 한다 —
    /// 창이 뒤로 갔다가 다시 불렀을 때 매번 우하단으로 튀면 옮겨 둔 의미가 없다.
    pub settings_pos: Option<(i32, i32)>,
    /// 캐릭터 스튜디오 창 크기 (물리 픽셀)
    pub studio_size: Option<(u32, u32)>,
    /// 캐릭터 스튜디오 창 위치 (물리 픽셀)
    pub studio_pos: Option<(i32, i32)>,
    /// 상황별 대사 말풍선 사용
    pub speech_enabled: bool,
    /// 대사 말풍선 표시 시간 (ms)
    pub speech_duration_ms: u64,
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
    /// 게이지 라벨(벤더·수치·리셋) 상시 표시 — 끄면 호버할 때만 펼쳐진다.
    /// 예전 "발밑 미니 라벨" 설정을 전환한 것이라 옛 키를 alias 로 읽는다.
    #[serde(alias = "showMiniLabel")]
    pub gauge_labels: bool,
    /// 도넛 게이지 위치 — "right" | "left" | "off"
    pub gauge_side: String,
    /// 상황별 사용자 문구 — 키("enter.working"·"poke"·"resetNotify" 등) → 문구 목록.
    /// 비어 있으면 내장 기본 문구. `{변수}` 는 표시 시점에 값으로 치환된다.
    /// 캐릭터별 말투는 여기가 아니라 팩 폴더의 `speech.json` 에 있다 (`pack_speech`).
    pub speech_lines: std::collections::HashMap<String, Vec<String>>,
    /// 추가로 스캔할 Claude 홈 (`.claude` 디렉토리). 그 아래 `projects`/`sessions` 를 본다.
    ///
    /// 자동 탐지는 프로세스 환경에 의존한다 — 트레이에서 뜬 앱은 터미널의 환경변수를
    /// 물려받지 못한다. 여기 적어 두면 실행 방식과 무관하게 항상 같은 범위를 본다.
    /// 자동 탐지분과 겹쳐도 어댑터의 이벤트 dedup 이 중복 집계를 막는다.
    pub extra_claude_homes: Vec<String>,
    /// 추가로 스캔할 Codex 홈. 그 아래 `sessions`/`archived_sessions`.
    /// 홈을 재배치했는데 마커 스캔이 닿지 않는 곳이면 여기로 등록한다 (`CODEX_HOME` 은 안 본다).
    pub extra_codex_homes: Vec<String>,
    /// 추가로 스캔할 Antigravity CLI(`agy`) 홈 (`antigravity-cli` 디렉토리).
    /// 그 아래 `conversations/<uuid>.db` 를 본다.
    pub extra_antigravity_homes: Vec<String>,
    /// 비용 표기 통화: `usd` | `krw`.
    ///
    /// 기본값이 `usd` 인 이유는 **그게 우리가 아는 값**이라서다 — 단가표(`prices.json`)가
    /// 달러이고, 원 표기는 아래 환율을 곱한 결과다. 곱하지 않은 쪽을 기본으로 두면
    /// 사용자가 환율을 확인하고 켜는 셈이 된다.
    pub currency: String,
    /// 1 USD = ? 원. **직접 넣는 값이다** — 이 앱은 네트워크를 쓰지 않는다(HTTP 의존성
    /// 자체가 없다). 자동 조회를 넣으면 실행 환경에 좌우되는 값이 하나 늘고, 방화벽
    /// 뒤에서 조용히 낡은 값을 쓰게 된다. 어차피 `costUSD` 자체가 단가표 추정치라
    /// 환율의 소수점 정밀도가 결과를 바꾸지 않는다.
    pub usd_to_krw: f64,
    /// 게이지에 태울 벤더: `auto` | `claude` | `codex` | `antigravity`.
    ///
    /// 게이지는 링 3개뿐이라 벤더 하나만 보여준다. `auto` 는 지금 작업 중인 쪽을
    /// 따라가지만, 한 벤더만 지켜보고 싶은 경우를 위해 고정할 수 있게 둔다.
    pub gauge_vendor: String,
    /// 계정별 집계 포함 여부의 **사용자 지정값**만 담는다 (`Account::setting_key()` → on/off).
    ///
    /// 여기 없는 계정은 기본값을 따른다: 표준 위치(홈·WSL 게스트·직접 추가)에서 발견된
    /// 계정은 켜고, 마커 스캔으로만 나온 처음 보는 계정은 꺼둔다. 오래된 백업이나 남의
    /// 계정이 조용히 합산되는 게 가장 나쁜 실패 모드라서다. 같은 계정의 새 설치본은
    /// 이미 켜진 계정에 합류하므로 자동으로 포함된다.
    pub accounts_enabled: std::collections::HashMap<String, bool>,
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
            context_alert_threshold: 0.9,
            reset_notify_minutes: 15,
            click_through: false,
            panel_pos: None,
            panel_size: None,
            settings_size: None,
            settings_pos: None,
            studio_size: None,
            studio_pos: None,
            speech_enabled: true,
            speech_duration_ms: 4000,
            start_hidden: false,
            character_pack: None,
            sleep_after_minutes: 30,
            character_rules: vec![],
            disabled_states: vec![],
            gauge_labels: false,
            gauge_side: "right".into(),
            speech_lines: Default::default(),
            extra_claude_homes: vec![],
            extra_codex_homes: vec![],
            extra_antigravity_homes: vec![],
            currency: "usd".into(),
            // 고정 기본값이라 시간이 지나면 낡는다 — 설정에서 고치라고 화면에 적어 둔다
            usd_to_krw: 1400.0,
            gauge_vendor: "auto".into(),
            accounts_enabled: Default::default(),
        }
    }
}

/// 사용자 캐릭터 팩 루트 (`<config>/token-chan/characters/<팩이름>/idle.gif ...`)
pub fn characters_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("token-chan").join("characters"))
}

/// 팩 이름이 경로로 오용되지 못하게 막는다 (디렉토리 목록에서 온 이름만 유효).
fn valid_pack_name(pack: &str) -> bool {
    !pack.is_empty() && !pack.contains(['/', '\\']) && pack != ".." && pack != "."
}

/// 팩 폴더 경로 (이름 검증 포함)
pub fn pack_dir(pack: &str) -> Option<PathBuf> {
    if !valid_pack_name(pack) {
        return None;
    }
    characters_dir().map(|d| d.join(pack))
}

/// 팩 폴더의 대사 파일 경로 — 대사는 캐릭터의 속성이라 이미지와 같은 폴더에서 관리한다
pub fn pack_speech_path(pack: &str) -> Option<PathBuf> {
    pack_dir(pack).map(|d| d.join("speech.json"))
}

/// 팩의 `speech.json` (상황 키 → 문구 목록). 없거나 못 읽으면 None — 기본 문구로 폴백.
pub fn load_pack_speech(
    pack: &str,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    let path = pack_speech_path(pack)?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 팩별 동작 설정 (`characters/<팩>/pack.json`) — 지금은 상태 사용 여부만.
/// 캐릭터마다 쓸 수 있는 상태(이미지·연출)가 달라서 팩의 속성으로 둔다.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PackConfig {
    /// 끈 상태 목록 (working/alert/…) — 꺼진 상태는 idle 로 폴백
    pub disabled_states: Vec<String>,
}

pub fn pack_config_path(pack: &str) -> Option<PathBuf> {
    pack_dir(pack).map(|d| d.join("pack.json"))
}

/// 팩 설정. 파일이 없으면 기본값(모든 상태 사용) — 전역 설정을 상속하지 않는다.
/// 캐릭터가 자기 설정을 온전히 들고 다녀야 폴더 공유가 자기완결이 된다.
pub fn load_pack_config(pack: &str) -> PackConfig {
    pack_config_path(pack)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 펫 창 **초기** 크기 (논리 px). 웹뷰가 뜨는 즉시 실측 크기로 다시 맞추므로
/// (`fit_pet_window`) 대략적인 값이면 된다 — 첫 프레임에 잘려 보이지만 않으면 충분.
pub const PET_BASE_W: f64 = 220.0;
pub const PET_BASE_H: f64 = 140.0;

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("token-chan").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = config_path() else { return Settings::default() };
    load_from(&path).unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let Some(path) = config_path() else { return };
    let _ = save_to(&path, settings);
}

/// 못 읽거나 깨졌으면 `None`. **깨진 파일은 `.bad` 로 옮겨 둔다** —
/// 그대로 두면 다음 저장이 덮어써서 무엇이 있었는지 영영 알 수 없다.
fn load_from(path: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(s) => Some(s),
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            None
        }
    }
}

/// 설정을 **원자적으로** 저장한다 — 임시 파일에 다 쓰고 `rename` 으로 갈아끼운다.
///
/// `fs::write` 로 직접 쓰면 안 된다. 그건 대상 파일을 **먼저 비우고** 쓰기 때문에, 그 사이에
/// 앱이 종료되면 잘린 JSON 이 남는다. 그러면 [`load_from`] 이 파싱에 실패하고 앱은 조용히
/// 기본값으로 뜬다 — 펫 위치·캐릭터 규칙·계정 토글·추가 홈이 통째로 사라진다.
/// 저장 지점이 11군데라 마주칠 확률도 낮지 않다.
///
/// `rename` 은 같은 디렉토리 안이라 원자적이고, Windows 에서도 기존 파일을 대체한다.
/// 내용이 디스크에 닿은 뒤에 갈아끼우도록 `sync_all` 을 먼저 부른다.
fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // 고정 이름이라 중간에 죽어 남더라도 다음 저장이 덮어쓴다.
    // 저장은 항상 설정 뮤텍스를 쥔 채 일어나므로 서로 겹치지 않는다.
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "발밑 미니 라벨" 시절 키가 게이지 라벨 설정으로 넘어와야 한다
    #[test]
    fn old_show_mini_label_key_migrates() {
        let s: super::Settings = serde_json::from_str(r#"{"showMiniLabel": true}"#).unwrap();
        assert!(s.gauge_labels);
    }

    fn sample() -> Settings {
        Settings { retention_days: 42, ..Default::default() }
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        save_to(&path, &sample()).unwrap();

        assert_eq!(load_from(&path).unwrap().retention_days, 42, "없는 상위 폴더도 만든다");
    }

    /// 원자적 저장의 핵심 — 갈아끼운 뒤에는 임시 파일이 남지 않는다
    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save_to(&path, &sample()).unwrap();

        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["settings.json".to_string()]);
    }

    /// 죽은 저장이 남긴 임시 파일이 있어도 다음 저장이 정상 동작해야 한다
    #[test]
    fn stale_temp_file_does_not_block_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(path.with_extension("json.tmp"), "{ 잘린").unwrap();

        save_to(&path, &sample()).unwrap();
        assert_eq!(load_from(&path).unwrap().retention_days, 42);
    }

    /// 기존 파일이 있어도 통째로 대체된다 (Windows 의 rename 도 대체를 허용한다)
    #[test]
    fn replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save_to(&path, &Settings { retention_days: 7, ..Default::default() }).unwrap();
        save_to(&path, &sample()).unwrap();

        assert_eq!(load_from(&path).unwrap().retention_days, 42);
    }

    /// 깨진 파일은 기본값으로 떨어지되 **덮어쓰지 않고** `.bad` 로 남긴다.
    /// 그대로 두면 다음 저장이 지워 버려 원인을 못 찾는다.
    #[test]
    fn corrupt_file_is_kept_as_bad() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ 저장 중 잘린 JSON").unwrap();

        assert!(load_from(&path).is_none());
        assert!(!path.exists(), "깨진 파일은 자리를 비워 준다");
        assert!(path.with_extension("json.bad").exists(), "내용은 .bad 로 보존된다");
    }
}

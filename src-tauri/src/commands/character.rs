//! 캐릭터 팩 — 폴더·이미지·팩 설정·대사 파일.

use tauri::{AppHandle, Manager, State};

use crate::settings;
use crate::AppState;

use super::config::save_settings;

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

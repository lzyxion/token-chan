//! 계정과 스캔 홈 — 켜고 끄기, 추가 경로, 다시 검색.

use tauri::{AppHandle, Manager, State};

use crate::settings::Settings;
use crate::AppState;

use super::config::save_settings;

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
    // 계정 목록을 먼저 복사한다 — settings 와 accounts 두 락을 겹쳐 잡지 않는다
    let accounts = { app.state::<AppState>().accounts.lock().unwrap().clone() };
    let updated = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.accounts_enabled.insert(setting_key, enabled);
        // 게이지를 그 벤더에 고정해 뒀는데 계정이 다 꺼졌으면 되돌린다. 그대로 두면
        // 게이지가 빈 링만 그리는데 화면 어디에도 이유가 없고, 설정 창의 선택 목록과
        // 저장된 값도 어긋난다. 설정 창이 닫혀 있어도 성립해야 해서 여기서 한다.
        if !gauge_vendor_is_live(&s.gauge_vendor, &accounts, &s.accounts_enabled) {
            s.gauge_vendor = "auto".into();
        }
        save_settings(&app, &s);
        s.clone()
    };
    crate::tray::refresh_menu(&app);
    use tauri::Emitter;
    let _ = app.emit("settings-changed", &updated);
    // 계정 목록 자체는 안 바뀌었지만 체크 상태가 바뀌었으므로 탭도 다시 그린다
    let _ = app.emit("accounts-changed", ());
}

/// 고정해 둔 게이지 벤더에 **켜 둔 계정이 아직 있는지**. `"auto"` 는 고정이 아니라 항상 참.
/// 모르는 문자열도 참으로 둔다 — 판정이 안 되는 값을 이유로 사용자 설정을 지우지 않는다.
fn gauge_vendor_is_live(
    pinned: &str,
    accounts: &[usage_core::accounts::Account],
    overrides: &std::collections::HashMap<String, bool>,
) -> bool {
    let Some(src) = usage_core::Source::ALL.into_iter().find(|s| s.id() == pinned) else {
        return true;
    };
    accounts
        .iter()
        .any(|a| a.source == src && crate::monitor::account_enabled(a, overrides))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use usage_core::accounts::Account;
    use usage_core::Source;

    fn account(source: Source, standard: bool) -> Account {
        Account {
            source,
            key: "k".into(),
            label: "a@b.c".into(),
            detail: String::new(),
            plan: String::new(),
            installs: vec![],
            standard,
        }
    }

    /// 고정한 벤더의 계정이 살아 있으면 그대로 둔다
    #[test]
    fn pinned_vendor_with_a_live_account_survives() {
        let accounts = [account(Source::Codex, true)];
        assert!(gauge_vendor_is_live("codex", &accounts, &HashMap::new()));
    }

    /// 사용자가 체크를 풀면(override=false) 고정이 풀려야 한다
    #[test]
    fn pinned_vendor_dies_when_its_only_account_is_switched_off() {
        let accounts = [account(Source::Codex, true)];
        let off = HashMap::from([(accounts[0].setting_key(), false)]);
        assert!(!gauge_vendor_is_live("codex", &accounts, &off));
    }

    /// 계정이 아예 없는 벤더에 고정된 경우도 풀린다
    #[test]
    fn pinned_vendor_with_no_account_at_all_dies() {
        let accounts = [account(Source::Claude, true)];
        assert!(!gauge_vendor_is_live("antigravity", &accounts, &HashMap::new()));
    }

    /// "auto" 와 모르는 문자열은 건드리지 않는다 — 판정이 안 되는 값을 이유로
    /// 사용자 설정을 지우면, 나중에 벤더가 늘었을 때 조용히 초기화된다
    #[test]
    fn auto_and_unknown_values_are_left_alone() {
        let accounts: [Account; 0] = [];
        assert!(gauge_vendor_is_live("auto", &accounts, &HashMap::new()));
        assert!(gauge_vendor_is_live("gemini", &accounts, &HashMap::new()));
    }
}

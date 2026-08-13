//! 설치본 발견 + 계정 식별.
//!
//! 같은 계정을 여러 곳에 설치해 두는 일이 흔하다 — Windows 와 WSL 에 각각 깔거나,
//! 다른 도구가 CLI 를 격리 실행하려고 홈을 자기 폴더로 돌리거나(`CODEX_HOME`).
//! 이때 사용량이 설치본별로 갈리는데, 앱은 **계정 단위**로 합쳐 보여줘야 정확하다.
//!
//! 그런데 재배치된 홈은 환경변수로만 지정되는 경우가 있고, 트레이에서 뜬 앱은 그
//! 변수를 물려받지 못한다. 그래서 표준 위치 탐색에 더해 **마커 파일 스캔**으로 찾는다
//! (실측: `%APPDATA%` 깊이 4 스캔이 0.5초, 재배치된 Codex 홈을 정확히 찾아냄).
//!
//! 계정 식별에 쓰는 파일:
//! - Claude: `<홈>/.claude.json` 의 `oauthAccount` — 일반 설정 파일이라 자격증명이 아니다
//! - Codex: `<홈>/auth.json` 의 `tokens.account_id` + `id_token`(JWT) 의 email 클레임
//! - Antigravity(agy): **없다.** 계정 파일을 아예 안 남겨서 `installation_id` 로
//!   **설치본 단위**로만 묶인다. `~/.gemini/oauth_creds.json` 과 `google_account_id` 는
//!   구 Gemini CLI 가 남긴 것이라(실측 mtime 2025-07-03, agy 실행에 안 건드려짐)
//!   agy 계정이라고 볼 근거가 없다 — 그 계정 id 는 agy DB 어디에도 없다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use crate::model::Source;

/// 마커 스캔 깊이. 4면 `%APPDATA%\<도구>\<런타임홈>\home` 형태까지 닿는다.
const MARKER_DEPTH: usize = 4;

/// 스캔에서 건너뛸 디렉토리 — 캐시/의존성은 크기만 크고 홈이 있을 리 없다.
/// `Caches` 는 macOS 표기 (`~/Library/Application Support/<앱>/Caches`).
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "Cache", "cache", "Caches", "Code Cache", "GPUCache",
    "Temp", "temp", "logs", "Crashpad", "Service Worker",
];

/// 사용자가 설정에 직접 적은 추가 스캔 홈들 (소스별).
/// 자동 탐지는 프로세스 환경에 좌우되므로 이걸로 보완한다.
#[derive(Clone, Debug, Default)]
pub struct ExtraHomes {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub antigravity: Vec<PathBuf>,
}

/// 한 설치본 = 홈 디렉토리 하나
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Install {
    pub source: Source,
    /// Claude 는 `.claude` 를 **담고 있는** 디렉토리, Codex 는 `.codex` 상당 디렉토리 자체,
    /// Antigravity 는 `antigravity-cli` 디렉토리 자체
    pub home: PathBuf,
    /// 표준 위치가 아니라 마커 스캔으로 찾아낸 것인지 (새 계정의 기본값 판단에 쓰인다)
    pub discovered: bool,
}

/// 같은 계정으로 묶인 설치본들
#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub source: Source,
    /// 계정 식별 키. Claude 는 `accountUuid`, Codex 는 `account_id`.
    /// 계정 파일을 못 읽으면 홈 경로를 키로 써서 다른 계정과 섞이지 않게 한다.
    pub key: String,
    /// 사람이 읽는 이름 — 이메일이 있으면 이메일, 없으면 계정 id 앞자리
    pub label: String,
    /// **로그인 방식** — 세 소스 모두 `{서비스} 로그인` 꼴 (예: "Claude 로그인", "ChatGPT 로그인")
    pub detail: String,
    /// **플랜 이름** (예: "Max 5x"). 계정 파일에서 알 수 있는 소스만 채운다 —
    /// Codex 는 비고, 화면은 rollout 에서 온 [`crate::plan::PlanUsage::detail`] 로 채운다.
    pub plan: String,
    pub installs: Vec<Install>,
    /// 표준 위치에서 한 번이라도 발견된 계정인지.
    /// 마커 스캔으로만 나온 처음 보는 계정은 조용히 합산되면 안 된다(오래된 백업 등).
    pub standard: bool,
}

impl Account {
    /// 이 계정에 속한 홈들 (집계 대상 루트를 만들 때 쓴다)
    pub fn homes(&self) -> Vec<PathBuf> {
        self.installs.iter().map(|i| i.home.clone()).collect()
    }

    /// 설정에 저장하는 키 — 소스가 달라도 계정 id 가 같을 수 있으므로 소스를 붙인다
    pub fn setting_key(&self) -> String {
        format!("{:?}:{}", self.source, self.key).to_lowercase()
    }
}

impl Install {
    /// 사용량 스캔 대상 루트
    pub fn transcript_root(&self) -> PathBuf {
        match self.source {
            Source::Claude => self.home.join(".claude").join("projects"),
            // Codex(sessions/·archived_sessions/) 와 Antigravity(conversations/) 는
            // 홈 자체가 루트다
            _ => self.home.clone(),
        }
    }

    /// Claude 라이브 세션 레지스트리 (다른 소스는 없음)
    pub fn session_dir(&self) -> Option<PathBuf> {
        (self.source == Source::Claude).then(|| self.home.join(".claude").join("sessions"))
    }
}

/// base64url 디코드 (JWT 페이로드용, 패딩 없음). 의존성을 늘리지 않으려고 직접 구현.
fn b64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// JWT 페이로드에서 email 클레임을 꺼낸다. 서명은 검증하지 않는다 —
/// 표시용 이름일 뿐이고 이 파일을 쓴 주체는 CLI 자신이다.
fn jwt_email(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let json: serde_json::Value = serde_json::from_slice(&b64url(payload)?).ok()?;
    for k in ["email", "preferred_username", "sub"] {
        if let Some(s) = json.get(k).and_then(|v| v.as_str()) {
            if s.contains('@') {
                return Some(s.to_string());
            }
        }
    }
    // 중첩된 클레임(예: profile.email)도 한 겹 본다
    json.as_object()?.values().find_map(|v| {
        v.as_object()?
            .get("email")?
            .as_str()
            .filter(|s| s.contains('@'))
            .map(String::from)
    })
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// 계정 파일에서 뽑아낸 신원. 소스마다 파일이 다르므로 어댑터별로 채운다.
///
/// `detail` 과 `plan` 을 나눠 두는 이유: 세 소스가 **같은 자리에 다른 것을** 준다.
/// Claude 는 계정 파일에 플랜이 있고 인증 방식은 없으며, Codex·agy 는 반대다. 한 필드에
/// 섞으면 카드마다 다른 의미가 같은 자리에 놓인다.
struct Ident {
    /// 계정 식별 키 (같은 계정의 설치본을 묶는 기준)
    key: String,
    /// 사람이 읽는 이름 — 보통 이메일
    label: String,
    /// **로그인 방식** — 세 소스 공통 의미
    detail: String,
    /// **플랜 이름** — 계정 파일에서 알 수 있는 소스만 채운다 (지금은 Claude 뿐).
    /// Codex 는 계정 파일에 없고 rollout 의 `rate_limits` 로 따로 온다.
    plan: String,
}

/// `<홈>/.claude.json` 에서 계정 식별. 자격증명 파일이 아니라 일반 설정 파일이다.
fn claude_account(home: &Path) -> Option<Ident> {
    let v = read_json(&home.join(".claude.json"))?;
    let oa = v.get("oauthAccount")?;
    let get = |k: &str| oa.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let uuid = get("accountUuid");
    let email = get("emailAddress");
    if uuid.is_empty() && email.is_empty() {
        return None;
    }
    let key = if uuid.is_empty() { email.clone() } else { uuid };
    let label = if email.is_empty() { short(&key) } else { email };
    // 티어가 플랜을 포함한다 (`claude_max` + `default_claude_max_5x`) — 둘을 이어 붙이면
    // 같은 말이 두 번 나오므로 더 자세한 쪽만 쓰고, 없을 때만 플랜으로 떨어진다.
    let tier = get("organizationRateLimitTier");
    let org = get("organizationType");
    let plan = match (tier.as_str(), org.as_str()) {
        ("", "") => String::new(),
        ("", o) => crate::plan::plan_label(o),
        (t, _) => crate::plan::plan_label(t),
    };
    // Codex 의 `auth_mode`, agy 의 `authMethod` 에 해당하는 필드가 **없다.** 대신 이
    // 객체의 이름이 `oauthAccount` 이고, API 키·Bedrock·Vertex 로 쓰면 이 객체 자체가
    // 없어 여기까지 오지도 않는다. 그래서 "여기 도달 = OAuth 로그인"으로 읽는다.
    Some(Ident { key, label, detail: "Claude 로그인".into(), plan })
}

/// `<홈>/auth.json` 에서 계정 식별.
fn codex_account(home: &Path) -> Option<Ident> {
    let v = read_json(&home.join("auth.json"))?;
    let tokens = v.get("tokens");
    let id = tokens
        .and_then(|t| t.get("account_id"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let mode = v.get("auth_mode").and_then(|x| x.as_str()).unwrap_or("");
    let email = tokens
        .and_then(|t| t.get("id_token"))
        .and_then(|x| x.as_str())
        .and_then(jwt_email);
    if id.is_empty() && email.is_none() && mode.is_empty() {
        return None;
    }
    let key = if id.is_empty() {
        email.clone().unwrap_or_else(|| home.display().to_string())
    } else {
        id.clone()
    };
    let label = email.unwrap_or_else(|| short(&key));
    let detail = match mode {
        "chatgpt" => "ChatGPT 로그인".to_string(),
        "" => String::new(),
        m => m.to_string(),
    };
    // 플랜은 계정 파일에 없다 — rollout 의 `rate_limits.plan_type` 으로 따로 온다
    Some(Ident { key, label, detail, plan: String::new() })
}

/// agy 는 **자격증명 파일을 안 쓴다** — 토큰은 OS 키링에 넣는다
/// (로그: `keyringAuth: loaded token` / `ChainedAuth: authenticated via keyring`).
/// 대신 로그인한 계정을 로그에 평문으로 남긴다:
///
/// ```text
/// server_oauth.go:189] applyAuthResult: email=you@example.com, authMethod=consumer, quotaProject=
/// ```
///
/// `authMethod` 뒤에 다른 필드가 더 붙으므로 값을 쉼표에서 한 번 더 잘라야 한다
/// (안 자르면 `consumer,` 가 되어 아래 매칭이 통째로 빗나간다).
/// 참고로 agy 는 셋 중 **유일하게 OAuth 를 로그에 명시**한다 —
/// `OAuth: authenticated successfully as …` · `Starting OAuth authentication flow`.
/// `authMethod=consumer` 는 프로토콜이 아니라 **계정 종류**(개인 vs 조직)를 뜻한다.
///
/// 실측에서 로그 6개 중 0바이트짜리 1개를 뺀 5개 전부에 있었다. 그래서 Claude·Codex 와
/// 마찬가지로 **이메일로 계정을 묶는다**. 로그를 하나도 못 읽으면 그때만
/// `installation_id` 로 떨어져 설치본 단위가 된다.
fn antigravity_account(home: &Path) -> Option<Ident> {
    if let Some((email, method)) = log_auth(home) {
        let detail = match method.as_str() {
            "consumer" => "Google 로그인".to_string(),
            "" => String::new(),
            m => m.to_string(),
        };
        // agy 는 플랜을 어디에도 안 남긴다 (한도 자체를 제공하지 않는다)
        return Some(Ident { key: email.clone(), label: email, detail, plan: String::new() });
    }
    // 로그가 없거나 로그인 기록이 아직 없는 설치본 — 다른 계정과 섞이지 않게 설치 id 로 분리
    let id = std::fs::read_to_string(home.join("installation_id")).ok()?;
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    Some(Ident {
        key: id.to_string(),
        label: format!("계정 미상 (설치 {})", short(id)),
        detail: "로그에 로그인 기록 없음".into(),
        plan: String::new(),
    })
}

/// agy 로그에서 가장 최근 로그인 기록 (email, authMethod).
/// 로그는 실행마다 새로 생기므로 **가장 최근 파일부터** 본다 — 계정을 바꿔 로그인하면
/// 옛 로그엔 옛 이메일이 남아 있다.
fn log_auth(home: &Path) -> Option<(String, String)> {
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = vec![];
    let mut push = |p: PathBuf| {
        if let Ok(m) = std::fs::metadata(&p) {
            if m.is_file() && m.len() > 0 {
                logs.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), p));
            }
        }
    };
    push(home.join("cli.log"));
    if let Ok(entries) = std::fs::read_dir(home.join("log")) {
        for e in entries.filter_map(|e| e.ok()) {
            push(e.path());
        }
    }
    logs.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, path) in logs {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        // 한 파일 안에서도 마지막 기록이 최신이다
        if let Some(found) = text.rmatch_indices("applyAuthResult: email=").next() {
            let rest = &text[found.0 + "applyAuthResult: email=".len()..];
            let line = rest.lines().next().unwrap_or("");
            let email = line.split(',').next().unwrap_or("").trim();
            if !email.contains('@') {
                continue;
            }
            let method = line
                .split("authMethod=")
                .nth(1)
                .map(|m| m.split(',').next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            return Some((email.to_string(), method));
        }
    }
    None
}

fn short(key: &str) -> String {
    key.chars().take(8).collect()
}

/// 표준 위치(홈·WSL 게스트)와 사용자가 설정에 적은 경로에서 나오는 설치본
fn standard_installs(extra: &ExtraHomes) -> Vec<Install> {
    let mut out = vec![];
    for root in crate::roots::claude_project_roots_with(&extra.claude) {
        // root = `<홈>/.claude/projects` → 계정 파일은 `<홈>/.claude.json`
        if let Some(home) = root.parent().and_then(|p| p.parent()) {
            out.push(Install { source: Source::Claude, home: home.to_path_buf(), discovered: false });
        }
    }
    for home in crate::roots::codex_homes_with(&extra.codex) {
        out.push(Install { source: Source::Codex, home, discovered: false });
    }
    for home in crate::roots::antigravity_homes_with(&extra.antigravity) {
        out.push(Install { source: Source::Antigravity, home, discovered: false });
    }
    out
}

/// 마커 스캔을 시작할 디렉토리들 — 재배치된 홈이 놓일 만한 곳만 본다
fn scan_bases() -> Vec<PathBuf> {
    let mut bases = vec![];
    for var in ["APPDATA", "LOCALAPPDATA", "XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
        if let Some(v) = std::env::var_os(var) {
            bases.push(PathBuf::from(v));
        }
    }
    if let Some(h) = crate::roots::home_dir() {
        bases.push(h.join(".local").join("share"));
        bases.push(h.join(".config"));
        // macOS 앱이 데이터를 두는 표준 위치. 이게 빠져 있어서 macOS 에서는 재배치된 홈을
        // 찾을 방법이 아예 없었다 — 환경변수를 안 보는 데다(roots::codex_homes) GUI 앱은
        // 셸 환경변수를 상속하지도 않으므로, 마커 스캔이 유일한 자동 경로다.
        bases.push(h.join("Library").join("Application Support"));
    }
    // 없는 경로는 여기서 걸러지므로 OS 별로 나눌 필요가 없다
    bases.retain(|p| p.is_dir());
    bases
}

/// 환경변수로만 지정돼 앱이 알 수 없는 재배치 홈을 마커 파일로 찾아낸다.
pub fn marker_scan() -> Vec<Install> {
    let mut out = vec![];
    for base in scan_bases() {
        let walker = WalkDir::new(&base)
            .max_depth(MARKER_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.depth() == 0
                    || !e.file_name().to_str().map(|n| SKIP_DIRS.contains(&n)).unwrap_or(false)
            });
        for e in walker.filter_map(|e| e.ok()) {
            if !e.file_type().is_dir() {
                continue;
            }
            let p = e.path();
            if p.join("auth.json").is_file() && p.join("sessions").is_dir() {
                out.push(Install { source: Source::Codex, home: p.to_path_buf(), discovered: true });
            }
            if p.join(".claude").join("projects").is_dir() {
                out.push(Install { source: Source::Claude, home: p.to_path_buf(), discovered: true });
            }
            // agy 홈은 `conversations/` + `installation_id` 조합으로 알아본다
            if p.join("conversations").is_dir() && p.join("installation_id").is_file() {
                out.push(Install { source: Source::Antigravity, home: p.to_path_buf(), discovered: true });
            }
        }
    }
    out
}

/// 발견된 설치본을 계정으로 묶는다. `scan` 이 false 면 마커 스캔을 건너뛴다(빠른 경로).
pub fn discover(extra: &ExtraHomes, scan: bool) -> Vec<Account> {
    let mut installs = standard_installs(extra);
    if scan {
        installs.extend(marker_scan());
    }
    group(installs)
}

/// 설치본 목록 → 계정 묶음. 발견 경로와 분리해 두어야 테스트가 실제 머신 상태에
/// 영향받지 않는다.
pub fn group(mut installs: Vec<Install>) -> Vec<Account> {
    // 같은 홈이 두 경로로 들어오는 경우 제거 (표준 탐색분을 우선 — discovered=false 유지)
    let mut seen = std::collections::HashSet::new();
    installs.retain(|i| {
        let key = (
            i.source,
            std::fs::canonicalize(&i.home).unwrap_or_else(|_| i.home.clone()),
        );
        seen.insert(key)
    });

    let mut groups: BTreeMap<(Source, String), Account> = BTreeMap::new();
    for inst in installs {
        let ident = match inst.source {
            Source::Claude => claude_account(&inst.home),
            Source::Codex => codex_account(&inst.home),
            Source::Antigravity => antigravity_account(&inst.home),
        };
        // 계정을 못 읽으면 홈 경로를 키로 — 다른 계정과 합쳐지는 것보다 낫다
        let id = ident.unwrap_or_else(|| {
            let h = inst.home.display().to_string();
            Ident {
                key: h.clone(),
                label: h,
                detail: "계정 정보 없음".into(),
                plan: String::new(),
            }
        });

        let entry = groups.entry((inst.source, id.key.clone())).or_insert_with(|| Account {
            source: inst.source,
            key: id.key,
            label: id.label,
            detail: id.detail,
            plan: id.plan,
            installs: vec![],
            standard: false,
        });
        entry.standard |= !inst.discovered;
        entry.installs.push(inst);
    }
    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 실제 auth.json 과 같은 모양의 JWT (서명 자리는 아무 값)
    fn fake_jwt(email: &str) -> String {
        fn enc(s: &str) -> String {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let b = s.as_bytes();
            let mut out = String::new();
            for c in b.chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | (*c.get(2).unwrap_or(&0) as u32);
                let take = c.len() + 1;
                for i in 0..take {
                    out.push(T[((n >> (18 - i * 6)) & 63) as usize] as char);
                }
            }
            out
        }
        format!("hdr.{}.sig", enc(&format!(r#"{{"email":"{email}"}}"#)))
    }

    #[test]
    fn decodes_email_from_jwt() {
        assert_eq!(jwt_email(&fake_jwt("a@b.com")).as_deref(), Some("a@b.com"));
        assert_eq!(jwt_email("not-a-jwt"), None);
        assert_eq!(jwt_email("hdr.!!!.sig"), None);
    }

    fn claude_home(dir: &Path, name: &str, uuid: &str, email: &str) -> Install {
        let home = dir.join(name);
        fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
        fs::write(
            home.join(".claude.json"),
            format!(
                r#"{{"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}","organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_5x"}}}}"#
            ),
        )
        .unwrap();
        Install { source: Source::Claude, home, discovered: false }
    }

    #[test]
    fn groups_two_installs_of_the_same_claude_account() {
        // 이 머신의 Windows·WSL 설치본이 같은 계정이었던 실제 상황
        let dir = tempfile::tempdir().unwrap();
        let uuid = "a88ff669-0f57-41e0-ae20-2db7eb9c5b94";
        let accounts = group(vec![
            claude_home(dir.path(), "win", uuid, "me@x.com"),
            claude_home(dir.path(), "wsl", uuid, "me@x.com"),
        ]);

        assert_eq!(accounts.len(), 1, "같은 계정의 설치본 둘은 하나로 묶여야 함");
        assert_eq!(accounts[0].label, "me@x.com");
        // `detail` 은 세 소스 공통으로 **로그인 방식** 자리다
        assert_eq!(accounts[0].detail, "Claude 로그인");
        // 플랜은 별도 필드. 원문(`claude_max` + `default_claude_max_5x`)이 아니라 읽을 수
        // 있는 이름이고, 티어가 플랜을 이미 포함하므로 둘을 잇지 않는다 (plan::plan_label)
        assert_eq!(accounts[0].plan, "Max 5x");
        assert_eq!(accounts[0].installs.len(), 2);
        assert!(accounts[0].standard);
    }

    #[test]
    fn different_accounts_stay_apart_and_track_standard_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut found = claude_home(dir.path(), "other", "uuid-2", "other@x.com");
        found.discovered = true; // 마커 스캔으로만 나온 처음 보는 계정
        let accounts = group(vec![
            claude_home(dir.path(), "mine", "uuid-1", "me@x.com"),
            found,
        ]);
        assert_eq!(accounts.len(), 2);
        let by_label = |l: &str| accounts.iter().find(|a| a.label == l).unwrap();
        assert!(by_label("me@x.com").standard);
        assert!(!by_label("other@x.com").standard, "스캔으로만 나온 계정은 표준이 아니다");
    }

    #[test]
    fn codex_account_uses_id_and_jwt_email() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codexhome");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(
            home.join("auth.json"),
            format!(
                r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"a2340ea8-b83c","id_token":"{}"}}}}"#,
                fake_jwt("me@x.com")
            ),
        )
        .unwrap();

        let id = codex_account(&home).unwrap();
        assert_eq!(id.key, "a2340ea8-b83c");
        assert_eq!(id.label, "me@x.com");
        assert_eq!(id.detail, "ChatGPT 로그인");
        assert_eq!(id.plan, "", "Codex 는 계정 파일에 플랜이 없다 — rollout 에서 온다");
    }

    /// 실제 agy 로그 한 줄과 같은 모양 (앞뒤로 무관한 로그가 섞여 있어야 진짜 테스트다)
    fn agy_home(dir: &Path, name: &str, id: &str, email: Option<&str>) -> Install {
        let home = dir.join(name);
        fs::create_dir_all(home.join("conversations")).unwrap();
        fs::create_dir_all(home.join("log")).unwrap();
        fs::write(home.join("installation_id"), id).unwrap();
        if let Some(e) = email {
            let log = format!(
                "ERROR: logging before google.Init: E0810 22:19:51.603971 83 errorreport.go:223] Failed to poll ListExperiments: error getting token source: You are not logged into Antigravity.\n\
                 ERROR: logging before google.Init: I0810 23:00:14.487238 156 keyring.go:81] keyringAuth: loaded token, expiry=2026-08-10 23:15:46 +0900 KST expired=false\n\
                 ERROR: logging before google.Init: I0810 23:00:14.656976 1 server_oauth.go:189] applyAuthResult: email={e}, authMethod=consumer, quotaProject=\n\
                 ERROR: logging before google.Init: I0810 23:00:20.536825 1 conversation_manager.go:374] Starting new conversation (agent=false)\n"
            );
            fs::write(home.join("log").join("cli-20260810_230014.log"), log).unwrap();
        }
        Install { source: Source::Antigravity, home, discovered: false }
    }

    /// agy 는 자격증명 파일이 없지만 로그에 로그인 이메일을 남긴다 → 계정으로 묶인다.
    /// (시작 직후 "not logged into Antigravity" 에러가 같은 파일에 섞여 있어도 무관해야 한다)
    #[test]
    fn antigravity_account_comes_from_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = group(vec![
            agy_home(dir.path(), "win", "id-1", Some("me@x.com")),
            agy_home(dir.path(), "wsl", "id-2", Some("me@x.com")),
        ]);

        assert_eq!(accounts.len(), 1, "같은 이메일이면 설치 id 가 달라도 한 계정");
        assert_eq!(accounts[0].label, "me@x.com");
        assert_eq!(accounts[0].detail, "Google 로그인");
        assert_eq!(accounts[0].installs.len(), 2);
        // 스캔 루트는 홈 자체 (그 아래 conversations/)
        assert_eq!(accounts[0].installs[0].transcript_root(), accounts[0].installs[0].home);
    }

    #[test]
    fn different_agy_accounts_stay_apart() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = group(vec![
            agy_home(dir.path(), "a", "id-1", Some("me@x.com")),
            agy_home(dir.path(), "b", "id-2", Some("other@x.com")),
        ]);
        assert_eq!(accounts.len(), 2);
    }

    /// 로그인 기록이 없으면 설치 id 로 떨어지되, 서로 합쳐지면 안 된다
    #[test]
    fn antigravity_without_log_falls_back_to_installation_id() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = group(vec![
            agy_home(dir.path(), "a", "8237cf26-e507-409d", None),
            agy_home(dir.path(), "b", "f5013b28-30d3-43cb", None),
        ]);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].label, "계정 미상 (설치 8237cf26)");
        assert_eq!(accounts[0].detail, "로그에 로그인 기록 없음");
    }

    #[test]
    fn unreadable_account_stays_separate() {
        // 계정 파일이 없으면 홈 경로가 키 — 다른 계정과 합쳐지면 안 된다
        let dir = tempfile::tempdir().unwrap();
        let installs: Vec<Install> = ["a", "b"]
            .iter()
            .map(|n| {
                let home = dir.path().join(n);
                fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
                Install { source: Source::Claude, home, discovered: false }
            })
            .collect();
        assert_eq!(group(installs).len(), 2, "식별 불가 설치본끼리 합쳐지면 안 됨");
    }
}

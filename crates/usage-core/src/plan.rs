//! 공식 플랜 한도 미터.
//!
//! 계정의 **공식** 소진율이다 — 로컬 추정치와 달리 다른 기기 사용량까지 반영된다.
//! Claude·Codex 는 실시간 API 가 1순위고, 폴백은 이미 읽고 있는 파일에서 나온다:
//!
//! | 소스 | 경로 | 값 |
//! |---|---|---|
//! | Claude ① | 사용량 API `GET /api/oauth/usage` ([`fetch_claude_usage`]) | 응답 본문 |
//! | Claude ② 폴백 | `<홈>/.claude.json` ([`read_utilization`]) | `cachedUsageUtilization` |
//! | Codex ① | 사용량 API `GET /backend-api/codex/usage` ([`fetch_codex_usage`]) | `rate_limit` |
//! | Codex ② 폴백 | rollout 의 `token_count` ([`crate::codex`]) | `payload.rate_limits` |
//! | Antigravity | **없음** — `quota_manager` 가 서버에서 받아 메모리에만 둔다\* | — |
//!
//! ① 두 개가 **이 앱의 네트워크 호출 전부**다. "홈 직독" 규칙의 예외인데, 예외 없이는
//! 사실을 보여줄 수 없어서다 — 두 소스의 ② 는 서로 다른 이유로 낡는다:
//!
//! - Claude ② 는 캐시라 아래 ⚠️ 처럼 세션이 도는 중에도 굳는다. 굳은 순간의 실측
//!   (2026-08-13): 캐시는 4% · 리셋 3시간 전(잔해), API 는 22% · 1시간 45분 남음 —
//!   **창 자체가 다른 세대였다.**
//! - Codex ② 는 캐시가 아니라 이벤트라 굳지는 않지만, **턴을 돌려야만 새 이벤트가
//!   생긴다** — 마지막 턴 이후의 리셋도, 다른 기기의 사용량도 다음 턴까지 안 보인다.
//!
//! 인증은 둘 다 이미 읽는 홈 파일에서 나온다 — Claude 는 `.claude/.credentials.json`,
//! Codex 는 `auth.json`(계정 식별에 쓰는 그 파일). 그래서 "홈마다 = 계정마다"라는 성질은
//! 그대로고, 규칙이 막으려던 것들(실행 환경 의존, 다중 계정 불가)은 생기지 않는다.
//! 실측 토큰 수명: Claude 는 수 시간, Codex 는 약 9.6일 — 만료면 호출 없이 ② 로 간다.
//!
//! \* Antigravity 도 quota RPC 자체는 있지만(바이너리에 `FetchQuotaStatus` 실재) 토큰이
//! OS 키링에만 있고 protobuf 요청을 리버싱해야 해서, 홈 파일로 인증하는 위 둘과 달리
//! 붙이지 않는다.
//!
//! ①과 ②는 **같은 모양을 준다.** Claude 는 API 응답 본문이 캐시의 `utilization` 과
//! 동일한 객체라(실측 대조) 미터 파싱을 공유하고, Codex 는 필드 이름만 다르다
//! (`primary_window`/`limit_window_seconds` vs rollout 의 `primary`/`window_minutes`).
//! 어느 쪽이든 같은 [`PlanUsage`] 로 나온다. 다른 건 `fetched_at` 뿐이다 — ① 은 방금
//! 받았고 ② 는 파일이 기록한 시각이 사실이다.
//!
//! Claude 는 예전에 `claude -p "/usage"` 를 띄워 텍스트 출력을 파싱했다. 그 방식은 셋을
//! 잃었다: 실행 방식에 좌우됐고(PATH·셸 심), 리셋 시각을 로컬 타임존 **문자열**로만 줘서
//! 다시 파싱해야 했고, 프로세스 하나 = 계정 하나라 다중 계정에 쓸 수 없었다. API 는
//! 셋 다 잃지 않는다 — CLI 실행의 대체가 아니라 캐시 파일 직독의 상위 호환이다.
//!
//! 실측 `cachedUsageUtilization` (Claude Code 2.1.220):
//! ```json
//! { "fetchedAtMs": 1786580825099, "accountUuid": "a88ff669-…",
//!   "utilization": {
//!     "five_hour": { "utilization": 3,  "resets_at": "2026-08-13T04:30:00Z" },
//!     "seven_day": { "utilization": 39, "resets_at": "2026-08-15T06:00:00Z" },
//!     "limits": [
//!       { "kind": "session",       "group": "session", "percent": 3  },
//!       { "kind": "weekly_all",    "group": "weekly",  "percent": 39 },
//!       { "kind": "weekly_scoped", "group": "weekly",  "percent": 45,
//!         "scope": { "model": { "display_name": "Fable" } } } ] } }
//! ```
//! `limits[]` 가 `/usage` 화면이 그리던 세 줄과 1:1 로 대응한다. 뜻을 모르는 코드네임
//! 창(`tangelo`, `nimbus_quill` 등)이 형제 키로 섞여 오지만 전부 무시한다 — 공개된
//! 목록이 아니라 의미를 모르는 값을 화면에 올릴 수 없다.
//!
//! ⚠️ **② 는 캐시다.** Claude Code 가 돌지 않으면 갱신되지 않는다. 실측 갱신 주기는 세션이
//! 도는 동안 5분이고, 낡음은 `fetchedAtMs` 로 판단한다 (그래서 [`PlanUsage::fetched_at`]
//! 에 `Utc::now()` 가 아니라 이 값을 넣는다 — 언제 서버에서 받은 값인지가 사실이다).
//!
//! ⚠️ **세션이 도는 중에도 굳는다.** "5분마다 갱신" 은 늘 성립하지 않는다. 실측
//! (2026-08-13): Claude Code 가 계속 돌고 `.claude.json` 자체는 43분 전에 쓰였는데
//! `fetchedAtMs` 는 **6시간 전**이었고, 그 사이 5시간 창의 `resets_at` 이 **2시간 전**으로
//! 지나가 있었다. 파일의 다른 필드는 갱신되면서 이 캐시만 굳는다.
//!
//! 그래서 **리셋 시각이 과거인 미터는 값이 아니라 잔해다** — 창은 이미 갈렸고 `percent`
//! 도 죽은 창의 것이다. 남은 시간을 0으로 깎으면 "지금 막 리셋된다" 는 거짓말이 되고,
//! 다음 리셋을 계산할 수도 없다(`limits[]` 에 창 길이가 없다). 화면에서 그 판정을 한다
//! — 프론트의 `resetIsStale`. 여기서 버리지 않는 이유는 "리셋 정보 없음"(agy)과
//! "리셋 정보가 낡음"(굳은 캐시)이 서로 다른 사실이기 때문이다.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::model::Source;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlanMeter {
    /// 예: "5시간", "주간", "주간 (Fable)"
    pub label: String,
    pub used_pct: u8,
    /// 리셋 시각. 두 소스 모두 기계가 읽는 형식으로 준다 — Codex 는 unix 초,
    /// Claude 는 RFC 3339 — 그래서 프론트가 문자열을 파싱할 일이 없다.
    /// 창에 리셋 개념이 없으면 None.
    pub resets_at: Option<DateTime<Utc>>,
    /// 이 리셋 시각을 **우리가 계산했는가** ([`crate::blocks`]).
    ///
    /// 공식 캐시가 굳어 리셋이 과거로 남으면 트랜스크립트에서 계산한 값으로 갈아 끼운다.
    /// 화면이 둘을 구분해 표시할 수 있게 표시를 남긴다 — 같은 숫자라도 출처가 다르다.
    /// Codex 는 늘 `false` (rollout 이 준 값을 그대로 쓴다).
    pub resets_computed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanUsage {
    pub source: Source,
    pub meters: Vec<PlanMeter>,
    /// 플랜 종류 등 부가 정보 (예: "free"). 없으면 빈 문자열.
    pub detail: String,
    pub fetched_at: DateTime<Utc>,
}

impl PlanUsage {
    /// 첫 미터(가장 짧은 창 = 세션)의 소진율 — 캐릭터 경고 상태 판단용
    pub fn session_pct(&self) -> Option<u8> {
        self.meters.first().map(|m| m.used_pct)
    }
}

/// 한도 창 길이(분) → 사람이 읽는 이름.
///
/// Codex 는 창을 분으로만 알려주고 이름(`limit_name`)은 null 로 온다. **창 길이가 플랜에
/// 따라 바뀐다** — 실측: free 는 43,200분(30일), plus 는 10,080분(7일)이고 **둘 다
/// `secondary` 는 null 인 단일 창**이었다. 두 창이 오는 플랜이 있는지는 미확정이라
/// 아래 표는 관측값 밖도 채워 둔다.
pub fn window_label(minutes: u64) -> String {
    match minutes {
        0 => "한도".into(),
        60 => "1시간".into(),
        300 => "5시간".into(),
        1440 => "일간".into(),
        10080 => "주간".into(),
        43200 => "월간".into(),
        m if m % 10080 == 0 => format!("{}주", m / 10080),
        m if m % 1440 == 0 => format!("{}일", m / 1440),
        m if m % 60 == 0 => format!("{}시간", m / 60),
        m => format!("{m}분"),
    }
}

/// 플랜 식별자(스네이크케이스 원문) → 사람이 읽는 이름.
///
/// 두 소스가 플랜을 **다른 파일에서, 다른 표기로** 준다. 그대로 보여주면 같은 화면에
/// `claude_max · default_claude_max_5x` 와 `plus` 가 나란히 놓인다 (전부 실측):
///
/// | 소스 | 원문 | 결과 |
/// |---|---|---|
/// | Claude `.claude.json` `organizationRateLimitTier` | `default_claude_max_5x` | `Max 5x` |
/// | Claude `organizationType` (티어가 없을 때) | `claude_max` | `Max` |
/// | Codex rollout `plan_type` | `plus` | `Plus` |
///
/// 표를 두지 않고 규칙으로 처리한다 — 값 목록이 공개 API 가 아니라서 새 플랜(`max_20x`
/// 등)이 나와도 표를 고칠 때까지 원문이 그대로 새어 나가면 안 된다. 붙어 오는 접두사를
/// 떼고, 나머지는 단어별로 첫 글자를 올린다. **`5x`·`20x` 처럼 숫자로 시작하는 조각은
/// 그대로 둔다** (`5X` 로 올리면 어색하다).
///
/// 떼는 접두사는 둘이다. `default_` 는 값에 붙어 오는 잡음이고, **`claude_` 는 벤더
/// 이름**이다 — 이 이름이 놓이는 자리(계정 카드의 칩)는 이미 어느 벤더인지 아이콘과
/// 계정으로 밝히고 있어서, 붙여 두면 `Claude · Claude Max 5x` 처럼 같은 말이 두 번 나온다.
/// Codex 는 `plan_type` 에 벤더 이름을 안 넣어 뗄 것이 없다.
pub fn plan_label(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw.strip_prefix("default_").unwrap_or(raw);
    let raw = raw.strip_prefix("claude_").unwrap_or(raw);
    raw.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                // 숫자로 시작하면 배수 표기(`5x`)라 손대지 않는다
                Some(f) if f.is_ascii_digit() => w.to_string(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 한도 창 하나의 이름. Codex 와 같은 화면에 나란히 놓이므로 같은 어휘를 쓴다.
///
/// 창 길이는 `limits[]` 안에 없지만 **형제 키가 확인해 준다** — 같은 payload 의
/// `five_hour`/`seven_day` 가 각 그룹과 같은 `resets_at` 을 갖는다(실측). 처음 보는
/// 그룹은 [`plan_label`] 로 떨어뜨려 원문이 그대로 새지 않게 한다.
fn meter_label(group: &str, model: Option<&str>) -> String {
    let base = match group {
        "session" => window_label(300),
        "weekly" => window_label(10080),
        other => plan_label(other),
    };
    match model {
        Some(m) => format!("{base} ({m})"),
        None => base,
    }
}

/// 짧은 창이 먼저 와야 첫 미터가 "지금 당장 걸리는 한도"가 된다 (Codex 와 같은 규칙).
/// 모르는 그룹은 뒤로 보낸다 — 첫 자리는 뜻을 아는 값만 차지해야 한다.
fn group_rank(group: &str) -> u8 {
    match group {
        "session" => 0,
        "weekly" => 1,
        _ => 2,
    }
}

/// `limits[]` 원소 하나 → 미터. 퍼센트가 없으면(자리만 잡은 항목) 버린다.
fn parse_limit(v: &Value) -> Option<(u8, bool, PlanMeter)> {
    let pct = v.get("percent").and_then(Value::as_f64)?;
    let group = v.get("group").and_then(Value::as_str).unwrap_or_default();
    let model = v
        .get("scope")
        .and_then(|s| s.get("model"))
        .and_then(|m| m.get("display_name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    Some((
        group_rank(group),
        model.is_some(),
        PlanMeter {
            label: meter_label(group, model),
            used_pct: pct.round().clamp(0.0, 100.0) as u8,
            resets_at: parse_rfc3339(v.get("resets_at")),
            resets_computed: false,
        },
    ))
}

fn parse_rfc3339(v: Option<&Value>) -> Option<DateTime<Utc>> {
    let s = v?.as_str()?;
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// 이름 있는 창(`five_hour`/`seven_day`) → 미터. `limits[]` 가 없는 옛 버전용 대비책이다.
fn parse_named_window(v: &Value, key: &str, minutes: u64) -> Option<PlanMeter> {
    let w = v.get(key).filter(|w| w.is_object())?;
    let pct = w.get("utilization").and_then(Value::as_f64)?;
    Some(PlanMeter {
        label: window_label(minutes),
        used_pct: pct.round().clamp(0.0, 100.0) as u8,
        resets_at: parse_rfc3339(w.get("resets_at")),
        resets_computed: false,
    })
}

/// utilization 객체 → 미터들. 캐시의 `utilization` 과 API 응답 본문이 **같은 모양**이라
/// (모듈 주석) 두 경로가 이 함수를 공유한다. 창이 하나도 없으면 빈 벡터.
fn parse_meters(util: &Value) -> Vec<PlanMeter> {
    let mut ranked: Vec<(u8, bool, PlanMeter)> = util
        .get("limits")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_limit).collect())
        .unwrap_or_default();
    // 정렬 기준이 같으면 payload 순서를 지킨다 (sort_by_key 는 안정 정렬)
    ranked.sort_by_key(|(rank, scoped, _)| (*rank, *scoped));
    let meters: Vec<PlanMeter> = ranked.into_iter().map(|(_, _, m)| m).collect();
    if !meters.is_empty() {
        return meters;
    }
    [
        parse_named_window(util, "five_hour", 300),
        parse_named_window(util, "seven_day", 10080),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `<홈>/.claude.json` 의 `cachedUsageUtilization` → 공식 한도 미터.
/// 파일이 없거나 캐시가 아직 안 채워졌으면 None (기능 조용히 비활성).
///
/// 같은 파일의 `oauthAccount` 로 [`crate::accounts`] 가 계정을 식별하므로, 이 값이 어느
/// 계정 것인지는 **홈이 곧 답이다** — payload 의 `accountUuid` 도 그 계정의
/// `oauthAccount.accountUuid` 와 일치한다(실측).
pub fn read_utilization(home: &Path) -> Option<PlanUsage> {
    let text = std::fs::read_to_string(home.join(".claude.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let cached = v.get("cachedUsageUtilization")?;
    // 언제 서버에서 받은 값인지 — 읽은 시각이 아니다
    let fetched_at = cached
        .get("fetchedAtMs")
        .and_then(Value::as_i64)
        .and_then(DateTime::from_timestamp_millis)?;
    let meters = parse_meters(cached.get("utilization")?);
    if meters.is_empty() {
        return None;
    }

    // 플랜 이름은 같은 파일의 계정 블록에 있다 — Codex 가 `plan_type` 을 싣는 자리와 같다
    let detail = v
        .get("oauthAccount")
        .and_then(|oa| oa.get("organizationRateLimitTier"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(plan_label)
        .unwrap_or_default();

    Some(PlanUsage { source: Source::Claude, meters, detail, fetched_at })
}

/// Claude Code 가 로그인 시 남기는 OAuth 자격증명 — `<홈>/.claude/.credentials.json`.
/// (macOS 는 파일이 아니라 키체인에 두므로 이 경로가 없다 → API 없이 폴백으로 간다.)
pub struct OauthCreds {
    pub access_token: String,
    /// 액세스 토큰 만료 시각. 실측 수명은 수 시간이고 CLI 가 도는 동안 계속 갱신된다.
    pub expires_at: Option<DateTime<Utc>>,
    /// 플랜 티어 원문 (예: `default_claude_max_5x`) — `.claude.json` 의
    /// `organizationRateLimitTier` 와 같은 값이 여기에도 있다(실측).
    pub rate_limit_tier: String,
}

/// 자격증명 파일 직독. 없거나(미로그인·macOS) 토큰이 비어 있으면 None.
pub fn read_credentials(home: &Path) -> Option<OauthCreds> {
    let path = home.join(".claude").join(".credentials.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let oa = v.get("claudeAiOauth")?;
    let access_token = oa
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();
    let expires_at = oa
        .get("expiresAt")
        .and_then(Value::as_i64)
        .and_then(DateTime::from_timestamp_millis);
    let rate_limit_tier = oa
        .get("rateLimitTier")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(OauthCreds { access_token, expires_at, rate_limit_tier })
}

/// Claude Code 자신이 `cachedUsageUtilization` 을 채울 때 부르는 것과 같은 엔드포인트.
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// OAuth 토큰 인증에 필요한 베타 헤더 (Claude Code 와 동일하게 보낸다)
const OAUTH_BETA: (&str, &str) = ("anthropic-beta", "oauth-2025-04-20");
/// Codex CLI 가 한도를 받는 것과 같은 엔드포인트 (바이너리 문자열에서 실측 확인).
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/codex/usage";

/// 두 벤더 호출이 공유하는 GET — 클라이언트 설정(10초 제한)을 한 곳에 둔다.
fn http_get(url: &str, headers: &[(&str, &str)]) -> Option<String> {
    let mut req = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    req.call().ok()?.into_string().ok()
}

/// Claude 공식 사용량 API 실시간 조회 (모듈 주석의 ①). 자격증명이 없거나, 토큰이
/// 만료됐거나, 네트워크가 안 되면 None — 호출자는 ② 캐시로 폴백한다.
///
/// ⚠️ **토큰 갱신은 절대 하지 않는다.** 리프레시 토큰은 회전식이라 여기서 갱신하면
/// CLI 자신의 갱신과 경합해 사용자가 로그아웃당할 수 있다. 만료면 그냥 물러난다 —
/// CLI 가 도는 동안 토큰은 어차피 새로 채워진다. (Codex 도 같은 규칙.)
pub fn fetch_claude_usage(home: &Path, now: DateTime<Utc>) -> Option<PlanUsage> {
    let creds = read_credentials(home)?;
    if creds.expires_at.map(|e| e <= now).unwrap_or(false) {
        return None;
    }
    let body = http_get(
        CLAUDE_USAGE_URL,
        &[("Authorization", &format!("Bearer {}", creds.access_token)), OAUTH_BETA],
    )?;
    parse_claude_api_usage(&body, &creds.rate_limit_tier, now)
}

/// Claude API 응답 본문 → [`PlanUsage`]. 본문이 캐시의 `utilization` 과 같은 객체라
/// 파서를 공유한다. `fetched_at` 은 수신 시각 — 이 경로에서는 방금이 곧 사실이다.
pub fn parse_claude_api_usage(
    body: &str,
    tier: &str,
    fetched_at: DateTime<Utc>,
) -> Option<PlanUsage> {
    let v: Value = serde_json::from_str(body).ok()?;
    let meters = parse_meters(&v);
    if meters.is_empty() {
        return None;
    }
    Some(PlanUsage { source: Source::Claude, meters, detail: plan_label(tier), fetched_at })
}

/// Codex CLI 의 OAuth 자격증명 — `<홈>/auth.json`, 계정 식별([`crate::accounts`])에
/// 쓰는 그 파일이다. 필요한 세 값이 전부 여기 있다.
pub struct CodexAuth {
    pub access_token: String,
    /// `chatgpt-account-id` 헤더로 보낸다 — 같은 로그인의 워크스페이스 구분자
    pub account_id: String,
    /// 액세스 토큰(JWT)의 `exp` 클레임. 실측 수명 약 9.6일 — Claude 보다 훨씬 길다.
    pub expires_at: Option<DateTime<Utc>>,
}

/// 자격증명 직독. 없거나(미로그인) 토큰이 비면 None.
pub fn read_codex_auth(home: &Path) -> Option<CodexAuth> {
    let text = std::fs::read_to_string(home.join("auth.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let t = v.get("tokens")?;
    let access_token = t
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();
    let account_id = t
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let expires_at = crate::accounts::jwt_payload(&access_token)
        .and_then(|p| p.get("exp").and_then(Value::as_i64))
        .and_then(|s| DateTime::from_timestamp(s, 0));
    Some(CodexAuth { access_token, account_id, expires_at })
}

/// Codex 공식 사용량 API 실시간 조회 (모듈 주석의 ①). 실패하면 None —
/// 호출자는 ② rollout 값으로 폴백한다. 토큰 갱신 금지 규칙은 Claude 와 같다.
pub fn fetch_codex_usage(home: &Path, now: DateTime<Utc>) -> Option<PlanUsage> {
    let auth = read_codex_auth(home)?;
    if auth.expires_at.map(|e| e <= now).unwrap_or(false) {
        return None;
    }
    let bearer = format!("Bearer {}", auth.access_token);
    let mut headers: Vec<(&str, &str)> = vec![("Authorization", &bearer)];
    if !auth.account_id.is_empty() {
        headers.push(("chatgpt-account-id", &auth.account_id));
    }
    let body = http_get(CODEX_USAGE_URL, &headers)?;
    parse_codex_api_usage(&body, now)
}

/// Codex API 응답 본문 → [`PlanUsage`]. rollout 의 `rate_limits` 와 내용은 같고 필드
/// 이름만 다르다 — `primary_window`/`limit_window_seconds`/`reset_at`(unix 초) vs
/// `primary`/`window_minutes`/`resets_at`. 정렬·이름 규칙은 [`crate::codex`] 와 동일하다.
pub fn parse_codex_api_usage(body: &str, fetched_at: DateTime<Utc>) -> Option<PlanUsage> {
    let v: Value = serde_json::from_str(body).ok()?;
    let rl = v.get("rate_limit")?;
    let mut meters: Vec<(u64, PlanMeter)> = vec![];
    for key in ["primary_window", "secondary_window"] {
        let Some(w) = rl.get(key).filter(|w| w.is_object()) else { continue };
        let Some(pct) = w.get("used_percent").and_then(Value::as_f64) else { continue };
        let minutes = w.get("limit_window_seconds").and_then(Value::as_u64).unwrap_or(0) / 60;
        let resets_at = w
            .get("reset_at")
            .and_then(Value::as_i64)
            .and_then(|s| DateTime::from_timestamp(s, 0));
        meters.push((
            minutes,
            PlanMeter {
                label: window_label(minutes),
                used_pct: pct.round().clamp(0.0, 100.0) as u8,
                resets_at,
                resets_computed: false,
            },
        ));
    }
    if meters.is_empty() {
        return None;
    }
    // 창이 짧은 것부터 — 첫 미터가 "지금 당장 걸리는 한도" (rollout 파서와 같은 규칙)
    meters.sort_by_key(|(m, _)| *m);
    let detail = v
        .get("plan_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(plan_label)
        .unwrap_or_default();
    Some(PlanUsage {
        source: Source::Codex,
        meters: meters.into_iter().map(|(_, m)| m).collect(),
        detail,
        fetched_at,
    })
}

/// 사용량 API 호출 간격 (초). CLI 자신이 5분마다 받아오므로 그보다 촘촘히 부를
/// 이유가 없다 — 호출 사이에는 마지막 성공값이 후보로 남는다.
pub const FETCH_INTERVAL_SECS: i64 = 300;

/// 플랜 사슬 전체 — 후보 수집(① API / ② 파일)과 "서버에서 가장 최근에 받은 값" 선택,
/// ③ 굳은 리셋 갈아 끼우기까지 여기서 끝낸다. 오케스트레이터(모니터의 플랜 스레드)는
/// 홈 목록과 계산 리셋값을 넘기고 결과를 슬롯에 올릴 뿐, 사슬의 단을 모른다.
#[derive(Default)]
pub struct PlanChain {
    /// 홈별 API 최근 성공값 — 호출 사이(간격)와 일시 실패(오프라인) 동안 유지한다.
    /// 실패한 홈만 낡은 값으로 남고, 선택(max fetched_at)이 알아서 가려낸다.
    claude_api: std::collections::HashMap<PathBuf, PlanUsage>,
    codex_api: std::collections::HashMap<PathBuf, PlanUsage>,
    last_fetch: Option<DateTime<Utc>>,
}

impl PlanChain {
    /// 한 회차. 실제 API 호출은 간격([`FETCH_INTERVAL_SECS`])이 찼을 때만 나가고,
    /// 파일 후보는 매번 모은다 — 루프를 촘촘히 돌려도 호출이 늘지 않는다.
    ///
    /// 반환은 소스별 최선 후보다 (Claude 하나, Codex 하나 — 없으면 빠진다).
    ///
    /// - **Claude**: ① API + ② `.claude.json` 캐시를 후보로 모아 가장 최근 수신값.
    ///   API 성공값은 `fetched_at` 이 방금이라 자연히 이기고, API 가 죽어 있으면
    ///   (오프라인·토큰 만료·macOS 키체인) CLI 가 갱신하는 캐시가 이긴다. 둘 다
    ///   굳었으면 ③ — 지난 리셋을 `computed_claude_reset` 으로 갈아 끼운다.
    /// - **Codex**: ① API 후보만 낸다. ② rollout 값은 사용량 스레드가 어댑터에서
    ///   직접 올리므로 여기 없다 — 누가 이기는지는 슬롯 쪽 중재(최신 수신값 규칙,
    ///   모니터의 `set_plan`)가 정한다.
    pub fn poll(
        &mut self,
        claude_homes: &[PathBuf],
        codex_homes: &[PathBuf],
        computed_claude_reset: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Vec<PlanUsage> {
        let due = self
            .last_fetch
            .map(|t| now - t >= chrono::Duration::seconds(FETCH_INTERVAL_SECS))
            .unwrap_or(true);
        // 홈이 하나도 없으면 간격을 소모하지 않는다 — 계정 발견이 끝나기 전의
        // 첫 회차들이 5분짜리 공회전이 되면 안 된다.
        if due && !(claude_homes.is_empty() && codex_homes.is_empty()) {
            self.last_fetch = Some(now);
            for h in claude_homes {
                if let Some(p) = fetch_claude_usage(h, now) {
                    self.claude_api.insert(h.clone(), p);
                }
            }
            for h in codex_homes {
                if let Some(p) = fetch_codex_usage(h, now) {
                    self.codex_api.insert(h.clone(), p);
                }
            }
        }

        let mut out = vec![];
        let claude = claude_homes
            .iter()
            .filter_map(|h| self.claude_api.get(h).cloned())
            .chain(claude_homes.iter().filter_map(|h| read_utilization(h)))
            .max_by_key(|p| p.fetched_at);
        if let Some(mut p) = claude {
            swap_stale_reset(&mut p, computed_claude_reset, now);
            out.push(p);
        }
        if let Some(p) = codex_homes
            .iter()
            .filter_map(|h| self.codex_api.get(h).cloned())
            .max_by_key(|p| p.fetched_at)
        {
            out.push(p);
        }
        out
    }
}

/// ③ — 굳은 캐시의 지난 리셋 시각을 우리 계산([`crate::blocks`])으로 갈아 끼운다.
///
/// API 가 살아 있으면 리셋이 과거일 일이 없어 여기 안 걸린다. 걸리는 건 API 가 물러난
/// 채(오프라인·토큰 만료·macOS 키체인) 캐시마저 굳었을 때다 (실측: 세션이 도는데도
/// 6시간 정지, 리셋은 2시간 전). 그때 "0분 남음" 을 보여주면 지금 막 리셋된다는
/// 거짓말이고, 창 길이가 `limits[]` 에 없어 다음 리셋을 유도할 수도 없다.
///
/// **첫 미터만** 손댄다. 5시간 창은 자주 갈리지만 주간 창은 며칠이 남아 있어 같은
/// 캐시라도 아직 사실이고, 무엇보다 주간은 우리가 계산할 수 없다.
fn swap_stale_reset(
    plan: &mut PlanUsage,
    computed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) {
    let Some(m) = plan.meters.first_mut() else { return };
    let expired = m.resets_at.map(|r| r <= now).unwrap_or(true);
    if expired {
        if let Some(end) = computed {
            m.resets_at = Some(end);
            m.resets_computed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실측 원문 두 개가 같은 화면에서 같은 모양으로 읽혀야 한다.
    #[test]
    fn plan_label_reads_like_a_plan_name() {
        // Claude `.claude.json` (이 머신 실측). 벤더 이름은 뗀다 — 칩이 놓이는
        // 계정 카드가 이미 어느 벤더인지 밝히고 있다
        assert_eq!(plan_label("default_claude_max_5x"), "Max 5x");
        assert_eq!(plan_label("claude_max"), "Max");
        // Codex rollout `plan_type` (실측 free/plus)
        assert_eq!(plan_label("plus"), "Plus");
        assert_eq!(plan_label("free"), "Free");
    }

    /// 값 목록이 공개 API 가 아니라, 처음 보는 플랜도 원문이 그대로 새면 안 된다.
    #[test]
    fn plan_label_handles_unseen_values() {
        assert_eq!(plan_label("default_claude_max_20x"), "Max 20x");
        assert_eq!(plan_label("claude_enterprise"), "Enterprise");
        // 벤더 이름만 남는 값이면 뗄 수 없다 — 빈 칩이 되면 안 된다
        assert_eq!(plan_label("claude"), "Claude");
        assert_eq!(plan_label("business"), "Business");
        // 빈 값·구분자만 있는 값에도 패닉하지 않는다
        assert_eq!(plan_label(""), "");
        assert_eq!(plan_label("default_"), "");
        assert_eq!(plan_label("__"), "");
        // 앞뒤 공백은 원문에 섞여 올 수 있다
        assert_eq!(plan_label("  plus  "), "Plus");
    }

    /// `.claude.json` 하나를 만들고 홈을 돌려준다
    fn home_with(json: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".claude.json"), json).unwrap();
        d
    }

    /// 이 머신에서 실제로 읽은 payload (토큰 등 무관한 필드는 뺐다)
    const REAL: &str = r#"{
      "oauthAccount": {
        "accountUuid": "a88ff669-0f57-41e0-ae20-2db7eb9c5b94",
        "emailAddress": "u@example.com",
        "organizationRateLimitTier": "default_claude_max_5x"
      },
      "cachedUsageUtilization": {
        "fetchedAtMs": 1786580825099,
        "accountUuid": "a88ff669-0f57-41e0-ae20-2db7eb9c5b94",
        "utilization": {
          "five_hour": {"utilization": 3, "resets_at": "2026-08-13T04:30:00.940401+00:00"},
          "seven_day": {"utilization": 39, "resets_at": "2026-08-15T06:00:00.940424+00:00"},
          "seven_day_opus": null,
          "tangelo": null,
          "nimbus_quill": {"utilization": 0, "resets_at": null},
          "limits": [
            {"kind": "session", "group": "session", "percent": 3,
             "resets_at": "2026-08-13T04:30:00.940401+00:00", "scope": null, "is_active": false},
            {"kind": "weekly_all", "group": "weekly", "percent": 39,
             "resets_at": "2026-08-15T06:00:00.940424+00:00", "scope": null, "is_active": false},
            {"kind": "weekly_scoped", "group": "weekly", "percent": 45,
             "resets_at": "2026-08-15T05:59:59.940701+00:00",
             "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null},
             "is_active": true}
          ]
        }
      }
    }"#;

    /// `/usage` 화면이 그리던 세 줄이 그대로 나와야 한다 — 이게 CLI 를 없앤 근거다.
    #[test]
    fn reads_the_three_meters_the_usage_screen_showed() {
        let home = home_with(REAL);
        let p = read_utilization(home.path()).unwrap();

        assert_eq!(p.source, Source::Claude);
        let seen: Vec<(&str, u8)> =
            p.meters.iter().map(|m| (m.label.as_str(), m.used_pct)).collect();
        // 짧은 창 먼저, 모델 한정은 전체 뒤에
        assert_eq!(seen, vec![("5시간", 3), ("주간", 39), ("주간 (Fable)", 45)]);
        assert_eq!(p.session_pct(), Some(3));

        // 리셋은 기계가 읽는 형식으로 온다 — 문자열을 다시 파싱할 일이 없다
        assert_eq!(
            p.meters[0].resets_at.unwrap().to_rfc3339(),
            "2026-08-13T04:30:00.940401+00:00"
        );
        // 플랜 이름은 Codex 의 `plan_type` 과 같은 자리(detail)에, 같은 규칙으로
        assert_eq!(p.detail, "Max 5x");
        // 읽은 시각이 아니라 서버에서 받은 시각
        assert_eq!(p.fetched_at.timestamp_millis(), 1_786_580_825_099);
    }

    /// 뜻을 모르는 코드네임 창(`tangelo`, `nimbus_quill`)은 화면에 올리지 않는다.
    #[test]
    fn unnamed_codename_windows_are_ignored() {
        let home = home_with(REAL);
        let p = read_utilization(home.path()).unwrap();
        assert_eq!(p.meters.len(), 3);
        assert!(!p.meters.iter().any(|m| m.label.to_lowercase().contains("quill")));
    }

    /// `limits[]` 가 없는 버전이라도 이름 있는 창으로 굴러가야 한다.
    #[test]
    fn falls_back_to_named_windows_without_limits() {
        let home = home_with(
            r#"{"cachedUsageUtilization": {"fetchedAtMs": 1786580825099, "utilization": {
                 "five_hour": {"utilization": 7, "resets_at": "2026-08-13T04:30:00+00:00"},
                 "seven_day": {"utilization": 50, "resets_at": null}}}}"#,
        );
        let p = read_utilization(home.path()).unwrap();
        assert_eq!(p.meters.len(), 2);
        assert_eq!((p.meters[0].label.as_str(), p.meters[0].used_pct), ("5시간", 7));
        assert_eq!(p.meters[1].label, "주간");
        assert!(p.meters[1].resets_at.is_none());
    }

    /// 처음 보는 그룹은 원문이 새지 않고, 뜻을 아는 창 뒤로 밀린다.
    #[test]
    fn unseen_groups_are_prettified_and_sorted_last() {
        let home = home_with(
            r#"{"cachedUsageUtilization": {"fetchedAtMs": 1, "utilization": {"limits": [
                 {"group": "monthly_burst", "percent": 10},
                 {"group": "session", "percent": 20}]}}}"#,
        );
        let p = read_utilization(home.path()).unwrap();
        assert_eq!(p.meters[0].label, "5시간");
        assert_eq!(p.meters[1].label, "Monthly Burst");
    }

    /// 이 머신에서 실제로 받은 API 응답 (2026-08-13, 무관한 필드 축약).
    /// 캐시의 `utilization` 과 같은 모양임을 실측으로 확인했다 — 이 픽스처가 그 증거다.
    const REAL_API: &str = r#"{
      "five_hour": {"utilization": 22.0, "resets_at": "2026-08-13T09:29:59.890032+00:00"},
      "seven_day": {"utilization": 43.0, "resets_at": "2026-08-15T05:59:59.890067+00:00"},
      "seven_day_opus": null,
      "tangelo": null,
      "nimbus_quill": {"utilization": 0.0, "resets_at": null},
      "extra_usage": {"is_enabled": false},
      "limits": [
        {"kind": "session", "group": "session", "percent": 22, "severity": "normal",
         "resets_at": "2026-08-13T09:29:59.890032+00:00", "scope": null, "is_active": false},
        {"kind": "weekly_all", "group": "weekly", "percent": 43, "severity": "normal",
         "resets_at": "2026-08-15T05:59:59.890067+00:00", "scope": null, "is_active": false},
        {"kind": "weekly_scoped", "group": "weekly", "percent": 45, "severity": "normal",
         "resets_at": "2026-08-15T05:59:59.890248+00:00",
         "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null},
         "is_active": true}
      ],
      "spend": {"used": {"amount_minor": 0}, "percent": 0, "enabled": false}
    }"#;

    /// API 응답이 캐시와 같은 세 미터로 나와야 한다 — 파서를 공유하는 근거.
    #[test]
    fn api_body_parses_like_the_cache() {
        let now = DateTime::parse_from_rfc3339("2026-08-13T07:45:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let p = parse_claude_api_usage(REAL_API, "default_claude_max_5x", now).unwrap();
        let seen: Vec<(&str, u8)> =
            p.meters.iter().map(|m| (m.label.as_str(), m.used_pct)).collect();
        assert_eq!(seen, vec![("5시간", 22), ("주간", 43), ("주간 (Fable)", 45)]);
        // 플랜 이름은 자격증명의 `rateLimitTier` 에서 — 캐시 경로와 같은 규칙으로
        assert_eq!(p.detail, "Max 5x");
        // 수신 시각이 곧 fetched_at — 캐시처럼 낡음을 따질 별도 시각이 없다
        assert_eq!(p.fetched_at, now);
        // API 가 준 리셋은 계산값이 아니다
        assert!(p.meters.iter().all(|m| !m.resets_computed));
    }

    /// 미터가 하나도 없는 응답(형태 변경·에러 본문)은 None — 빈 미터를 올리지 않는다.
    #[test]
    fn api_body_without_meters_is_rejected() {
        assert!(parse_claude_api_usage("{}", "x", Utc::now()).is_none());
        assert!(parse_claude_api_usage("not json", "x", Utc::now()).is_none());
    }

    /// `.claude/.credentials.json` 하나를 만들고 홈을 돌려준다
    fn home_with_creds(json: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".credentials.json"), json).unwrap();
        d
    }

    /// 이 머신 실측 구조 (토큰 값만 가짜) — 필드 위치가 바뀌면 여기서 걸린다.
    #[test]
    fn reads_the_credentials_file() {
        let home = home_with_creds(
            r#"{"claudeAiOauth": {"accessToken": "tok-x", "refreshToken": "tok-y",
                 "expiresAt": 1786635103494, "refreshTokenExpiresAt": 1788000914494,
                 "scopes": ["user:profile"], "subscriptionType": "max",
                 "rateLimitTier": "default_claude_max_5x"}}"#,
        );
        let c = read_credentials(home.path()).unwrap();
        assert_eq!(c.access_token, "tok-x");
        assert_eq!(c.expires_at.unwrap().timestamp_millis(), 1_786_635_103_494);
        assert_eq!(c.rate_limit_tier, "default_claude_max_5x");
    }

    /// 파일이 없거나(미로그인·macOS 키체인) 토큰이 비면 None — API 없이 폴백으로 간다.
    #[test]
    fn missing_or_empty_credentials_disable_the_api() {
        assert!(read_credentials(std::path::Path::new("/이런/홈은/없다")).is_none());
        let home = home_with_creds(r#"{"claudeAiOauth": {"accessToken": ""}}"#);
        assert!(read_credentials(home.path()).is_none());
    }

    /// 만료된 토큰으로는 **호출 자체를 안 한다** — 갱신은 CLI 몫이다 (경합하면 로그아웃).
    /// 네트워크를 건드리기 전에 물러나므로 이 테스트는 오프라인에서도 성립한다.
    #[test]
    fn expired_token_skips_the_call() {
        let home = home_with_creds(
            r#"{"claudeAiOauth": {"accessToken": "tok-x", "expiresAt": 1000}}"#,
        );
        assert!(fetch_claude_usage(home.path(), Utc::now()).is_none());
    }

    /// 이 머신에서 실제로 받은 Codex API 응답 (2026-08-13, 무관한 필드 축약).
    /// rollout 의 `rate_limits` 와 내용은 같고 필드 이름만 다르다는 실측 증거다.
    const REAL_CODEX_API: &str = r#"{
      "account_id": "a2340ea8-b835-4dc8-a4ff-e20e8982f492",
      "plan_type": "plus",
      "rate_limit": {
        "allowed": true, "limit_reached": false,
        "primary_window": {
          "used_percent": 7, "limit_window_seconds": 604800,
          "reset_after_seconds": 588701, "reset_at": 1787196790
        },
        "secondary_window": null
      },
      "credits": {"has_credits": false},
      "spend_control": {"reached": false}
    }"#;

    /// Codex API 응답이 rollout 경로와 같은 화면 어휘로 나와야 한다.
    #[test]
    fn codex_api_body_parses_like_the_rollout() {
        let now = Utc::now();
        let p = parse_codex_api_usage(REAL_CODEX_API, now).unwrap();
        assert_eq!(p.source, Source::Codex);
        // 604,800초 = 10,080분 → rollout 의 `window_minutes` 와 같은 이름 규칙
        assert_eq!(
            p.meters.iter().map(|m| (m.label.as_str(), m.used_pct)).collect::<Vec<_>>(),
            vec![("주간", 7)]
        );
        // 리셋은 unix 초 → UTC (rollout 의 `resets_at` 과 같은 변환)
        assert_eq!(p.meters[0].resets_at.unwrap().timestamp(), 1_787_196_790);
        assert_eq!(p.detail, "Plus");
        assert_eq!(p.fetched_at, now);
    }

    /// 두 창이 오면 짧은 창이 먼저다 — 첫 미터가 "지금 당장 걸리는 한도" (rollout 과 동일).
    #[test]
    fn codex_api_windows_sort_shortest_first() {
        let body = r#"{"rate_limit": {
          "primary_window": {"used_percent": 40, "limit_window_seconds": 604800},
          "secondary_window": {"used_percent": 12, "limit_window_seconds": 18000}}}"#;
        let p = parse_codex_api_usage(body, Utc::now()).unwrap();
        assert_eq!(
            p.meters.iter().map(|m| (m.label.as_str(), m.used_pct)).collect::<Vec<_>>(),
            vec![("5시간", 12), ("주간", 40)]
        );
        // plan_type 이 없으면 detail 은 조용히 빈다
        assert_eq!(p.detail, "");
    }

    /// 창이 하나도 없는 응답(형태 변경·에러 본문)은 None.
    #[test]
    fn codex_api_body_without_windows_is_rejected() {
        assert!(parse_codex_api_usage("{}", Utc::now()).is_none());
        assert!(parse_codex_api_usage(r#"{"rate_limit": {}}"#, Utc::now()).is_none());
    }

    /// 테스트용 JWT — 페이로드만 진짜 모양이면 된다 (서명은 애초에 검증하지 않는다)
    fn fake_jwt(payload: &str) -> String {
        fn enc(data: &[u8]) -> String {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
                let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
                for i in 0..(1 + chunk.len()) {
                    out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
                }
            }
            out
        }
        format!("{}.{}.sig", enc(br#"{"alg":"RS256"}"#), enc(payload.as_bytes()))
    }

    /// `auth.json` 하나를 만들고 홈을 돌려준다 (Codex 는 홈 바로 아래다)
    fn codex_home_with(json: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("auth.json"), json).unwrap();
        d
    }

    /// 이 머신 실측 구조 (토큰만 가짜) — 만료는 JWT `exp` 클레임에서 읽는다.
    #[test]
    fn reads_the_codex_auth_file() {
        let tok = fake_jwt(r#"{"exp": 1787196790}"#);
        let home = codex_home_with(&format!(
            r#"{{"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
                 "tokens": {{"access_token": "{tok}", "account_id": "acc-1"}}}}"#
        ));
        let a = read_codex_auth(home.path()).unwrap();
        assert_eq!(a.access_token, tok);
        assert_eq!(a.account_id, "acc-1");
        assert_eq!(a.expires_at.unwrap().timestamp(), 1_787_196_790);
    }

    /// Codex 도 만료 토큰이면 호출 자체를 건너뛴다 — 오프라인에서도 성립.
    #[test]
    fn expired_codex_token_skips_the_call() {
        let tok = fake_jwt(r#"{"exp": 1000}"#);
        let home = codex_home_with(&format!(r#"{{"tokens": {{"access_token": "{tok}"}}}}"#));
        assert!(fetch_codex_usage(home.path(), Utc::now()).is_none());
    }

    /// 파일이 없거나 토큰이 비면 None — rollout 폴백으로 간다.
    #[test]
    fn missing_codex_auth_disables_the_api() {
        assert!(read_codex_auth(std::path::Path::new("/이런/홈은/없다")).is_none());
        let home = codex_home_with(r#"{"tokens": {"access_token": ""}}"#);
        assert!(read_codex_auth(home.path()).is_none());
        // `exp` 를 못 읽는 불투명 토큰은 만료를 모르는 채로 쓴다 (서버가 401 로 거르면 폴백)
        let home = codex_home_with(r#"{"tokens": {"access_token": "opaque"}}"#);
        assert!(read_codex_auth(home.path()).unwrap().expires_at.is_none());
    }

    // ── PlanChain ──
    //
    // 아래 테스트의 홈들엔 자격증명 파일이 없어 ① API 가 조용히 빠진다 — 네트워크 없이
    // ② 파일 후보와 선택·스왑 규칙만 검증하는 것이고, 그게 사슬의 판단 로직 전부다.

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// 이름 있는 창 두 개짜리 최소 캐시 (5시간 + 주간, 주간 리셋은 항상 과거)
    fn cache_json(fetched_ms: i64, five_hour_reset: &str) -> String {
        format!(
            r#"{{"cachedUsageUtilization": {{"fetchedAtMs": {fetched_ms}, "utilization": {{
                 "five_hour": {{"utilization": 7, "resets_at": "{five_hour_reset}"}},
                 "seven_day": {{"utilization": 50, "resets_at": "2026-08-10T00:00:00Z"}}}}}}}}"#
        )
    }

    /// 후보가 여럿이면(홈 여러 개) 서버에서 **가장 최근에 받은** 값이 이긴다.
    #[test]
    fn the_chain_picks_the_most_recently_received_candidate() {
        let now = ts("2026-08-13T10:00:00Z");
        let old = home_with(&cache_json(1_000, "2026-08-13T12:00:00Z"));
        let new = home_with(&cache_json(2_000, "2026-08-13T12:30:00Z"));
        let homes = vec![old.path().to_path_buf(), new.path().to_path_buf()];

        let plans = PlanChain::default().poll(&homes, &[], None, now);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].source, Source::Claude);
        assert_eq!(plans[0].fetched_at.timestamp_millis(), 2_000);
        // 리셋이 미래(살아 있는 값)라 계산값으로 갈리지 않는다
        assert!(!plans[0].meters[0].resets_computed);
    }

    /// ③ — 리셋이 과거로 굳은 첫 미터만 계산값으로 갈고, 출처 표시를 남긴다.
    #[test]
    fn a_dead_reset_is_replaced_by_the_computed_end() {
        let now = ts("2026-08-13T10:00:00Z");
        let computed = ts("2026-08-13T11:30:00Z");
        // 리셋이 3시간 전 — 죽은 창의 잔해
        let home = home_with(&cache_json(1_000, "2026-08-13T07:00:00Z"));
        let homes = vec![home.path().to_path_buf()];

        let plans = PlanChain::default().poll(&homes, &[], Some(computed), now);
        let m = &plans[0].meters[0];
        assert_eq!(m.resets_at, Some(computed));
        assert!(m.resets_computed, "계산값임을 표시해야 화면이 출처를 가른다");
        // 주간 창은 리셋이 지났어도 손대지 않는다 — 우리가 계산할 수 없는 창이다
        assert!(!plans[0].meters[1].resets_computed);

        // 계산값마저 없으면(창 닫힘) 잔해를 그대로 둔다 — "낡음" 판정은 프론트
        // `resetIsStale` 의 몫이고, 여기서 지우면 "리셋 정보 없음"(agy)과 구분이 안 된다
        let plans = PlanChain::default().poll(&homes, &[], None, now);
        assert_eq!(plans[0].meters[0].resets_at, Some(ts("2026-08-13T07:00:00Z")));
        assert!(!plans[0].meters[0].resets_computed);
    }

    /// Codex 의 ② rollout 은 사슬 후보가 아니다 — 그 값은 사용량 스레드가 어댑터에서
    /// 직접 올리고 슬롯 중재가 정리한다. 여기서도 내면 같은 값이 두 경로로 흐른다.
    #[test]
    fn codex_files_never_enter_the_chain() {
        let (_guard, roots) = crate::codex::conformance_roots();
        assert!(PlanChain::default().poll(&[], &roots, None, Utc::now()).is_empty());
    }

    /// 아무 데이터도 없으면 빈 손으로 돌아온다 — 슬롯의 마지막 성공값이 유지된다.
    #[test]
    fn an_empty_world_yields_no_candidates() {
        assert!(PlanChain::default().poll(&[], &[], None, Utc::now()).is_empty());
    }

    /// 캐시가 아직 없거나 파일 자체가 없으면 조용히 비활성.
    #[test]
    fn missing_cache_disables_the_meter() {
        assert!(read_utilization(std::path::Path::new("/이런/홈은/없다")).is_none());
        // 로그인은 했지만 아직 한 번도 안 받아온 홈
        let home = home_with(r#"{"oauthAccount": {"accountUuid": "x"}}"#);
        assert!(read_utilization(home.path()).is_none());
        // 캐시는 있는데 창이 하나도 없는 경우
        let home = home_with(r#"{"cachedUsageUtilization": {"fetchedAtMs": 1, "utilization": {}}}"#);
        assert!(read_utilization(home.path()).is_none());
    }
}

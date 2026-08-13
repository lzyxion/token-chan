//! 공식 플랜 한도 미터.
//!
//! 계정의 **공식** 소진율이다 — 로컬 추정치와 달리 다른 기기 사용량까지 반영된다.
//! 한도를 주는 두 소스 모두 **이미 읽고 있는 파일**에서 나온다. 프로세스를 띄우지 않는다:
//!
//! | 소스 | 파일 | 필드 |
//! |---|---|---|
//! | Claude | `<홈>/.claude.json` (이 파일) | `cachedUsageUtilization` |
//! | Codex | rollout 의 `token_count` ([`crate::codex`]) | `payload.rate_limits` |
//! | Antigravity | **없음** — `quota_manager` 가 서버에서 받아 메모리에만 둔다 | — |
//!
//! Claude 는 예전에 `claude -p "/usage"` 를 띄워 텍스트 출력을 파싱했다. 그 방식은 셋을
//! 잃었다: 실행 방식에 좌우됐고(PATH·셸 심), 리셋 시각을 로컬 타임존 **문자열**로만 줘서
//! 다시 파싱해야 했고, 프로세스 하나 = 계정 하나라 다중 계정에 쓸 수 없었다. 같은 값이
//! 홈마다 파일로 있으므로 셋 다 사라지고, 홈 직독이라는 이 앱의 규칙과도 맞는다.
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
//! ⚠️ **캐시다.** Claude Code 가 돌지 않으면 갱신되지 않는다. 실측 갱신 주기는 세션이
//! 도는 동안 5분이고, 낡음은 `fetchedAtMs` 로 판단한다 (그래서 [`PlanUsage::fetched_at`]
//! 에 `Utc::now()` 가 아니라 이 값을 넣는다 — 언제 서버에서 받은 값인지가 사실이다).

use std::path::Path;

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
    })
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
    let util = cached.get("utilization")?;

    let mut ranked: Vec<(u8, bool, PlanMeter)> = util
        .get("limits")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_limit).collect())
        .unwrap_or_default();
    // 정렬 기준이 같으면 payload 순서를 지킨다 (sort_by_key 는 안정 정렬)
    ranked.sort_by_key(|(rank, scoped, _)| (*rank, *scoped));
    let mut meters: Vec<PlanMeter> = ranked.into_iter().map(|(_, _, m)| m).collect();

    if meters.is_empty() {
        meters = [
            parse_named_window(util, "five_hour", 300),
            parse_named_window(util, "seven_day", 10080),
        ]
        .into_iter()
        .flatten()
        .collect();
    }
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

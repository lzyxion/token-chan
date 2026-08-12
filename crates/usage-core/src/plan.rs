//! 공식 플랜 한도 미터.
//!
//! 계정의 **공식** 소진율이다 — 로컬 추정치와 달리 다른 기기 사용량까지 반영된다.
//! 소스마다 얻는 경로가 다르다:
//!
//! | 소스 | 경로 | 비용 |
//! |---|---|---|
//! | Claude | `claude -p "/usage"` 출력 파싱 (이 파일) | 프로세스 1회 실행 |
//! | Codex | rollout 의 `token_count` → `payload.rate_limits` ([`crate::codex`]) | 0 (이미 읽는 파일) |
//! | Antigravity | **없음** — `quota_manager` 가 서버에서 받아 메모리에만 둔다 | — |
//!
//! Codex 쪽이 더 좋은 경로다: 서버가 준 값이 이미 파일에 있어서 프로세스를 띄울 필요가
//! 없고, 리셋 시각도 문자열이 아니라 unix 타임스탬프로 정확히 온다.
//!
//! 관찰된 출력 형식 (Claude Code 2.1.220):
//! ```text
//! Current session: 60% used · resets Jul 30, 7:10pm (Asia/Seoul)
//! Current week (all models): 44% used · resets Aug 1, 3pm (Asia/Seoul)
//! Current week (Fable): 40% used · resets Aug 1, 2:59pm (Asia/Seoul)
//! ```
//! ⚠️ 출력 포맷은 버전에 따라 바뀔 수 있으므로 방어적으로 파싱하고,
//! 파싱 실패 시 None (기능 비활성) 으로 처리한다.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::Source;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlanMeter {
    /// 예: "Current session", "Current week (all models)", "월간"
    pub label: String,
    pub used_pct: u8,
    /// 리셋 시각 원문 (예: "Jul 30, 7:10pm (Asia/Seoul)"). 소스가 문자열로만 줄 때 쓴다.
    pub resets: String,
    /// 소스가 정확한 시각을 준 경우 (Codex 의 `resets_at`). 있으면 이쪽이 정답이라
    /// 프론트가 `resets` 문자열을 파싱할 필요가 없다.
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

/// `/usage` 텍스트 출력에서 미터들을 추출. 하나도 못 찾으면 None.
pub fn parse_usage_output(text: &str, now: DateTime<Utc>) -> Option<PlanUsage> {
    let mut meters = vec![];
    for line in text.lines() {
        let line = line.trim();
        let Some(pct_pos) = line.find("% used") else { continue };
        // "% used" 앞의 연속 숫자 추출
        let digits: String = line[..pct_pos]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let Ok(pct) = digits.parse::<u8>() else { continue };

        // 레이블: 콜론 앞부분
        let label = match line.find(':') {
            Some(i) => line[..i].trim().to_string(),
            None => continue,
        };
        // 리셋 시각: "resets " 이후 원문
        let resets = line
            .find("resets ")
            .map(|i| line[i + "resets ".len()..].trim().to_string())
            .unwrap_or_default();

        // Claude 는 리셋을 문자열로만 준다 — 정확한 시각은 프론트가 로컬 타임존
        // 기준으로 해석해야 해서(`parse_reset_datetime`) 여기서 채우지 않는다.
        meters.push(PlanMeter { label, used_pct: pct.min(100), resets, resets_at: None });
    }
    if meters.is_empty() {
        return None;
    }
    Some(PlanUsage {
        source: Source::Claude,
        meters,
        detail: String::new(),
        fetched_at: now,
    })
}

/// `claude -p "/usage"` 를 실행해 공식 한도를 조회.
/// Claude Code 미설치/실패/파싱 불가 시 None.
pub fn fetch_plan_usage() -> Option<PlanUsage> {
    let output = run_claude_usage()?;
    parse_usage_output(&output, Utc::now())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 리셋 원문("Jul 30, 7:09pm (Asia/Seoul)", "Aug 1, 3pm (…)")을 로컬 시각으로 파싱.
/// 연도는 now 기준(12시간 이상 과거면 +1년, 연말 경계). 형식이 다르면 None.
pub fn parse_reset_datetime(
    resets: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::{Datelike, NaiveDate, TimeZone};

    let month = MONTHS.iter().position(|m| resets.starts_with(m))? as u32 + 1;
    let rest = &resets[3..];
    let day: u32 = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;

    // 시간부: "7:09pm" 또는 "3pm"
    let lower = resets.to_ascii_lowercase();
    let ampm_pos = lower.find("am").or_else(|| lower.find("pm"))?;
    let is_pm = &lower[ampm_pos..ampm_pos + 2] == "pm";
    // ampm 앞의 "H" 또는 "H:MM" 추출
    let time_part: String = lower[..ampm_pos]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let time_part = time_part.trim_matches(':');
    let (h_str, m_str) = match time_part.split_once(':') {
        Some((h, m)) => (h, m),
        None => (time_part, "0"),
    };
    let mut hour: u32 = h_str.parse().ok()?;
    let minute: u32 = m_str.parse().ok()?;
    if hour == 12 {
        hour = 0;
    }
    if is_pm {
        hour += 12;
    }

    let build = |year: i32| {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|d| d.and_hms_opt(hour, minute, 0))
            .and_then(|ndt| chrono::Local.from_local_datetime(&ndt).single())
    };
    let mut dt = build(now.year())?;
    if dt < now - chrono::Duration::hours(12) {
        dt = build(now.year() + 1)?;
    }
    Some(dt)
}

#[cfg(windows)]
fn run_claude_usage() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // npm 전역 셸 심(claude.cmd) 해석을 위해 cmd /C 경유
    let out = std::process::Command::new("cmd")
        .args(["/C", "claude", "-p", "/usage", "--output-format", "text"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(windows))]
fn run_claude_usage() -> Option<String> {
    let out = std::process::Command::new("claude")
        .args(["-p", "/usage", "--output-format", "text"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"You are currently using your subscription to power your Claude Code usage

Current session: 60% used · resets Jul 30, 7:10pm (Asia/Seoul)
Current week (all models): 44% used · resets Aug 1, 3pm (Asia/Seoul)
Current week (Fable): 40% used · resets Aug 1, 2:59pm (Asia/Seoul)

What's contributing to your limits usage?
Last 24h · 267 requests · 2 sessions
  88% of your usage was at >150k context
"#;

    #[test]
    fn parses_observed_output() {
        let p = parse_usage_output(SAMPLE, Utc::now()).unwrap();
        assert_eq!(p.meters.len(), 3);
        assert_eq!(p.meters[0].label, "Current session");
        assert_eq!(p.meters[0].used_pct, 60);
        assert_eq!(p.meters[0].resets, "Jul 30, 7:10pm (Asia/Seoul)");
        assert_eq!(p.meters[1].label, "Current week (all models)");
        assert_eq!(p.meters[1].used_pct, 44);
        assert_eq!(p.meters[2].used_pct, 40);
        assert_eq!(p.session_pct(), Some(60));
        // 하단 통계 라인("88% of your usage...")은 "% used" 가 없어 미터로 오인하지 않음
    }

    #[test]
    fn unparseable_output_returns_none() {
        assert!(parse_usage_output("no meters here", Utc::now()).is_none());
        assert!(parse_usage_output("", Utc::now()).is_none());
    }

    #[test]
    fn parses_reset_datetime_variants() {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let now = Local.with_ymd_and_hms(2026, 7, 30, 14, 0, 0).unwrap();

        // 분 있는 형식
        let d = parse_reset_datetime("Jul 30, 7:09pm (Asia/Seoul)", now).unwrap();
        assert_eq!((d.month(), d.day(), d.hour(), d.minute()), (7, 30, 19, 9));

        // 분 없는 형식
        let d = parse_reset_datetime("Aug 1, 3pm (Asia/Seoul)", now).unwrap();
        assert_eq!((d.month(), d.day(), d.hour(), d.minute()), (8, 1, 15, 0));

        // 12am/12pm 경계
        let d = parse_reset_datetime("Jul 31, 12am (Asia/Seoul)", now).unwrap();
        assert_eq!(d.hour(), 0);
        let d = parse_reset_datetime("Jul 31, 12pm (Asia/Seoul)", now).unwrap();
        assert_eq!(d.hour(), 12);

        // 연말 경계: 12월 30일 시점에 "Jan 2" → 내년
        let dec = Local.with_ymd_and_hms(2026, 12, 30, 22, 0, 0).unwrap();
        let d = parse_reset_datetime("Jan 2, 1am (Asia/Seoul)", dec).unwrap();
        assert_eq!(d.year(), 2027);

        // 형식 불일치
        assert!(parse_reset_datetime("무슨 요일", now).is_none());
    }
}

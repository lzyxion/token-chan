use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 사용량 데이터 소스 (CLI 종류)
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
    /// Antigravity CLI (`agy`). Gemini CLI 를 대체한 도구라, 예전에 저장된 설정이
    /// 계속 읽히도록 `gemini` 도 같은 값으로 받는다.
    #[serde(alias = "gemini")]
    Antigravity,
}

impl Source {
    /// 다루는 소스 전부 — 화면·설정이 늘어놓는 순서이기도 하다.
    /// 목록을 곳곳에 복사해 두면 소스를 늘릴 때 한 곳이 조용히 빠진다.
    pub const ALL: [Source; 3] = [Source::Claude, Source::Codex, Source::Antigravity];

    /// 설정 파일·IPC 에 실리는 문자열. `Serialize` 와 **같은 값**이어야 한다
    /// (`rename_all = "lowercase"`) — 프론트가 이 문자열로 벤더를 지목한다.
    pub fn id(&self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Antigravity => "antigravity",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Source::Claude => "Claude Code",
            Source::Codex => "Codex CLI",
            Source::Antigravity => "Antigravity CLI",
        }
    }
}

/// 모든 어댑터가 정규화해 내보내는 사용량 이벤트 (API 요청 1건 단위)
#[derive(Clone, Debug, Serialize)]
pub struct UsageEvent {
    pub source: Source,
    pub model: String,
    pub ts: DateTime<Utc>,
    /// 캐시를 제외한 순수 입력 토큰
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    /// `cache_write` 중 **1시간 TTL** 로 적재된 몫 — 부분집합이지 별도 종류가 아니다.
    ///
    /// 다섯 번째 토큰 종류가 아니므로 [`UsageEvent::total`] 에도 `Totals` 에도 들어가지
    /// 않는다 (넣으면 이중 계산된다 — `total_ignores_cache_write_1h` 가 지킨다).
    /// 단가만 갈라야 해서 존재한다: 5분 TTL 은 입력의 1.25배, 1시간은 2배다.
    /// 값을 못 얻으면 0 이고, 그러면 전액 5분 단가 — 이 필드가 생기기 전과 같은 결과다.
    pub cache_write_1h: u64,
    pub cache_read: u64,
    /// 서브에이전트(사이드체인) 이벤트 여부 — "활성 모델" 판정에서 제외됨
    pub sidechain: bool,
}

impl UsageEvent {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

/// 소스별 데이터 가용성 상태
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceStatus {
    /// 데이터 파싱 성공
    Ok,
    /// 파일은 있지만 현재 형식에서 사용량을 해석하지 못함
    Degraded { checked: u64, failed: u64 },
    /// 데이터 루트/파일이 없음 (CLI 미설치 또는 미사용)
    NoData,
}

/// 소스별 파서가 수집하는 최소 진단값. `checked` 는 사용량 후보 기록 수이고,
/// `parsed` 는 그중 현재 스키마로 해석한 수다. 파일을 못 연 경우는 기록 수를 알 수
/// 없으므로 별도로 센다.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ParseDiagnostics {
    pub checked: u64,
    pub parsed: u64,
    pub files_read: u64,
    pub files_failed: u64,
}

impl ParseDiagnostics {
    pub fn add(&mut self, other: Self) {
        self.checked += other.checked;
        self.parsed += other.parsed;
        self.files_read += other.files_read;
        self.files_failed += other.files_failed;
    }
}

/// 빈 세션과 파일이 쓰이는 순간의 단일 불완전 행은 정상으로 둔다. 발견한 파일을 전혀
/// 열지 못했거나, 후보 기록이 충분한데 하나도 해석하지 못한 경우만 경고한다.
pub(crate) fn source_status(any_file: bool, diagnostics: ParseDiagnostics) -> SourceStatus {
    if !any_file {
        SourceStatus::NoData
    } else if diagnostics.files_read == 0 && diagnostics.files_failed > 0 {
        SourceStatus::Degraded {
            checked: diagnostics.files_failed,
            failed: diagnostics.files_failed,
        }
    } else if diagnostics.checked >= 3 && diagnostics.parsed == 0 {
        SourceStatus::Degraded {
            checked: diagnostics.checked,
            failed: diagnostics.checked,
        }
    } else {
        SourceStatus::Ok
    }
}

/// 어댑터 스캔 결과
pub struct ScanOutcome {
    pub events: Vec<UsageEvent>,
    pub status: SourceStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_warning_requires_a_strong_failure_signal() {
        assert_eq!(source_status(false, ParseDiagnostics::default()), SourceStatus::NoData);
        assert_eq!(
            source_status(
                true,
                ParseDiagnostics { checked: 2, parsed: 0, files_read: 1, files_failed: 0 }
            ),
            SourceStatus::Ok
        );
        assert_eq!(
            source_status(
                true,
                ParseDiagnostics { checked: 3, parsed: 0, files_read: 1, files_failed: 0 }
            ),
            SourceStatus::Degraded { checked: 3, failed: 3 }
        );
        assert_eq!(
            source_status(
                true,
                ParseDiagnostics { checked: 0, parsed: 0, files_read: 0, files_failed: 1 }
            ),
            SourceStatus::Degraded { checked: 1, failed: 1 }
        );
    }
}

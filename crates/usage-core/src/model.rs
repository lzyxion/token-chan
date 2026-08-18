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
    /// 데이터 루트/파일이 없음 (CLI 미설치 또는 미사용)
    NoData,
}

/// 어댑터 스캔 결과
pub struct ScanOutcome {
    pub events: Vec<UsageEvent>,
    pub status: SourceStatus,
}

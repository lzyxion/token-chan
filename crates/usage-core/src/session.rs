//! 최근 세션 목록 — "어느 프로젝트에서 얼마나 태웠나".
//!
//! 벤더 페이지는 벤더 단위라 프로젝트를 모르고, 통계 페이지는 날짜 단위라 무엇을
//! 했는지를 모른다. 이 목록만 그 질문에 답한다.
//!
//! 세 CLI 모두 작업 디렉토리를 정확히 남긴다 — 경로를 되짚어 추측할 필요가 없다:
//!
//! | | 세션 id | 작업 위치 | 제목 |
//! |---|---|---|---|
//! | Claude | 트랜스크립트의 `sessionId` | 같은 행의 `cwd` (+ `gitBranch`) | 없음 → 폴더명 |
//! | Codex | `session_meta.payload.id` | `session_meta.payload.cwd` | 없음 → 폴더명 |
//! | Antigravity | 대화 uuid | `conversation_summaries.workspace_uris` | **`preview`** (첫 사용자 메시지) |

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::Source;

/// 최근 세션 한 줄
#[derive(Clone, Debug, Serialize)]
pub struct SessionRow {
    pub source: Source,
    /// 세션/대화 id — 목록 키
    pub id: String,
    /// 화면에 뜨는 이름. agy 는 대화 제목, 나머지는 작업 폴더 이름.
    pub label: String,
    /// 작업 디렉토리 전체 경로 (툴팁용, 없으면 빈 문자열)
    pub cwd: String,
    /// 이 세션에서 마지막으로 쓴 모델
    pub model: String,
    /// git 브랜치 — Claude 만 알려준다
    pub branch: String,
    /// 마지막 활동 시각
    pub at: DateTime<Utc>,
    /// 이 세션이 쓴 토큰 총합 (스캔 범위 안에서)
    pub tokens: u64,
}

/// 경로에서 표시용 이름 하나를 뽑는다. 구분자가 OS마다 다르고 agy 는 `file://` URI 로
/// 주므로 둘 다 받아들인다. 마지막 조각이 비면(끝에 구분자) 그 앞을 쓴다.
pub fn dir_label(path: &str) -> String {
    let cleaned = path.trim_end_matches(['/', '\\']);
    cleaned
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(cleaned)
        .to_string()
}

/// `file:///C:/Users/x/proj` → `C:/Users/x/proj` (agy 의 workspace_uris 형식)
pub fn from_file_uri(uri: &str) -> String {
    let s = uri.strip_prefix("file://").unwrap_or(uri);
    // Windows 경로는 `file:///C:/...` 라 앞에 슬래시가 하나 더 붙는다
    let s = if s.len() > 2 && s.starts_with('/') && s.as_bytes()[2] == b':' { &s[1..] } else { s };
    percent_decode(s)
}

/// 공백 등이 `%20` 으로 인코딩돼 오므로 되돌린다. 의존성을 늘리지 않으려고 직접 구현.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// 여러 소스의 세션을 최근순으로 합친다. 같은 id 는 한 번만 (복사본 대비).
pub fn merge(mut rows: Vec<SessionRow>, limit: usize) -> Vec<SessionRow> {
    rows.sort_by(|a, b| b.at.cmp(&a.at));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| seen.insert((r.source, r.id.clone())));
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_label_handles_both_separators() {
        assert_eq!(dir_label(r"C:\Users\u\projects\token-chan"), "token-chan");
        assert_eq!(dir_label("/home/u/projects/api"), "api");
        assert_eq!(dir_label("/home/u/projects/api/"), "api");
        assert_eq!(dir_label("solo"), "solo");
        assert_eq!(dir_label(""), "");
    }

    #[test]
    fn file_uri_becomes_a_path() {
        // agy 가 실제로 주는 형태
        assert_eq!(from_file_uri("file:///C:/Users/u"), "C:/Users/u");
        assert_eq!(from_file_uri("file:///home/u/proj"), "/home/u/proj");
        assert_eq!(from_file_uri("file:///C:/my%20proj"), "C:/my proj");
        // URI 가 아니면 그대로
        assert_eq!(from_file_uri("/plain/path"), "/plain/path");
    }

    fn row(source: Source, id: &str, at: &str) -> SessionRow {
        SessionRow {
            source,
            id: id.into(),
            label: id.into(),
            cwd: String::new(),
            model: String::new(),
            branch: String::new(),
            at: DateTime::parse_from_rfc3339(at).unwrap().with_timezone(&Utc),
            tokens: 0,
        }
    }

    #[test]
    fn merge_sorts_recent_first_and_dedups() {
        let out = merge(
            vec![
                row(Source::Claude, "a", "2026-08-11T01:00:00Z"),
                row(Source::Codex, "b", "2026-08-11T03:00:00Z"),
                row(Source::Claude, "a", "2026-08-11T02:00:00Z"), // 같은 세션의 복사본
                row(Source::Antigravity, "c", "2026-08-11T02:30:00Z"),
            ],
            10,
        );
        assert_eq!(out.len(), 3, "같은 소스·id 는 한 번만");
        assert_eq!(out[0].id, "b");
        assert_eq!(out[1].id, "c");
        // 복사본 중에서는 더 최근 것이 남아야 한다
        assert_eq!(out[2].at.to_rfc3339(), "2026-08-11T02:00:00+00:00");
    }

    #[test]
    fn merge_respects_limit() {
        let rows: Vec<SessionRow> = (0..20)
            .map(|i| row(Source::Claude, &format!("s{i}"), "2026-08-11T01:00:00Z"))
            .collect();
        assert_eq!(merge(rows, 8).len(), 8);
    }
}

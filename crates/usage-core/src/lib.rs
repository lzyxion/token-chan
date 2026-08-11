//! usage-core: Claude Code / Codex CLI / Antigravity CLI(agy) 토큰 사용량 파싱·집계 코어.
//!
//! Tauri 등 UI 계층과 무관한 순수 로직만 담는다. 시스템 GUI 의존성 없이
//! `cargo test -p usage-core`로 전체 검증 가능.

pub mod accounts;
pub mod aggregate;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod context;
pub mod live;
pub mod model;
pub mod plan;
pub mod pricing;
pub mod protobuf;
pub mod roots;
pub mod session;

pub use aggregate::{build_summary, Summary};
pub use context::ContextState;
pub use model::{ScanOutcome, Source, SourceStatus, UsageEvent};

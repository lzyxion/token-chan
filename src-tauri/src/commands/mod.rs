//! 프론트엔드 ↔ 백엔드 IPC 커맨드.
//!
//! 한 파일 1300줄에 사용량·설정·펫·창·캐릭터 팩·계정이 섞여 있었다. 도메인별로
//! 나누되 **경로는 그대로 둔다** — `commands::` 아래 전부 재노출하므로 `lib.rs` 의
//! `invoke_handler!` 목록도, 다른 모듈의 호출부도 손댈 필요가 없다.
//!
//! 나눈 기준은 "무엇에 대한 명령인가"다:
//!
//! | 모듈 | 다루는 것 |
//! |---|---|
//! | [`usage`] | 모니터 스레드가 채운 사용량·라이브·플랜 읽기 |
//! | [`config`] | 설정 읽기·쓰기, 저장 실패 추적, 클릭 통과 |
//! | [`pet`] | 펫 창·말풍선의 위치·크기·드래그·대사 |
//! | [`windows`] | 창 열고 닫기와 리사이즈 (기하 규칙은 [`crate::window`]) |
//! | [`character`] | 캐릭터 팩 폴더·이미지·대사 파일 |
//! | [`accounts`] | 계정 켜고 끄기, 추가 스캔 홈, 다시 검색 |

mod accounts;
mod character;
mod config;
mod pet;
mod usage;
mod windows;

pub use accounts::*;
pub use character::*;
pub use config::*;
pub use pet::*;
pub use usage::*;
pub use windows::*;

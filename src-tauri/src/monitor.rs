//! 백그라운드 스캔 스레드.
//!
//! - 사용량 스레드(10초 주기): 어댑터 스캔 → 요약 빌드 → `usage-updated` emit
//!   (notify/inotify 대신 폴링: WSL의 /mnt/c drvfs 에서는 inotify가 동작하지 않으므로
//!   폴링이 모든 환경에서 신뢰 가능한 유일한 방식. 어댑터 내부의 mtime/size 파일 캐시로
//!   변경된 파일만 재파싱하므로 주기 스캔 비용은 낮다.)
//! - 라이브 스레드(2초 주기): 세션 busy/idle → 변경 시에만 `live-state` emit

use std::time::Duration;

use chrono::{Local, Utc};
use tauri::{AppHandle, Emitter, Manager};
use usage_core::claude::ClaudeAdapter;
use usage_core::codex::CodexAdapter;
use usage_core::gemini::GeminiAdapter;
use usage_core::live::read_live_state;
use usage_core::pricing::PriceTable;
use usage_core::{build_summary, Source, UsageEvent};

use crate::AppState;

const USAGE_INTERVAL: Duration = Duration::from_secs(10);
const LIVE_INTERVAL: Duration = Duration::from_secs(2);
/// 공식 플랜 한도(`claude -p "/usage"`) 폴링 주기 — CLI 프로세스를 띄우므로 여유 있게
const PLAN_INTERVAL: Duration = Duration::from_secs(300);
const SUMMARY_DAYS: usize = 14;

pub fn spawn(app: AppHandle) {
    spawn_usage_thread(app.clone());
    spawn_live_thread(app.clone());
    spawn_plan_thread(app);
}

/// 공식 플랜 한도 미터 폴링 (Claude Code 미설치 시 조용히 비활성)
/// + 블록 리셋 임박 OS 알림 (설정 분 전, 0=끔, 리셋 시각당 1회)
fn spawn_plan_thread(app: AppHandle) {
    std::thread::spawn(move || {
        use tauri_plugin_notification::NotificationExt;

        // 부팅 직후 CLI 스폰으로 시스템 부하 주지 않도록 짧게 대기
        std::thread::sleep(Duration::from_secs(5));
        let mut last_notified_reset: Option<chrono::DateTime<Local>> = None;

        loop {
            if let Some(plan) = usage_core::plan::fetch_plan_usage() {
                {
                    let state = app.state::<AppState>();
                    *state.plan.lock().unwrap() = Some(plan.clone());
                }
                let _ = app.emit("plan-updated", &plan);

                // 리셋 임박 알림
                let notify_min = {
                    let state = app.state::<AppState>();
                    let s = state.settings.lock().unwrap();
                    s.reset_notify_minutes
                };
                if notify_min > 0 {
                    if let Some(reset) = plan
                        .meters
                        .first()
                        .and_then(|m| usage_core::plan::parse_reset_datetime(&m.resets, Local::now()))
                    {
                        let remain = reset - Local::now();
                        let in_window = remain > chrono::Duration::zero()
                            && remain <= chrono::Duration::minutes(notify_min as i64);
                        if in_window && last_notified_reset != Some(reset) {
                            last_notified_reset = Some(reset);
                            let _ = app
                                .notification()
                                .builder()
                                .title("Token Pet")
                                .body(format!(
                                    "5시간 블록이 {}분 후 리셋됩니다 ({})",
                                    remain.num_minutes().max(1),
                                    reset.format("%H:%M")
                                ))
                                .show();
                        }
                    }
                }
            }
            // 실패 시 마지막 성공값 유지 (오래되면 프론트에서 fetched_at 으로 판단 가능)
            std::thread::sleep(PLAN_INTERVAL);
        }
    });
}

fn spawn_usage_thread(app: AppHandle) {
    std::thread::spawn(move || {
        let mut claude = ClaudeAdapter::with_default_roots();
        let mut codex = CodexAdapter::with_default_roots();
        let mut gemini = GeminiAdapter::with_default_roots();

        loop {
            let (retention_days, price_override) = {
                let state = app.state::<AppState>();
                let s = state.settings.lock().unwrap();
                (s.retention_days, s.price_override_path.clone())
            };
            let since = Utc::now() - chrono::Duration::days(retention_days as i64);

            let c = claude.scan(since);
            let x = codex.scan(since);
            let g = gemini.scan(since);

            let mut events: Vec<UsageEvent> = Vec::with_capacity(c.events.len() + x.events.len() + g.events.len());
            events.extend(c.events);
            events.extend(x.events);
            events.extend(g.events);
            events.sort_by_key(|e| e.ts);

            let statuses = vec![
                (Source::Claude, c.status),
                (Source::Codex, x.status),
                (Source::Gemini, g.status),
            ];

            let pricing = PriceTable::with_overrides(
                price_override.as_deref().map(std::path::Path::new),
            );
            let now = Utc::now();
            let offset = *Local::now().offset();
            let summary = build_summary(&events, &statuses, &pricing, SUMMARY_DAYS, now, offset.into());

            {
                let state = app.state::<AppState>();
                *state.summary.lock().unwrap() = Some(summary.clone());
            }
            let _ = app.emit("usage-updated", &summary);

            std::thread::sleep(USAGE_INTERVAL);
        }
    });
}

fn spawn_live_thread(app: AppHandle) {
    std::thread::spawn(move || {
        let dirs = usage_core::roots::claude_session_dirs();
        let mut prev = String::new();
        loop {
            let now_ms = Utc::now().timestamp_millis();
            let live = read_live_state(&dirs, now_ms);
            let json = serde_json::to_string(&live).unwrap_or_default();
            if json != prev {
                prev = json;
                {
                    let state = app.state::<AppState>();
                    *state.live.lock().unwrap() = live.clone();
                }
                let _ = app.emit("live-state", &live);
            }
            std::thread::sleep(LIVE_INTERVAL);
        }
    });
}

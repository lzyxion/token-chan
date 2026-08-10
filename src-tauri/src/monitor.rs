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

/// 리셋 임박 기본 문구 — 프론트 speech.ts `DEFAULT_LINES.resetNotify` 와 동일하게 유지.
/// 여기서 치환하는 변수는 `{분}`·`{시각}` 뿐이다.
const DEFAULT_RESET_NOTIFY: &[&str] = &[
    "{분}분 뒤에 블록이 리셋돼! ({시각})",
    "{분}분만 버티면 리셋이야 ({시각})",
    "곧 리셋! {시각}에 새 블록이 열려",
    "{시각} 리셋까지 {분}분 남았어",
    "리셋 임박! {분}분 뒤에 충전돼 ({시각})",
];

pub fn spawn(app: AppHandle) {
    spawn_usage_thread(app.clone());
    spawn_live_thread(app.clone());
    spawn_plan_thread(app);
}

/// 후보 중 하나를 무작위로. `last` 와 같은 문구는 후보에서 뺀다
/// (뺐더니 남는 게 없으면 = 후보가 하나뿐이면 그대로 반복). `lines` 는 비어 있지 않아야 한다.
fn pick_line(lines: &[String], last: Option<&str>) -> String {
    let pool: Vec<&String> = lines.iter().filter(|l| Some(l.as_str()) != last).collect();
    let from: Vec<&String> = if pool.is_empty() {
        lines.iter().collect()
    } else {
        pool
    };
    // 전용 rng 크레이트 없이 — 나노초 하위 비트로 충분히 흩어진다
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0)
        % from.len();
    from[idx].clone()
}

/// 공식 플랜 한도 미터 폴링 (Claude Code 미설치 시 조용히 비활성)
/// + 블록 리셋 임박 시 캐릭터 대사 (설정 분 전, 0=끔, 리셋 시각당 1회)
fn spawn_plan_thread(app: AppHandle) {
    std::thread::spawn(move || {
        // 부팅 직후 CLI 스폰으로 시스템 부하 주지 않도록 짧게 대기
        std::thread::sleep(Duration::from_secs(5));
        let mut last_notified_reset: Option<chrono::DateTime<Local>> = None;
        // 직전에 쓴 문구 — 후보가 둘 이상이면 연속 중복을 피한다 (프론트 speech.ts pick 과 동일 규칙)
        let mut last_notify_line: Option<String> = None;

        loop {
            if let Some(plan) = usage_core::plan::fetch_plan_usage() {
                {
                    let state = app.state::<AppState>();
                    *state.plan.lock().unwrap() = Some(plan.clone());
                }
                let _ = app.emit("plan-updated", &plan);

                // 리셋 임박 — OS 알림 대신 캐릭터가 직접 말한다.
                // 문구는 활성 모델의 문구 세트 → 기본 문구 → 내장 순 (프론트 speech.ts 와 동일 규칙)
                let (notify_min, custom_lines) = {
                    let state = app.state::<AppState>();
                    let model = state
                        .summary
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|sum| sum.last_model.clone());
                    let s = state.settings.lock().unwrap();
                    let set_lines = model
                        .as_deref()
                        .and_then(|m| {
                            // 최장 접두사 매칭 (캐릭터 팩 규칙과 동일)
                            let mut best_len = 0usize;
                            let mut best: Option<&str> = None;
                            for r in &s.speech_rules {
                                if r.set.is_empty() {
                                    continue;
                                }
                                for p in r.prefixes.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                                    if m.starts_with(p) && p.len() > best_len {
                                        best_len = p.len();
                                        best = Some(r.set.as_str());
                                    }
                                }
                            }
                            best
                        })
                        .and_then(|name| s.speech_sets.get(name))
                        .and_then(|set| set.get("resetNotify"))
                        .filter(|v| v.iter().any(|l| !l.trim().is_empty()))
                        .cloned();
                    let lines = set_lines.or_else(|| s.speech_lines.get("resetNotify").cloned());
                    (s.reset_notify_minutes, lines)
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
                            // 사용자 문구가 있으면 그중에서, 없으면 내장 기본에서 무작위 선택.
                            let lines: Vec<String> = custom_lines
                                .unwrap_or_default()
                                .iter()
                                .map(|l| l.trim().to_string())
                                .filter(|l| !l.is_empty())
                                .collect();
                            let lines = if lines.is_empty() {
                                DEFAULT_RESET_NOTIFY.iter().map(|l| l.to_string()).collect()
                            } else {
                                lines
                            };
                            let template = pick_line(&lines, last_notify_line.as_deref());
                            last_notify_line = Some(template.clone());
                            let text = template
                                .replace("{분}", &remain.num_minutes().max(1).to_string())
                                .replace("{시각}", &reset.format("%H:%M").to_string())
                                .replace('|', "\n");
                            crate::commands::show_speech(app.clone(), text);
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

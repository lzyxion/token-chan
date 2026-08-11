//! 백그라운드 스캔 스레드.
//!
//! - 사용량 스레드(10초 주기): 어댑터 스캔 → 요약 빌드 → `usage-updated` emit
//!   (notify/inotify 대신 폴링: WSL의 /mnt/c drvfs 에서는 inotify가 동작하지 않으므로
//!   폴링이 모든 환경에서 신뢰 가능한 유일한 방식. 어댑터 내부의 mtime/size 파일 캐시로
//!   변경된 파일만 재파싱하므로 주기 스캔 비용은 낮다.)
//! - 라이브 스레드(2초 주기): 세션 busy/idle → 변경 시에만 `live-state` emit

use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, Utc};
use tauri::{AppHandle, Emitter, Manager};
use usage_core::claude::ClaudeAdapter;
use usage_core::codex::CodexAdapter;
use usage_core::antigravity::AntigravityAdapter;
use usage_core::live::read_live_state;
use usage_core::pricing::PriceTable;
use usage_core::{build_summary, Source, UsageEvent};

use crate::AppState;

const USAGE_INTERVAL: Duration = Duration::from_secs(10);
const LIVE_INTERVAL: Duration = Duration::from_secs(2);
/// 공식 플랜 한도(`claude -p "/usage"`) 폴링 주기 — CLI 프로세스를 띄우므로 여유 있게
const PLAN_INTERVAL: Duration = Duration::from_secs(300);
/// 일별 집계 기간 — 통계 페이지의 잔디(13주 격자)를 채울 만큼 길어야 한다.
/// 91일이면 마지막 열이 이번 주가 되도록 주 단위로 딱 떨어진다.
/// 기본 보존기간(90일)을 넘는 날은 자연히 0으로 나온다.
const SUMMARY_DAYS: usize = 91;
/// 최근 세션 목록에 실을 개수 — 패널 한 페이지에 들어가는 만큼
const RECENT_SESSIONS: usize = 8;

/// 리셋 임박 기본 문구 — 프론트 speech.ts `DEFAULT_LINES.resetNotify` 와 동일하게 유지.
/// 여기서 치환하는 변수는 `{분}`·`{시각}` 뿐이다.
const DEFAULT_RESET_NOTIFY: &[&str] = &[
    "{분}분 뒤에 블록이 리셋돼! ({시각})",
    "{분}분만 버티면 리셋이야 ({시각})",
    "곧 리셋! {시각}에 새 블록이 열려",
    "{시각} 리셋까지 {분}분 남았어",
    "리셋 임박! {분}분 뒤에 충전돼 ({시각})",
];

/// 설정의 추가 스캔 경로
pub fn extra_homes(app: &AppHandle) -> usage_core::accounts::ExtraHomes {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap();
    usage_core::accounts::ExtraHomes {
        claude: s.extra_claude_homes.iter().map(PathBuf::from).collect(),
        codex: s.extra_codex_homes.iter().map(PathBuf::from).collect(),
        antigravity: s.extra_antigravity_homes.iter().map(PathBuf::from).collect(),
    }
}

/// 계정을 다시 발견해 상태에 캐시한다. 마커 스캔이 수백 ms 걸리므로 자주 부르지 않는다.
pub fn rediscover(app: &AppHandle) {
    let extra = extra_homes(app);
    let found = usage_core::accounts::discover(&extra, true);
    let state = app.state::<AppState>();
    *state.accounts.lock().unwrap() = found;
}

/// 집계에 포함할 계정인지 — 사용자가 지정했으면 그 값, 아니면 기본 규칙.
/// 기본값이 `standard` 인 이유는 settings.rs 의 `accounts_enabled` 주석 참고.
pub fn account_enabled(
    a: &usage_core::accounts::Account,
    overrides: &std::collections::HashMap<String, bool>,
) -> bool {
    overrides.get(&a.setting_key()).copied().unwrap_or(a.standard)
}

/// 활성 계정의 설치본에서 뽑은 스캔 루트들
#[derive(Default)]
pub struct EnabledRoots {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub antigravity: Vec<PathBuf>,
    /// Claude 라이브 세션 레지스트리 (다른 소스엔 없음)
    pub sessions: Vec<PathBuf>,
}

fn enabled_roots(app: &AppHandle) -> EnabledRoots {
    let state = app.state::<AppState>();
    // 두 락을 겹쳐 잡지 않는다 — 다른 경로에서 settings 를 먼저 잡는 곳이 있어 교착 위험
    let overrides = { state.settings.lock().unwrap().accounts_enabled.clone() };
    let accounts = { state.accounts.lock().unwrap().clone() };

    let mut r = EnabledRoots::default();
    for a in accounts.iter().filter(|a| account_enabled(a, &overrides)) {
        for i in &a.installs {
            match i.source {
                Source::Claude => {
                    r.claude.push(i.transcript_root());
                    if let Some(d) = i.session_dir() {
                        r.sessions.push(d);
                    }
                }
                Source::Codex => r.codex.push(i.transcript_root()),
                Source::Antigravity => r.antigravity.push(i.transcript_root()),
            }
        }
    }
    r
}

/// 한 소스의 공식 한도를 갱신하고 프론트에 알린다.
/// Claude 는 플랜 스레드가, Codex 는 사용량 스레드가 부르므로 소스별로만 덮어쓴다.
fn set_plan(app: &AppHandle, plan: usage_core::plan::PlanUsage) {
    let all = {
        let state = app.state::<AppState>();
        let mut list = state.plan.lock().unwrap();
        match list.iter_mut().find(|p| p.source == plan.source) {
            Some(slot) => *slot = plan,
            None => list.push(plan),
        }
        // 소스 순서를 고정해 두면 프론트에서 줄이 튀지 않는다
        list.sort_by_key(|p| p.source);
        list.clone()
    };
    let _ = app.emit("plan-updated", &all);
}

pub fn spawn(app: AppHandle) {
    // 계정 발견을 **setup 에서 하면 안 된다** — 마커 스캔이 파일시스템을 훑고,
    // 느린 경로가 하나라도 끼면 창이 뜨기 전에 앱이 통째로 멈춘다.
    // (실측: 멈춘 WSL 배포판 때문에 4분간 얼어붙음)
    // 별도 스레드로 돌리면 창은 즉시 뜨고 계정 목록만 잠깐 뒤에 채워진다.
    let discover_app = app.clone();
    std::thread::spawn(move || {
        rediscover(&discover_app);
        // 목록이 채워졌으니 트레이 메뉴를 다시 만든다 (빈 상태로 만들어졌으므로)
        crate::tray::refresh_menu(&discover_app);
    });
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
                set_plan(&app, plan.clone());

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
        // 루트는 설정(추가 스캔 경로)에 따라 달라지므로 첫 회차에 채워진다
        let mut claude = ClaudeAdapter::new(vec![]);
        let mut codex = CodexAdapter::new(vec![]);
        let mut agy = AntigravityAdapter::new(vec![]);
        let mut claude_roots: Vec<PathBuf> = vec![];
        let mut codex_roots: Vec<PathBuf> = vec![];
        let mut agy_roots: Vec<PathBuf> = vec![];
        let mut first = true;

        loop {
            let (retention_days, price_override) = {
                let state = app.state::<AppState>();
                let s = state.settings.lock().unwrap();
                (s.retention_days, s.price_override_path.clone())
            };

            // 활성 계정이 바뀌면(토글·재검색·경로 추가) 어댑터를 새로 만든다. 파일 캐시가
            // 비워져 다음 스캔은 전체 재파싱이지만, 설정을 건드릴 때만 일어난다.
            let roots = enabled_roots(&app);
            if first || roots.claude != claude_roots {
                claude_roots = roots.claude.clone();
                claude = ClaudeAdapter::new(roots.claude);
            }
            if first || roots.codex != codex_roots {
                codex_roots = roots.codex.clone();
                codex = CodexAdapter::new(roots.codex);
            }
            if first || roots.antigravity != agy_roots {
                agy_roots = roots.antigravity.clone();
                agy = AntigravityAdapter::new(roots.antigravity);
            }
            first = false;
            let since = Utc::now() - chrono::Duration::days(retention_days as i64);

            let c = claude.scan(since);
            let x = codex.scan(since);
            let g = agy.scan(since);

            let mut events: Vec<UsageEvent> = Vec::with_capacity(c.events.len() + x.events.len() + g.events.len());
            events.extend(c.events);
            events.extend(x.events);
            events.extend(g.events);
            events.sort_by_key(|e| e.ts);

            let statuses = vec![
                (Source::Claude, c.status),
                (Source::Codex, x.status),
                (Source::Antigravity, g.status),
            ];

            // Codex 는 공식 한도가 rollout 안에 들어 있다 — CLI 를 띄우는 Claude 와 달리
            // 방금 읽은 파일에서 그냥 나온다
            if let Some(p) = codex.plan() {
                set_plan(&app, p);
            }
            // 세션 레지스트리가 없는 소스의 감시 파일을 라이브 스레드에 넘긴다
            {
                let mut watch = vec![];
                if let Some(p) = codex.watch_path() {
                    watch.push((Source::Codex, p, "codex".to_string()));
                }
                if let Some(p) = agy.watch_path() {
                    watch.push((Source::Antigravity, p, "agy".to_string()));
                }
                *app.state::<AppState>().watch.lock().unwrap() = watch;
            }

            let pricing = PriceTable::with_overrides(
                price_override.as_deref().map(std::path::Path::new),
            );
            let now = Utc::now();
            let offset = *Local::now().offset();
            let mut summary = build_summary(&events, &statuses, &pricing, SUMMARY_DAYS, now, offset.into());
            // 컨텍스트는 트랜스크립트 파일 단위 정보라 이벤트 집계로 안 나온다 —
            // 어댑터가 스캔하며 모아 둔 것 중 가장 최근에 움직인 세션을 얹는다.
            // (세 CLI 를 번갈아 쓰면 방금 만진 쪽이 게이지에 뜬다)
            // 어느 벤더를 게이지에 태울지는 프론트가 정하므로 여기서 고르지 않는다
            summary.contexts = [
                claude.context(&pricing),
                codex.context(&pricing),
                agy.context(&pricing),
            ]
            .into_iter()
            .flatten()
            .collect();
            // 최근 세션 — 소스별 목록을 합쳐 최근순 상위 N개만
            summary.sessions = usage_core::session::merge(
                [claude.sessions(), codex.sessions(), agy.sessions()].concat(),
                RECENT_SESSIONS,
            );

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
        let mut prev = String::new();
        loop {
            // 라이브 세션도 활성 계정의 설치본만 본다 (계정을 끄면 그쪽 세션은 무시)
            let dirs = enabled_roots(&app).sessions;

            let now = Utc::now();
            let mut live = read_live_state(&dirs, now.timestamp_millis());
            // Claude 외 소스는 레지스트리가 없어 감시 파일 mtime 으로 유도한다.
            // 사용량 스캔(10초)이 아니라 여기(2초)서 stat 해야 작업 시작이 늦게 안 보인다.
            {
                let watch = { app.state::<AppState>().watch.lock().unwrap().clone() };
                usage_core::live::add_inferred(&mut live, &watch, now);
            }
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

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
/// 공식 플랜 한도 폴링 주기. 홈마다 `.claude.json` 하나를 읽을 뿐이라 값이 싸다
/// (예전엔 CLI 를 띄워 300초였다). 원본 캐시가 5분마다 갱신되므로 이보다 촘촘히
/// 읽어도 더 새 값이 나오진 않지만, 갱신을 늦게 보는 일은 없어진다.
const PLAN_INTERVAL: Duration = Duration::from_secs(30);
/// 사용량 API 호출 주기 (플랜 루프 안에서 이 간격으로만 실제 호출한다).
/// Claude Code 자신이 5분마다 받아오므로 그보다 촘촘히 부를 이유가 없다 —
/// 루프(30초)는 캐시 파일 확인과 리셋 임박 판정을 위해 그대로 촘촘하게 둔다.
const PLAN_FETCH_INTERVAL: Duration = Duration::from_secs(300);
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

/// 사용자가 누른 "다시 검색" — 캐시된 WSL 조회 결과까지 버리고 처음부터 찾는다.
/// 자동 재발견(홈 추가·제거)과 달리 사용자가 **상황을 바꾸고** 부르는 것이라,
/// 앱을 켠 뒤 시작한 WSL 배포판도 이때 잡혀야 한다.
pub fn rescan(app: &AppHandle) {
    usage_core::roots::forget_wsl_guest_homes();
    rediscover(app);
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
    /// Claude 홈 자체 — 공식 한도(`.claude.json`)가 트랜스크립트 루트가 아니라
    /// 홈에 있어서 따로 들고 간다
    pub claude_homes: Vec<PathBuf>,
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
                    r.claude_homes.push(i.home.clone());
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

/// 한 소스의 공식 한도를 갱신하고 프론트에 알린다. 소스별로만 덮어쓴다.
///
/// Codex 슬롯엔 **두 스레드가 쓴다** — 사용량 스레드가 rollout 값(10초 주기)을, 플랜
/// 스레드가 API 값(5분 주기)을 넣는다. 어느 쪽이 이기는지는 여기서 한 번만 정한다:
/// **서버에서 더 최근에 받은 값**이다. 방금 턴이 돌았으면 rollout 이 API 보다 새것이라
/// 이기고, 턴이 없는 동안엔 API 가 이긴다 — 호출 순서와 무관하게 결과가 같다.
/// 같으면 덮어쓴다 — Claude 플랜 스레드는 같은 fetched_at 으로 리셋 시각만 갈아 끼워
/// 다시 부르기 때문이다.
fn set_plan(app: &AppHandle, plan: usage_core::plan::PlanUsage) {
    let all = {
        let state = app.state::<AppState>();
        let mut list = state.plan.lock().unwrap();
        match list.iter_mut().find(|p| p.source == plan.source) {
            Some(slot) if plan.fetched_at < slot.fetched_at => return,
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

/// 공식 플랜 한도 미터 폴링 (CLI 미설치·미로그인 시 소스별로 조용히 비활성)
/// — Claude 는 ① API 실시간 → ② 캐시 파일 → ③ 트랜스크립트 계산의 3단 사슬,
///   Codex 는 ① API 실시간 → ② rollout(사용량 스레드가 넣는다)의 2단 — 중재는 `set_plan`
/// + 블록 리셋 임박 시 캐릭터 대사 (설정 분 전, 0=끔, 리셋 시각당 1회)
fn spawn_plan_thread(app: AppHandle) {
    std::thread::spawn(move || {
        // 계정 발견이 별도 스레드에서 도는 중이라 첫 회차는 홈 목록이 비어 있다
        std::thread::sleep(Duration::from_secs(5));
        let mut last_notified_reset: Option<chrono::DateTime<Local>> = None;
        // 직전에 쓴 문구 — 후보가 둘 이상이면 연속 중복을 피한다 (프론트 speech.ts pick 과 동일 규칙)
        let mut last_notify_line: Option<String> = None;
        // 홈별 API 최근 성공값 — 호출 사이(5분) + 일시 실패(오프라인) 동안 유지한다.
        // 실패한 홈만 낡은 값으로 남고, 후보 선정(max fetched_at)이 알아서 가려낸다.
        let mut api_plans: std::collections::HashMap<PathBuf, usage_core::plan::PlanUsage> =
            std::collections::HashMap::new();
        let mut codex_api_plans: std::collections::HashMap<PathBuf, usage_core::plan::PlanUsage> =
            std::collections::HashMap::new();
        let mut last_fetch: Option<std::time::Instant> = None;

        loop {
            let now = Utc::now();
            let roots = enabled_roots(&app);
            let homes = roots.claude_homes;
            // Codex 는 트랜스크립트 루트가 곧 홈이다 (auth.json 도 거기 있다)
            let codex_homes = roots.codex;
            // ① 사용량 API 실시간 (plan.rs 모듈 주석) — 5분마다, 홈별로(=계정별로) 시도
            let due = last_fetch.map(|t| t.elapsed() >= PLAN_FETCH_INTERVAL).unwrap_or(true);
            if due && !(homes.is_empty() && codex_homes.is_empty()) {
                last_fetch = Some(std::time::Instant::now());
                for h in &homes {
                    if let Some(p) = usage_core::plan::fetch_claude_usage(h, now) {
                        api_plans.insert(h.clone(), p);
                    }
                }
                for h in &codex_homes {
                    if let Some(p) = usage_core::plan::fetch_codex_usage(h, now) {
                        codex_api_plans.insert(h.clone(), p);
                    }
                }
            }
            // Codex 는 여기서 API 성공값만 올린다 — rollout 값은 사용량 스레드가 계속
            // 넣고 있고, 누가 이기는지는 `set_plan` 의 최신 수신값 규칙이 정한다.
            if let Some(p) = codex_homes
                .iter()
                .filter_map(|h| codex_api_plans.get(h).cloned())
                .max_by_key(|p| p.fetched_at)
            {
                set_plan(&app, p);
            }
            // ② 캐시 파일도 매 회차 후보에 넣는다 — 후보 전체에서 서버에서 가장 최근에
            // 받아온 값을 쓴다 (Codex 가 `CodexAdapter::plan()` 에서 하는 것과 같은 규칙).
            // API 성공값은 fetched_at 이 방금이라 자연히 이기고, API 가 죽어 있으면
            // (오프라인·토큰 만료·macOS 키체인) CLI 가 갱신하는 캐시가 이긴다.
            let plan = homes
                .iter()
                .filter_map(|h| api_plans.get(h).cloned())
                .chain(homes.iter().filter_map(|h| usage_core::plan::read_utilization(h)))
                .max_by_key(|p| p.fetched_at);
            if let Some(mut plan) = plan {
                // ③ 굳은 캐시의 리셋 시각을 우리 계산으로 갈아 끼운다.
                //
                // API 가 살아 있으면 리셋이 과거일 일이 없어 여기 안 걸린다. 걸리는 건
                // API 가 물러난 채(오프라인·토큰 만료·macOS 키체인) 캐시마저 굳었을 때다
                // (실측: 세션이 도는데도 6시간 정지, 리셋은 2시간 전). 그때 "0분 남음" 을
                // 보여주면 지금 막 리셋된다는 거짓말이고, 창 길이가 `limits[]` 에 없어
                // 다음 리셋을 유도할 수도 없다. 대신 우리가 늘 읽는 트랜스크립트에서
                // 같은 값을 만든다 (`blocks` — 실측으로 공식 값과 일치 확인).
                //
                // **첫 미터만** 손댄다. 5시간 창은 자주 갈리지만 주간 창은 며칠이 남아 있어
                // 같은 캐시라도 아직 사실이고, 무엇보다 주간은 우리가 계산할 수 없다.
                if let Some(m) = plan.meters.first_mut() {
                    let expired = m.resets_at.map(|r| r <= now).unwrap_or(true);
                    if expired {
                        let computed =
                            { app.state::<AppState>().claude_reset.lock().unwrap().to_owned() };
                        if let Some(end) = computed {
                            m.resets_at = Some(end);
                            m.resets_computed = true;
                        }
                    }
                }
                set_plan(&app, plan.clone());

                // 리셋 임박 — OS 알림 대신 캐릭터가 직접 말한다.
                // 문구는 활성 캐릭터 팩의 speech.json → 기본 문구 → 내장 순
                // (대사는 캐릭터의 속성 — 프론트 Pet 의 폴백 체인과 동일 규칙)
                let (notify_min, custom_lines) = {
                    let state = app.state::<AppState>();
                    let model = state
                        .summary
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|sum| sum.last_model.clone());
                    let s = state.settings.lock().unwrap();
                    // 활성 팩 = 모델 → 캐릭터 규칙(최장 접두사) → 기본 팩 (프론트 resolvePack 과 동일)
                    let pack = model
                        .as_deref()
                        .and_then(|m| {
                            let mut best_len = 0usize;
                            let mut best: Option<&str> = None;
                            for r in &s.character_rules {
                                if r.pack.is_empty() {
                                    continue;
                                }
                                for p in r.prefixes.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                                    if m.starts_with(p) && p.len() > best_len {
                                        best_len = p.len();
                                        best = Some(r.pack.as_str());
                                    }
                                }
                            }
                            best.map(String::from)
                        })
                        .or_else(|| s.character_pack.clone());
                    let pack_lines = pack
                        .as_deref()
                        .and_then(crate::settings::load_pack_speech)
                        .and_then(|mut sp| sp.remove("resetNotify"))
                        .filter(|v| v.iter().any(|l| !l.trim().is_empty()));
                    let lines = pack_lines.or_else(|| s.speech_lines.get("resetNotify").cloned());
                    (s.reset_notify_minutes, lines)
                };
                if notify_min > 0 {
                    // 첫 미터 = 가장 짧은 창 = 지금 당장 걸리는 한도
                    if let Some(reset) = plan
                        .meters
                        .first()
                        .and_then(|m| m.resets_at)
                        .map(|t| t.with_timezone(&Local))
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

            // Claude 5시간 창의 끝을 트랜스크립트에서 계산해 둔다 — 어댑터가 여기 있어서
            // 여기서 재고, 쓰는 곳은 플랜 스레드다(굳은 캐시를 대체할 때만).
            // 창이 닫혀 있으면 None — 다음 창은 다음 메시지가 열어서 미리 알 수 없다.
            *app.state::<AppState>().claude_reset.lock().unwrap() = claude.session_reset(Utc::now());

            // Codex 공식 한도의 ② rollout 값 — 방금 읽은 파일에서 그냥 나온다.
            // 플랜 스레드의 ① API 값과 같은 슬롯을 놓고 경쟁하지만, `set_plan` 이
            // 서버 수신 시각으로 중재하므로 그냥 넣으면 된다.
            if let Some(p) = codex.plan() {
                set_plan(&app, p);
            }
            let pricing = PriceTable::with_overrides(
                price_override.as_deref().map(std::path::Path::new),
            );
            let now = Utc::now();
            let offset = *Local::now().offset();
            // 잔디 격자 기간은 보존기간에서 유도한다 (아는 범위를 넘겨 그리면 빈칸이 거짓말이 된다)
            let days = usage_core::aggregate::daily_window(retention_days);
            let mut summary = build_summary(&events, &statuses, &pricing, days, now, offset.into());
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
        // 턴 추적 — 파일에 적힌 턴 경계를 직접 읽는다. 소스마다 파일이 달라
        // 추적기도 따로지만 결과(`TurnPoll`)는 같은 모양이다.
        let mut turns = usage_core::codex::TurnWatcher::default();
        let mut agy_turns = usage_core::antigravity::TurnWatcher::default();
        // Claude 세션의 직전 회차 status. 저쪽은 턴 감시기가 아니라 레지스트리를 읽으므로
        // 완료를 여기서 가려낸다 — 세 소스를 프론트에 **같은 모양**으로 내보내기 위한 변환이고,
        // 소스별 지식은 프론트가 아니라 여기 남는다.
        let mut claude_prev: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        loop {
            // 라이브 세션도 활성 계정의 설치본만 본다 (계정을 끄면 그쪽 세션은 무시)
            let roots = enabled_roots(&app);

            let now = Utc::now();
            let mut live = read_live_state(&roots.sessions, now.timestamp_millis());

            // Claude 완료 판정 — **레지스트리에 남아 있으면서** status 가 허용목록으로
            // 바뀐 세션만 완료다. 목록에서 사라진 것은 완료가 아니다: 레지스트리가
            // `<pid>.json` 이라 프로세스가 끝나면 파일째 사라지고, 크래시도 (신선도로
            // 밀려나) 같은 모양이 된다. 둘 다 "턴이 끝났다"는 뜻이 아니다.
            let mut claude_now = std::collections::HashMap::new();
            for s in live.sessions.iter().filter(|s| s.source == Source::Claude && !s.id.is_empty())
            {
                claude_now.insert(s.id.clone(), s.status.clone());
            }
            for (id, status) in &claude_now {
                let Some(before) = claude_prev.get(id) else { continue };
                if usage_core::live::claude_turn_finished(before, status) {
                    live.completed.push(usage_core::live::CompletedSession {
                        source: Source::Claude,
                        id: id.clone(),
                    });
                }
            }
            claude_prev = claude_now;

            // Codex·Antigravity 는 턴 경계를 파일에서 **직접 읽는다**. 유도가 아니라
            // 사실이라 상태 이름도 Claude 와 같은 `busy` 를 쓴다.
            let poll = turns.poll(&roots.codex, now);
            let agy_poll = agy_turns.poll(&roots.antigravity, now);
            let turn_sessions = [
                (Source::Codex, &poll.running, &poll.completed),
                (Source::Antigravity, &agy_poll.running, &agy_poll.completed),
            ];
            for (source, running, completed) in turn_sessions {
                for id in running {
                    live.busy = true;
                    live.busy_count += 1;
                    live.sessions.push(usage_core::live::LiveSessionView {
                        source,
                        id: id.clone(),
                        name: id.chars().take(8).collect(),
                        status: "busy".into(),
                        cwd: String::new(),
                    });
                }
                for id in completed {
                    live.completed
                        .push(usage_core::live::CompletedSession { source, id: id.clone() });
                }
            }

            // 값이 안 바뀌면 안 보낸다. `completed` 는 회차 단위 신호라 이 비교에 걸릴까
            // 싶지만 안 걸린다 — 완료 회차의 직전 회차엔 그 세션이 `sessions` 에 들어 있어
            // 두 JSON 이 반드시 다르다. (비교 방식을 바꾸면 여기를 다시 봐야 한다.)
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

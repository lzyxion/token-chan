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
use usage_core::live::read_live_state;
use usage_core::pricing::PriceTable;
use usage_core::{build_summary, Source, SourceAdapter, TurnWatch, UsageEvent};

use crate::AppState;

const USAGE_INTERVAL: Duration = Duration::from_secs(10);
const LIVE_INTERVAL: Duration = Duration::from_secs(2);
/// 공식 플랜 한도 폴링 주기. 회차마다 나가는 건 홈별 캐시 파일 읽기뿐이라 값이 싸고,
/// 실제 API 호출은 사슬이 자기 간격([`usage_core::plan::FETCH_INTERVAL_SECS`])으로
/// 따로 제한한다 — 루프를 촘촘히 돌리는 건 리셋 임박 판정을 늦게 보지 않기 위해서다.
const PLAN_INTERVAL: Duration = Duration::from_secs(30);
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

/// 소스의 스캔 루트. 스레드들이 소스 불문 루프를 돌 때 이 매핑 하나만 소스를 안다.
fn roots_for<'a>(r: &'a EnabledRoots, s: Source) -> &'a Vec<PathBuf> {
    match s {
        Source::Claude => &r.claude,
        Source::Codex => &r.codex,
        Source::Antigravity => &r.antigravity,
    }
}

/// 소스 → 어댑터. 구체 타입 이름이 등장하는 유일한 곳 — 소스 추가는 여기와
/// usage-core 의 계약 가입([`usage_core::SourceAdapter`] 구현)으로 닫힌다.
fn make_adapter(source: Source, roots: Vec<PathBuf>) -> Box<dyn SourceAdapter> {
    match source {
        Source::Claude => Box::new(usage_core::claude::ClaudeAdapter::new(roots)),
        Source::Codex => Box::new(usage_core::codex::CodexAdapter::new(roots)),
        Source::Antigravity => Box::new(usage_core::antigravity::AntigravityAdapter::new(roots)),
    }
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
        // 사슬(① API → ② 파일 → ③ 계산)의 상태와 규칙은 usage-core 가 들고 있다
        // ([`usage_core::plan::PlanChain`]) — 여기는 홈 목록과 계산 리셋값을 넘기고
        // 결과를 슬롯에 올릴 뿐이다.
        let mut chain = usage_core::plan::PlanChain::default();

        loop {
            let now = Utc::now();
            let roots = enabled_roots(&app);
            // ③ 의 재료 — 사용량 스레드가 트랜스크립트에서 계산해 둔 5시간 창 종료
            let computed = { app.state::<AppState>().claude_reset.lock().unwrap().to_owned() };
            // Codex 는 트랜스크립트 루트가 곧 홈이다 (auth.json 도 거기 있다)
            let plans = chain.poll(&roots.claude_homes, &roots.codex, computed, now);
            for p in &plans {
                set_plan(&app, p.clone());
            }

            // 리셋 임박 기준은 5시간 창을 가진 Claude 미터다 (Codex 는 주간뿐이라 제외)
            if let Some(plan) = plans.iter().find(|p| p.source == Source::Claude) {
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
        // 소스 불문 어댑터 목록 — 이 스레드는 계약([`usage_core::SourceAdapter`])만 안다.
        // 루트는 설정(추가 스캔 경로)에 따라 달라지므로 첫 회차에 채워진다.
        let mut adapters: Vec<(Source, Vec<PathBuf>, Box<dyn SourceAdapter>)> =
            [Source::Claude, Source::Codex, Source::Antigravity]
                .map(|s| (s, vec![], make_adapter(s, vec![])))
                .into();
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
            for (source, cur, adapter) in adapters.iter_mut() {
                let want = roots_for(&roots, *source);
                if first || want != cur {
                    *cur = want.clone();
                    *adapter = make_adapter(*source, want.clone());
                }
            }
            first = false;
            let since = Utc::now() - chrono::Duration::days(retention_days as i64);

            let mut events: Vec<UsageEvent> = vec![];
            let mut statuses = vec![];
            for (source, _, adapter) in adapters.iter_mut() {
                let out = adapter.scan(since);
                events.extend(out.events);
                statuses.push((*source, out.status));
            }
            events.sort_by_key(|e| e.ts);

            // 소스별 능력은 계약의 `Option` 메서드로 온다 — 어느 소스가 주는지는 여기서
            // 특정하지 않는다 (지금은 리셋 계산 = Claude, 파일 한도 = Codex 뿐이다).
            //
            // 리셋 계산값을 쓰는 곳은 플랜 스레드다(굳은 캐시를 대체할 때만). 창이 닫혀
            // 있으면 None — 다음 창은 다음 메시지가 열어서 미리 알 수 없다.
            *app.state::<AppState>().claude_reset.lock().unwrap() =
                adapters.iter().find_map(|(_, _, a)| a.session_reset(Utc::now()));
            // 파일에 실려 온 공식 한도 — 플랜 스레드의 ① API 값과 같은 슬롯을 놓고
            // 경쟁하지만, `set_plan` 이 서버 수신 시각으로 중재하므로 그냥 넣으면 된다.
            for (_, _, adapter) in adapters.iter() {
                if let Some(p) = adapter.plan() {
                    set_plan(&app, p);
                }
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
            summary.contexts =
                adapters.iter().filter_map(|(_, _, a)| a.context(&pricing)).collect();
            // 최근 세션 — 소스별 목록을 합쳐 최근순 상위 N개만
            summary.sessions = usage_core::session::merge(
                adapters.iter().flat_map(|(_, _, a)| a.sessions()).collect(),
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
        // 턴 추적 — 파일에 적힌 턴 경계를 직접 읽는다. 소스마다 파일이 달라 추적기도
        // 따로지만 계약([`usage_core::TurnWatch`])이 같아 목록으로 돈다.
        // Claude 가 여기 없는 건 방식이 달라서다 — 레지스트리 status 직독 (아래).
        let mut watchers: Vec<(Source, Box<dyn TurnWatch>)> = vec![
            (Source::Codex, Box::new(usage_core::codex::TurnWatcher::default())),
            (Source::Antigravity, Box::new(usage_core::antigravity::TurnWatcher::default())),
        ];
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
            for (source, watcher) in watchers.iter_mut() {
                let poll = watcher.poll(roots_for(&roots, *source), now);
                for id in &poll.running {
                    live.busy = true;
                    live.busy_count += 1;
                    live.sessions.push(usage_core::live::LiveSessionView {
                        source: *source,
                        id: id.clone(),
                        name: id.chars().take(8).collect(),
                        status: "busy".into(),
                        cwd: String::new(),
                    });
                }
                for id in &poll.completed {
                    live.completed
                        .push(usage_core::live::CompletedSession { source: *source, id: id.clone() });
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

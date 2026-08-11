# Token Pet 🐱

AI CLI(Claude Code · Codex CLI · Antigravity CLI) 토큰 사용량을 **바탕화면 위 데스크톱 펫**으로 보여주는 Tauri 2 앱.

- 투명·항상 위·프레임 없는 창에 캐릭터만 떠 있음 (드래그로 이동)
- **상황이 생기면** 캐릭터가 말풍선으로 한마디 — 한도 경고·소진, 블록 리셋, 작업 시작/종료, 잠자기/깨기 (몇 초 뒤 자동으로 사라짐)
- **펫을 클릭하면** 폴짝 뛰며 지금 사용량을 말풍선으로 알려줌 (드래그는 그대로 이동)
- **펫 우클릭**(또는 트레이)으로 사용량 패널 — 오늘 사용량/비용, 플랜 한도 게이지, 5시간 블록, 모델 분포. 원하는 자리로 옮겨 두면 위치를 기억
- Claude Code 세션이 작업 중이면 캐릭터가 타자 모션, 블록 소진율 80%↑면 경고 표정, 30분 무활동이면 잠자기
- 시스템 트레이: 펫 보이기 / 사용량 패널 / 설정 / 종료

## 아키텍처

```
crates/usage-core   # 순수 Rust: 어댑터(claude/codex/antigravity) + 집계/단가/5h블록 (cargo test 가능)
src-tauri           # Tauri 앱: 창/트레이/IPC + 10초 폴링 스캔 스레드
src                 # React: Pet(캐릭터) + Speech(대사 말풍선) + UsagePanel(사용량) + SettingsPanel
```

### 데이터 소스별 상태

| 소스 | 방식 | 상태 |
|---|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` 재귀 파싱, `message.id` dedup, `<synthetic>` 필터. WSL이면 `/mnt/c/Users/*/.claude`도 병합. 라이브 상태는 `~/.claude/sessions/*.json` | ✅ 실데이터 검증 |
| Codex CLI | `~/.codex/{sessions,archived_sessions}/**/*.jsonl` 의 `token_count` 이벤트 (누적/델타) + 같은 이벤트의 `rate_limits`(공식 한도). `CODEX_HOME`이 설정돼 있으면 `~/.codex` **대신** 그 경로 | ✅ 실데이터 검증 |
| Antigravity CLI (`agy`) | `~/.gemini/antigravity-cli/conversations/<uuid>.db` — 대화 하나가 **SQLite 파일 하나**. `gen_metadata` 테이블의 protobuf blob 에서 요청별 토큰·컨텍스트를 읽고, 요청 id 로 dedup | ✅ 실데이터 검증 |

> **Gemini CLI → agy**: Gemini CLI 가 Antigravity CLI 로 전환되면서 어댑터도 교체했습니다.
> 텔레메트리를 켜야만 기록되던 예전과 달리 **설정 없이 항상** 남고, 컨텍스트까지 직접 알려줍니다.

비용은 `crates/usage-core/pricing/prices.json` 단가표(최장 접두사 매칭)로 계산. 설정의 `priceOverridePath`로 덮어쓰기 가능. `costUSD`는 어떤 CLI도 기록하지 않으므로 전부 추정치.

### 공식 플랜 한도 · 작업 중 감지

소스마다 얻는 경로가 달라서 **가능한 것과 아닌 것이 다릅니다.** 셋을 한 덩어리로 묶어 보여주면 거짓말이 되므로 소스별로 나눠 표시합니다.

| | 공식 한도 | 작업 중 감지 |
|---|---|---|
| Claude Code | ✅ `claude -p "/usage"` 출력 파싱 (5분 주기, 프로세스 1회 실행) | ✅ `~/.claude/sessions/*.json` + pid 생존 확인 |
| Codex CLI | ✅ **rollout 의 `payload.rate_limits`** — 이미 읽는 파일에 서버가 준 값이 들어 있어 프로세스가 필요 없고, 리셋도 문자열이 아니라 유닉스 타임스탬프로 정확히 옵니다 | ⚠️ rollout mtime 신선도로 유도 |
| Antigravity CLI | ❌ `quota_manager` 가 서버에서 받아 **메모리에만** 둡니다 (로그에 호출 기록만 있고 값이 없음) | ⚠️ DB·WAL mtime 신선도로 유도 |

Codex 의 창은 이름 없이 분으로만 옵니다 — 실측 free 플랜은 `window_minutes: 43200`(30일) 하나뿐이고 `secondary`가 null입니다. 유료 플랜은 5시간 + 주간 두 창이 오는 구조로 보여서, **창이 짧은 것부터** 정렬해 첫 미터가 늘 "지금 당장 걸리는 한도"가 되게 했습니다.

작업 중 유도는 마지막 쓰기가 45초 안이면 작업 중으로 봅니다. 두 CLI 모두 응답을 스트리밍하며 파일에 계속 덧쓰기 때문입니다. 정확한 신호가 아니라서 상태 이름도 `busy`가 아니라 `active`로 구분해 둡니다. AGY 에는 `presence/<uuid>.lock` 이 있어 그쪽이 나아 보이지만 **쓸 수 없습니다** — 실측에서 agy 프로세스가 하나도 없는데 lock 파일 4개가 그대로 남아 있었습니다(종료 시 정리 안 함).

### 캐릭터 옆 게이지

캐릭터 옆에 도넛 3개, 발밑에 가로 바 하나가 붙습니다 (설정에서 좌/우/끄기).

| | 의미 | 방향 |
|---|---|---|
| 세션 5h · 주간 · **컨텍스트** (도넛) | 소진율 % | 찰수록 나쁨 → 60%/85%에서 주황·빨강 |
| 리셋까지 (가로 바) | 5시간 블록 경과율 | 다 차면 리셋(=좋음) → 파랑 고정 |

컨텍스트 링은 **세 CLI를 모두 지원**하며, 가장 최근에 움직인 세션이 표시됩니다. 셋은 필요한 값을 주는 방식이 다릅니다:

| | Claude Code | Codex CLI | Antigravity CLI |
|---|---|---|---|
| 현재 컨텍스트 | 마지막 응답의 `input + cache_creation + cache_read + output` (유도) | `last_token_usage.total_tokens` (직접) | `1.9.10.1` (직접, 구성 내역까지) |
| 컨텍스트 창 | 단가표 `ctx` → 없으면 관측 승격 | `model_context_window` (직접) | `1.9.10.4` (직접, 실측 256,000) |
| compact | `compactMetadata` 로 실측 (횟수·버려진 양·잠정값 처리) | 미지원 | 미관측 |

Claude 쪽 유도 공식은 compact 직전 실측치(`compactMetadata.preTokens`)와 대조해 검증했고, 오차는 마지막 응답 뒤 사용자 메시지분(최대 한 턴)뿐입니다. Codex 의 `model_context_window` 는 모델 창 전체가 아니라 예비분을 뺀 **실효 창**입니다 (`models_cache.json` 의 272,000 × `effective_context_window_percent` 95% = 258,400 으로 확인).

agy 는 셋 중 가장 친절합니다 — 총계뿐 아니라 **구성 내역**까지 주는데, 실측에서 내역 합이 총계와 오차 없이 맞아떨어졌습니다 (System Prompt 9,870 + Tools 14,992 + Chat Messages 12,967 = 37,829). 다만 **대화의 첫 요청은 값이 엉터리**라(실측 164 — 시스템 프롬프트·툴 정의가 아직 안 잡힘) 컨텍스트 판정에서 제외합니다. 자세한 근거는 `crates/usage-core/src/context.rs` 와 `antigravity.rs` 주석 참고.

단가표에 `ctx` 가 없는 모델은 200k로 가정하되 실제 관측치가 그걸 넘으면 상위 구간으로 자동 승격되므로, 새 모델이 나와도 표를 고치지 않고 스스로 맞춰갑니다. 반대로 소스가 창을 직접 알려준 경우(Codex·agy)에는 그 값이 정답이라 승격하지 않습니다.

## 개발

요구사항: Rust(stable), pnpm, Linux는 Tauri 시스템 의존성:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

```sh
pnpm install
cargo test -p usage-core   # 코어 로직 테스트 (GUI 의존성 불필요)
pnpm tauri dev             # 앱 실행 (WSLg에서도 동작)
pnpm tauri build           # 패키징 (msi/dmg는 GitHub Actions 매트릭스 빌드 권장)
```

설정 파일: `~/.config/token-pet/settings.json` (펫 위치, 보존기간, 단가 오버라이드, 자동시작, 추가 스캔 경로)

### 연결된 계정

**펫 우클릭(또는 트레이) → 연결된 계정**에서 이 머신에 로그인된 CLI 계정을 확인하고, 계정 단위로 집계 포함 여부를 켜고 끕니다. 같은 계정을 여러 곳에 설치해 둔 경우 **한 줄로 묶입니다**:

```
연결된 계정 ▸  ✓ Claude · you@example.com (claude_max · default_claude_max_5x) · 설치 2곳
               ✓ Codex · you@example.com (ChatGPT 로그인) · 설치 2곳
               ✓ AGY · you@example.com (Google 로그인)
               ─────────
               Claude 홈 추가…   ← 폴더 선택 다이얼로그
               Codex 홈 추가…
               AGY 홈 추가…
               다시 검색
```

계정 식별에 쓰는 파일 (전부 로컬):

| 소스 | 파일 | 얻는 것 |
|---|---|---|
| Claude | `<홈>/.claude.json` — 일반 설정 파일 | `emailAddress`, `accountUuid`, 플랜, rate limit tier |
| Codex | `<홈>/auth.json` | `tokens.account_id`, `auth_mode`, `id_token`(JWT)의 email 클레임 |
| AGY | `<홈>/log/*.log` · `<홈>/cli.log` | `applyAuthResult: email=…, authMethod=…` |

**AGY 만 자격증명이 파일에 없습니다.** 토큰을 OS 키링에 넣기 때문입니다 (`keyringAuth: loaded token` / `ChainedAuth: authenticated via keyring`). 대신 로그인한 계정을 로그에 평문으로 남겨서, 거기서 이메일을 읽어 Claude·Codex와 똑같이 계정으로 묶습니다. 실측에서 로그 6개 중 0바이트짜리 1개를 뺀 5개 전부에 있었습니다. 로그를 하나도 못 읽으면 그때만 `installation_id`로 떨어져 설치본 단위가 됩니다.

참고로 `~/.gemini/oauth_creds.json`과 `google_account_id`는 **AGY 것이 아닙니다** — 구 Gemini CLI가 남긴 것이고(mtime 2025-07-03, AGY 실행에 안 건드려짐), 그 계정 id는 AGY DB 어디에도 없습니다.

설치본 발견은 표준 위치(홈·WSL·`CODEX_HOME`·직접 추가)에 더해 **마커 파일 스캔**을 씁니다. `%APPDATA%`·`%LOCALAPPDATA%` 등을 깊이 4로 훑으며 `auth.json`+`sessions/`(Codex), `.claude/projects`(Claude), `conversations/`+`installation_id`(agy)를 찾습니다 — 실측 0.8초, 시작 시 1회 + "다시 검색" 할 때만 돕니다.

**기본 포함 규칙**: 표준 위치에서 발견된 계정은 켜고, 마커 스캔으로만 나온 **처음 보는 계정은 꺼둡니다**. 오래된 백업이나 남의 계정이 조용히 합산되는 게 가장 나쁜 실패 모드라서입니다. 반면 이미 켜진 계정의 새 설치본은 그 계정에 합류하므로 자동으로 포함됩니다.

### 스캔 경로가 환경에 좌우되는 문제

자동 탐지는 **앱 프로세스의 환경**을 봅니다. 트레이나 자동시작으로 뜬 앱은 터미널의 `CODEX_HOME` 같은 변수를 물려받지 못해서, 그 경로에 세션이 있어도 존재 자체를 모릅니다. 같은 머신인데 실행 방식에 따라 집계 범위가 달라지는 겁니다.

그래서 설정 → 일반 → **데이터 소스 경로**에서 (1) 지금 스캔 중인 루트와 파일 수를 확인하고, (2) 빠진 홈을 직접 추가할 수 있습니다 (`extraClaudeHomes` · `extraCodexHomes` · `extraAntigravityHomes`). 설정에 적어 두면 실행 방식과 무관하게 항상 같은 범위를 봅니다.

**합쳐도 중복 집계되지 않습니다.** 세 어댑터 모두 파일이 아니라 이벤트 단위로 dedup 합니다 — Claude 는 `message.id`, Codex 는 `세션 id + 시각 + 사용량`, agy 는 `gen_metadata` 의 요청 id(실측 19건 전부 고유). 실제로 같은 rollout 이 `~/.codex` 와 `CODEX_HOME` 양쪽에 있던 상황에서 검증했습니다:

| | 스캔 루트 | 이벤트 |
|---|---|---|
| 자동 탐지만 | `~/.codex` | 3건 (다른 홈의 세션은 안 보임) |
| orca 홈 추가 | `~/.codex` + orca | **4건** (중복 1건은 한 번만) |
| 이미 잡힌 경로를 또 적음 | 그대로 2개 | 4건 (경로도 이벤트도 중복 없음) |

## 캐릭터 커스터마이징

캐릭터는 **상태별 이미지 한 장**으로만 그립니다. 코드로 그리는 캐릭터는 없고,
기본 캐릭터도 저장소에 들어 있는 이미지 팩(`src/assets/pet-default/`)입니다 — **그림 파일만 갈아끼우면
코드 수정 없이 캐릭터가 바뀝니다.**

기본 팩은 젤리 슬라임(SVG 7장)이고, 상태마다 몸 색·잔량·표정이 다릅니다 —
대기 민트 / 작업 시안(기포가 끓어오름) / 경고 주황 / 잠 남보라 / 소진 무채(녹아내림) /
초기화 형광민트 / 클릭 라임.

그림 위에는 앱이 얹는 연출이 더해집니다: 상태별 모션(숨쉬기·흔들림·폴짝),
소품 배지(`z` `!` `🪫` `✨`), 그리고 소진율에 따라 느려지는 호흡과 맺히는 땀방울.

트레이 → 설정 → 캐릭터에서 "폴더 열기" 후, 팩 폴더를 만들어 이미지를 넣으면 드롭다운에서 선택 가능:

```
<설정폴더>/token-pet/characters/
└─ my-cat/
   ├─ idle.gif        # 필수 — 평상시
   ├─ working.gif     # 작업 중 (없으면 idle)
   ├─ alert.gif       # 한도 경고 (없으면 idle)
   ├─ sleep.gif       # 잠자기 (없으면 idle)
   ├─ exhausted.gif   # 세션 한도 100% 소진 (없으면 idle)
   ├─ refreshed.gif   # 블록 초기화 직후 (없으면 idle)
   └─ poke.gif        # 클릭했을 때의 반응 (없으면 idle)
```

### 지원 이미지 형식

확장자 **`.gif` · `.webp` · `.apng` · `.png` · `.svg`** 다섯 가지만 인식합니다 (`src-tauri/src/commands.rs`의 `CHAR_EXTS`).

| 형식 | 확장자 | 애니메이션 | 비고 |
|---|---|---|---|
| GIF | `.gif` | ✅ | 256색 제한 |
| WebP | `.webp` | ✅ | 애니메이션 가능 · GIF보다 용량/색상 유리 (래스터 중 권장) |
| APNG | `.apng` | ✅ | 풀컬러 + 알파. 확장자가 `.png`면 정적 취급 |
| PNG | `.png` | ❌ | 정적 |
| SVG | `.svg` | ✅ | 벡터 — 배율 250%에서도 안 흐려지고 용량이 작음 (기본 팩이 이 형식). 애니메이션은 **파일 안에** 넣어야 함 |

- **탐색 우선순위는 위 표 순서**입니다. 한 상태에 여러 형식이 있으면 `idle.gif` → `idle.webp` → `idle.apng` → `idle.png` → `idle.svg` 중 **먼저 발견된 하나만** 사용됩니다.
- SVG는 `<img>`로 그려져 바깥 CSS가 내부에 닿지 않습니다. 움직임이 필요하면 SVG 파일 안에 `<style>`과 `@keyframes`를 직접 넣으세요 (기본 팩의 `working.svg` 기포가 그 예시).
- 파일명의 상태 이름은 위 트리의 7개와 **정확히** 일치해야 합니다. 예를 들어 `refresh.gif`는 인식되지 않습니다 — `refreshed.gif`가 맞습니다.
- **영상(mp4·webm·mov 등)은 지원하지 않습니다.** 이미지는 `<img>` 태그로 렌더링되므로 움직임이 필요하면 GIF / 애니메이션 WebP / APNG로 만들어 주세요.
- 파일당 **20MB 상한** (data URL로 인라인하므로 실제 여유는 이보다 작습니다)

### 이미지 제작 가이드

- **투명 배경 필수**
- 권장 256×256px 이상, 하단 중앙이 발 기준점
- 정적 PNG여도 상태별 모션(숨쉬기·타자·떨림)이 CSS로 적용됨

### 모델/벤더별 캐릭터

설정 → "모델별 캐릭터 규칙"에서 접두사 → 팩을 매핑하면 **지금 사용 중인 모델에 따라 캐릭터가 자동 전환**됩니다:

| 접두사 | 효과 |
|---|---|
| `claude` | Claude 모든 모델 (신규 모델 자동 포함) |
| `claude-opus` | Opus 계열만 예외 처리 (더 긴 접두사가 우선) |
| `gpt, o3, codex` | OpenAI 계열 (콤마로 복수 접두사) |

미매칭 모델은 기본 캐릭터로 폴백. "최근 관측된 모델" 칩을 클릭하면 규칙이 자동 추가됩니다.
상태(작업/경고/잠/소진/초기화)는 개별 on/off 가능합니다.

## 실데이터 진단

어댑터가 이 머신에서 실제로 뭘 읽는지 확인할 때 (GUI 없이 동작):

```sh
cargo run -p usage-core --example accounts     # 발견된 설치본·계정 묶음
cargo run -p usage-core --example scan         # Claude 모델별 합계
cargo run -p usage-core --example scan_codex   # Codex 요청별 사용량
cargo run -p usage-core --example scan_agy     # agy 요청별 사용량 + 컨텍스트
```

`agy` 는 별도 설정이 필요 없습니다 — 설치하고 한 번 쓰면 바로 잡힙니다.

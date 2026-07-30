# Token Pet 🐱

AI CLI(Claude Code · Codex CLI · Gemini CLI) 토큰 사용량을 **바탕화면 위 데스크톱 펫**으로 보여주는 Tauri 2 앱.

- 투명·항상 위·프레임 없는 창에 캐릭터만 떠 있음 (드래그로 이동)
- **마우스 호버 시** 말풍선으로 오늘 사용량/비용, 5시간 블록, 모델 분포 표시 (우클릭: 말풍선 고정)
- Claude Code 세션이 작업 중이면 캐릭터가 타자 모션, 블록 소진율 80%↑면 경고 표정, 30분 무활동이면 잠자기
- 시스템 트레이: 펫 보이기 / 말풍선 고정 / 자동 시작 / 종료

## 아키텍처

```
crates/usage-core   # 순수 Rust: 어댑터(claude/codex/gemini) + 집계/단가/5h블록 (cargo test 가능)
src-tauri           # Tauri 앱: 창/트레이/IPC + 10초 폴링 스캔 스레드
src                 # React: Pet(캐릭터) + Bubble(말풍선)
```

### 데이터 소스별 상태

| 소스 | 방식 | 상태 |
|---|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` 재귀 파싱, `message.id` dedup, `<synthetic>` 필터. WSL이면 `/mnt/c/Users/*/.claude`도 병합. 라이브 상태는 `~/.claude/sessions/*.json` | ✅ 실데이터 검증 |
| Codex CLI | `~/.codex/{sessions,archived_sessions}/**/*.jsonl` 의 `token_count` 이벤트 (누적/델타) | ⚠️ 문서 기반, 실데이터 미검증 |
| Gemini CLI | OTel 텔레메트리 outfile 파싱. 미설정 시 말풍선에 안내 표시 | ⚠️ 문서 기반, 실데이터 미검증 |

비용은 `crates/usage-core/pricing/prices.json` 단가표(최장 접두사 매칭)로 계산. 설정의 `priceOverridePath`로 덮어쓰기 가능. `costUSD`는 어떤 CLI도 기록하지 않으므로 전부 추정치.

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

설정 파일: `~/.config/token-pet/settings.json` (펫 위치, 보존기간, 단가 오버라이드, 자동시작)

## 캐릭터 커스터마이징

트레이 → 설정 → 캐릭터에서 "폴더 열기" 후, 팩 폴더를 만들어 이미지를 넣으면 드롭다운에서 선택 가능:

```
<설정폴더>/token-pet/characters/
└─ my-cat/
   ├─ idle.gif        # 필수 — 평상시
   ├─ working.gif     # 작업 중 (없으면 idle)
   ├─ alert.gif       # 한도 경고 (없으면 idle)
   ├─ sleep.gif       # 잠자기 (없으면 idle)
   ├─ exhausted.gif   # 세션 한도 100% 소진 (없으면 idle)
   └─ refreshed.gif   # 블록 초기화 직후 (없으면 idle)
```

- 포맷: GIF / 애니메이션 WebP / APNG / 정적 PNG (**투명 배경 필수**)
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

## Gemini 텔레메트리 활성화

Gemini CLI는 기본적으로 토큰을 로컬에 기록하지 않습니다. `~/.gemini/settings.json`:

```json
{ "telemetry": { "enabled": true, "target": "local", "outfile": "~/.gemini/telemetry.log" } }
```

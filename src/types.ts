// Rust usage-core의 Serialize 결과와 1:1 매칭 (snake_case 유지)

export type Source = "claude" | "codex" | "antigravity";

export interface Totals {
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
}

export type SourceStatus = { kind: "ok" } | { kind: "no_data" };

export interface SourceSummary {
  source: Source;
  label: string;
  status: SourceStatus;
  today: Totals;
  today_cost: number;
  cost_partial: boolean;
}

export interface ModelRow {
  model: string;
  source: Source;
  totals: Totals;
  cost: number;
  cost_known: boolean;
}

export interface DailyRow {
  date: string;
  totals: Totals;
  cost: number;
}

/** 가장 최근에 움직인 세션의 컨텍스트 창 사용량 (Claude·Codex 지원) */
export interface ContextState {
  source: Source;
  session: string;
  model: string;
  /** 현재 컨텍스트 크기 (토큰) */
  tokens: number;
  /** 분모로 쓴 컨텍스트 창 */
  window: number;
  used_pct: number;
  at: string | null;
  /** compact 직후 다음 턴이 아직 없어 잠정값을 쓰는 중 */
  interim: boolean;
  /** 창 크기가 단가표가 아니라 관측치로 승격된 값인지 */
  window_inferred: boolean;
  compactions: number;
  /** compact 로 버려진 누적 토큰 */
  dropped: number;
  /** 버려진 분량까지 합친 이 세션의 실제 대화 총량 */
  total: number;
  last_compact_at: string | null;
  last_compact_trigger: string | null;
}

/** 최근 세션 한 줄 — 어느 프로젝트에서 얼마나 태웠나 */
export interface SessionRow {
  source: Source;
  id: string;
  /** agy 는 대화 제목(첫 사용자 메시지), 나머지는 작업 폴더 이름 */
  label: string;
  cwd: string;
  model: string;
  /** git 브랜치 — Claude 만 알려준다 */
  branch: string;
  at: string;
  tokens: number;
}

export interface Summary {
  generated_at: string;
  today_date: string;
  today: Totals;
  today_cost: number;
  cost_partial: boolean;
  sources: SourceSummary[];
  models_today: ModelRow[];
  daily: DailyRow[];
  /** 스캔 범위에서 가장 오래된 이벤트 — 잔디의 "기록 없음" 경계 */
  first_event_ts: string | null;
  last_event_ts: string | null;
  /** 최근 메인체인 이벤트의 모델 (활성 모델) */
  last_model: string | null;
  /** 스캔 범위에서 관측된 모델명 목록 */
  observed_models: string[];
  /** 소스별 활성 세션의 컨텍스트 사용량 (최근 세션이 있는 소스만) */
  contexts: ContextState[];
  /** 최근 세션 (소스 합쳐 최근순) */
  sessions: SessionRow[];
}

export interface LiveSessionView {
  source: Source;
  name: string;
  /** `busy`/`idle` 은 세션 레지스트리의 정확한 값, `active` 는 파일 신선도로 유도한 값 */
  status: string;
  cwd: string;
}

export interface LiveState {
  busy: boolean;
  busy_count: number;
  sessions: LiveSessionView[];
}

export interface PlanMeter {
  label: string;
  used_pct: number;
  /** 리셋 시각 원문 (Claude). 소스가 문자열로만 줄 때 쓴다 */
  resets: string;
  /** 소스가 정확한 시각을 준 경우 (Codex) — 있으면 `resets` 파싱이 필요 없다 */
  resets_at: string | null;
}

/** 소스 하나의 공식 한도. Claude 는 `claude -p /usage`, Codex 는 rollout 의 `rate_limits` */
export interface PlanUsage {
  source: Source;
  meters: PlanMeter[];
  /** 플랜 종류 등 (예: "free 플랜") */
  detail: string;
  fetched_at: string;
}

export type PetState =
  | "idle"
  | "working"
  | "alert"
  | "sleep"
  | "exhausted"
  | "refreshed"
  /** 클릭했을 때의 짧은 반응 (사용량을 말풍선으로 알려줌) */
  | "poke";

/** Rust settings::Settings (serde camelCase) */
export interface AppSettings {
  petPos: [number, number] | null;
  retentionDays: number;
  alertThreshold: number;
  priceOverridePath: string | null;
  autostart: boolean;
  petScale: number;
  weeklyAlertThreshold: number;
  resetNotifyMinutes: number;
  clickThrough: boolean;
  panelPos: [number, number] | null;
  panelSize: [number, number] | null;
  settingsSize: [number, number] | null;
  speechEnabled: boolean;
  speechDurationMs: number;
  startHidden: boolean;
  characterPack: string | null;
  sleepAfterMinutes: number;
  characterRules: CharacterRule[];
  disabledStates: string[];
  showMiniLabel: boolean;
  gaugeSide: GaugeSide;
  /** 상황 키("enter.working"·"poke"·"resetNotify" 등) → 사용자 문구 목록 (비면 기본 문구) */
  speechLines: Record<string, string[]>;
  /** 이름 붙은 문구 세트(모델별 말투) — 세트에 없는 상황은 speechLines → 내장 기본 순 폴백 */
  speechSets: Record<string, Record<string, string[]>>;
  /** 모델 접두사 → 문구 세트 매핑 (최장 접두사 우선, 미매칭 시 기본 문구) */
  speechRules: SpeechRule[];
  /** 추가로 스캔할 Claude 홈(`.claude` 디렉토리) — 자동 탐지가 환경에 좌우되는 걸 보완 */
  extraClaudeHomes: string[];
  /** 추가로 스캔할 Codex 홈(`CODEX_HOME` 에 해당) */
  extraCodexHomes: string[];
  /** 추가로 스캔할 agy 홈(`antigravity-cli` 디렉토리) */
  extraAntigravityHomes: string[];
  /** 게이지에 태울 벤더 — 링이 3개뿐이라 한 벤더만 보여준다 */
  gaugeVendor: "auto" | Source;
}

/** 모델 접두사(콤마 구분) → 문구 세트 매핑 규칙 (캐릭터 규칙과 같은 방식) */
export interface SpeechRule {
  prefixes: string;
  set: string;
}

/** 도넛 게이지 위치 — off면 표시하지 않음 */
export type GaugeSide = "right" | "left" | "off";

/** 모델 접두사(콤마 구분) → 캐릭터 팩 매핑 규칙 (최장 접두사 우선) */
export interface CharacterRule {
  prefixes: string;
  pack: string;
}

/** 상태 → data URL (팩 미선택 시 null) */
export type CharacterImages = Record<string, string>;

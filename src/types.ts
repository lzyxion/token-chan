// Rust usage-core의 Serialize 결과와 1:1 매칭 (snake_case 유지)

export type Source = "claude" | "codex" | "gemini";

export interface Totals {
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
}

export type SourceStatus =
  | { kind: "ok" }
  | { kind: "no_data" }
  | { kind: "needs_setup"; guide: string };

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

export interface BlockSummary {
  start: string;
  end: string;
  totals: Totals;
  cost: number;
  cost_partial: boolean;
  remaining_minutes: number;
  time_ratio: number;
  token_ratio: number | null;
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
  block: BlockSummary | null;
  last_event_ts: string | null;
  /** 최근 메인체인 이벤트의 모델 (활성 모델) */
  last_model: string | null;
  /** 스캔 범위에서 관측된 모델명 목록 */
  observed_models: string[];
}

export interface LiveSessionView {
  name: string;
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
  resets: string;
}

export interface PlanUsage {
  meters: PlanMeter[];
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

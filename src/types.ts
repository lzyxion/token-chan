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

export type PetState = "idle" | "working" | "alert" | "sleep" | "exhausted" | "refreshed";

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
  hoverDelayMs: number;
  startHidden: boolean;
  characterPack: string | null;
  sleepAfterMinutes: number;
  characterRules: CharacterRule[];
  disabledStates: string[];
  showMiniLabel: boolean;
}

/** 모델 접두사(콤마 구분) → 캐릭터 팩 매핑 규칙 (최장 접두사 우선) */
export interface CharacterRule {
  prefixes: string;
  pack: string;
}

/** 상태 → data URL (팩 미선택 시 null) */
export type CharacterImages = Record<string, string>;

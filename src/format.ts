import type { Source, Totals } from "./types";

/** 소스 id → 화면 표기. 게이지·패널·설정이 같은 이름을 써야 벤더를 헷갈리지 않는다. */
export const SOURCE_LABEL: Record<Source, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "AGY",
};

/** 1234567 → "1.2M", 84500 → "84.5K" */
export function fmtTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

export function totalOf(t: Totals): number {
  return t.input + t.output + t.cache_write + t.cache_read;
}

export function fmtCost(usd: number, partial = false): string {
  const s = usd >= 100 ? `$${usd.toFixed(0)}` : `$${usd.toFixed(2)}`;
  return partial ? `${s}+` : s;
}

/** 분 → "2h 13m" */
export function fmtMinutes(min: number): string {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h <= 0) return `${m}m`;
  return `${h}h ${m}m`;
}

/**
 * 리셋까지 남은 시간 — 절대 시각(`9/10 10:46`)보다 행동에 가깝고 폭도 절반이라
 * 좁은 카드에 들어간다. 창 길이가 5시간부터 30일까지라 단위를 눈금에 맞춰 바꾼다.
 */
export function fmtRemaining(target: Date, now: Date = new Date()): string {
  const min = Math.max(0, Math.round((target.getTime() - now.getTime()) / 60000));
  if (min < 60) return `${min}분`;
  const h = Math.floor(min / 60);
  if (h < 24) {
    const m = min % 60;
    return m ? `${h}시간 ${m}분` : `${h}시간`;
  }
  const d = Math.floor(h / 24);
  const rh = h % 24;
  // 일 단위가 커지면 시간은 노이즈다 (월간 창에서 "29일 3시간"은 읽을 이유가 없다)
  if (d >= 7 || !rh) return `${d}일`;
  return `${d}일 ${rh}시간`;
}

const MONTHS: Record<string, number> = {
  Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5,
  Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11,
};

/**
 * 공식 /usage 리셋 문자열("Jul 30, 7:09pm (Asia/Seoul)", "Aug 1, 3pm (…)")을
 * 로컬 타임존 Date로 파싱. 연도는 현재 연도로 가정, 과거로 나오면 +1년 (연말 경계).
 * 형식이 다르면 null (호출부는 로컬 추정치로 폴백).
 */
export function parseResetTime(resets: string, now: Date = new Date()): Date | null {
  const m = resets.match(/([A-Z][a-z]{2})\s+(\d{1,2}),?\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)/i);
  if (!m) return null;
  const month = MONTHS[m[1] as keyof typeof MONTHS];
  if (month == null) return null;
  const day = parseInt(m[2], 10);
  let hour = parseInt(m[3], 10) % 12;
  if (m[5].toLowerCase() === "pm") hour += 12;
  const minute = m[4] ? parseInt(m[4], 10) : 0;
  let d = new Date(now.getFullYear(), month, day, hour, minute);
  // 12시간 이상 과거면 내년으로 (연말 경계)
  if (d.getTime() < now.getTime() - 12 * 3600 * 1000) {
    d = new Date(now.getFullYear() + 1, month, day, hour, minute);
  }
  return d;
}

/** 얼마나 지났는지 — 세션 로그는 "언제였나"보다 "얼마나 전인가"가 읽기 쉽다 */
export function fmtAgo(at: string, now: Date = new Date()): string {
  const d = new Date(at);
  const min = Math.floor((now.getTime() - d.getTime()) / 60000);
  if (min < 1) return "방금";
  if (min < 60) return `${min}분 전`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}시간 전`;
  // 하루가 넘어가면 상대 표기가 오히려 헷갈린다 ("3일 전"이 며칠인지 세게 됨)
  const days = Math.floor(h / 24);
  if (days === 1) return "어제";
  if (days < 7) return `${days}일 전`;
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

/** 모델 ID를 짧은 표시명으로 (claude-opus-4-8 → opus-4-8) */
export function shortModel(model: string): string {
  return model
    .replace(/^claude-/, "")
    .replace(/-\d{8,}$/, "") // 날짜 접미사 제거
    .replace(/^gemini-/, "gem-");
}

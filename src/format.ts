import type { Totals } from "./types";

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

/** 모델 ID를 짧은 표시명으로 (claude-opus-4-8 → opus-4-8) */
export function shortModel(model: string): string {
  return model
    .replace(/^claude-/, "")
    .replace(/-\d{8,}$/, "") // 날짜 접미사 제거
    .replace(/^gemini-/, "gem-");
}

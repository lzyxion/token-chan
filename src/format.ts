import type { Source, Totals } from "./types";

/** 소스 id → 화면 표기. 패널·설정·대사가 같은 이름을 써야 벤더를 헷갈리지 않는다. */
export const SOURCE_LABEL: Record<Source, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "Antigravity",
};

/** 캐릭터 옆 게이지 라벨 전용 짧은 표기.
 *
 * 게이지는 캐릭터 옆에 붙는 좁은 열이라 이름이 길면 라벨이 캐릭터 폭을 넘어간다.
 * 여기서만 줄이고 나머지(패널·설정·말풍선)는 정식 이름을 쓴다. */
export const SOURCE_SHORT: Record<Source, string> = {
  ...SOURCE_LABEL,
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

/** 비용 표기 통화. 환율은 설정에서 직접 넣는다 — 이 앱은 네트워크를 쓰지 않는다. */
export interface Currency {
  code: "usd" | "krw";
  /** 1 USD = ? 원 */
  usdToKrw: number;
}

/** 설정을 아직 못 읽었을 때 쓰는 값 — 곱하지 않은 달러가 안전한 기본값이다 */
export const DEFAULT_CURRENCY: Currency = { code: "usd", usdToKrw: 1400 };

/**
 * 비용 표기. `costUSD` 를 기록하는 CLI 가 없어 **모두 단가표 추정치**이고,
 * 원 표기는 거기에 환율까지 곱한 값이다. 그래서 자릿수를 늘려도 정밀해지지 않는다 —
 * 달러는 $100 부터 소수점을 떼고, 원은 전(錢) 단위가 없어 애초에 정수로만 적는다.
 */
export function fmtCost(usd: number, partial = false, cur: Currency = DEFAULT_CURRENCY): string {
  const s =
    cur.code === "krw"
      ? `₩${Math.round(usd * cur.usdToKrw).toLocaleString("ko-KR")}`
      : usd >= 100
        ? `$${usd.toFixed(0)}`
        : `$${usd.toFixed(2)}`;
  return partial ? `${s}+` : s;
}

/**
 * 분 → "41m" · "5h 41m" · "6d 5h".
 *
 * 창이 5시간부터 30일까지라 `h`만 쓰면 "149h 41m" 같은 값이 나온다 — 며칠인지 알려면
 * 24로 나눠야 해서 한눈에 안 읽힌다. 단위를 눈금에 맞춰 바꾸고 **두 자리까지만** 쓴다
 * (게이지 한 줄에 라벨·%와 같이 들어가므로 폭이 곧 비용이다).
 * 큰 단위가 커지면 작은 단위는 노이즈다 — 월간 창의 "29d 3h" 는 3h 를 읽을 이유가 없다.
 */
export function fmtMinutes(min: number): string {
  const h = Math.floor(min / 60);
  if (h <= 0) return `${min}m`;
  if (h < 24) {
    const m = min % 60;
    return m ? `${h}h ${m}m` : `${h}h`;
  }
  const d = Math.floor(h / 24);
  const rh = h % 24;
  return d >= 7 || !rh ? `${d}d` : `${d}d ${rh}h`;
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

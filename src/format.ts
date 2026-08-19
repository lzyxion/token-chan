import type { Account, GaugeStyle, Source, Totals } from "./types";

/** 다루는 소스 전부 — 화면에 늘어놓는 순서이기도 하다 (설정의 홈 목록·게이지 순환).
 *  목록을 화면마다 따로 들면 벤더를 추가할 때 한 곳이 조용히 빠진다. */
export const SOURCES: Source[] = ["claude", "codex", "antigravity"];

/** 고를 수 있는 게이지 모양 — 백엔드 `settings::GAUGE_STYLES` 와 **같은 목록·같은 순서**.
 *  첫 항목이 기본값이다. */
export const GAUGE_STYLES: GaugeStyle[] = ["ring", "bar", "orb"];

/** 게이지 모양 → 설정 화면 표기 */
export const GAUGE_STYLE_LABEL: Record<GaugeStyle, string> = {
  ring: "도넛 링",
  bar: "HP 바 (RPG)",
  orb: "물방울",
};

/**
 * 모르는 값을 기본 모양으로 떨어뜨린다. 설정 파일은 사람이 고치는 JSON 이고, 옛
 * 버전에서 저장된 모양이 남아 있을 수도 있다 — 백엔드도 같은 규칙으로 거른다.
 *
 * 예전엔 호출부마다 `x === "bar" ? "bar" : "ring"` 이었다. 모양을 하나 늘렸을 때
 * 저장·검증은 통과하는데 **화면만 조용히 기본 모양으로 되돌아가** 원인을 못 찾는다.
 */
export function gaugeStyleOf(v: unknown): GaugeStyle {
  return GAUGE_STYLES.includes(v as GaugeStyle) ? (v as GaugeStyle) : GAUGE_STYLES[0];
}

/**
 * 게이지에 실을 수 있는 벤더 — 켜 둔 계정이 하나라도 있는 소스.
 *
 * 꺼 둔 계정의 벤더는 스캔 자체를 안 하므로(`monitor::enabled_roots`) 고를 수는 있어도
 * 게이지가 빈 링만 그린다. `accounts` 가 아직 없으면(null) 판단을 미루고 null 을
 * 돌려준다 — 호출부가 "전부 보여주기" 로 떨어지게 해서, 로딩 한순간에 항목이
 * 사라졌다 나타나는 깜빡임을 막는다.
 *
 * 포함 여부(`Account.enabled`)는 백엔드가 규칙을 풀어서 준 값이다. 여기서 다시
 * 계산하지 않는다 — 두 곳이 어긋나면 화면과 스캔 대상이 갈린다.
 */
export function enabledSources(accounts: Account[] | null): Source[] | null {
  if (!accounts) return null;
  return SOURCES.filter((src) => accounts.some((a) => a.source === src && a.enabled));
}

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

/** 소진율 위험 단계 — CSS 클래스(`.meter.ok` 등)와 이름이 같다 */
export type MeterLevel = "ok" | "warn" | "danger";

/**
 * 위험 한도 (%). **설정(알림 탭)이 정하는 값**이다.
 *
 * 둘로 나뉘는 이유는 성격이 다르기 때문이다 — 공식 한도는 넘으면 **기다려야** 하고,
 * 컨텍스트는 차면 대화가 **잘린다**. 반면 5시간·주간·월간은 셋 다 "기다려야 한다"는
 * 같은 성격이라 하나로 묶는다: 어느 미터가 주간인지는 소스와 플랜마다 달라
 * (Codex 는 창이 하나뿐인 플랜이 있다) 나눠 두면 어느 설정이 걸리는지 화면에서
 * 구분되지 않았다.
 */
export interface AlertThresholds {
  /** 공식 한도 미터 전부 (5시간·주간·월간·모델별 주간) — `alertThreshold` */
  plan: number;
  /** 컨텍스트 — `contextAlertThreshold` */
  context: number;
}

/** 설정을 아직 못 읽었을 때 (백엔드 `Settings::default` 와 같은 값) */
export const DEFAULT_THRESHOLDS: AlertThresholds = {
  plan: 80,
  context: 90,
};

/**
 * 노랑이 켜지는 지점 = 위험 한도의 이 비율.
 *
 * 단계는 셋인데 사용자가 정하는 선은 **하나**다. 그 하나는 빨강에 준다 —
 * 색이 빨개지는 순간과 펫이 경고하는 순간이 어긋나면 안 되기 때문이다
 * (`Pet.tsx` 의 alert 판정이 같은 값을 쓴다). 노랑은 그 앞의 예고고,
 * 비율로 두면 한도를 낮게 잡은 사람도 예고 구간을 그대로 갖는다.
 */
export const WARN_RATIO = 0.75;

/**
 * 소진율(%) → 위험 단계. `dangerPct` 는 그 미터의 위험 한도(%)다.
 *
 * 예전에는 60/85 고정이었는데, 그러면 세션 한도를 75% 로 낮춰 둔 사람은
 * 펫이 경고하는 동안에도 게이지가 노란색이었다 — 설정이 색에 닿지 않았다.
 */
export function meterLevel(pct: number, dangerPct: number): MeterLevel {
  if (pct >= dangerPct) return "danger";
  if (pct >= dangerPct * WARN_RATIO) return "warn";
  return "ok";
}


/**
 * 소진율(%) → 색. 값이 아니라 **CSS 변수 참조**를 돌려준다 (`styles.css` `:root`).
 *
 * 게이지는 `conic-gradient` 를 인라인 style 로 그려야 해서 JS 가 색을 넘겨야 하는데,
 * 여기서 hex 를 들고 있으면 같은 색이 CSS 와 TS 두 곳에 적힌다 — 실제로 그랬고,
 * 주석으로 "같은 기준"이라고 약속해 두는 방식이었다. `var()` 를 넘기면 정의는
 * CSS 한 곳에 남고 약속이 코드가 된다.
 */
export function meterColor(pct: number, dangerPct: number): string {
  return `var(--${meterLevel(pct, dangerPct)})`;
}

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
 * 초 → "45초" · "3분 12초" · "1시간 5분".
 *
 * `fmtMinutes` 와 따로 두는 이유는 **다루는 눈금이 다르기 때문**이다. 저쪽은 5시간~30일
 * 창의 잔여라 분 아래가 노이즈지만, 여기는 한 턴이 걸린 시간이라 값의 대부분이 분 아래에서
 * 끝난다 — 초를 버리면 거의 모든 턴이 "0분"이 된다.
 * 말풍선에 들어가는 값이라 폭보다 읽기 쉬움을 택해 단위를 한글로 쓴다.
 */
export function fmtDuration(sec: number): string {
  const total = Math.max(0, Math.round(sec));
  if (total < 60) return `${total}초`;
  const min = Math.floor(total / 60);
  if (min < 60) {
    const s = total % 60;
    return s ? `${min}분 ${s}초` : `${min}분`;
  }
  const h = Math.floor(min / 60);
  const m = min % 60;
  return m ? `${h}시간 ${m}분` : `${h}시간`;
}

/**
 * 리셋 시각이 **이미 지났는가** = 그 미터가 낡았는가.
 *
 * 공식 한도는 CLI 가 캐시에 적어 둔 값을 읽는 것이라(README 참고) CLI 가 갱신을 멈추면
 * 그대로 굳는다. 실측(2026-08-13): `fetchedAtMs` 가 6시간 전이고 5시간 창의 `resets_at`
 * 이 **2시간 전**이었다 — 창은 이미 갈렸는데 캐시는 옛 창을 가리키고 있었다.
 * (`.claude.json` 자체는 43분 전에 쓰였다. 다른 필드는 갱신되는데 이 캐시만 굳는다.)
 *
 * 그때 남은 시간을 0으로 깎아 보여주면 **"지금 막 리셋된다"는 거짓말**이 된다. 게다가
 * 다음 리셋을 계산할 수도 없다 — Claude 의 `limits[]` 는 창 길이를 주지 않는다(Codex 는
 * `window_minutes` 를 주지만 두 소스가 같은 모양이어야 한다). 그래서 모른다고 말한다.
 *
 * 소진율(%)은 계속 보여준다. 그쪽은 옛 창의 값이라 **실제보다 높게** 나오는데, 한도
 * 게이지에서 높게 틀리는 건 안전한 방향이고 숨기면 링이 통째로 사라진다.
 */
export function resetIsStale(target: Date, now: Date = new Date()): boolean {
  return target.getTime() <= now.getTime();
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

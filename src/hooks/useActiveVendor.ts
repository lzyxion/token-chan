import { useEffect, useRef, useState } from "react";
import type { ContextState, LiveState, PlanUsage, Source, Summary } from "../types";

/**
 * 벤더를 바꾼 뒤 이 시간 안에는 다시 바꾸지 않는다.
 * 두 CLI 를 번갈아 쓰면 근거(작업 중·마지막 활동)가 초 단위로 뒤집혀서, 이게 없으면
 * 게이지가 깜빡이며 어느 쪽 숫자를 보는지 알 수 없게 된다.
 */
const SWITCH_HOLD_MS = 5000;

/** 게이지가 보여줄 벤더와 그 벤더의 값 묶음 */
export interface ActiveVendor {
  source: Source;
  /** 그 벤더의 컨텍스트 (세션이 없으면 null) */
  context: ContextState | null;
  /** 그 벤더의 공식 한도 (없는 벤더도 있다 — agy) */
  plan: PlanUsage | null;
  /** 지금 실제로 돌고 있는지 */
  busy: boolean;
  /** 근거가 하나도 없어 마지막으로 알던 벤더를 계속 쓰는 중인지 */
  stale: boolean;
}

/**
 * ISO 시각 → 비교 가능한 수. 없거나 못 읽으면 가장 오래된 것으로 친다.
 * 문자열끼리 비교하면 안 된다 — 소수부 자릿수가 값마다 달라서
 * `…:00.5Z` 와 `…:00.5000001Z` 의 앞뒤가 뒤집힌다.
 */
function ts(at: string | null | undefined): number {
  if (!at) return -Infinity;
  const t = Date.parse(at);
  return Number.isNaN(t) ? -Infinity : t;
}

/** 오늘 가장 많이 쓴 벤더 — 다른 근거가 전부 없을 때의 마지막 기준 */
function topBySpend(summary: Summary | null): Source | null {
  if (!summary) return null;
  let best: Source | null = null;
  let bestTotal = 0;
  for (const s of summary.sources) {
    const total = s.today.input + s.today.output + s.today.cache_write + s.today.cache_read;
    if (total > bestTotal) {
      bestTotal = total;
      best = s.source;
    }
  }
  return best;
}

/**
 * 게이지에 태울 벤더를 고른다.
 *
 * 우선순위: 지금 작업 중 → 마지막으로 움직인 세션 → 오늘 사용량 1위 → 직전 선택 유지.
 * "작업 중"을 맨 위에 두는 이유는 게이지가 답하는 질문이 "지금 이 세션이 얼마나
 * 남았나"이기 때문이다. **여럿이 동시에 작업 중이면 마지막으로 쓴 쪽**이다.
 * 사용자가 설정에서 벤더를 고정하면 그 값이 전부 무시된다.
 */
export function useActiveVendor(
  summary: Summary | null,
  live: LiveState,
  plans: PlanUsage[],
  pinned: "auto" | Source,
): ActiveVendor | null {
  const [source, setSource] = useState<Source | null>(null);
  // 마지막으로 벤더를 바꾼 시각 — 히스테리시스 기준
  const switchedAt = useRef(0);
  // 홀드가 끝나면 바꿀 예정 — 이 동안은 근거가 있는 상태라 stale 이 아니다
  const [pending, setPending] = useState(false);

  // 지금 작업 중인 벤더들. live.sessions 에는 idle 세션도 실려 오므로 상태로 거른다.
  const busySet = new Set<Source>();
  for (const s of live.sessions) {
    if (s.status === "busy") busySet.add(s.source);
  }
  const contextAt = new Map<Source, number>();
  for (const c of summary?.contexts ?? []) contextAt.set(c.source, ts(c.at));
  // 여럿이 동시에 돌면 **마지막으로 쓴 쪽**을 고른다. 배열 순서로 집으면 그건 활동 순이
  // 아니라 `read_live_state` 가 소스를 넣는 순서(항상 Claude 먼저)라, 게이지가 답해야 할
  // "지금 이 세션이 얼마나 남았나" 와 무관한 벤더가 뽑힌다.
  // Set 은 삽입 순서를 지키므로 시각을 모르거나 동률이면 예전 동작(배열 순서)으로 떨어진다.
  let busySource: Source | null = null;
  for (const s of busySet) {
    const cur = contextAt.get(s) ?? -Infinity;
    if (busySource === null || cur > (contextAt.get(busySource) ?? -Infinity)) {
      busySource = s;
    }
  }
  const latestContext =
    summary?.contexts?.length
      ? summary.contexts.reduce((a, b) => (ts(a.at) >= ts(b.at) ? a : b))
      : null;
  // 마지막 폴백으로 "한도를 아는 벤더"까지 본다. 오늘 아직 아무것도 안 썼어도
  // 플랜 게이지는 보여줘야 하는데, 이게 없으면 게이지가 통째로 사라진다.
  const candidate = busySource ?? latestContext?.source ?? topBySpend(summary) ?? plans[0]?.source;

  useEffect(() => {
    if (pinned !== "auto" || !candidate || candidate === source) {
      setPending(false);
      return;
    }
    // 첫 선택은 즉시, 이후 전환만 붙잡아 둔다
    const wait = source === null ? 0 : SWITCH_HOLD_MS - (Date.now() - switchedAt.current);
    if (wait <= 0) {
      setPending(false);
      switchedAt.current = Date.now();
      setSource(candidate);
      return;
    }
    // 홀드가 남았으면 **버리지 말고 미룬다.** 그냥 return 하면 candidate·source 가 그대로라
    // effect 가 다시 돌지 않아, 홀드에 걸린 전환이 영영 폐기되고 게이지가 옛 벤더에 붙박인다.
    setPending(true);
    const timer = setTimeout(() => {
      switchedAt.current = Date.now();
      setSource(candidate);
    }, wait);
    return () => clearTimeout(timer);
  }, [candidate, source, pinned]);

  const shown = pinned === "auto" ? source : pinned;
  if (!shown) return null;

  return {
    source: shown,
    context: summary?.contexts?.find((c) => c.source === shown) ?? null,
    plan: plans.find((p) => p.source === shown) ?? null,
    // 고정 중이어도 그 벤더가 실제로 도는지는 그대로 알려준다 —
    // 동시에 둘이 돌 때 busySource 는 하나뿐이라 그걸로 판정하면 고정한 쪽을 놓친다
    busy: busySet.has(shown),
    // 근거가 사라졌는데 예전 선택을 계속 쓰는 중 (세션이 다 끝난 상태).
    // 홀드 대기 중은 근거가 멀쩡히 있는 상태라 제외한다 — 안 그러면 전환 직전 5초 동안
    // 로고가 흐려져 "세션이 끝났다" 로 잘못 읽힌다.
    stale: pinned === "auto" && candidate !== shown && !pending,
  };
}

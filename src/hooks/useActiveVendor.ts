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
 * 남았나"이기 때문이다. 사용자가 설정에서 벤더를 고정하면 그 값이 전부 무시된다.
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

  // 작업 중인 세션 중 가장 최근 것. live.sessions 는 busy/active 만 실려 오지 않으므로
  // 상태로 한 번 더 거른다 (idle 세션은 근거가 못 된다).
  const busySource =
    live.sessions.find((s) => s.status === "busy" || s.status === "active")?.source ?? null;
  const latestContext =
    summary?.contexts?.length
      ? summary.contexts.reduce((a, b) => ((a.at ?? "") >= (b.at ?? "") ? a : b))
      : null;
  // 마지막 폴백으로 "한도를 아는 벤더"까지 본다. 오늘 아직 아무것도 안 썼어도
  // 플랜 게이지는 보여줘야 하는데, 이게 없으면 게이지가 통째로 사라진다.
  const candidate = busySource ?? latestContext?.source ?? topBySpend(summary) ?? plans[0]?.source;

  useEffect(() => {
    if (pinned !== "auto" || !candidate) return;
    if (candidate === source) return;
    const now = Date.now();
    // 첫 선택은 즉시, 이후 전환만 붙잡아 둔다
    if (source !== null && now - switchedAt.current < SWITCH_HOLD_MS) return;
    switchedAt.current = now;
    setSource(candidate);
  }, [candidate, source, pinned]);

  const shown = pinned === "auto" ? source : pinned;
  if (!shown) return null;

  return {
    source: shown,
    context: summary?.contexts?.find((c) => c.source === shown) ?? null,
    plan: plans.find((p) => p.source === shown) ?? null,
    busy: busySource === shown,
    // 근거가 사라졌는데 예전 선택을 계속 쓰는 중 (세션이 다 끝난 상태)
    stale: pinned === "auto" && candidate !== shown,
  };
}

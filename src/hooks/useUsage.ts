import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LiveState, PlanUsage, Summary } from "../types";

/** 요약 데이터 구독: 초기 invoke + usage-updated 이벤트 */
export function useSummary(): Summary | null {
  const [summary, setSummary] = useState<Summary | null>(null);
  useEffect(() => {
    let alive = true;
    invoke<Summary | null>("get_summary").then((s) => {
      if (alive && s) setSummary(s);
    });
    const un = listen<Summary>("usage-updated", (e) => {
      if (alive) setSummary(e.payload);
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return summary;
}

/** 공식 플랜 한도 구독: 초기 invoke + plan-updated 이벤트 (claude CLI 미설치 시 null 유지) */
export function usePlan(): PlanUsage | null {
  const [plan, setPlan] = useState<PlanUsage | null>(null);
  useEffect(() => {
    let alive = true;
    invoke<PlanUsage | null>("get_plan").then((p) => {
      if (alive && p) setPlan(p);
    });
    const un = listen<PlanUsage>("plan-updated", (e) => {
      if (alive) setPlan(e.payload);
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return plan;
}

/** 라이브 세션 상태 구독: 초기 invoke + live-state 이벤트 */
export function useLive(): LiveState {
  const [live, setLive] = useState<LiveState>({ busy: false, busy_count: 0, sessions: [] });
  useEffect(() => {
    let alive = true;
    invoke<LiveState>("get_live").then((l) => {
      if (alive) setLive(l);
    });
    const un = listen<LiveState>("live-state", (e) => {
      if (alive) setLive(e.payload);
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return live;
}

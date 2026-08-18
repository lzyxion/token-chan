import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  DEFAULT_CURRENCY,
  DEFAULT_THRESHOLDS,
  type AlertThresholds,
  type Currency,
} from "../format";
import type { AppSettings, LiveState, PlanUsage, Source, Summary } from "../types";

/**
 * 비용 표기 통화 구독 — 설정을 바꾸면 `settings-changed` 로 바로 반영된다.
 * 비용을 찍는 창이 여럿이라(패널·펫) 각자 설정을 읽는 대신 여기서 한 번에 맞춘다.
 */
export function useCurrency(): Currency {
  const [cur, setCur] = useState<Currency>(DEFAULT_CURRENCY);
  useEffect(() => {
    let alive = true;
    const apply = (s: AppSettings | null) => {
      if (!alive || !s) return;
      setCur({
        code: s.currency === "krw" ? "krw" : "usd",
        // 0 이면 원 표기가 전부 ₩0 이 된다 — 설정이 비어 있을 때의 방어
        usdToKrw: s.usdToKrw > 0 ? s.usdToKrw : DEFAULT_CURRENCY.usdToKrw,
      });
    };
    invoke<AppSettings>("get_settings").then(apply).catch(() => {});
    const un = listen<AppSettings>("settings-changed", (e) => apply(e.payload));
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return cur;
}

/**
 * 위험 한도 구독 — 알림 탭에서 바꾸면 게이지·패널 색이 즉시 따라간다.
 *
 * 설정은 0..1 로 저장되고 미터는 % 로 오므로 여기서 한 번만 환산한다.
 * 0 은 "설정 안 됨"이 아니라 "항상 위험"이 되므로 기본값으로 되돌린다.
 */
export function useThresholds(): AlertThresholds {
  const [t, setT] = useState<AlertThresholds>(DEFAULT_THRESHOLDS);
  useEffect(() => {
    let alive = true;
    const pct = (v: number | undefined, fallback: number) =>
      v && v > 0 ? Math.round(v * 100) : fallback;
    const apply = (s: AppSettings | null) => {
      if (!alive || !s) return;
      setT({
        session: pct(s.alertThreshold, DEFAULT_THRESHOLDS.session),
        weekly: pct(s.weeklyAlertThreshold, DEFAULT_THRESHOLDS.weekly),
        context: pct(s.contextAlertThreshold, DEFAULT_THRESHOLDS.context),
      });
    };
    invoke<AppSettings>("get_settings").then(apply).catch(() => {});
    const un = listen<AppSettings>("settings-changed", (e) => apply(e.payload));
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return t;
}

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

/**
 * 소스별 공식 플랜 한도 구독: 초기 invoke + plan-updated 이벤트.
 * 한도를 알 수 있는 소스만 들어온다 — Claude(CLI 설치 시)와 Codex(rollout 에 있음).
 * agy 는 서버에서 받아 메모리에만 둬서 여기 안 들어온다.
 */
export function usePlans(): PlanUsage[] {
  const [plans, setPlans] = useState<PlanUsage[]>([]);
  useEffect(() => {
    let alive = true;
    invoke<PlanUsage[]>("get_plan").then((p) => {
      if (alive && p) setPlans(p);
    });
    const un = listen<PlanUsage[]>("plan-updated", (e) => {
      if (alive) setPlans(e.payload);
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);
  return plans;
}

/** 특정 소스의 한도만 (없으면 null) */
export function usePlanOf(source: Source): PlanUsage | null {
  return usePlans().find((p) => p.source === source) ?? null;
}

/** 라이브 세션 상태 구독: 초기 invoke + live-state 이벤트 */
export function useLive(): LiveState {
  const [live, setLive] = useState<LiveState>({
    busy: false,
    busy_count: 0,
    sessions: [],
    completed: [],
  });
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

import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useLive, usePlan, useSummary } from "../hooks/useUsage";
import { fmtMinutes, parseResetTime } from "../format";
import type { AppSettings, CharacterImages, CharacterRule, PetState } from "../types";
import "./pet.css";

/** 게이지 색상 단계 (말풍선 meterClass와 동일 기준) */
function ringColor(pct: number): string {
  if (pct >= 85) return "#ff5d47";
  if (pct >= 60) return "#ffb45e";
  return "#35d07f";
}

/** 활성 모델 → 캐릭터 팩 결정 (최장 접두사 매칭, 미매칭 시 기본 팩) */
function resolvePack(
  model: string | null,
  rules: CharacterRule[],
  fallback: string | null,
): string | null {
  if (model) {
    let bestLen = -1;
    let bestPack: string | null = null;
    for (const r of rules) {
      if (!r.pack) continue;
      for (const p of r.prefixes.split(",").map((x) => x.trim()).filter(Boolean)) {
        if (model.startsWith(p) && p.length > bestLen) {
          bestLen = p.length;
          bestPack = r.pack;
        }
      }
    }
    if (bestPack) return bestPack;
  }
  return fallback;
}


export default function Pet() {
  const live = useLive();
  const summary = useSummary();
  const plan = usePlan();
  const hideTimer = useRef<number | undefined>(undefined);
  const [scale, setScale] = useState(1.0);
  const [threshold, setThreshold] = useState(0.8);
  const [weeklyThreshold, setWeeklyThreshold] = useState(0.9);
  const [packImages, setPackImages] = useState<CharacterImages | null>(null);
  const [sleepAfterMin, setSleepAfterMin] = useState(30);
  const [rules, setRules] = useState<CharacterRule[]>([]);
  const [defaultPack, setDefaultPack] = useState<string | null>(null);
  const [disabledStates, setDisabledStates] = useState<string[]>([]);
  const [showMiniLabel, setShowMiniLabel] = useState(false);
  const hideDelayRef = useRef(250);
  const packCacheRef = useRef<Map<string, CharacterImages | null>>(new Map());
  const charRef = useRef<HTMLDivElement | null>(null);

  // 저장된 설정 + 설정 패널/트레이의 실시간 변경(settings-changed, pet-scale) 반영
  useEffect(() => {
    let alive = true;
    const apply = (s: AppSettings | null) => {
      if (!alive || !s) return;
      if (s.petScale) setScale(s.petScale);
      if (s.alertThreshold) setThreshold(s.alertThreshold);
      if (s.weeklyAlertThreshold) setWeeklyThreshold(s.weeklyAlertThreshold);
      if (s.sleepAfterMinutes) setSleepAfterMin(s.sleepAfterMinutes);
      hideDelayRef.current = s.hoverDelayMs ?? 250;
      setRules(s.characterRules ?? []);
      setDefaultPack(s.characterPack ?? null);
      setDisabledStates(s.disabledStates ?? []);
      setShowMiniLabel(s.showMiniLabel ?? false);
      // 설정 변경 시 팩 파일이 바뀌었을 수 있으므로 이미지 캐시 무효화
      packCacheRef.current.clear();
    };
    invoke<AppSettings>("get_settings").then(apply).catch(() => {});
    const unScale = listen<number>("pet-scale", (e) => {
      if (alive) setScale(e.payload);
    });
    const unSettings = listen<AppSettings>("settings-changed", (e) => apply(e.payload));
    return () => {
      alive = false;
      unScale.then((f) => f());
      unSettings.then((f) => f());
    };
  }, []);

  // 활성 팩 = 최근 모델 → 규칙 매칭 → 기본 팩. 팩별 이미지는 캐시.
  const activePack = useMemo(
    () => resolvePack(summary?.last_model ?? null, rules, defaultPack),
    [summary?.last_model, rules, defaultPack],
  );
  useEffect(() => {
    let alive = true;
    if (!activePack) {
      setPackImages(null);
      return;
    }
    const cached = packCacheRef.current.get(activePack);
    if (cached !== undefined) {
      setPackImages(cached);
      return;
    }
    invoke<CharacterImages | null>("get_character_images", { pack: activePack })
      .then((imgs) => {
        packCacheRef.current.set(activePack, imgs);
        if (alive) setPackImages(imgs);
      })
      .catch(() => {
        if (alive) setPackImages(null);
      });
    return () => {
      alive = false;
    };
  }, [activePack]);

  // 미니 라벨용 리셋 카운트다운 (summary 갱신 주기(10s)에 맞춰 재계산)
  const resetRemainMin = useMemo(() => {
    const resets = plan?.meters?.[0]?.resets;
    if (!resets) return null;
    const d = parseResetTime(resets);
    if (!d) return null;
    return Math.max(0, Math.round((d.getTime() - Date.now()) / 60000));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, summary?.generated_at]);

  // 블록 초기화 감지: 공식 리셋 시각 문자열이 바뀌면 새 5시간 윈도우 시작 → 5분간 refreshed
  const [refreshedUntil, setRefreshedUntil] = useState(0);
  const prevResetsRef = useRef<string | null>(null);
  useEffect(() => {
    const resets = plan?.meters?.[0]?.resets ?? null;
    if (resets && prevResetsRef.current && resets !== prevResetsRef.current) {
      setRefreshedUntil(Date.now() + 5 * 60 * 1000);
    }
    if (resets) prevResetsRef.current = resets;
  }, [plan]);

  // 공식 미터 (링/컨디션/라벨/상태 판정 공용)
  const sessionPct = plan?.meters?.[0]?.used_pct ?? null;
  const weeklyPct =
    (plan?.meters?.find((m) => /week/i.test(m.label) && /all/i.test(m.label)) ?? plan?.meters?.[1])
      ?.used_pct ?? null;
  /// 컨디션(피로도) 0..1 — 공식 세션 % 우선, 없으면 로컬 블록 추정
  const fatigue = Math.min(
    1,
    Math.max(0, (sessionPct ?? (summary?.block?.token_ratio ?? 0) * 100) / 100),
  );

  const state: PetState = useMemo(() => {
    const off = new Set(disabledStates); // 사용자가 끈 상태는 건너뜀 (idle 취급)

    // 한도 완전 소진 — 작업 중이어도 최우선 표시
    if (sessionPct != null && sessionPct >= 100 && !off.has("exhausted")) return "exhausted";
    if (live.busy && !off.has("working")) return "working";
    // 경고: 공식 세션 % ≥ 세션 한도 또는 공식 주간 % ≥ 주간 한도. 공식 없으면 로컬 추정 폴백
    const alert =
      sessionPct != null
        ? sessionPct >= threshold * 100 || (weeklyPct != null && weeklyPct >= weeklyThreshold * 100)
        : (summary?.block?.token_ratio ?? 0) >= threshold;
    if (alert && !off.has("alert")) return "alert";
    if (Date.now() < refreshedUntil && !off.has("refreshed")) return "refreshed";
    const last = summary?.last_event_ts ? Date.parse(summary.last_event_ts) : null;
    if ((last == null || Date.now() - last > sleepAfterMin * 60 * 1000) && !off.has("sleep")) {
      return "sleep";
    }
    return "idle";
  }, [live, summary, plan, threshold, weeklyThreshold, refreshedUntil, sleepAfterMin, disabledStates]);

  // 드래그로 이동한 위치를 저장 (연속 이벤트 debounce)
  useEffect(() => {
    const win = getCurrentWindow();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const unlisten = win.onMoved(({ payload }) => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        void invoke("save_pet_position", { x: payload.x, y: payload.y });
      }, 500);
    });
    return () => {
      clearTimeout(timer);
      unlisten.then((f) => f());
    };
  }, []);

  const onMouseEnter = () => {
    window.clearTimeout(hideTimer.current);
    void invoke("set_hover", { zone: "pet", hovering: true });
    // 캐릭터(팩 이미지 포함)의 실제 머리 위 여백을 측정해 전달 → 말풍선이 정확히 머리 위에
    const el = charRef.current;
    let headroom = 0;
    if (el) {
      const img = el.querySelector("img");
      const rect = (img ?? el).getBoundingClientRect();
      headroom = Math.max(0, Math.round(rect.top));
    }
    void invoke("show_bubble", { headroom });
  };
  const onMouseLeave = () => {
    // 말풍선으로 이동할 수 있으므로 호버 해제 후 지연 숨김 (말풍선 호버 중이면 백엔드가 무시)
    void invoke("set_hover", { zone: "pet", hovering: false });
    hideTimer.current = window.setTimeout(() => {
      void invoke("hide_bubble");
    }, hideDelayRef.current);
  };
  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    void invoke("toggle_bubble_pin");
  };

  return (
    <div
      className={`pet ${state}`}
      style={{ "--fatigue": String(fatigue) } as React.CSSProperties}
      data-tauri-drag-region
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onContextMenu={onContextMenu}
    >
      <div className="stage" style={{ transform: `scale(${scale})` }}>
        {packImages ? (
          // 사용자 캐릭터 팩: 상태별 이미지 (idle 폴백은 백엔드에서 처리됨).
          // 상태 모션(숨쉬기/타자/떨림)은 .pet.<state> .cat 셀렉터로 그대로 적용됨.
          <div className="cat pack" ref={charRef}>
            <img src={packImages[state] ?? packImages.idle} alt="" draggable={false} />
          </div>
        ) : (
          <div className="cat" ref={charRef}>
            <div className="horn horn-l" />
            <div className="horn horn-r" />
            <div className="wing wing-l" />
            <div className="wing wing-r" />
            <div className="tail-d" />
            <div className="body">
              <div className="belly" />
              <div className="eye eye-l" />
              <div className="eye eye-r" />
              <div className="nostril nostril-l" />
              <div className="nostril nostril-r" />
              <div className="mouth" />
              <div className="cheek cheek-l" />
              <div className="cheek cheek-r" />
            </div>
            <div className="steam steam-l">💨</div>
            <div className="steam steam-r">💨</div>
            <div className="laptop">⌨️</div>
            <div className="zzz">
              z<span>z</span>
            </div>
            <div className="alert-mark">!</div>
            <div className="ko-mark">🪫</div>
            <div className="sparkle">✨</div>
            <div className="sweat">💦</div>
          </div>
        )}
        {sessionPct != null ? (
          // 발밑 진행 링: 안쪽 = 세션 5h 소진율, 바깥 얇은 링 = 주간 소진율
          <div className="ring-wrap">
            {weeklyPct != null && (
              <div
                className="ring ring-weekly"
                style={{
                  background: `conic-gradient(${ringColor(weeklyPct)} ${weeklyPct}%, rgba(255,255,255,0.07) 0)`,
                }}
              />
            )}
            <div
              className="ring ring-session"
              style={{
                background: `conic-gradient(${ringColor(sessionPct)} ${sessionPct}%, rgba(255,255,255,0.12) 0)`,
              }}
            />
          </div>
        ) : (
          <div className="ground-shadow" />
        )}
        {showMiniLabel && sessionPct != null && (
          <div className="mini-label">
            {sessionPct}%{resetRemainMin != null ? ` · ${fmtMinutes(resetRemainMin)}` : ""}
          </div>
        )}
      </div>
    </div>
  );
}

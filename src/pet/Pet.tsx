import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCurrency, useLive, usePlans, useSummary } from "../hooks/useUsage";
import { useActiveVendor } from "../hooks/useActiveVendor";
import VendorIcon from "../components/VendorIcon";
import {
  fmtCost,
  fmtMinutes,
  fmtTokens,
  shortModel,
  SOURCE_LABEL,
  SOURCE_SHORT,
  totalOf,
} from "../format";
import type {
  AppSettings,
  CharacterImages,
  CharacterRule,
  GaugeSide,
  PackConfig,
  PetState,
  Source,
} from "../types";
import { interpolate, linesFor, mergeLines, pick, speechFor } from "./speech";
import { DEFAULT_PACK_IMAGES } from "./defaultPack";
import "./pet.css";

/** 머리 위로 소품(`!`·`zzz`·`✨`)이 뜨는 상태 — 그때만 위쪽 여백을 잡는다 */
const ABOVE_HEAD_STATES = new Set<PetState>(["alert", "sleep", "refreshed", "poke"]);
/** 소품이 캐릭터 박스 위로 솟는 최대치 (기본 배율 px) */
const PROP_OVERHANG = 22;
/** 소품이 없을 때 남기는 최소 여백 */
const IDLE_OVERHANG = 4;

/** 클릭 반응 모션이 유지되는 시간 (ms) */
const POKE_MS = 1800;
/** 이 거리(px)를 넘게 움직여야 드래그로 본다 — 그 전이면 클릭 */
const DRAG_THRESHOLD_PX = 4;
/** 이 간격 안에 다시 클릭되면 더블클릭으로 보고 두 번째 대사를 생략 */
const DOUBLE_CLICK_MS = 400;

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
  const plans = usePlans();
  const currency = useCurrency();
  const [gaugeVendor, setGaugeVendor] = useState<"auto" | Source>("auto");
  // 게이지는 링이 3개뿐이라 벤더 하나만 태운다. 어느 벤더인지는 벤더 점으로 밝힌다.
  const active = useActiveVendor(summary, live, plans, gaugeVendor);
  const plan = active?.plan ?? null;
  const [scale, setScale] = useState(1.0);
  const [threshold, setThreshold] = useState(0.8);
  const [weeklyThreshold, setWeeklyThreshold] = useState(0.9);
  const [ctxThreshold, setCtxThreshold] = useState(0.9);
  const [packImages, setPackImages] = useState<CharacterImages | null>(null);
  const [sleepAfterMin, setSleepAfterMin] = useState(30);
  const [rules, setRules] = useState<CharacterRule[]>([]);
  const [defaultPack, setDefaultPack] = useState<string | null>(null);
  const [disabledStates, setDisabledStates] = useState<string[]>([]);
  const [gaugeLabels, setGaugeLabels] = useState(false);
  const [gaugeSide, setGaugeSide] = useState<GaugeSide>("right");
  const [speechLines, setSpeechLines] = useState<Record<string, string[]>>({});
  /** 활성 팩의 speech.json (없으면 null → 기본 문구) — 대사는 캐릭터의 속성 */
  const [packSpeech, setPackSpeech] = useState<Record<string, string[]> | null>(null);
  /** 활성 팩의 pack.json 끈 상태 목록 (기본 캐릭터면 null → 전역 disabledStates) */
  const [packDisabled, setPackDisabled] = useState<string[] | null>(null);
  const packCacheRef = useRef<Map<string, CharacterImages | null>>(new Map());
  const charRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const lastPokeAtRef = useRef(0);

  // 저장된 설정 + 설정 패널/트레이의 실시간 변경(settings-changed, pet-scale) 반영
  useEffect(() => {
    let alive = true;
    const apply = (s: AppSettings | null) => {
      if (!alive || !s) return;
      if (s.petScale) setScale(s.petScale);
      if (s.alertThreshold) setThreshold(s.alertThreshold);
      if (s.weeklyAlertThreshold) setWeeklyThreshold(s.weeklyAlertThreshold);
      if (s.contextAlertThreshold) setCtxThreshold(s.contextAlertThreshold);
      if (s.sleepAfterMinutes) setSleepAfterMin(s.sleepAfterMinutes);
      setRules(s.characterRules ?? []);
      setDefaultPack(s.characterPack ?? null);
      setDisabledStates(s.disabledStates ?? []);
      setGaugeLabels(s.gaugeLabels ?? false);
      setGaugeSide(s.gaugeSide ?? "right");
      setGaugeVendor(s.gaugeVendor ?? "auto");
      setSpeechLines(s.speechLines ?? {});
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

  // 사용자 팩 우선, 없으면 내장 기본 이미지 팩 (그것도 없으면 CSS 캐릭터)
  const activeImages = packImages ?? DEFAULT_PACK_IMAGES;

  // 활성 팩의 대사(speech.json)·동작 설정(pack.json) — 이미지처럼 팩이 바뀔 때
  // 로드하고, 스튜디오가 저장하면 이벤트로 다시 읽는다. 파일이 작아 캐시는 두지 않는다.
  useEffect(() => {
    let alive = true;
    const load = () => {
      if (!activePack) {
        setPackSpeech(null);
        setPackDisabled(null);
        return;
      }
      invoke<Record<string, string[]> | null>("get_character_speech", { pack: activePack })
        .then((v) => {
          if (alive) setPackSpeech(v);
        })
        .catch(() => {
          if (alive) setPackSpeech(null);
        });
      invoke<PackConfig>("get_character_config", { pack: activePack })
        .then((v) => {
          if (alive) setPackDisabled(v?.disabledStates ?? []);
        })
        .catch(() => {
          if (alive) setPackDisabled(null);
        });
    };
    load();
    const unSpeech = listen<string>("character-speech-changed", (e) => {
      if (e.payload === activePack) load();
    });
    const unConfig = listen<string>("character-config-changed", (e) => {
      if (e.payload === activePack) load();
    });
    return () => {
      alive = false;
      unSpeech.then((f) => f());
      unConfig.then((f) => f());
    };
  }, [activePack]);

  // 상태 사용은 캐릭터의 속성 — 팩이면 pack.json, 기본 캐릭터면 전역 설정
  const effectiveDisabled = packDisabled ?? disabledStates;

  // 첫 미터 = 가장 짧은 창 = 지금 당장 걸리는 한도. 두 소스 모두 파일에서 정확한
  // 시각으로 주므로 문자열을 파싱하지 않는다.
  const resets = plan?.meters?.[0]?.resets_at ?? null;

  // 세션 라벨·대사 변수용 리셋 카운트다운 (summary 갱신 주기(10s)에 맞춰 재계산)
  const resetRemainMin = useMemo(() => {
    if (!resets) return null;
    const d = new Date(resets);
    if (Number.isNaN(d.getTime())) return null;
    return Math.max(0, Math.round((d.getTime() - Date.now()) / 60000));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resets, summary?.generated_at]);

  // 블록 초기화 감지: 공식 리셋 시각이 바뀌면 새 창이 열린 것 → 5분간 refreshed
  const [refreshedUntil, setRefreshedUntil] = useState(0);
  const prevResetsRef = useRef<string | null>(null);
  useEffect(() => {
    if (resets && prevResetsRef.current && resets !== prevResetsRef.current) {
      setRefreshedUntil(Date.now() + 5 * 60 * 1000);
    }
    if (resets) prevResetsRef.current = resets;
  }, [resets]);

  // 클릭 반응이 유지되는 시각 (0 = 반응 아님)
  const [pokeUntil, setPokeUntil] = useState(0);
  useEffect(() => {
    if (pokeUntil <= Date.now()) return;
    const t = setTimeout(() => setPokeUntil(0), pokeUntil - Date.now());
    return () => clearTimeout(t);
  }, [pokeUntil]);

  // 공식 미터 (링/컨디션/라벨/상태 판정 공용).
  // 백엔드가 **짧은 창부터** 담아 주므로 [0]=지금 걸리는 한도, [1]=그 다음 긴 창이다.
  // (라벨로 찾지 않는다 — 두 소스가 서로 다른 창 이름을 쓴다)
  const sessionPct = plan?.meters?.[0]?.used_pct ?? null;
  const weeklyPct = plan?.meters?.[1]?.used_pct ?? null;
  /// 컨디션(피로도) 0..1 — 활성 벤더의 공식 세션 %.
  /// 공식 한도를 안 주는 벤더(agy)를 보고 있으면 피로도가 없다. 추정치로 캐릭터를
  /// 지치게 만드느니 아무 말도 안 하는 게 낫다.
  const fatigue = Math.min(1, Math.max(0, (sessionPct ?? 0) / 100));

  const state: PetState = useMemo(() => {
    const off = new Set(effectiveDisabled); // 캐릭터에서 끈 상태는 건너뜀 (idle 취급)

    // 방금 클릭했다면 무엇보다 먼저 반응을 보여준다 (짧게 지나감)
    if (Date.now() < pokeUntil && !off.has("poke")) return "poke";
    // 한도 완전 소진 — 작업 중이어도 최우선 표시
    if (sessionPct != null && sessionPct >= 100 && !off.has("exhausted")) return "exhausted";
    if (live.busy && !off.has("working")) return "working";
    // 경고: 공식 세션 % ≥ 세션 한도, 공식 주간 % ≥ 주간 한도, 또는
    // 활성 벤더 컨텍스트 ≥ 컨텍스트 한도 (compact 임박 — agy 처럼 공식
    // 한도가 없는 벤더도 컨텍스트는 직접 주는 값이라 경고 근거가 된다).
    // 공식 한도는 없는 벤더면 겁주지 않는다 (추정치로 경고하지 않는다).
    const ctxPct = active?.context?.used_pct ?? null;
    const alert =
      (sessionPct != null &&
        (sessionPct >= threshold * 100 ||
          (weeklyPct != null && weeklyPct >= weeklyThreshold * 100))) ||
      (ctxPct != null && ctxPct >= ctxThreshold * 100);
    if (alert && !off.has("alert")) return "alert";
    if (Date.now() < refreshedUntil && !off.has("refreshed")) return "refreshed";
    const last = summary?.last_event_ts ? Date.parse(summary.last_event_ts) : null;
    if ((last == null || Date.now() - last > sleepAfterMin * 60 * 1000) && !off.has("sleep")) {
      return "sleep";
    }
    return "idle";
  }, [
    live,
    summary,
    plan,
    threshold,
    weeklyThreshold,
    ctxThreshold,
    active,
    refreshedUntil,
    pokeUntil,
    sleepAfterMin,
    effectiveDisabled,
  ]);

  // 캐릭터(팩 이미지 포함)의 실측 위치 — 말풍선을 머리 위·가로 중심에 맞추는 기준값.
  // 게이지 열 때문에 캐릭터가 창 중앙이 아니므로 중심 x도 함께 보고한다.
  // 발밑 여백(footroom)도 같이 — 말풍선이 아래로 뒤집힐 때 그림자·무대 패딩만큼
  // 파고들어야 위로 띄울 때와 간격이 같아진다.
  const reportAnchor = () => {
    const el = charRef.current;
    if (!el) return;
    const img = el.querySelector("img");
    const rect = (img ?? el).getBoundingClientRect();
    void invoke("set_anchor", {
      headroom: Math.max(0, Math.round(rect.top)),
      footroom: Math.max(0, Math.round(window.innerHeight - rect.bottom)),
      centerX: Math.round(rect.left + rect.width / 2),
    });
  };

  // 창을 콘텐츠(.stage)에 딱 맞춘다 — 남는 투명 여백이 없어야 화면 가장자리에
  // 붙였을 때 캐릭터도 같이 붙는다. 소품이 뜨는 상태에서만 위쪽을 더 확보한다.
  const fitWindow = () => {
    const el = stageRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const overhang = (ABOVE_HEAD_STATES.has(state) ? PROP_OVERHANG : IDLE_OVERHANG) * scale;
    void invoke("fit_pet_window", {
      width: Math.ceil(r.width) + 2,
      height: Math.ceil(r.height + overhang),
    });
  };

  // 창 크기가 바뀌면(배율 다른 모니터로 이동 등) 콘텐츠 크기와 앵커를 다시 맞춘다.
  // 핸들러는 한 번만 등록하고 최신 클로저는 ref 로 참조 — 등록/해제 반복을 피한다.
  const onResizeRef = useRef<() => void>(() => {});
  onResizeRef.current = () => {
    // 드래그 중에는 리사이즈가 위치 보정을 유발해 커서와 싸우므로 건드리지 않는다
    if (!dragRef.current) fitWindow();
    reportAnchor();
  };
  useEffect(() => {
    const h = () => onResizeRef.current();
    window.addEventListener("resize", h);
    return () => window.removeEventListener("resize", h);
  }, []);

  // 크기·팩·상태·게이지 위치가 바뀌면 기준값도 바뀌므로 미리 보고해 둔다.
  useEffect(() => {
    const id = requestAnimationFrame(() => {
      fitWindow();
      reportAnchor();
    });
    return () => cancelAnimationFrame(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    scale,
    packImages,
    state,
    gaugeSide,
    sessionPct != null,
    weeklyPct != null,
    active?.source,
    active?.context != null,
    plan?.meters?.length,
  ]);

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

  // 활성 캐릭터 팩의 대사를 기본 문구 위에 덮은 실효 문구 —
  // 말투가 외모(팩)를 따라가므로 캐릭터와 대사가 어긋날 수 없다
  const effectiveSpeechLines = useMemo(
    () => mergeLines(speechLines, packSpeech ?? undefined),
    [speechLines, packSpeech],
  );

  // 스튜디오 ▶ 테스트 — 문구를 실제 경로(변수 치환 → 말풍선) 그대로 말해본다.
  // 리스너는 한 번만 걸고 최신 값은 ref 로 본다 (speechVars 가 렌더마다 새 클로저라서).
  const speechVarsRef = useRef<() => Record<string, string | null>>(() => ({}));
  useEffect(() => {
    const un = listen<string>("test-speech", (e) => {
      // 값을 모르는 변수 때문에 전부 생략되면 원문이라도 보여준다 — 테스트니까
      const text = interpolate(e.payload, speechVarsRef.current()) ?? e.payload.split("|").join("\n");
      void invoke("show_speech", { text });
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 문구 템플릿의 `{변수}` 에 넣을 표시 시점 값 (null = 아직 모르는 값 → 그 줄 생략)
  const speechVars = (): Record<string, string | null> => {
    const resetAt = resets ? new Date(resets) : null;
    return {
      오늘토큰: summary ? fmtTokens(totalOf(summary.today)) : null,
      오늘비용: summary ? fmtCost(summary.today_cost, summary.cost_partial, currency) : null,
      세션: sessionPct != null ? String(sessionPct) : null,
      주간: weeklyPct != null ? String(weeklyPct) : null,
      컨텍스트: active?.context ? String(Math.round(active.context.used_pct)) : null,
      벤더: active ? SOURCE_LABEL[active.source] : null,
      리셋: resetRemainMin != null ? fmtMinutes(resetRemainMin) : null,
      리셋시각: resetAt
        ? `${String(resetAt.getHours()).padStart(2, "0")}:${String(resetAt.getMinutes()).padStart(2, "0")}`
        : null,
      모델: summary?.last_model ? shortModel(summary.last_model) : null,
    };
  };
  speechVarsRef.current = speechVars;

  // 상태 전이를 대사로. 데이터가 오기 전 상태(무활동=sleep)는 신뢰할 수 없으므로
  // 첫 summary 도착 후부터 추적하고, 그 첫 상태 자체는 대사 없이 기준으로만 삼는다.
  const prevStateRef = useRef<PetState | null>(null);
  useEffect(() => {
    if (!summary) return;
    const prev = prevStateRef.current;
    prevStateRef.current = state;
    if (prev === null || prev === state) return;
    // 클릭 반응은 전용 대사가 따로 나가므로 전이 대사에서는 제외
    if (prev === "poke" || state === "poke") return;
    const tpl = speechFor(prev, state, effectiveSpeechLines);
    const line = tpl ? interpolate(tpl, speechVars()) : null;
    if (line) void invoke("show_speech", { text: line });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, summary]);

  // 우클릭 = 트레이와 동일한 메뉴 (펫 숨기기 / 사용량 패널 / 설정 / 종료)
  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    void invoke("show_pet_menu");
  };

  // 클릭 반응: 짧은 모션 + 지금 사용량을 말풍선으로
  const onPoke = () => {
    if (disabledStates.includes("poke")) return;
    setPokeUntil(Date.now() + POKE_MS);
    if (!summary) {
      void invoke("show_speech", { text: "아직 사용량을 읽는 중이야…" });
      return;
    }
    const tpl = pick(linesFor("poke", effectiveSpeechLines), "poke");
    const text = tpl ? interpolate(tpl, speechVars()) : null;
    if (text) void invoke("show_speech", { text });
  };

  // 드래그를 직접 처리한다.
  //  · data-tauri-drag-region 을 쓰면 좌클릭이 드래그에 먹혀 클릭 반응을 못 만든다.
  //  · OS 드래그(startDragging)는 창 상단이 작업 영역 위로 못 나가게 막혀서
  //    캐릭터를 화면 밖으로 걸쳐 둘 수 없다 → 좌표를 직접 지정한다.
  // 포인터 캡처로 커서가 창 밖으로 나가도 이벤트를 계속 받는다.
  // 실제 이동 계산은 백엔드가 OS 물리 좌표로만 한다 — 여기서 screenX(CSS px)를
  // 환산하면 배율이 다른 모니터로 넘어갈 때 기준이 바뀌어 커서와 창이 어긋난다.
  const dragRef = useRef<{ sx: number; sy: number; moved: boolean } | null>(null);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = { sx: e.screenX, sy: e.screenY, moved: false };
    void invoke("start_pet_drag");
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    // 임계값 판정에만 화면 좌표를 쓴다 (클릭인지 드래그인지 구분용)
    if (!d.moved) {
      if (Math.abs(e.screenX - d.sx) + Math.abs(e.screenY - d.sy) < DRAG_THRESHOLD_PX) return;
      d.moved = true;
    }
    void invoke("drag_pet");
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    void invoke("end_pet_drag");
    if (!d || d.moved || e.button !== 0) return;
    // 더블클릭의 두 번째 클릭에서는 대사를 내지 않는다 (패널만 열리게).
    // pointerup 의 detail 은 항상 0이라 클릭 횟수로 못 세므로 간격으로 판단한다.
    const now = Date.now();
    if (now - lastPokeAtRef.current < DOUBLE_CLICK_MS) return;
    lastPokeAtRef.current = now;
    onPoke();
  };

  // 더블클릭 = 사용량 패널 토글. pointerup 이 아니라 dblclick 이벤트로 받아야
  // 브라우저가 세어 준 클릭 횟수를 그대로 쓸 수 있다.
  const onDoubleClick = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    void invoke("toggle_panel");
  };

  // ── 도넛 게이지 열 (세션 5h · 주간 · 컨텍스트) ──
  // 셋 다 "%가 오르면 나빠진다"는 성격이 같아 같은 모양·같은 색 규칙으로 묶인다.
  // 리셋까지 남은 시간은 전용 줄 없이 세션 라벨에 텍스트로 붙인다 — 그 미터의 창이
  // 리셋되는 시각이라 의미가 같은 줄이고, 줄을 더 쌓으면 열이 포화된다.
  // .stage 안이라 캐릭터와 같은 배율로 커지고, 라벨은 캐릭터 반대쪽(바깥)으로 뻗는다.
  const ctx = active?.context ?? null;
  const contextPct = ctx ? Math.round(ctx.used_pct) : null;

  /**
   * 링 한 줄. `pct`가 null 이면 **값이 없다**는 뜻이라 0%처럼 보이면 안 된다 —
   * 점선 테두리로 "아직 모름"과 "0%"를 구분한다 (컨텍스트가 아직 안 잡힌 경우).
   */
  const ringRow = (key: string, pct: number | null, label: React.ReactNode) => (
    <div className="gauge-row" key={key}>
      <div
        className={`gauge-ring ${pct == null ? "empty" : ""}`}
        style={
          pct == null
            ? undefined
            : {
                background: `conic-gradient(${ringColor(pct)} ${pct}%, rgba(255,255,255,0.14) 0)`,
              }
        }
      />
      <span className="gauge-label">{label}</span>
    </div>
  );

  // 링 2·3은 활성 벤더의 한도 미터를 짧은 창부터 그대로 얹는다.
  // Claude 는 5시간 + 주간, Codex free 는 월간 하나, agy 는 없음.
  // (Claude 의 세 번째 미터인 모델별 주간은 링이 모자라 여기선 빠진다 — 사용량 패널엔 있다)
  const meters = plan?.meters ?? [];
  const meterRow = (slot: number) => {
    const m = meters[slot];
    const key = `meter${slot}`;
    // 한도를 안 주는 벤더(agy)·창이 하나뿐인 벤더(Codex — free/plus 모두 secondary 가
    // null 이라 실측상 항상 한 줄)는 빈 줄 대신 줄 자체를 숨긴다
    if (!m) return null;
    return ringRow(
      key,
      m.used_pct,
      <>
        {m.label} <b>{m.used_pct}%</b>
        {/* 리셋은 첫 미터(가장 짧은 창)의 것 — 그 창이 리셋되는 시각이라 같은 줄에 얹는다 */}
        {slot === 0 && resetRemainMin != null ? <> · 리셋 {fmtMinutes(resetRemainMin)}</> : null}
      </>,
    );
  };

  // 로고 클릭 = 벤더 순환 (자동 → Claude → Codex → Antigravity → 자동).
  // 설정 창을 열지 않고 게이지가 보는 벤더를 바꾼다.
  const VENDOR_CYCLE: ("auto" | Source)[] = ["auto", "claude", "codex", "antigravity"];
  const cycleVendor = () => {
    const i = VENDOR_CYCLE.indexOf(gaugeVendor);
    const next = VENDOR_CYCLE[(i + 1) % VENDOR_CYCLE.length];
    setGaugeVendor(next);
    void invoke("set_gauge_vendor", { vendor: next });
  };

  // 열 높이는 벤더의 미터 수에 따라 달라진다 — 한도 없는 벤더에 빈 링을 채워
  // 3줄을 고정하는 것보다 짧은 열이 낫다는 판단. 창 크기는 meters.length 가
  // 바뀔 때 다시 맞춘다 (fit effect 의존성).
  const gauges =
    gaugeSide === "off" || active == null ? null : (
      <div className={`gauge ${gaugeSide}${gaugeLabels ? " labels-on" : ""}`}>
        {/* 벤더 로고 — 라벨이 접혀 있으면(기본) 이게 벤더를 밝히는 유일한 수단이다.
            클릭하면 벤더 순환. 펫 전체가 드래그·클릭 반응을 받으므로(.pet * 는
            pointer-events:none) 버튼만 이벤트를 되살리고 전파를 끊는다. */}
        <div className="gauge-row" key="vendor">
          <button
            className="gauge-dot-btn"
            onPointerDown={(e) => e.stopPropagation()}
            onPointerUp={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              cycleVendor();
            }}
          >
            <VendorIcon
              source={active.source}
              className={`gauge-dot ${active.busy ? "busy" : ""} ${active.stale ? "stale" : ""}`}
            />
          </button>
          {/* 게이지만 짧은 표기 — 캐릭터 옆 좁은 열이라 이름이 길면 넘친다 */}
          <span className="gauge-label">
            {SOURCE_SHORT[active.source]}
            {ctx?.model ? ` · ${shortModel(ctx.model)}` : ""}
            {active.busy ? " · 작업 중" : ""}
            {/* 고정 중 표시 — 글자 대신 핀 아이콘 (이모지는 색을 못 입혀 SVG) */}
            {gaugeVendor !== "auto" && (
              <svg
                className="gauge-pin"
                width="9"
                height="9"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M12 17v5" />
                {/* 헤드는 채운다 — 통상적인 "고정됨" 압정 모양 */}
                <path fill="currentColor" d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
              </svg>
            )}
          </span>
        </div>
        {ctx != null && contextPct != null
          ? ringRow(
              "context",
              contextPct,
              <>
                컨텍스트 <b>{contextPct}%</b>
                {ctx.interim ? " · 정리 중" : ""}
              </>,
            )
          : ringRow("context", null, <>컨텍스트 <b>—</b></>)}
        {meterRow(0)}
        {meterRow(1)}
      </div>
    );

  return (
    <div
      className={`pet ${state}`}
      style={{ "--fatigue": String(fatigue) } as React.CSSProperties}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <div className="stage" ref={stageRef} style={{ transform: `scale(${scale})` }}>
        {gaugeSide === "left" && gauges}
        <div className="char-col">
        {/* 캐릭터는 이미지 팩 한 가지 방식뿐 — 사용자 팩(상태 폴백은 백엔드 처리) 또는 내장 기본 팩.
            상태 모션(숨쉬기·흔들림·폴짝)과 소품은 .pet.<state> .cat 셀렉터로 그림 위에 얹힌다. */}
        <div className="cat pack" ref={charRef}>
          {activeImages && (
            /* 이미지 로드 전에는 높이가 0 → 로드 완료 후 여백 재측정 */
            <img
              src={activeImages[state] ?? activeImages.idle}
              alt=""
              draggable={false}
              onLoad={reportAnchor}
            />
          )}
          <div className="steam steam-l">💨</div>
          <div className="steam steam-r">💨</div>
          <div className="zzz">
            z<span>z</span>
          </div>
          <div className="alert-mark">!</div>
          <div className="ko-mark">🪫</div>
          <div className="sparkle">✨</div>
          <div className="sweat">💦</div>
        </div>
        <div className="ground-shadow" />
        </div>
        {gaugeSide === "right" && gauges}
      </div>
    </div>
  );
}

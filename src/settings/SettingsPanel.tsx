import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AppSettings,
  CharacterRule,
  GaugeSide,
  ScanRoot,
  SpeechRule,
  Summary,
} from "../types";
import { SOURCE_LABEL } from "../format";
import ResizeGrips from "../components/ResizeGrips";
import { DEFAULT_LINES } from "../pet/speech";
import "./settings.css";

const STATE_LABELS: [string, string][] = [
  ["working", "작업"],
  ["alert", "경고"],
  ["sleep", "잠"],
  ["exhausted", "소진"],
  ["refreshed", "초기화"],
  ["poke", "클릭 반응"],
];

/** 설정 탭 — 일반(창·한도·시스템) / 캐릭터(팩·상태·크기) / 대사(말풍선·문구) */

type Tab = "general" | "character" | "speech";
const TABS: [Tab, string][] = [
  ["general", "일반"],
  ["character", "캐릭터"],
  ["speech", "대사"],
];

/** 문구 편집기에 노출할 상황 (키는 speech.ts DEFAULT_LINES · settings.speechLines 와 동일) */
const SPEECH_SITUATIONS: [string, string][] = [
  ["enter.working", "작업 시작"],
  ["leave.working", "작업 종료"],
  ["enter.alert", "한도 경고"],
  ["enter.exhausted", "토큰 소진"],
  ["enter.refreshed", "블록 초기화"],
  ["enter.sleep", "잠들 때"],
  ["leave.sleep", "깨어날 때"],
  ["poke", "클릭 반응 (사용량 보고)"],
  ["resetNotify", "리셋 임박 (변수: {분}·{시각}만)"],
];

export default function SettingsPanel() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [packs, setPacks] = useState<string[]>([]);
  const [observedModels, setObservedModels] = useState<string[]>([]);
  const [tab, setTab] = useState<Tab>("general");
  const [scanRoots, setScanRoots] = useState<ScanRoot[]>([]);
  /** 문구 편집기가 보고 있는 세트 ("" = 기본 문구) */
  const [editingSet, setEditingSet] = useState("");
  const [newSetName, setNewSetName] = useState("");

  const refreshScanRoots = () => {
    invoke<ScanRoot[]>("get_scan_roots")
      .then(setScanRoots)
      .catch(() => setScanRoots([]));
  };

  const refreshPacks = () => {
    invoke<string[]>("list_character_packs")
      .then(setPacks)
      .catch(() => setPacks([]));
    invoke<Summary | null>("get_summary")
      .then((sum) => setObservedModels(sum?.observed_models ?? []))
      .catch(() => {});
  };

  useEffect(() => {
    let alive = true;
    invoke<AppSettings>("get_settings")
      .then((v) => {
        if (alive) setS(v);
      })
      .catch(() => {});
    refreshPacks();
    refreshScanRoots();
    // 트레이 토글 등 외부 변경과 동기화 (내용 동일하면 스킵 — 입력 커서 보존)
    const un = listen<AppSettings>("settings-changed", (e) => {
      if (alive) {
        setS((prev) =>
          JSON.stringify(prev) === JSON.stringify(e.payload) ? prev : e.payload,
        );
      }
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);

  // 사용자가 조절한 창 크기 기억 (연속 이벤트 debounce — 패널과 동일)
  useEffect(() => {
    const win = getCurrentWindow();
    let sizeTimer: ReturnType<typeof setTimeout> | undefined;
    const unResized = win.onResized(({ payload }) => {
      clearTimeout(sizeTimer);
      sizeTimer = setTimeout(() => {
        void invoke("save_window_size", {
          label: "settings",
          width: payload.width,
          height: payload.height,
        });
      }, 500);
    });
    return () => {
      clearTimeout(sizeTimer);
      unResized.then((f) => f());
    };
  }, []);

  if (!s) {
    return (
      <div className="settings-root">
        <ResizeGrips />
        <div className="settings-card">불러오는 중…</div>
      </div>
    );
  }

  /** 필드 변경 → 로컬 반영 + 백엔드 일괄 저장 (side effect는 백엔드가 처리) */
  const update = (patch: Partial<AppSettings>) => {
    const next = { ...s, ...patch };
    setS(next);
    void invoke("set_settings", { newSettings: next });
  };

  /** 크기 슬라이더는 드래그 중 실시간 리사이즈 전용 빠른 경로 사용 */
  const onScaleChange = (pct: number) => {
    const scale = pct / 100;
    setS({ ...s, petScale: scale });
    void invoke("set_pet_scale", { scale });
  };

  const updateRule = (i: number, patch: Partial<CharacterRule>) => {
    const rules = [...(s.characterRules ?? [])];
    rules[i] = { ...rules[i], ...patch };
    update({ characterRules: rules });
  };
  const addRule = () =>
    update({
      characterRules: [...(s.characterRules ?? []), { prefixes: "", pack: "" }],
    });
  const addRuleWith = (model: string) =>
    update({
      characterRules: [
        ...(s.characterRules ?? []),
        { prefixes: model, pack: "" },
      ],
    });
  const removeRule = (i: number) =>
    update({
      characterRules: (s.characterRules ?? []).filter((_, j) => j !== i),
    });
  /** 문구 편집 — editingSet 이 비면 기본 문구(speechLines), 아니면 그 세트(speechSets) */
  const updateSpeechKey = (key: string, raw: string) => {
    if (!editingSet) {
      const map = { ...(s.speechLines ?? {}) };
      if (raw === "") {
        delete map[key];
      } else {
        map[key] = raw.split("\n");
      }
      update({ speechLines: map });
    } else {
      const sets = { ...(s.speechSets ?? {}) };
      const cur = { ...(sets[editingSet] ?? {}) };
      if (raw === "") {
        delete cur[key];
      } else {
        cur[key] = raw.split("\n");
      }
      sets[editingSet] = cur;
      update({ speechSets: sets });
    }
  };
  const addSpeechSet = () => {
    const name = newSetName.trim();
    if (!name || (s.speechSets ?? {})[name]) return;
    update({ speechSets: { ...(s.speechSets ?? {}), [name]: {} } });
    setNewSetName("");
    setEditingSet(name);
  };
  const removeSpeechSet = () => {
    if (!editingSet) return;
    const sets = { ...(s.speechSets ?? {}) };
    delete sets[editingSet];
    // 세트를 지우면 그 세트를 가리키던 규칙도 함께 정리
    update({
      speechSets: sets,
      speechRules: (s.speechRules ?? []).filter((r) => r.set !== editingSet),
    });
    setEditingSet("");
  };
  const updateSpeechRule = (i: number, patch: Partial<SpeechRule>) => {
    const rules = [...(s.speechRules ?? [])];
    rules[i] = { ...rules[i], ...patch };
    update({ speechRules: rules });
  };
  const addSpeechRule = () =>
    update({
      speechRules: [...(s.speechRules ?? []), { prefixes: "", set: "" }],
    });
  const removeSpeechRule = (i: number) =>
    update({ speechRules: (s.speechRules ?? []).filter((_, j) => j !== i) });

  const speechSetNames = Object.keys(s.speechSets ?? {});
  /** 편집 중 대상의 현재 값 / placeholder(폴백 문구) */
  const speechValue = (key: string): string =>
    (editingSet
      ? s.speechSets?.[editingSet]?.[key]
      : s.speechLines?.[key]
    )?.join("\n") ?? "";
  const speechPlaceholder = (key: string): string => {
    if (editingSet && s.speechLines?.[key]?.some((l) => l.trim())) {
      return (s.speechLines[key] ?? []).join("\n");
    }
    return (DEFAULT_LINES[key] ?? []).join("\n");
  };
  /** 지금 실제로 쓰이는 후보 개수. 빈 줄을 걸러 남는 게 없으면 폴백 문구가 후보 —
   *  런타임 linesFor 와 같은 판정이라 편집기 표시와 실제 동작이 어긋나지 않는다. */
  const nonBlank = (raw: string): number =>
    raw.split("\n").filter((l) => l.trim()).length;
  const speechCount = (key: string): number =>
    nonBlank(speechValue(key)) || nonBlank(speechPlaceholder(key));

  const toggleState = (key: string, enabled: boolean) => {
    const cur = new Set(s.disabledStates ?? []);
    if (enabled) {
      cur.delete(key);
    } else {
      cur.add(key);
    }
    update({ disabledStates: [...cur] });
  };

  return (
    <div className="settings-root">
      <ResizeGrips />
      <div className="settings-card">
        <div className="settings-head" data-tauri-drag-region>
          <span data-tauri-drag-region>설정</span>
          <button
            className="settings-close"
            onClick={() => void getCurrentWindow().hide()}
          >
            ✕
          </button>
        </div>

        <div className="settings-tabs">
          {TABS.map(([key, label]) => (
            <button
              key={key}
              className={`settings-tab${tab === key ? " active" : ""}`}
              onClick={() => setTab(key)}
            >
              {label}
            </button>
          ))}
        </div>

        {tab === "character" && (
          <>
            <div className="settings-group">
              <div className="settings-label">캐릭터</div>
              <div className="settings-row">
                <select
                  className="settings-select"
                  value={s.characterPack ?? ""}
                  onChange={(e) =>
                    update({ characterPack: e.currentTarget.value || null })
                  }
                >
                  <option value="">기본 (젤리 슬라임)</option>
                  {packs.map((p) => (
                    <option key={p} value={p}>
                      {p}
                    </option>
                  ))}
                </select>
                <button
                  className="settings-btn"
                  onClick={refreshPacks}
                  title="팩 목록 새로고침"
                >
                  ↻
                </button>
                <button
                  className="settings-btn"
                  onClick={() => void invoke("open_characters_dir")}
                >
                  폴더 열기
                </button>
              </div>
              <div className="settings-hint">
                characters/&lt;팩이름&gt;/ 에
                idle(필수)·working·alert·sleep·exhausted·refreshed.(gif|png|webp)
                — 투명 배경
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">
                모델별 캐릭터 규칙 (최장 접두사 우선)
              </div>
              {(s.characterRules ?? []).map((r, i) => (
                <div className="settings-row" key={i}>
                  <input
                    className="settings-input"
                    placeholder="접두사 (예: claude-opus 또는 gpt, o3)"
                    value={r.prefixes}
                    onChange={(e) =>
                      updateRule(i, { prefixes: e.currentTarget.value })
                    }
                  />
                  <select
                    className="settings-select rule-pack"
                    value={r.pack}
                    onChange={(e) =>
                      updateRule(i, { pack: e.currentTarget.value })
                    }
                  >
                    <option value="">팩 선택</option>
                    {packs.map((p) => (
                      <option key={p} value={p}>
                        {p}
                      </option>
                    ))}
                  </select>
                  <button
                    className="settings-btn"
                    onClick={() => removeRule(i)}
                  >
                    ✕
                  </button>
                </div>
              ))}
              <div className="settings-row">
                <button className="settings-btn" onClick={addRule}>
                  + 규칙 추가
                </button>
              </div>
              {observedModels.length > 0 && (
                <>
                  <div className="settings-hint">
                    최근 관측된 모델 (클릭 → 규칙으로 추가):
                  </div>
                  <div className="chips">
                    {observedModels.map((m) => (
                      <button
                        key={m}
                        className="chip-btn"
                        onClick={() => addRuleWith(m)}
                      >
                        {m}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>

            <div className="settings-group">
              <div className="settings-label">
                상태 사용 (끄면 해당 상태 대신 idle 유지)
              </div>
              <div className="state-toggles">
                {STATE_LABELS.map(([key, label]) => (
                  <label key={key} className="settings-check inline">
                    <input
                      type="checkbox"
                      checked={!(s.disabledStates ?? []).includes(key)}
                      onChange={(e) =>
                        toggleState(key, e.currentTarget.checked)
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">
                캐릭터 크기 <b>{Math.round(s.petScale * 100)}%</b>
              </div>
              <div className="settings-row">
                <span className="settings-min">50%</span>
                <input
                  type="range"
                  min={50}
                  max={250}
                  step={5}
                  value={Math.round(s.petScale * 100)}
                  onChange={(e) =>
                    onScaleChange(parseInt(e.currentTarget.value, 10))
                  }
                />
                <span className="settings-max">250%</span>
              </div>
            </div>

          </>
        )}

        {tab === "general" && (
          <>
            <div className="settings-group">
              <div className="settings-label">
                위험 한도 · 세션{" "}
                <b className="warn-b">{Math.round(s.alertThreshold * 100)}%</b>
              </div>
              <div className="settings-row">
                <span className="settings-min">10%</span>
                <input
                  type="range"
                  min={10}
                  max={100}
                  step={5}
                  value={Math.round(s.alertThreshold * 100)}
                  onChange={(e) =>
                    update({
                      alertThreshold: parseInt(e.currentTarget.value, 10) / 100,
                    })
                  }
                />
                <span className="settings-max">100%</span>
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">
                위험 한도 · 주간{" "}
                <b className="warn-b">
                  {Math.round(s.weeklyAlertThreshold * 100)}%
                </b>
              </div>
              <div className="settings-row">
                <span className="settings-min">10%</span>
                <input
                  type="range"
                  min={10}
                  max={100}
                  step={5}
                  value={Math.round(s.weeklyAlertThreshold * 100)}
                  onChange={(e) =>
                    update({
                      weeklyAlertThreshold:
                        parseInt(e.currentTarget.value, 10) / 100,
                    })
                  }
                />
                <span className="settings-max">100%</span>
              </div>
              <div className="settings-hint">
                공식 세션/주간 사용률이 한도를 넘으면 펫이 경고 상태로 변합니다
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">
                잠자기 진입 시간 <b>{s.sleepAfterMinutes}분</b>
              </div>
              <div className="settings-row">
                <span className="settings-min">5분</span>
                <input
                  type="range"
                  min={5}
                  max={120}
                  step={5}
                  value={s.sleepAfterMinutes}
                  onChange={(e) =>
                    update({
                      sleepAfterMinutes: parseInt(e.currentTarget.value, 10),
                    })
                  }
                />
                <span className="settings-max">2h</span>
              </div>
              <div className="settings-hint">
                마지막 AI 사용 후 이 시간이 지나면 캐릭터가 잠듭니다
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">
                블록 리셋 임박 대사{" "}
                <b>
                  {s.resetNotifyMinutes === 0
                    ? "끔"
                    : `${s.resetNotifyMinutes}분 전`}
                </b>
              </div>
              <div className="settings-row">
                <span className="settings-min">끔</span>
                <input
                  type="range"
                  min={0}
                  max={120}
                  step={5}
                  value={s.resetNotifyMinutes}
                  onChange={(e) =>
                    update({
                      resetNotifyMinutes: parseInt(e.currentTarget.value, 10),
                    })
                  }
                />
                <span className="settings-max">120분</span>
              </div>
              <div className="settings-hint">
                캐릭터가 말풍선으로 알려줍니다 · 5분 주기로 확인하므로 5분 이상
                권장
              </div>
            </div>
          </>
        )}

        {tab === "speech" && (
          <>
            <div className="settings-group">
              <label className="settings-check">
                <input
                  type="checkbox"
                  checked={s.speechEnabled}
                  onChange={(e) =>
                    update({ speechEnabled: e.currentTarget.checked })
                  }
                />
                상황별 대사 말풍선
                <span className="settings-hint-inline">
                  (경고·리셋·작업·잠자기)
                </span>
              </label>
              <div className="settings-label">
                대사 표시 시간 <b>{(s.speechDurationMs / 1000).toFixed(1)}s</b>
              </div>
              <div className="settings-row">
                <span className="settings-min">1s</span>
                <input
                  type="range"
                  min={1000}
                  max={15000}
                  step={500}
                  value={s.speechDurationMs}
                  disabled={!s.speechEnabled}
                  onChange={(e) =>
                    update({
                      speechDurationMs: parseInt(e.currentTarget.value, 10),
                    })
                  }
                />
                <span className="settings-max">15s</span>
              </div>
              <div className="settings-hint">
                상태를 끄면(캐릭터 탭 &quot;상태 사용&quot;) 그 상황의 대사도
                함께 사라집니다
              </div>
            </div>

            <div className="settings-group">
              <div className="settings-label">상황별 문구 편집</div>
              <div className="settings-hint">
                한 줄에 문구 하나 — 여러 줄을 쓰면 그중 하나가 무작위로 나옵니다
                (직전과 같은 문구는 피함) · 비우면 기본 문구로 돌아갑니다.
                <br />
                {
                  "{변수} 자리에 현재 값이 들어갑니다: {오늘토큰} {오늘비용} {세션} {주간} {컨텍스트} {리셋} {리셋시각} {모델}"
                }
                <br />
                문구 안 | 는 말풍선 줄바꿈 · 값을 아직 모르는 변수가 든 줄은
                생략됩니다
              </div>

              <div className="settings-label">편집 대상</div>
              <div className="settings-row">
                <select
                  className="settings-select"
                  value={editingSet}
                  onChange={(e) => setEditingSet(e.currentTarget.value)}
                >
                  <option value="">기본 문구 (모든 모델)</option>
                  {speechSetNames.map((n) => (
                    <option key={n} value={n}>
                      세트: {n}
                    </option>
                  ))}
                </select>
                {editingSet && (
                  <button
                    className="settings-btn"
                    onClick={removeSpeechSet}
                    title="이 세트와 규칙 삭제"
                  >
                    세트 삭제
                  </button>
                )}
              </div>
              <div className="settings-row">
                <input
                  className="settings-input"
                  placeholder="새 문구 세트 이름 (예: 갓지피티)"
                  value={newSetName}
                  onChange={(e) => setNewSetName(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") addSpeechSet();
                  }}
                />
                <button className="settings-btn" onClick={addSpeechSet}>
                  + 세트
                </button>
              </div>
              {editingSet && (
                <div className="settings-hint">
                  세트에서 비워 둔 상황은 기본 문구를 그대로 씁니다 (흐린 글씨 =
                  지금 폴백되는 문구)
                </div>
              )}

              {SPEECH_SITUATIONS.map(([key, label]) => (
                <div className="settings-speech-item" key={key}>
                  <div className="settings-speech-head">
                    <span className="settings-hint">{label}</span>
                    <span className="settings-hint">
                      {speechCount(key) > 1
                        ? `${speechCount(key)}개 중 무작위`
                        : "1개 (여러 줄로 늘려보세요)"}
                    </span>
                  </div>
                  <textarea
                    className="settings-textarea"
                    rows={4}
                    spellCheck={false}
                    placeholder={speechPlaceholder(key)}
                    value={speechValue(key)}
                    onChange={(e) =>
                      updateSpeechKey(key, e.currentTarget.value)
                    }
                  />
                </div>
              ))}

              <div className="settings-label">
                모델별 문구 규칙 (최장 접두사 우선)
              </div>
              {(s.speechRules ?? []).map((r, i) => (
                <div className="settings-row" key={i}>
                  <input
                    className="settings-input"
                    placeholder="접두사 (예: gpt, o3)"
                    value={r.prefixes}
                    onChange={(e) =>
                      updateSpeechRule(i, { prefixes: e.currentTarget.value })
                    }
                  />
                  <select
                    className="settings-select rule-pack"
                    value={r.set}
                    onChange={(e) =>
                      updateSpeechRule(i, { set: e.currentTarget.value })
                    }
                  >
                    <option value="">세트 선택</option>
                    {speechSetNames.map((n) => (
                      <option key={n} value={n}>
                        {n}
                      </option>
                    ))}
                  </select>
                  <button
                    className="settings-btn"
                    onClick={() => removeSpeechRule(i)}
                  >
                    ✕
                  </button>
                </div>
              ))}
              <div className="settings-row">
                <button className="settings-btn" onClick={addSpeechRule}>
                  + 규칙 추가
                </button>
              </div>
              <div className="settings-hint">
                활성 모델이 접두사와 맞으면 그 세트의 문구로 말합니다 (캐릭터
                규칙과 같은 방식) · 리셋 임박 대사도 함께 따라갑니다
              </div>
            </div>
          </>
        )}

        {tab === "general" && (
          <>
            <div className="settings-group">
              <div className="settings-label">소진율 도넛 게이지</div>
              <div className="settings-row">
                <select
                  className="settings-select"
                  value={s.gaugeSide}
                  onChange={(e) =>
                    update({ gaugeSide: e.currentTarget.value as GaugeSide })
                  }
                >
                  <option value="right">캐릭터 오른쪽</option>
                  <option value="left">캐릭터 왼쪽</option>
                  <option value="off">표시 안 함</option>
                </select>
              </div>
              {s.gaugeSide !== "off" && (
                <div className="settings-row">
                  <span className="settings-sublabel">보여줄 벤더</span>
                  <select
                    className="settings-select"
                    value={s.gaugeVendor ?? "auto"}
                    onChange={(e) =>
                      update({ gaugeVendor: e.currentTarget.value as AppSettings["gaugeVendor"] })
                    }
                  >
                    <option value="auto">자동 (작업 중인 쪽)</option>
                    <option value="claude">Claude 고정</option>
                    <option value="codex">Codex 고정</option>
                    <option value="antigravity">AGY 고정</option>
                  </select>
                </div>
              )}
              <div className="settings-hint">
                링이 3개뿐이라 <b>벤더 하나</b>만 보여줍니다 — 맨 위 로고가 어느 CLI인지
                알려주고, 작업 중이면 로고가 깜빡입니다.
                아래는 컨텍스트 · 그 벤더의 공식 한도 2개 순입니다. 점선 링은 0%가 아니라
                <b> 그 벤더가 안 알려주는 값</b>입니다 (AGY는 공식 한도가 없습니다).
                캐릭터에 마우스를 올리면 라벨이 바깥쪽으로 펼쳐지고, 리셋까지 남은 시간은
                발밑 가로 바입니다
              </div>
            </div>

            <label className="settings-check">
              <input
                type="checkbox"
                checked={s.clickThrough}
                onChange={(e) =>
                  update({ clickThrough: e.currentTarget.checked })
                }
              />
              클릭 통과 모드{" "}
              <span className="settings-hint-inline">
                (우클릭·드래그 불가, 패널은 트레이로)
              </span>
            </label>

            <label className="settings-check">
              <input
                type="checkbox"
                checked={s.showMiniLabel}
                onChange={(e) =>
                  update({ showMiniLabel: e.currentTarget.checked })
                }
              />
              미니 라벨 표시{" "}
              <span className="settings-hint-inline">
                (발밑에 소진율·남은시간 상시)
              </span>
            </label>

            <label className="settings-check">
              <input
                type="checkbox"
                checked={s.startHidden}
                onChange={(e) =>
                  update({ startHidden: e.currentTarget.checked })
                }
              />
              시작 시 펫 숨김{" "}
              <span className="settings-hint-inline">(트레이로만 시작)</span>
            </label>

            <label className="settings-check">
              <input
                type="checkbox"
                checked={s.autostart}
                onChange={(e) => update({ autostart: e.currentTarget.checked })}
              />
              로그인 시 자동 시작
            </label>

            <div className="settings-group">
              <div className="settings-label">데이터 소스 경로</div>
              <div className="settings-hint">
                발견된 설치본입니다. 같은 계정의 설치본은 하나로 묶여 집계되고,
                포함 여부는 <b>펫 우클릭 → 연결된 계정</b>에서 켜고 끕니다.
                자동 탐지가 못 찾는 경로만 아래에 직접 추가하세요 — 겹쳐도 중복
                집계되지 않습니다.
              </div>
              {scanRoots.length === 0 ? (
                <div className="settings-hint">발견된 설치본이 없습니다</div>
              ) : (
                <ul className="settings-rootlist">
                  {scanRoots.map((r) => (
                    <li key={`${r.source}:${r.path}`} className={r.enabled ? "" : "off"}>
                      <b>{SOURCE_LABEL[r.source] ?? r.source}</b> {r.account}
                      {r.enabled ? "" : " · 제외됨"}
                      <div className="settings-hint-inline">
                        <code>{r.path}</code> · {r.files}개
                      </div>
                    </li>
                  ))}
                </ul>
              )}
              {(
                [
                  ["extraClaudeHomes", "Claude 홈 추가 (.claude 디렉토리)"],
                  ["extraCodexHomes", "Codex 홈 추가 (CODEX_HOME 에 해당)"],
                  ["extraAntigravityHomes", "AGY 홈 추가 (antigravity-cli 디렉토리)"],
                ] as const
              ).map(([key, label]) => (
                <div key={key} className="settings-subgroup">
                  <div className="settings-sublabel">{label}</div>
                  {(s[key] ?? []).map((p, i) => (
                    <div className="settings-row" key={i}>
                      <input
                        className="settings-input settings-input-wide"
                        value={p}
                        placeholder="예: D:\\other-profile\\.claude"
                        onChange={(e) => {
                          const next = [...(s[key] ?? [])];
                          next[i] = e.currentTarget.value;
                          update({ [key]: next } as Partial<AppSettings>);
                        }}
                      />
                      <button
                        className="settings-btn"
                        onClick={() =>
                          update({
                            [key]: (s[key] ?? []).filter((_, j) => j !== i),
                          } as Partial<AppSettings>)
                        }
                      >
                        삭제
                      </button>
                    </div>
                  ))}
                  <button
                    className="settings-btn"
                    onClick={() =>
                      update({ [key]: [...(s[key] ?? []), ""] } as Partial<AppSettings>)
                    }
                  >
                    + 경로 추가
                  </button>
                </div>
              ))}
              <div className="settings-row">
                <button className="settings-btn" onClick={refreshScanRoots}>
                  다시 확인
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AppSettings,
  CharacterRule,
  GaugeSide,
  Summary,
} from "../types";
import ResizeGrips from "../components/ResizeGrips";
import "./settings.css";

/** 설정 탭 — 일반(창·한도·말풍선·시스템) / 캐릭터(팩·규칙·크기).
 *  상태 사용·이미지·대사 편집은 캐릭터 스튜디오(전용 창)가 맡는다. */

type Tab = "general" | "character";
const TABS: [Tab, string][] = [
  ["general", "일반"],
  ["character", "캐릭터"],
];

export default function SettingsPanel() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [packs, setPacks] = useState<string[]>([]);
  const [observedModels, setObservedModels] = useState<string[]>([]);
  const [tab, setTab] = useState<Tab>("general");

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
              </div>
              <div className="settings-row">
                <button
                  className="settings-btn"
                  onClick={() => void invoke("open_studio")}
                >
                  캐릭터 스튜디오 열기…
                </button>
              </div>
              <div className="settings-hint">
                캐릭터 만들기·상태별 이미지·상태 사용·대사 편집은 전부
                스튜디오에서 합니다
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
                위험 한도 · 컨텍스트{" "}
                <b className="warn-b">
                  {Math.round((s.contextAlertThreshold ?? 0.9) * 100)}%
                </b>
              </div>
              <div className="settings-row">
                <span className="settings-min">10%</span>
                <input
                  type="range"
                  min={10}
                  max={100}
                  step={5}
                  value={Math.round((s.contextAlertThreshold ?? 0.9) * 100)}
                  onChange={(e) =>
                    update({
                      contextAlertThreshold:
                        parseInt(e.currentTarget.value, 10) / 100,
                    })
                  }
                />
                <span className="settings-max">100%</span>
              </div>
              <div className="settings-hint">
                활성 벤더의 컨텍스트가 이만큼 차면 경고 — 곧 압축(compact)되거나
                창이 바닥난다는 뜻입니다
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
                문구 내용은 캐릭터 스튜디오(캐릭터 탭)에서 편집합니다
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
            </div>

            <label className="settings-check">
              <input
                type="checkbox"
                checked={s.gaugeLabels}
                onChange={(e) =>
                  update({ gaugeLabels: e.currentTarget.checked })
                }
              />
              게이지 라벨 상시 표시{" "}
              <span className="settings-hint-inline">
                (끄면 마우스를 올렸을 때만)
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

            <div className="settings-hint">
              데이터 소스(계정 켜고 끄기·홈 경로 추가/제거)는{" "}
              <b>펫 우클릭 → 연결된 계정</b>에서 관리합니다
            </div>
          </>
        )}
      </div>
    </div>
  );
}

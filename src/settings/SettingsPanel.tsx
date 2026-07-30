import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppSettings, CharacterRule, Summary } from "../types";
import "./settings.css";

const STATE_LABELS: [string, string][] = [
  ["working", "작업"],
  ["alert", "경고"],
  ["sleep", "잠"],
  ["exhausted", "소진"],
  ["refreshed", "초기화"],
];

export default function SettingsPanel() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [packs, setPacks] = useState<string[]>([]);
  const [observedModels, setObservedModels] = useState<string[]>([]);

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

  if (!s) {
    return (
      <div className="settings-root">
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
    update({ characterRules: [...(s.characterRules ?? []), { prefixes: "", pack: "" }] });
  const addRuleWith = (model: string) =>
    update({ characterRules: [...(s.characterRules ?? []), { prefixes: model, pack: "" }] });
  const removeRule = (i: number) =>
    update({ characterRules: (s.characterRules ?? []).filter((_, j) => j !== i) });
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
      <div className="settings-card">
        <div className="settings-head" data-tauri-drag-region>
          <span data-tauri-drag-region>설정</span>
          <button className="settings-close" onClick={() => void getCurrentWindow().hide()}>
            ✕
          </button>
        </div>

        <div className="settings-group">
          <div className="settings-label">캐릭터</div>
          <div className="settings-row">
            <select
              className="settings-select"
              value={s.characterPack ?? ""}
              onChange={(e) => update({ characterPack: e.currentTarget.value || null })}
            >
              <option value="">기본 (치비 드래곤)</option>
              {packs.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
            <button className="settings-btn" onClick={refreshPacks} title="팩 목록 새로고침">
              ↻
            </button>
            <button className="settings-btn" onClick={() => void invoke("open_characters_dir")}>
              폴더 열기
            </button>
          </div>
          <div className="settings-hint">
            characters/&lt;팩이름&gt;/ 에 idle(필수)·working·alert·sleep·exhausted·refreshed.(gif|png|webp) — 투명 배경
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">모델별 캐릭터 규칙 (최장 접두사 우선)</div>
          {(s.characterRules ?? []).map((r, i) => (
            <div className="settings-row" key={i}>
              <input
                className="settings-input"
                placeholder="접두사 (예: claude-opus 또는 gpt, o3)"
                value={r.prefixes}
                onChange={(e) => updateRule(i, { prefixes: e.currentTarget.value })}
              />
              <select
                className="settings-select rule-pack"
                value={r.pack}
                onChange={(e) => updateRule(i, { pack: e.currentTarget.value })}
              >
                <option value="">팩 선택</option>
                {packs.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
              <button className="settings-btn" onClick={() => removeRule(i)}>
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
              <div className="settings-hint">최근 관측된 모델 (클릭 → 규칙으로 추가):</div>
              <div className="chips">
                {observedModels.map((m) => (
                  <button key={m} className="chip-btn" onClick={() => addRuleWith(m)}>
                    {m}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>

        <div className="settings-group">
          <div className="settings-label">상태 사용 (끄면 해당 상태 대신 idle 유지)</div>
          <div className="state-toggles">
            {STATE_LABELS.map(([key, label]) => (
              <label key={key} className="settings-check inline">
                <input
                  type="checkbox"
                  checked={!(s.disabledStates ?? []).includes(key)}
                  onChange={(e) => toggleState(key, e.currentTarget.checked)}
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
              onChange={(e) => onScaleChange(parseInt(e.currentTarget.value, 10))}
            />
            <span className="settings-max">250%</span>
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            위험 한도 · 세션 <b className="warn-b">{Math.round(s.alertThreshold * 100)}%</b>
          </div>
          <div className="settings-row">
            <span className="settings-min">10%</span>
            <input
              type="range"
              min={10}
              max={100}
              step={5}
              value={Math.round(s.alertThreshold * 100)}
              onChange={(e) => update({ alertThreshold: parseInt(e.currentTarget.value, 10) / 100 })}
            />
            <span className="settings-max">100%</span>
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            위험 한도 · 주간 <b className="warn-b">{Math.round(s.weeklyAlertThreshold * 100)}%</b>
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
                update({ weeklyAlertThreshold: parseInt(e.currentTarget.value, 10) / 100 })
              }
            />
            <span className="settings-max">100%</span>
          </div>
          <div className="settings-hint">공식 세션/주간 사용률이 한도를 넘으면 펫이 경고 상태로 변합니다</div>
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
              onChange={(e) => update({ sleepAfterMinutes: parseInt(e.currentTarget.value, 10) })}
            />
            <span className="settings-max">2h</span>
          </div>
          <div className="settings-hint">마지막 AI 사용 후 이 시간이 지나면 캐릭터가 잠듭니다</div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            말풍선 숨김 지연 <b>{s.hoverDelayMs}ms</b>
          </div>
          <div className="settings-row">
            <span className="settings-min">0</span>
            <input
              type="range"
              min={0}
              max={1500}
              step={50}
              value={s.hoverDelayMs}
              onChange={(e) => update({ hoverDelayMs: parseInt(e.currentTarget.value, 10) })}
            />
            <span className="settings-max">1.5s</span>
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            블록 리셋 임박 알림{" "}
            <b>{s.resetNotifyMinutes === 0 ? "끔" : `${s.resetNotifyMinutes}분 전`}</b>
          </div>
          <div className="settings-row">
            <span className="settings-min">끔</span>
            <input
              type="range"
              min={0}
              max={120}
              step={5}
              value={s.resetNotifyMinutes}
              onChange={(e) => update({ resetNotifyMinutes: parseInt(e.currentTarget.value, 10) })}
            />
            <span className="settings-max">120분</span>
          </div>
          <div className="settings-hint">5분 주기로 확인하므로 5분 이상 권장</div>
        </div>

        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.clickThrough}
            onChange={(e) => update({ clickThrough: e.currentTarget.checked })}
          />
          클릭 통과 모드 <span className="settings-hint-inline">(호버·드래그 불가, 트레이에서 해제)</span>
        </label>

        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.showMiniLabel}
            onChange={(e) => update({ showMiniLabel: e.currentTarget.checked })}
          />
          미니 라벨 표시 <span className="settings-hint-inline">(발밑에 소진율·남은시간 상시)</span>
        </label>

        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.startHidden}
            onChange={(e) => update({ startHidden: e.currentTarget.checked })}
          />
          시작 시 펫 숨김 <span className="settings-hint-inline">(트레이로만 시작)</span>
        </label>

        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.autostart}
            onChange={(e) => update({ autostart: e.currentTarget.checked })}
          />
          로그인 시 자동 시작
        </label>
      </div>
    </div>
  );
}

import { invoke } from "@tauri-apps/api/core";
import type { CharacterRule } from "../../types";
import type { TabProps } from "./types";

interface Props extends TabProps {
  /** 설치된 캐릭터 팩 목록 */
  packs: string[];
  /** 실제로 관측된 모델 id — 규칙을 손으로 치지 않고 고르게 한다 */
  observedModels: string[];
  /** 크기 슬라이더 전용 빠른 경로 (드래그 중 실시간 리사이즈) */
  onScaleChange: (pct: number) => void;
  /** 스튜디오에서 팩을 만들고 돌아왔을 때 목록을 다시 읽는다 */
  refreshPacks: () => void;
}

/** 캐릭터 — 팩 선택·모델별 규칙·크기·말풍선 */
export default function CharacterTab({
  s,
  update,
  packs,
  observedModels,
  onScaleChange,
  refreshPacks,
}: Props) {
  // 규칙 편집은 이 탭 밖에서 쓰이지 않는다 — `s` 와 `update` 만 있으면 되므로
  // 껍데기가 아니라 여기 둔다.
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
      characterRules: [...(s.characterRules ?? []), { prefixes: model, pack: "" }],
    });
  const removeRule = (i: number) =>
    update({
      characterRules: (s.characterRules ?? []).filter((_, j) => j !== i),
    });

  return (
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
  );
}

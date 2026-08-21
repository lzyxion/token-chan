import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { AppSettings, PackConfig } from "../types";
import ResizeGrips from "../components/ResizeGrips";
import { SpeechField } from "../components/SpeechEditor";
import { useWindowPersist } from "../hooks/useWindowPersist";
import { DEFAULT_PACK_IMAGES } from "../pet/defaultPack";
import "../settings/settings.css";
import "./studio.css";

/** 상태 슬롯 — 앱 로직(한도·활동 감지)이 정하는 고정 집합이라 추가/발명은 불가.
 *  idle 은 모든 상태의 폴백이라 끌 수 없다.
 *  `speech` 는 그 상태에 속한 대사 상황들 — 카드 한 장에서 이미지와 대사를 같이 본다. */
const STATES: {
  key: string;
  label: string;
  toggleable: boolean;
  speech: [string, string][];
}[] = [
  { key: "idle", label: "평상시", toggleable: false, speech: [] },
  {
    key: "working",
    label: "작업",
    toggleable: true,
    speech: [["enter.working", "작업 시작"]],
  },
  // 완료는 `working` 에서 갈라져 나온 **자기 상태**다 — 작업 중이 벤더를 통틀어 하나인
  // 것과 달리 완료는 세션마다 따로 일어나므로, 작업 카드에 얹으면 둘의 단위가 어긋난다
  { key: "done", label: "작업 완료", toggleable: true, speech: [["done", "작업 완료 (세션마다)"]] },
  { key: "alert", label: "경고", toggleable: true, speech: [["enter.alert", "한도 경고"]] },
  {
    key: "sleep",
    label: "잠",
    toggleable: true,
    speech: [
      ["enter.sleep", "잠들 때"],
      ["leave.sleep", "깨어날 때"],
    ],
  },
  { key: "exhausted", label: "소진", toggleable: true, speech: [["enter.exhausted", "토큰 소진"]] },
  { key: "refreshed", label: "초기화", toggleable: true, speech: [["enter.refreshed", "블록 초기화"]] },
  { key: "poke", label: "클릭", toggleable: true, speech: [["poke", "클릭 시 (사용량 보고)"]] },
];

/** 상태에 안 딸린 대사 — 리셋 임박은 시각 기준이라 어떤 상태에서도 나올 수 있다 */
const EXTRA_SPEECH: [string, string][] = [["resetNotify", "리셋 임박 (변수: {분}·{시각}만)"]];

/** 대사 상황 → 그때 펫이 서 있는 상태. `enter.X` 는 X 로 들어가는 순간이고
 *  `leave.X` 는 이미 빠져나온 뒤라 idle 이다. 상태 이름 그대로인 상황(done·poke)은
 *  자기 상태, 어느 상태에도 안 붙은 상황(resetNotify)은 포즈를 바꾸지 않는다(null). */
const stateOfSpeech = (key: string): string | null => {
  if (key.startsWith("enter.")) return key.slice(6);
  if (key.startsWith("leave.")) return "idle";
  return STATES.some((st) => st.key === key) ? key : null;
};

/** 캐릭터 스튜디오 — 좌: 캐릭터 목록 / 우: 선택된 캐릭터의 상태·이미지·대사.
 *  "기본 캐릭터"(내장 팩)도 같은 자리에서 관리한다: 상태 사용은 전역 설정,
 *  대사는 기본 문구(settings.speechLines), 이미지는 내장이라 편집 불가. */
export default function CharacterStudio() {
  const [s, setS] = useState<AppSettings | null>(null);
  /** 모든 팩 폴더 (idle 없는 미완성 포함) / idle 이 있어 펫이 쓸 수 있는 팩 */
  const [dirs, setDirs] = useState<string[]>([]);
  const [validPacks, setValidPacks] = useState<string[]>([]);
  /** 선택된 캐릭터 ("" = 기본 캐릭터) */
  const [selected, setSelected] = useState("");
  /** 상태별 **자기 파일** 이미지 (폴백 없음 — null 이면 idle 폴백으로 동작한다는 뜻) */
  const [images, setImages] = useState<Record<string, string | null> | null>(null);
  const [config, setConfig] = useState<PackConfig>({ disabledStates: [] });
  const [packSpeech, setPackSpeech] = useState<Record<string, string[]>>({});
  const [newName, setNewName] = useState("");
  const [error, setError] = useState("");
  /** 인라인 이름 변경 중인 팩과 입력값 (null = 편집 아님) */
  const [renaming, setRenaming] = useState<{ pack: string; value: string } | null>(null);
  /** 드래그 중인 파일이 올라가 있는 상태 카드 (하이라이트용) */
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const refreshLists = () => {
    invoke<string[]>("list_character_dirs").then(setDirs).catch(() => setDirs([]));
    invoke<string[]>("list_character_packs")
      .then(setValidPacks)
      .catch(() => setValidPacks([]));
  };

  /** 선택된 팩을 다시 읽는 함수 — ↻ 가 이펙트 밖에서도 부를 수 있게 ref 로 잡아 둔다 */
  const loadSelectedRef = useRef<() => void>(() => {});

  /** ↻ — 폴더에서 직접 바꾼 내용을 다시 읽는다. 목록만이 아니라 **선택된 팩의 파일과
   *  펫까지** 같이 — 폴더 편집은 앱을 거치지 않아 아무 이벤트도 안 나므로, 사용자가
   *  "바꿨다" 고 알려 주는 이 순간이 유일하게 정확한 신호다 (파일 감시자나 주기적
   *  재읽기보다 싸고 정확하다). 펫은 `characters-refreshed` 를 듣고 캐시를 버린다.
   *  보낸 쪽(자기 자신)은 이미 여기서 읽었으므로 payload 로 걸러낸다. */
  const refreshAll = () => {
    refreshLists();
    loadSelectedRef.current();
    void emit("characters-refreshed", "studio");
  };

  useEffect(() => {
    let alive = true;
    invoke<AppSettings>("get_settings")
      .then((v) => {
        if (alive) setS(v);
      })
      .catch(() => {});
    refreshLists();
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

  // 폴더에서 직접 지우거나 넣은 변화를 창에 돌아왔을 때 자동 반영
  useEffect(() => {
    const un = getCurrentWindow().onFocusChanged(({ payload }) => {
      if (payload) refreshLists();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 이미지 파일을 상태 카드에 드래그&드롭으로 등록.
  // OS 파일 드롭은 Tauri 가 가로채 경로+좌표 이벤트로 주므로(HTML5 drop 아님)
  // 좌표를 CSS px 로 바꿔 어느 카드 위인지 직접 히트테스트한다.
  useEffect(() => {
    if (!selected) return; // 기본 캐릭터는 내장 이미지라 드롭 없음
    const stateAt = (x: number, y: number): string | null => {
      const el = document.elementFromPoint(x / window.devicePixelRatio, y / window.devicePixelRatio);
      return el?.closest<HTMLElement>("[data-state]")?.dataset.state ?? null;
    };
    const un = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over") {
        setDropTarget(stateAt(e.payload.position.x, e.payload.position.y));
      } else if (e.payload.type === "drop") {
        const state = stateAt(e.payload.position.x, e.payload.position.y);
        const path = e.payload.paths[0];
        setDropTarget(null);
        if (!state || !path) return;
        invoke("import_state_image_from_path", { pack: selected, state, path })
          .then(() => setError(""))
          .catch((err) => setError(String(err)));
      } else {
        setDropTarget(null);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [selected]);

  useWindowPersist("studio");

  // 선택된 팩의 이미지·설정·대사 로드 + 이미지 첨부/삭제 반영
  useEffect(() => {
    let alive = true;
    const load = () => {
      if (!selected) {
        setImages(DEFAULT_PACK_IMAGES);
        setConfig({ disabledStates: [] });
        setPackSpeech({});
        return;
      }
      invoke<Record<string, string | null>>("get_state_images", { pack: selected })
        .then((v) => {
          if (alive) setImages(v);
        })
        .catch(() => {
          if (alive) setImages(null);
        });
      invoke<PackConfig>("get_character_config", { pack: selected })
        .then((v) => {
          if (alive) setConfig(v ?? { disabledStates: [] });
        })
        .catch(() => {});
      invoke<Record<string, string[]> | null>("get_character_speech", { pack: selected })
        .then((v) => {
          if (alive) setPackSpeech(v ?? {});
        })
        .catch(() => {});
    };
    load();
    loadSelectedRef.current = load;
    const un = listen<string>("character-images-changed", (e) => {
      refreshLists(); // idle 이 생기면 미완성 표시가 풀린다
      if (e.payload === selected) load();
    });
    // 설정 창의 ↻ — 여기서 쏜 신호는 이미 처리했으니 건너뛴다
    const unAll = listen<string>("characters-refreshed", (e) => {
      if (e.payload === "studio") return;
      refreshLists();
      load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
      unAll.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  if (!s) {
    return (
      <div className="studio-root">
        <ResizeGrips />
        <div className="studio-card">불러오는 중…</div>
      </div>
    );
  }

  const updateSettings = (patch: Partial<AppSettings>) => {
    const next = { ...s, ...patch };
    setS(next);
    void invoke("set_settings", { newSettings: next });
  };

  /** 상태 사용 토글 — 기본 캐릭터는 전역 설정, 팩은 pack.json */
  const disabledStates = selected ? config.disabledStates : (s.disabledStates ?? []);
  const toggleState = (key: string, enabled: boolean) => {
    const cur = new Set(disabledStates);
    if (enabled) {
      cur.delete(key);
    } else {
      cur.add(key);
    }
    const list = [...cur];
    if (!selected) {
      updateSettings({ disabledStates: list });
    } else {
      const next = { ...config, disabledStates: list };
      setConfig(next);
      void invoke("set_character_config", { pack: selected, config: next });
    }
  };

  const updateSpeech = (key: string, raw: string) => {
    if (!selected) {
      const map = { ...(s.speechLines ?? {}) };
      if (raw === "") {
        delete map[key];
      } else {
        map[key] = raw.split("\n");
      }
      updateSettings({ speechLines: map });
    } else {
      const cur = { ...packSpeech };
      if (raw === "") {
        delete cur[key];
      } else {
        cur[key] = raw.split("\n");
      }
      setPackSpeech(cur);
      void invoke("set_character_speech", { pack: selected, lines: cur });
    }
  };

  const createPack = () => {
    const name = newName.trim();
    if (!name) return;
    invoke("create_character_pack", { name })
      .then(() => {
        setError("");
        setNewName("");
        refreshLists();
        setSelected(name);
      })
      .catch((e) => setError(String(e)));
  };

  /** ▶ 테스트 — 펫이 실제 경로(변수 치환 포함)로 그 문구를 말하게 한다.
   *  대사만으로는 그 상황의 절반만 보이므로 **포즈와 캐릭터도 같이** 보낸다 —
   *  지금 편집 중인 팩(selected)의 그 상태 이미지로, 말풍선이 떠 있는 동안.
   *  리셋 임박의 {분}·{시각}은 백엔드가 채우는 값이라 여기서 견본으로 채워 보낸다. */
  const testSpeech = (key: string, template: string) => {
    let t = template;
    if (key === "resetNotify") {
      const min = Math.max(1, s.resetNotifyMinutes || 15);
      const at = new Date(Date.now() + min * 60000);
      t = t
        .replace(/\{분\}/g, String(min))
        .replace(
          /\{시각\}/g,
          `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`,
        );
    }
    void emit("test-speech", {
      text: t,
      state: stateOfSpeech(key),
      pack: selected || null,
    });
  };

  /** 이름 변경 커밋 — 백엔드가 폴더와 설정 참조(선택 팩·규칙)를 함께 고친다 */
  const commitRename = () => {
    if (!renaming) return;
    const { pack, value } = renaming;
    const name = value.trim();
    if (!name || name === pack) {
      setRenaming(null);
      return;
    }
    invoke("rename_character_pack", { old: pack, new: name })
      .then(() => {
        setError("");
        setRenaming(null);
        if (selected === pack) setSelected(name);
        refreshLists();
      })
      .catch((e) => setError(String(e)));
  };

  /** 팩 폴더째 휴지통으로 — 되돌릴 수 있어 확인창은 안 띄운다 */
  const deletePack = (pack: string) => {
    invoke("delete_character_pack", { pack })
      .then(() => {
        setError("");
        if (selected === pack) setSelected("");
        refreshLists();
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div className="studio-root">
      <ResizeGrips />
      <div className="studio-card">
        <div className="settings-head" data-tauri-drag-region>
          <span data-tauri-drag-region>캐릭터 스튜디오</span>
          <button
            className="settings-close"
            onClick={() => void getCurrentWindow().hide()}
          >
            ✕
          </button>
        </div>

        <div className="studio-body">
          <aside className="studio-side">
            <div
              className={`studio-pack${selected === "" ? " active" : ""}`}
              onClick={() => setSelected("")}
            >
              <div className="studio-pack-info">
                기본 캐릭터
                <span className="studio-pack-sub">내장 · 토큰짱</span>
              </div>
            </div>
            {dirs.map((d) =>
              renaming?.pack === d ? (
                <div key={d} className="studio-pack active">
                  <input
                    className="settings-input studio-rename"
                    value={renaming.value}
                    autoFocus
                    onChange={(e) => setRenaming({ pack: d, value: e.currentTarget.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename();
                      if (e.key === "Escape") setRenaming(null);
                    }}
                    onBlur={() => setRenaming(null)}
                  />
                </div>
              ) : (
                <div
                  key={d}
                  className={`studio-pack${selected === d ? " active" : ""}`}
                  onClick={() => setSelected(d)}
                >
                  <div className="studio-pack-info">
                    {d}
                    {!validPacks.includes(d) && (
                      <span className="studio-pack-sub warn">idle 이미지 필요</span>
                    )}
                  </div>
                  <button
                    className="studio-pack-del"
                    title="이름 변경"
                    onClick={(e) => {
                      e.stopPropagation();
                      setRenaming({ pack: d, value: d });
                    }}
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M17 3a2.8 2.8 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                    </svg>
                  </button>
                  <button
                    className="studio-pack-del"
                    title="캐릭터 삭제"
                    onClick={(e) => {
                      e.stopPropagation();
                      deletePack(d);
                    }}
                  >
                    {/* 이모지는 색을 못 입혀 배경에 묻힌다 — currentColor 를 따르는 SVG 로 */}
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M3 6h18" />
                      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
                      <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                      <path d="M10 11v6M14 11v6" />
                    </svg>
                  </button>
                </div>
              ),
            )}
            <div className="studio-side-bottom">
              <input
                className="settings-input"
                placeholder="새 캐릭터 이름"
                value={newName}
                onChange={(e) => setNewName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") createPack();
                }}
              />
              <button className="settings-btn" onClick={createPack}>
                + 만들기
              </button>
              {error && <div className="settings-hint warn-b">{error}</div>}
              <div className="settings-row">
                <button
                  className="settings-btn studio-grow"
                  onClick={() => void invoke("open_characters_dir")}
                >
                  폴더 열기
                </button>
                <button
                  className="settings-btn"
                  onClick={refreshAll}
                  title="폴더에서 직접 바꾼 내용 다시 읽기 (펫에도 바로 반영)"
                >
                  ↻
                </button>
              </div>
            </div>
          </aside>

          <main className="studio-main">
            <div className="settings-group">
              <div className="settings-label">
                상태별 이미지 · 대사{" "}
                <span className="settings-hint-inline">
                  (상태를 끄거나 이미지가 없으면 평상시(idle)로 대신합니다)
                </span>
              </div>
              <div className="settings-hint">
                대사는 한 줄에 문구 하나 — 여러 줄이면 무작위, 비우면 흐린
                글씨의 문구로 폴백{" "}
                {selected
                  ? `· characters/${selected}/speech.json 에 저장`
                  : "· 기본 문구(모든 캐릭터의 폴백)를 편집 중"}
                <br />
                문구 칸을 클릭하면 넣을 수 있는 {"{변수}"} 칩이 아래에 뜹니다
                (클릭 = 커서 위치에 삽입) · | 는 말풍선 줄바꿈 · ▶ 테스트를
                누르면 펫이 실제로 말해봅니다
              </div>

              {STATES.map(({ key, label, toggleable, speech }) => {
                const has = images?.[key] != null;
                /* 자기 이미지가 없으면 실제 동작(idle 폴백)을 그대로 보여준다 —
                   점선 테두리·흐림이 "폴백 중"임을 구분한다 */
                const shown = images?.[key] ?? images?.idle ?? null;
                const enabled = !disabledStates.includes(key);
                return (
                  <div
                    className={`studio-card-row${dropTarget === key ? " drop-target" : ""}`}
                    key={key}
                    data-state={selected ? key : undefined}
                  >
                    <div className="studio-state-side">
                      {toggleable ? (
                        <label className="settings-check">
                          <input
                            type="checkbox"
                            checked={enabled}
                            onChange={(e) => toggleState(key, e.currentTarget.checked)}
                          />
                          {label}
                        </label>
                      ) : (
                        <span className="settings-check">
                          {label}
                          <span className="settings-hint-inline">(필수)</span>
                        </span>
                      )}
                      <div
                        className={`studio-thumb${has ? "" : " fallback"}${enabled ? "" : " off"}`}
                        title={has ? undefined : "자기 이미지 없음 — idle 폴백"}
                      >
                        {shown ? (
                          <img src={shown} alt="" />
                        ) : (
                          <span>이미지 필요</span>
                        )}
                      </div>
                      {selected ? (
                        <div className="studio-state-btns">
                          <button
                            className="settings-btn"
                            onClick={() =>
                              void invoke("import_state_image", {
                                pack: selected,
                                state: key,
                              })
                            }
                          >
                            {has ? "교체…" : "+ 이미지…"}
                          </button>
                          {has && key !== "idle" && (
                            <button
                              className="settings-btn"
                              onClick={() =>
                                void invoke("remove_state_image", {
                                  pack: selected,
                                  state: key,
                                })
                              }
                            >
                              ✕
                            </button>
                          )}
                        </div>
                      ) : (
                        <span className="settings-hint-inline">내장 이미지</span>
                      )}
                    </div>
                    <div className="studio-state-speech">
                      {speech.length === 0 ? (
                        <div className="settings-hint studio-idle-hint">
                          평상시 전용 대사는 없습니다 — 상황이 생길 때만
                          말합니다
                        </div>
                      ) : (
                        speech.map(([sk, sl]) => (
                          <SpeechField
                            key={sk}
                            situationKey={sk}
                            label={sl}
                            lines={selected ? packSpeech : (s.speechLines ?? {})}
                            baseLines={selected ? (s.speechLines ?? {}) : null}
                            onChange={updateSpeech}
                            onTest={testSpeech}
                          />
                        ))
                      )}
                    </div>
                  </div>
                );
              })}

              {/* 상태에 안 딸린 대사 — 이미지 열 없이 문구만 */}
              <div className="studio-card-row">
                <div className="studio-state-side">
                  <span className="settings-check">기타</span>
                </div>
                <div className="studio-state-speech">
                  {EXTRA_SPEECH.map(([sk, sl]) => (
                    <SpeechField
                      key={sk}
                      situationKey={sk}
                      label={sl}
                      lines={selected ? packSpeech : (s.speechLines ?? {})}
                      baseLines={selected ? (s.speechLines ?? {}) : null}
                      onChange={updateSpeech}
                      onTest={testSpeech}
                    />
                  ))}
                </div>
              </div>

              {selected !== "" && (
                <div className="settings-hint">
                  이미지는 gif · webp · apng · png · svg — 투명 배경 권장, 하단
                  중앙이 발 기준점 · 파일을 상태 카드에 끌어다 놓아도 등록됩니다
                </div>
              )}
            </div>

          </main>
        </div>
      </div>
    </div>
  );
}

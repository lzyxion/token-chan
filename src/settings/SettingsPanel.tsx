import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  Account,
  AppSettings,
  CharacterRule,
  GaugeSide,
  Source,
  Summary,
} from "../types";
import { fmtDuration } from "../format";
import { usePlans } from "../hooks/useUsage";
import ResizeGrips from "../components/ResizeGrips";
import VendorIcon from "../components/VendorIcon";
import "./settings.css";

/** 설정 탭 — 일반(비용·게이지·시스템) / 알림(한도·리셋·작업 완료·잠자기) /
 *  캐릭터(팩·규칙·크기·말풍선) / 계정(계정·홈·플랜).
 *  상태 사용·이미지·대사 편집은 캐릭터 스튜디오(전용 창)가 맡는다. */

type Tab = "general" | "alerts" | "character" | "account";
const TABS: [Tab, string][] = [
  ["general", "일반"],
  ["alerts", "알림"],
  ["character", "캐릭터"],
  ["account", "계정"],
];

/** 홈 추가/제거 커맨드가 받는 소스 키 — `Source` 와 같은 문자열을 쓴다 */
const HOME_SOURCES: [Source, string][] = [
  ["claude", "Claude"],
  ["codex", "Codex"],
  ["antigravity", "Antigravity"],
];

export default function SettingsPanel() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [packs, setPacks] = useState<string[]>([]);
  const [observedModels, setObservedModels] = useState<string[]>([]);
  const [tab, setTab] = useState<Tab>("general");
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  // 플랜은 `usePlans` 로 **구독**한다. 손으로 한 번만 읽으면 Codex 칩이 빈 채로 굳는다 —
  // Codex 플랜은 rollout 스캔이 끝나야 나오는데(부팅 후 10초 남짓), 이 창은 그보다
  // 먼저 뜬다. Claude 는 계정 파일에서 바로 와서(`Account.plan`) 티가 안 났다.
  const plans = usePlans();
  // 다시 검색은 별도 스레드에서 돌고 끝나면 accounts-changed 로 알려 온다
  const [rescanning, setRescanning] = useState(false);

  const refreshPacks = () => {
    invoke<string[]>("list_character_packs")
      .then(setPacks)
      .catch(() => setPacks([]));
    invoke<Summary | null>("get_summary")
      .then((sum) => setObservedModels(sum?.observed_models ?? []))
      .catch(() => {});
  };

  // 검색이 끝났다는 신호는 `accounts-changed` 하나뿐이라, 그게 안 오면 버튼이 영영
  // 잠긴다. 실측 최악(마커 스캔 + WSL 재조회)이 수 초라 넉넉히 잡고 풀어 준다.
  useEffect(() => {
    if (!rescanning) return;
    const t = setTimeout(() => setRescanning(false), 20000);
    return () => clearTimeout(t);
  }, [rescanning]);

  const refreshAccounts = () => {
    invoke<Account[]>("get_accounts")
      .then(setAccounts)
      .catch(() => setAccounts([]));
  };

  useEffect(() => {
    let alive = true;
    invoke<AppSettings>("get_settings")
      .then((v) => {
        if (alive) setS(v);
      })
      .catch(() => {});
    refreshPacks();
    refreshAccounts();
    // 트레이 토글 등 외부 변경과 동기화 (내용 동일하면 스킵 — 입력 커서 보존)
    const un = listen<AppSettings>("settings-changed", (e) => {
      if (alive) {
        setS((prev) =>
          JSON.stringify(prev) === JSON.stringify(e.payload) ? prev : e.payload,
        );
      }
    });
    // 계정 목록은 다시 검색·홈 추가/제거 뒤에 백엔드가 알려 준다 (오래 걸려 응답으로 못 준다)
    const unAcct = listen("accounts-changed", () => {
      if (alive) {
        setRescanning(false);
        refreshAccounts();
      }
    });
    // 트레이의 "계정 설정…" 으로 열면 그 탭으로 바로 간다
    const unTab = listen<string>("settings-tab", (e) => {
      if (alive && TABS.some(([k]) => k === e.payload)) setTab(e.payload as Tab);
    });
    return () => {
      alive = false;
      un.then((f) => f());
      unAcct.then((f) => f());
      unTab.then((f) => f());
    };
  }, []);

  // 사용자가 옮긴 위치·조절한 크기 기억 (연속 이벤트 debounce — 패널과 동일).
  // 이 창은 최상단이 아니라 뒤로 갔다가 다시 불려 오는 일이 잦다 — 그때마다 자리가
  // 튀지 않으려면 위치도 기억해야 한다.
  useEffect(() => {
    const win = getCurrentWindow();
    let moveTimer: ReturnType<typeof setTimeout> | undefined;
    let sizeTimer: ReturnType<typeof setTimeout> | undefined;
    const unMoved = win.onMoved(({ payload }) => {
      clearTimeout(moveTimer);
      moveTimer = setTimeout(() => {
        void invoke("save_window_position", { label: "settings", x: payload.x, y: payload.y });
      }, 500);
    });
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
      clearTimeout(moveTimer);
      clearTimeout(sizeTimer);
      unMoved.then((f) => f());
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
              <div className="settings-label">비용 표기</div>
              <div className="settings-row">
                <select
                  className="settings-select"
                  value={s.currency ?? "usd"}
                  onChange={(e) =>
                    update({ currency: e.currentTarget.value as AppSettings["currency"] })
                  }
                >
                  <option value="usd">달러 · $</option>
                  <option value="krw">원 · ₩</option>
                </select>
              </div>
              {/* 환율은 원 표기일 때만 물어본다 — 달러로 두면 쓰이지 않는 값이다 */}
              {s.currency === "krw" && (
                <>
                  <div className="settings-row">
                    <span className="settings-sublabel">1 USD =</span>
                    <input
                      className="settings-input rate-input"
                      type="number"
                      min={1}
                      max={100000}
                      step={10}
                      value={s.usdToKrw || 1400}
                      onChange={(e) =>
                        update({ usdToKrw: parseFloat(e.currentTarget.value) || 0 })
                      }
                    />
                    <span className="settings-sublabel">원</span>
                  </div>
                  <div className="settings-hint">환율은 직접 넣습니다.</div>
                </>
              )}
            </div>
          </>
        )}

        {tab === "alerts" && (
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
              <div className="settings-label">
                작업 완료 대사{" "}
                <b>
                  {s.doneNoticeSeconds === 0
                    ? "끔"
                    : `${fmtDuration(s.doneNoticeSeconds)} 이상`}
                </b>
              </div>
              <div className="settings-row">
                <span className="settings-min">끔</span>
                <input
                  type="range"
                  min={0}
                  max={600}
                  step={10}
                  value={s.doneNoticeSeconds}
                  onChange={(e) =>
                    update({
                      doneNoticeSeconds: parseInt(e.currentTarget.value, 10),
                    })
                  }
                />
                <span className="settings-max">10분</span>
              </div>
              <div className="settings-hint">
                이보다 오래 걸린 작업이 끝나면 세션마다 알려줍니다
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
          </>
        )}

        {tab === "general" && (
          <>
            <div className="settings-group">
              <div className="settings-label">소진율 게이지</div>
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
                  <span className="settings-sublabel">모양</span>
                  <select
                    className="settings-select"
                    value={s.gaugeStyle ?? "ring"}
                    onChange={(e) =>
                      update({ gaugeStyle: e.currentTarget.value as AppSettings["gaugeStyle"] })
                    }
                  >
                    <option value="ring">도넛 링</option>
                    <option value="bar">HP 바 (RPG)</option>
                  </select>
                </div>
              )}
              {/* HP 바는 채움이 "남은 양"이다 — 링(사용률 채움)과 반대라 여기서 못 박는다 */}
              {s.gaugeSide !== "off" && s.gaugeStyle === "bar" && (
                <div className="settings-hint">
                  바는 남은 양입니다 — 가득 = 리셋 직후, 빈 바 = 소진
                </div>
              )}
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
                    <option value="auto">자동 (작업 중인 벤더)</option>
                    <option value="claude">Claude 고정</option>
                    <option value="codex">Codex 고정</option>
                    <option value="antigravity">Antigravity 고정</option>
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
              데이터 소스(계정 켜고 끄기·홈 경로)는 <b>계정</b> 탭에서 관리합니다
            </div>
          </>
        )}

        {tab === "account" && (
          <>
            <div className="settings-group">
              <div className="settings-label row">
                연결된 계정
                <button
                  className="settings-btn"
                  disabled={rescanning}
                  onClick={() => {
                    setRescanning(true);
                    void invoke("rescan_accounts");
                  }}
                  title="표준 위치 + 마커 스캔 + WSL 배포판을 다시 훑습니다"
                >
                  {rescanning ? "검색 중…" : "다시 검색"}
                </button>
              </div>

              {accounts == null ? (
                <div className="settings-hint">불러오는 중…</div>
              ) : accounts.length === 0 ? (
                <div className="settings-hint">
                  발견된 계정이 없습니다. CLI 를 한 번도 실행하지 않았거나, 홈이 표준
                  위치 밖에 있을 수 있습니다 — 아래에서 홈을 직접 추가해 보세요.
                </div>
              ) : (
                accounts.map((a) => {
                  // 플랜이 오는 길이 둘이다. 계정 파일에서 온 것(Claude)은 그 계정 것이
                  // 확실하지만, 소스 단위로 온 것(Codex 의 rollout)은 CLI 가 지금 로그인된
                  // 계정 기준이라 같은 소스에 계정이 여럿이면 누구 것인지 못 가린다.
                  const fromSource = plans.find((p) => p.source === a.source)?.detail;
                  // 플랜이 안 온다는 게 뜻하는 바가 **소스마다 다르다.** 한 규칙으로
                  // 묶어 Free 를 박으면 셋 중 둘에서 거짓말이 된다:
                  //
                  // - Codex 는 무료면 `plan_type: "free"` 를 **직접 보낸다**(실측 90건 중
                  //   65건). 값이 없다 = 무료가 아니라, 스캔 범위에 `rate_limits` 가 든
                  //   rollout 이 아직 없다는 뜻이다 (한동안 안 쓴 계정).
                  // - Claude 는 무료 티어로 Claude Code 를 못 쓴다. 카드가 있다 = 유료라,
                  //   값이 없으면 티어를 못 읽은 것이다.
                  // - agy 만 플랜 **개념 자체가 없다**. 여기서만 Free 가 사실이다.
                  const noPlanConcept = a.source === "antigravity";
                  const planText = a.plan || fromSource || (noPlanConcept ? "Free" : "");
                  // "?" 는 **소스 단위로 온 플랜**이 이 계정 것인지 모른다는 뜻이다.
                  // 추론한 Free 에는 붙이지 않는다 — 거긴 가릴 플랜 자체가 없다.
                  const ambiguous =
                    !a.plan &&
                    !!fromSource &&
                    accounts.filter((x) => x.source === a.source).length > 1;
                  return (
                    <div className="account-card" key={a.setting_key}>
                      <label className="account-head">
                        {/* 체크가 곧 집계 포함 여부 — 트레이 체크와 같은 커맨드를 쓴다 */}
                        <input
                          type="checkbox"
                          checked={a.enabled}
                          onChange={(e) =>
                            void invoke("set_account_enabled", {
                              settingKey: a.setting_key,
                              enabled: e.currentTarget.checked,
                            })
                          }
                        />
                        <VendorIcon source={a.source} size={14} />
                        <span className="account-name">{a.label}</span>
                        {planText && (
                          <span
                            className={`account-badge plan${ambiguous ? " dim" : ""}`}
                            title={
                              ambiguous
                                ? "이 소스에 계정이 여럿이라 어느 계정의 플랜인지 구분할 수 없습니다 — CLI 가 지금 로그인된 계정 기준입니다."
                                : a.plan
                                  ? "이 계정의 설정 파일에 적힌 플랜입니다."
                                  : fromSource
                                    ? "CLI 가 지금 로그인된 계정 기준입니다."
                                    : "Antigravity 는 플랜 구분이 없습니다."
                            }
                          >
                            {planText}
                            {ambiguous ? " ?" : ""}
                          </span>
                        )}
                        {/* 표준 위치에서 안 나온 계정은 오래된 백업일 수 있어 조용히
                            합산되면 안 된다 — 왜 기본이 꺼짐인지 여기서 알려 준다 */}
                        {!a.standard && (
                          <span
                            className="account-badge warn"
                            title="표준 위치가 아니라 마커 스캔으로만 발견됐습니다. 오래된 백업일 수 있어 기본적으로 집계에서 빠집니다."
                          >
                            스캔으로 발견
                          </span>
                        )}
                      </label>

                      {a.detail && <div className="account-detail">{a.detail}</div>}

                      <div className="account-homes">
                        {a.installs.map((i) => (
                          <div className="account-home" key={i.home} title={i.home}>
                            <span className="account-home-path">{i.home}</span>
                            <span className="account-badge">
                              {i.discovered ? "스캔" : "표준"}
                            </span>
                          </div>
                        ))}
                      </div>
                    </div>
                  );
                })
              )}
            </div>

            <div className="settings-group">
              <div className="settings-label">직접 추가한 홈</div>
              <div className="settings-hint">
                자동 탐지는 앱을 어떻게 띄웠는지에 좌우됩니다. 위 목록에 빠진 설치본이
                있으면 홈을 직접 지정하세요 — Claude 는 <code>.claude</code> 를 담고 있는
                폴더, Codex·Antigravity 는 홈 폴더 자체입니다.
              </div>

              {HOME_SOURCES.map(([src, disp]) => {
                const list =
                  src === "codex"
                    ? s.extraCodexHomes
                    : src === "antigravity"
                      ? s.extraAntigravityHomes
                      : s.extraClaudeHomes;
                return (
                  <div className="home-block" key={src}>
                    <div className="home-head">
                      <VendorIcon source={src} size={12} />
                      <span>{disp}</span>
                      <button
                        className="settings-btn"
                        onClick={() => void invoke("add_home", { source: src })}
                      >
                        홈 추가…
                      </button>
                    </div>
                    {list.filter((p) => p.trim()).length === 0 ? (
                      <div className="home-empty">없음 (자동 탐지만 사용)</div>
                    ) : (
                      list
                        .filter((p) => p.trim())
                        .map((p) => (
                          <div className="home-row" key={p} title={p}>
                            <span className="home-path">{p}</span>
                            <button
                              className="settings-btn danger"
                              onClick={() =>
                                void invoke("remove_home", { source: src, path: p })
                              }
                              title="이 경로를 스캔 대상에서 제거"
                            >
                              ✕
                            </button>
                          </div>
                        ))
                    )}
                  </div>
                );
              })}
            </div>

          </>
        )}
      </div>
    </div>
  );
}

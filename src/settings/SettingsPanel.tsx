import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Account, AppSettings, Summary } from "../types";
import { usePlans } from "../hooks/useUsage";
import { useWindowPersist } from "../hooks/useWindowPersist";
import ResizeGrips from "../components/ResizeGrips";
import AccountTab from "./tabs/AccountTab";
import AlertsTab from "./tabs/AlertsTab";
import CharacterTab from "./tabs/CharacterTab";
import GeneralTab from "./tabs/GeneralTab";
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
  // 설정 저장 실패 배너 (null = 정상). 실패는 이 창이 닫혀 있는 동안에도
  // 일어나므로(드래그·트레이 토글) 마운트 시 현재 상태를 묻고, 이후엔
  // 백엔드가 성공↔실패 전환 시에만 보내는 이벤트를 받는다.
  const [saveError, setSaveError] = useState<string | null>(null);
  // ✕ 로 닫으면 같은 실패가 지속되는 동안은 다시 안 띄운다. 복구됐다가
  // **새로 실패하면**(전환 이벤트) 다시 보여야 하므로 그때 리셋한다.
  const [saveErrorDismissed, setSaveErrorDismissed] = useState(false);

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
    invoke<string | null>("get_save_error")
      .then((v) => {
        if (alive) setSaveError(v);
      })
      .catch(() => {});
    const unSave = listen<string | null>("settings-save-error", (e) => {
      if (alive) {
        setSaveError(e.payload);
        setSaveErrorDismissed(false);
      }
    });
    return () => {
      alive = false;
      un.then((f) => f());
      unAcct.then((f) => f());
      unTab.then((f) => f());
      unSave.then((f) => f());
    };
  }, []);

  // 이 창은 최상단이 아니라 뒤로 갔다가 다시 불려 오는 일이 잦다 — 그때마다 자리가
  // 튀지 않으려면 위치도 기억해야 한다.
  useWindowPersist("settings");

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

  return (
    <div className="settings-root">
      <ResizeGrips />
      {/* 저장 실패 배너 — 카드 **위에 띄우는** 오버레이. 문서 흐름에 넣으면
          기존 설정 내용이 통째로 밀려 내려가고, 탭 아래에 띄우면 내비게이션을
          가린다. 제목줄("설정" 글자)만 덮고 ✕·탭·콘텐츠는 그대로 보이도록
          스크롤 컨테이너(.settings-card) 밖에 둔다. */}
      {saveError && !saveErrorDismissed && (
        <div className="settings-save-error" role="alert">
          <span className="settings-save-error-text">
            ⚠️ 설정 저장 실패: {saveError} — 변경 사항이 파일에 반영되지 않고
            있습니다. 디스크 공간·권한을 확인한 뒤 아무 설정이나 바꾸면 다시
            저장을 시도합니다.
          </span>
          <button
            className="settings-save-error-close"
            title="닫기 (같은 오류가 지속되는 동안 다시 띄우지 않음)"
            onClick={() => setSaveErrorDismissed(true)}
          >
            ✕
          </button>
        </div>
      )}
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
          <CharacterTab
            s={s}
            update={update}
            packs={packs}
            observedModels={observedModels}
            onScaleChange={onScaleChange}
            refreshPacks={refreshPacks}
          />
        )}

        {tab === "general" && <GeneralTab s={s} update={update} />}

        {tab === "alerts" && <AlertsTab s={s} update={update} />}

        {tab === "account" && (
          <AccountTab
            s={s}
            accounts={accounts}
            plans={plans}
            rescanning={rescanning}
            setRescanning={setRescanning}
          />
        )}
      </div>
    </div>
  );
}

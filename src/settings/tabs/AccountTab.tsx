import { invoke } from "@tauri-apps/api/core";
import type { Account, AppSettings, PlanUsage } from "../../types";
import { SOURCE_LABEL, SOURCES } from "../../format";
import VendorIcon from "../../components/VendorIcon";

interface Props {
  s: AppSettings;
  /** null = 아직 못 읽음 (빈 배열과 다르다 — 그건 "계정 없음") */
  accounts: Account[] | null;
  plans: PlanUsage[];
  /** 다시 검색이 도는 중 — 끝나면 accounts-changed 가 온다 */
  rescanning: boolean;
  setRescanning: (v: boolean) => void;
}

/** 계정 — 연결된 계정 켜고 끄기·추가 홈 경로·공식 플랜 */
export default function AccountTab({ s, accounts, plans, rescanning, setRescanning }: Props) {
  return (
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
                    {/* 배지 순서 = 화면 왼→오. 플랜(티어)을 **맨 마지막**에 두어 모든 줄에서
                        같은 자리(오른쪽 끝)에 오게 한다 — 줄마다 티어 위치가 들쭉날쭉하면
                        여러 계정의 플랜을 훑어볼 수 없다. 나머지 배지는 그 왼쪽에 쌓인다. */}

                    {/* 어디에 사는 계정인지 — WSL 일 때만 붙인다. Windows 는 기본값이라
                        모든 줄에 배지를 달면 소음이 된다. 켜는 것의 대가를 여기서 밝힌다 */}
                    {a.wsl_distro && (
                      <span
                        className="account-badge warn"
                        title={
                          `${a.wsl_distro} 안의 계정입니다. 켜면 사용량을 읽으려고 이 배포판의 ` +
                          `파일을 몇 초마다 열어야 해서, wsl --shutdown 으로 꺼도 곧 다시 켜집니다. ` +
                          `그래서 기본적으로 집계에서 빠집니다 — WSL 을 잠재워 두려면 꺼 두세요.`
                        }
                      >
                        WSL: {a.wsl_distro}
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
                        {/* "?" 는 값이 아니라 **출처에 대한 표시**다 — 이 줄의 계정이
                            자기 플랜을 못 밝혔고(계정 파일을 못 읽음), 소스 단위 플랜을
                            빌려 왔는데 그 소스에 계정이 여럿이라 누구 것인지 못 가린다는 뜻.
                            빌려 온 값을 아무 표시 없이 적으면 그 계정의 플랜이라는 거짓말이 된다. */}
                        {planText}
                        {ambiguous ? " ?" : ""}
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

          {SOURCES.map((src) => {
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
                  <span>{SOURCE_LABEL[src]}</span>
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
  );
}

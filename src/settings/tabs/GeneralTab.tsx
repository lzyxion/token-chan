import type { Account, AppSettings, GaugeSide } from "../../types";
import {
  defaultFill,
  enabledSources,
  fillsRemaining,
  GAUGE_FILLS,
  GAUGE_LABEL_SHOWS,
  gaugeFillLabel,
  gaugeFillOf,
  gaugeLabelShowLabel,
  gaugeLabelShowOf,
  GAUGE_STYLES,
  gaugeStyleLabel,
  gaugeStyleOf,
  SOURCE_LABEL,
  SOURCES,
} from "../../format";
import { useI18n } from "../../i18n";
import type { TabProps } from "./types";

interface Props extends TabProps {
  /** 계정 목록 (null = 아직 못 읽음). 게이지에 실을 수 있는 벤더를 여기서 가린다 */
  accounts: Account[] | null;
}

/** 일반 — 비용 표기·소진율 게이지·시스템 동작.
 *
 *  예전에는 이 내용이 `{tab === "general" && …}` 두 조각으로 갈려 있었고 그 사이에
 *  알림 탭 블록이 끼어 있었다. 동작은 맞지만 뒤쪽 조각은 찾지 못한다 — 탭 하나가
 *  파일 하나면 그 종류의 사고가 아예 생기지 않는다. */
export default function GeneralTab({ s, update, accounts }: Props) {
  const { language, t } = useI18n();
  // 계정을 꺼 둔 벤더는 게이지에 실을 게 없다 — 목록에서 뺀다.
  // 고정해 둔 벤더가 꺼졌을 때 "자동" 으로 되돌리는 건 백엔드가 한다
  // (`set_account_enabled`) — 이 탭이 열려 있지 않을 때도 성립해야 한다.
  const pickable = enabledSources(accounts) ?? SOURCES;

  // 채움 방향은 반쯤 찬 게이지만 봐서는 구분되지 않고, "기본값" 이 무엇으로 풀리는지도
  // 화면 어디에도 없다. 그래서 고른 값을 되풀이하지 않고 **지금 적용되는 방향**을
  // 밝히고, 그게 뜻하는 그림을 덧붙인다.
  const style = gaugeStyleOf(s.gaugeStyle);
  // 셋 다 "언제 보이나" 가 헷갈릴 수 있어 고른 값의 뜻을 한 줄로 되돌려 준다
  const labelHint = {
    hover: t("펫에 마우스를 올렸을 때만 펼쳐집니다.", "Expands only while the pointer is over the pet."),
    busy: t(
      "작업 중에는 계속 펼쳐져 경과 시간이 보입니다. 그 외에는 마우스를 올렸을 때만.",
      "Stays expanded with elapsed time while working; otherwise, only on hover.",
    ),
    always: t("항상 펼쳐 둡니다.", "Always stays expanded."),
  }[gaugeLabelShowOf(s.gaugeLabelShow)];
  const remaining = fillsRemaining(style, s.gaugeFill);
  const fillHint =
    (gaugeFillOf(s.gaugeFill) === "auto"
      ? t(
          `${gaugeStyleLabel(style, language).replace(/\s*\(.+\)$/, "")}의 기본값은 ${gaugeFillLabel(defaultFill(style), language)}입니다`,
          `${gaugeStyleLabel(style, language).replace(/\s*\(.+\)$/, "")} defaults to ${gaugeFillLabel(defaultFill(style), language).toLowerCase()}`,
        )
      : t(
          `채움이 ${gaugeFillLabel(remaining ? "left" : "used", language)}입니다`,
          `Fill represents ${gaugeFillLabel(remaining ? "left" : "used", language).toLowerCase()}`,
        )) +
    (remaining
      ? t(" — 가득 = 리셋 직후, 비면 소진", " — full after reset, empty when exhausted")
      : t(" — 비었을 때가 리셋 직후, 가득 차면 소진", " — empty after reset, full when exhausted"));

  return (
    <>
        <div className="settings-group">
          <div className="settings-label">{t("언어", "Language")}</div>
          <div className="settings-row">
            <select
              className="settings-select"
              value={s.language ?? "en"}
              onChange={(e) =>
                update({ language: e.currentTarget.value as AppSettings["language"] })
              }
            >
              <option value="ko">한국어</option>
              <option value="en">English</option>
            </select>
          </div>
        </div>
        <div className="settings-group">
          <div className="settings-label">{t("비용 표기", "Cost display")}</div>
          <div className="settings-row">
            <select
              className="settings-select"
              value={s.currency ?? "usd"}
              onChange={(e) =>
                update({ currency: e.currentTarget.value as AppSettings["currency"] })
              }
            >
              <option value="usd">{t("달러", "US dollar")} · $</option>
              <option value="krw">{t("원", "Korean won")} · ₩</option>
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
                <span className="settings-sublabel">{t("원", "KRW")}</span>
              </div>
              <div className="settings-hint">{t("환율은 직접 넣습니다.", "Enter the exchange rate manually.")}</div>
            </>
          )}
        </div>
        <div className="settings-group">
          <div className="settings-label">{t("소진율 게이지", "Usage gauge")}</div>
          <div className="settings-row">
            <select
              className="settings-select"
              value={s.gaugeSide}
              onChange={(e) =>
                update({ gaugeSide: e.currentTarget.value as GaugeSide })
              }
            >
              <option value="right">{t("캐릭터 오른쪽", "Right of character")}</option>
              <option value="left">{t("캐릭터 왼쪽", "Left of character")}</option>
              <option value="off">{t("표시 안 함", "Hidden")}</option>
            </select>
          </div>
          {s.gaugeSide !== "off" && (
            <div className="settings-row">
              <span className="settings-sublabel">{t("모양", "Style")}</span>
              <select
                className="settings-select"
                value={gaugeStyleOf(s.gaugeStyle)}
                onChange={(e) =>
                  update({ gaugeStyle: e.currentTarget.value as AppSettings["gaugeStyle"] })
                }
              >
                {GAUGE_STYLES.map((g) => (
                  <option key={g} value={g}>
                    {gaugeStyleLabel(g, language)}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && (
            <div className="settings-row">
              <span className="settings-sublabel">{t("채움", "Fill")}</span>
              <select
                className="settings-select"
                value={gaugeFillOf(s.gaugeFill)}
                onChange={(e) =>
                  update({ gaugeFill: e.currentTarget.value as AppSettings["gaugeFill"] })
                }
              >
                {GAUGE_FILLS.map((f) => (
                  <option key={f} value={f}>
                    {gaugeFillLabel(f, language)}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && <div className="settings-hint">{fillHint}</div>}
          {s.gaugeSide !== "off" && (
            <div className="settings-row">
              <span className="settings-sublabel">{t("보여줄 벤더", "Provider")}</span>
              <select
                className="settings-select"
                value={s.gaugeVendor ?? "auto"}
                onChange={(e) =>
                  update({ gaugeVendor: e.currentTarget.value as AppSettings["gaugeVendor"] })
                }
              >
                <option value="auto">{t("자동 (작업 중인 벤더)", "Automatic (active provider)")}</option>
                {pickable.map((src) => (
                  <option key={src} value={src}>
                    {t(`${SOURCE_LABEL[src]} 고정`, `Pin ${SOURCE_LABEL[src]}`)}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && (
            <div className="settings-row">
              <span className="settings-sublabel">{t("라벨", "Labels")}</span>
              <select
                className="settings-select"
                value={gaugeLabelShowOf(s.gaugeLabelShow)}
                onChange={(e) =>
                  update({
                    gaugeLabelShow: e.currentTarget.value as AppSettings["gaugeLabelShow"],
                  })
                }
              >
                {GAUGE_LABEL_SHOWS.map((v) => (
                  <option key={v} value={v}>
                    {gaugeLabelShowLabel(v, language)}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && (
            <div className="settings-hint">{labelHint}</div>
          )}
        </div>


        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.startHidden}
            onChange={(e) =>
              update({ startHidden: e.currentTarget.checked })
            }
          />
          {t("시작 시 펫 숨김", "Hide pet on launch")}{" "}
          <span className="settings-hint-inline">{t("(트레이로만 시작)", "(start in tray)")}</span>
        </label>

        <label className="settings-check">
          <input
            type="checkbox"
            checked={s.autostart}
            onChange={(e) => update({ autostart: e.currentTarget.checked })}
          />
          {t("로그인 시 자동 시작", "Launch at login")}
        </label>

        <div className="settings-hint">
          {t("데이터 소스(계정 켜고 끄기·홈 경로)는", "Manage data sources, account inclusion, and home directories in the")} <b>{t("계정", "Accounts")}</b> {t("탭에서 관리합니다", "tab.")}
        </div>
    </>
  );
}

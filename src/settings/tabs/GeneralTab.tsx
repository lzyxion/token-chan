import type { Account, AppSettings, GaugeSide } from "../../types";
import {
  defaultFill,
  enabledSources,
  fillsRemaining,
  GAUGE_FILL_LABEL,
  GAUGE_FILLS,
  gaugeFillOf,
  GAUGE_STYLE_LABEL,
  GAUGE_STYLES,
  gaugeStyleOf,
  SOURCE_LABEL,
  SOURCES,
} from "../../format";
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
  // 계정을 꺼 둔 벤더는 게이지에 실을 게 없다 — 목록에서 뺀다.
  // 고정해 둔 벤더가 꺼졌을 때 "자동" 으로 되돌리는 건 백엔드가 한다
  // (`set_account_enabled`) — 이 탭이 열려 있지 않을 때도 성립해야 한다.
  const pickable = enabledSources(accounts) ?? SOURCES;

  // 채움 방향은 반쯤 찬 게이지만 봐서는 구분되지 않고, "기본값" 이 무엇으로 풀리는지도
  // 화면 어디에도 없다. 그래서 고른 값을 되풀이하지 않고 **지금 적용되는 방향**을
  // 밝히고, 그게 뜻하는 그림을 덧붙인다.
  const style = gaugeStyleOf(s.gaugeStyle);
  const remaining = fillsRemaining(style, s.gaugeFill);
  const fillHint =
    (gaugeFillOf(s.gaugeFill) === "auto"
      ? // 모양 이름의 괄호 설명("HP 바 (RPG)")은 문장 안에서 걸린다
        `${GAUGE_STYLE_LABEL[style].replace(/\s*\(.+\)$/, "")}의 기본값은 ` +
        `${GAUGE_FILL_LABEL[defaultFill(style)]}입니다`
      : `채움이 ${GAUGE_FILL_LABEL[remaining ? "left" : "used"]}입니다`) +
    (remaining
      ? " — 가득 = 리셋 직후, 비면 소진"
      : " — 비었을 때가 리셋 직후, 가득 차면 소진");

  return (
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
                value={gaugeStyleOf(s.gaugeStyle)}
                onChange={(e) =>
                  update({ gaugeStyle: e.currentTarget.value as AppSettings["gaugeStyle"] })
                }
              >
                {GAUGE_STYLES.map((g) => (
                  <option key={g} value={g}>
                    {GAUGE_STYLE_LABEL[g]}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && (
            <div className="settings-row">
              <span className="settings-sublabel">채움</span>
              <select
                className="settings-select"
                value={gaugeFillOf(s.gaugeFill)}
                onChange={(e) =>
                  update({ gaugeFill: e.currentTarget.value as AppSettings["gaugeFill"] })
                }
              >
                {GAUGE_FILLS.map((f) => (
                  <option key={f} value={f}>
                    {GAUGE_FILL_LABEL[f]}
                  </option>
                ))}
              </select>
            </div>
          )}
          {s.gaugeSide !== "off" && <div className="settings-hint">{fillHint}</div>}
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
                {pickable.map((src) => (
                  <option key={src} value={src}>
                    {SOURCE_LABEL[src]} 고정
                  </option>
                ))}
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
  );
}

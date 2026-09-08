import { fmtDuration } from "../../format";
import { useI18n } from "../../i18n";
import type { TabProps } from "./types";

/** 알림 — 위험 한도·리셋 임박·작업 완료·잠자기 */
export default function AlertsTab({ s, update }: TabProps) {
  const { language, t } = useI18n();
  return (
    <>
        <div className="settings-group">
          <div className="settings-label">
            {t("위험 한도 · 컨텍스트", "Warning threshold · Context")}{" "}
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
            {t("활성 벤더의 컨텍스트가 이만큼 차면 경고 — 곧 압축(compact)됩니다", "Warn when the active provider's context reaches this level — compaction is near.")}
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            {t("위험 한도 · 공식 한도", "Warning threshold · Official limits")}{" "}
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
          <div className="settings-hint">
            {t("5시간·주간·월간에 모두 적용 — 넘으면 펫이 경고하고 게이지도 빨강 (한도의 75%부터 노랑)", "Applies to every rate-limit window. At the threshold the pet warns and the gauge turns red; yellow begins at 75% of it.")}
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            {t("블록 리셋 임박 대사", "Upcoming reset dialogue")}{" "}
            <b>
              {s.resetNotifyMinutes === 0
                ? t("끔", "Off")
                : t(`${s.resetNotifyMinutes}분 전`, `${s.resetNotifyMinutes}m before`)}
            </b>
          </div>
          <div className="settings-row">
            <span className="settings-min">{t("끔", "Off")}</span>
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
            <span className="settings-max">{t("120분", "120m")}</span>
          </div>
          <div className="settings-hint">
            {t("캐릭터가 말풍선으로 알려줍니다 · 5분 주기로 확인하므로 5분 이상 권장", "The character reports it in a speech bubble. Five minutes or more is recommended because checks run every five minutes.")}
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            {t("작업 완료 대사", "Task completion dialogue")}{" "}
            <b>
              {s.doneNoticeSeconds === 0
                ? t("끔", "Off")
                : t(`${fmtDuration(s.doneNoticeSeconds, language)} 이상`, `${fmtDuration(s.doneNoticeSeconds, language)} or longer`)}
            </b>
          </div>
          <div className="settings-row">
            <span className="settings-min">{t("끔", "Off")}</span>
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
            <span className="settings-max">{t("10분", "10m")}</span>
          </div>
          <div className="settings-hint">
            {t("이보다 오래 걸린 작업이 끝나면 세션마다 알려줍니다", "Notify for each completed task that ran at least this long.")}
          </div>
        </div>

        <div className="settings-group">
          <div className="settings-label">
            {t("잠자기 진입 시간", "Sleep after")} <b>{t(`${s.sleepAfterMinutes}분`, `${s.sleepAfterMinutes}m`)}</b>
          </div>
          <div className="settings-row">
            <span className="settings-min">{t("5분", "5m")}</span>
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
            {t("마지막 AI 사용 후 이 시간이 지나면 캐릭터가 잠듭니다", "The character falls asleep after this much time without AI activity.")}
          </div>
        </div>
    </>
  );
}

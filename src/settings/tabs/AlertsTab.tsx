import { fmtDuration } from "../../format";
import type { TabProps } from "./types";

/** 알림 — 위험 한도·리셋 임박·작업 완료·잠자기 */
export default function AlertsTab({ s, update }: TabProps) {
  return (
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
  );
}

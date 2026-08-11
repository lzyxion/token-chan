import { fmtCost, fmtTokens, shortModel, totalOf } from "../format";
import type { DailyRow, ModelRow } from "../types";

/** 잔디 격자의 주 수. 91일이라 마지막 열이 이번 주가 되도록 딱 떨어진다. */
const WEEKS = 13;
/** 0(없음) + 4단계. GitHub 잔디와 같은 규칙 — 색이 진할수록 많이 썼다는 뜻이고
 *  경고가 아니다(패널의 주황·빨강과 달리 단계 색을 쓰지 않는 이유). */
const LEVELS = 4;

/** "2026-08-11" → 로컬 Date. `new Date(문자열)` 은 UTC 로 읽어 요일이 하루 밀린다. */
function localDate(iso: string): Date {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function levelOf(v: number, max: number): number {
  if (v <= 0 || max <= 0) return 0;
  return Math.min(LEVELS, Math.ceil((v / max) * LEVELS));
}

/**
 * 사용량 잔디 — 열 = 주, 행 = 요일.
 *
 * 마지막 날이 오늘이라 오른쪽 아래가 현재다. 첫 주는 일요일부터 시작하도록 앞을
 * 빈칸으로 채워야 요일 행이 맞는다.
 */
export function UsageHeatmap({ daily }: { daily: DailyRow[] }) {
  if (daily.length === 0) return null;
  const days = daily.slice(-WEEKS * 7);
  const max = Math.max(...days.map((d) => totalOf(d.totals)));

  // 첫 날의 요일만큼 앞을 비워 열/행을 맞춘다 (0=일)
  const pad = localDate(days[0].date).getDay();
  const cells: (DailyRow | null)[] = [...Array(pad).fill(null), ...days];

  return (
    <div className="grass">
      {/* grid-auto-flow: column 이라 셀이 세로(요일)로 먼저 채워진다 = GitHub 과 같은 배치 */}
      <div className="grass-grid">
        {cells.map((d, i) => {
          if (!d) return <span className="grass-cell empty" key={`pad${i}`} />;
          const total = totalOf(d.totals);
          return (
            <span
              key={d.date}
              className={`grass-cell lv${levelOf(total, max)}`}
              title={`${d.date} · ${fmtTokens(total)} tokens · ${fmtCost(d.cost, false)}`}
            />
          );
        })}
      </div>
      <div className="grass-legend">
        <span>{days.length}일</span>
        <span className="grass-scale">
          적음
          {Array.from({ length: LEVELS + 1 }, (_, i) => (
            <span className={`grass-cell lv${i}`} key={i} />
          ))}
          많음
        </span>
      </div>
    </div>
  );
}

/** 누적 막대에 따로 세울 조각 수. 274px 에서 그 이상은 실오라기가 된다. */
const MIX_MAX = 4;

/**
 * 오늘 모델 구성비 — **누적 막대 한 줄**.
 *
 * 같은 페이지의 잔디·주간 막대는 *시간축*이고 이건 *구성비*라, 형태를 다르게 해야
 * 다른 질문에 대한 답이라는 게 읽힌다 (막대를 또 나열하면 같은 종류로 보인다).
 * 하루 총량을 100%로 잡아서 "오늘 뭘 주로 썼나"가 바로 나온다 — 최대값 대비
 * 상대 길이로 그리던 예전 방식은 그 비중을 못 보여줬다.
 *
 * 색은 벤더별이라 모델 구성비와 벤더 구성비를 한 막대로 같이 읽는다. 다만 채도를
 * 낮춰 뒀다 — 원래 Codex 색(`#10a37f`)이 잔디 초록과 거의 같아서, 같은 페이지에서
 * 초록이 "양"이었다가 "Codex"가 되는 혼선이 있었다.
 */
export function ModelMix({ models }: { models: ModelRow[] }) {
  const sorted = [...models].sort((a, b) => totalOf(b.totals) - totalOf(a.totals));
  const total = sorted.reduce((s, m) => s + totalOf(m.totals), 0);
  if (total === 0) return null;

  const parts = sorted.slice(0, MIX_MAX).map((m) => ({
    key: `${m.source}-${m.model}`,
    label: shortModel(m.model),
    tone: m.source as string,
    tokens: totalOf(m.totals),
  }));
  const rest = sorted.slice(MIX_MAX).reduce((s, m) => s + totalOf(m.totals), 0);
  if (rest > 0) {
    parts.push({ key: "rest", label: "기타", tone: "rest", tokens: rest });
  }

  const pct = (n: number) => Math.round((n / total) * 100);
  return (
    <div className="mix">
      {/* flex-grow 를 토큰 수로 주면 비율이 그대로 폭이 된다 */}
      <div className="mix-bar">
        {parts.map((p) => (
          <span
            key={p.key}
            className={`mix-seg ${p.tone}`}
            style={{ flexGrow: p.tokens }}
            title={`${p.label} · ${fmtTokens(p.tokens)} (${pct(p.tokens)}%)`}
          />
        ))}
      </div>
      <div className="mix-legend">
        {parts.map((p) => (
          <span className="mix-item" key={p.key}>
            <span className={`mix-dot ${p.tone}`} />
            {p.label} <b>{fmtTokens(p.tokens)}</b> {pct(p.tokens)}%
          </span>
        ))}
      </div>
    </div>
  );
}

/** 최근 7일 막대 — 잔디가 흐름을 보여준다면 이쪽은 요일별 크기를 읽게 한다. */
export function WeekBars({ daily }: { daily: DailyRow[] }) {
  const days = daily.slice(-7);
  if (days.length === 0) return null;
  const max = Math.max(...days.map((d) => totalOf(d.totals)), 1);
  const labels = ["일", "월", "화", "수", "목", "금", "토"];

  return (
    <div className="weekbars">
      {days.map((d) => {
        const total = totalOf(d.totals);
        return (
          <div
            className="weekbar"
            key={d.date}
            title={`${d.date} · ${fmtTokens(total)} tokens · ${fmtCost(d.cost, false)}`}
          >
            <div className="weekbar-track">
              {/* 0 인 날도 바닥선이 보이도록 최소 높이를 준다 — 안 그러면 "데이터 없음"으로 읽힌다 */}
              <div
                className="weekbar-fill"
                style={{ height: `${total > 0 ? Math.max(6, (total / max) * 100) : 2}%` }}
              />
            </div>
            <span className="weekbar-label">{labels[localDate(d.date).getDay()]}</span>
          </div>
        );
      })}
    </div>
  );
}

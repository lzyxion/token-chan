import { fmtCost, fmtTokens, shortModel, totalOf, type Currency } from "../format";
import type { DailyModels, DailyRow, DayModel, ModelRow } from "../types";

/** 주간 막대가 덮는 일수 — 백엔드 `aggregate::WEEK_DAYS` 와 같아야 날짜가 맞물린다 */
const WEEK_DAYS = 7;
/** 주간 막대에서 색을 따로 주는 모델 수 (나머지는 「기타」).
 *  패널이 좁아(약 210px) 범례가 두 줄을 넘지 않는 선이다. */
const WEEK_MODEL_MAX = 4;
/** 한 벤더 안에서 쓰는 밝기 단계 수(원색 제외). 더 늘리면 서로 구분이 안 된다. */
const SHADE_MAX = 2;

/**
 * 칸 하나가 차지하는 최대 폭(칸 12 + 간격 2). 주 수는 백엔드가 보존기간에서
 * 정해 보내므로(`aggregate::daily_window`) 여기서 고정하지 않는다 — 대신 이 값으로
 * 격자 최대 폭을 계산해, 주가 적으면 가운데 정렬하고 많으면 칸이 알아서 줄어든다.
 */
const CELL_MAX = 14;
/** 0(없음) + 4단계. GitHub 잔디와 같은 규칙 — 색이 진할수록 많이 썼다는 뜻이고
 *  경고가 아니다(패널의 주황·빨강과 달리 단계 색을 쓰지 않는 이유). */
const LEVELS = 4;

/** "2026-08-11" → 로컬 Date. `new Date(문자열)` 은 UTC 로 읽어 요일이 하루 밀린다. */
function localDate(iso: string): Date {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/**
 * 실제로 기록이 있는 날 수 — 첫 이벤트가 있던 날부터 오늘까지.
 *
 * 격자 기간(보존기간에서 유도)과 다를 수 있다. CLI 를 최근에 깔았으면 앞쪽은
 * 데이터가 존재할 수 없는 날이다. 합계 라벨을 격자 기간으로 붙이면
 * "84일에 42.3M" 으로 읽혀 일평균을 잘못 계산하게 되므로, 잔디 범례와 이 값을 공유한다.
 */
export function recordedDays(daily: DailyRow[], firstEvent: string | null): number {
  if (!firstEvent) return daily.length;
  const d = new Date(firstEvent);
  const started = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  return daily.filter((r) => localDate(r.date).getTime() >= started).length;
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
export function UsageHeatmap({
  daily,
  firstEvent,
  currency,
}: {
  daily: DailyRow[];
  /** 가장 오래된 이벤트 시각. 이전 날짜는 "안 씀"이 아니라 **기록이 없던 날**이다. */
  firstEvent: string | null;
  currency: Currency;
}) {
  if (daily.length === 0) return null;
  const days = daily;
  const max = Math.max(...days.map((d) => totalOf(d.totals)));

  // 기록이 시작된 날. 이전 칸을 0 과 같은 모양으로 그리면 CLI 를 깔기도 전 날짜까지
  // "그날 안 썼음"으로 보인다 — 격자 모양은 그대로 두고 칸 모양만 다르게 한다.
  const startedAt = firstEvent ? new Date(firstEvent) : null;
  const started = startedAt
    ? new Date(startedAt.getFullYear(), startedAt.getMonth(), startedAt.getDate()).getTime()
    : null;
  const recorded = recordedDays(days, firstEvent);

  // 첫 날의 요일만큼 앞을 비워 열/행을 맞춘다 (0=일)
  const pad = localDate(days[0].date).getDay();
  const cells: (DailyRow | null)[] = [...Array(pad).fill(null), ...days];
  const weeks = Math.ceil(cells.length / 7);

  return (
    <div className="grass">
      {/* grid-auto-flow: column 이라 셀이 세로(요일)로 먼저 채워진다 = GitHub 과 같은 배치.
          열 수를 인라인으로 주는 이유: 칸 크기를 CSS 에 12px 로 박으면 주가 많을 때
          (보존기간을 늘리면 최대 26주) 격자가 카드를 넘쳐 잘린다. 1fr + 최대 폭으로
          두면 주가 적으면 가운데 정렬되고 많으면 칸이 알아서 줄어든다. */}
      <div
        className="grass-grid"
        style={{
          gridTemplateColumns: `repeat(${weeks}, minmax(0, 1fr))`,
          maxWidth: weeks * CELL_MAX,
        }}
      >
        {cells.map((d, i) => {
          if (!d) return <span className="grass-cell empty" key={`pad${i}`} />;
          const total = totalOf(d.totals);
          if (started != null && localDate(d.date).getTime() < started) {
            return (
              <span className="grass-cell nodata" key={d.date} title={`${d.date} · 기록 없음`} />
            );
          }
          return (
            <span
              key={d.date}
              className={`grass-cell lv${levelOf(total, max)}`}
              title={`${d.date} · ${fmtTokens(total)} tokens · ${fmtCost(d.cost, false, currency)}`}
            />
          );
        })}
      </div>
      <div className="grass-legend">
        {/* 격자는 91칸이지만 기록은 그중 일부뿐일 수 있다 — 둘을 같이 밝힌다 */}
        <span>
          {recorded < days.length ? `기록 ${recorded}일 / ${days.length}일` : `${days.length}일`}
        </span>
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

/**
 * 최근 7일 막대 — 잔디가 흐름을 보여준다면 이쪽은 요일별 크기를 읽게 한다.
 * 막대는 **모델별로 쌓는다**: 크기와 구성을 한 그래프에서 같이 읽는 게 목적이다.
 *
 * 색은 「오늘 모델」과 **같은 규칙(색 = 벤더)** 이고, 같은 벤더 안에서만 밝기로 가른다.
 * 모델마다 새 색을 주면 한 페이지에서 같은 색이 한쪽은 벤더, 한쪽은 모델을 뜻하게 되고
 * 색 수도 벤더 3 + 모델 4 로 불어난다. 색상은 벤더가 갖고 밝기는 모델이 갖는 쪽이,
 * 두 구분을 한 막대에서 같이 읽게 해 준다.
 */
export function WeekBars({
  daily,
  weekModels,
  currency,
}: {
  daily: DailyRow[];
  weekModels: DailyModels[];
  currency: Currency;
}) {
  const days = daily.slice(-WEEK_DAYS);
  if (days.length === 0) return null;
  const max = Math.max(...days.map((d) => totalOf(d.totals)), 1);
  const labels = ["일", "월", "화", "수", "목", "금", "토"];

  const byDate = new Map(weekModels.map((w) => [w.date, w.models]));
  const keyOf = (m: DayModel) => `${m.source}-${m.model}`;

  // 색은 **주 전체 순위**로 정한다 — 하루씩 정하면 같은 모델이 날마다 색이 바뀐다.
  const weekTotals = new Map<string, { label: string; source: string; tokens: number }>();
  for (const w of weekModels) {
    for (const m of w.models) {
      const cur = weekTotals.get(keyOf(m));
      if (cur) cur.tokens += m.tokens;
      else weekTotals.set(keyOf(m), { label: shortModel(m.model), source: m.source, tokens: m.tokens });
    }
  }
  const ranked = [...weekTotals.entries()].sort((a, b) => b[1].tokens - a[1].tokens);

  // 밝기 단계는 **벤더 안에서** 매긴다 — 그 벤더의 주력 모델이 원색이고, 덜 쓴 모델일수록
  // 밝아진다. 전체 순위로 매기면 Codex 를 조금만 써도 Claude 2위와 같은 단계가 된다.
  const shadeCount = new Map<string, number>();
  const legend = ranked.slice(0, WEEK_MODEL_MAX).map(([k, v]) => {
    const n = shadeCount.get(v.source) ?? 0;
    shadeCount.set(v.source, n + 1);
    return {
      key: k,
      tone: n === 0 ? v.source : `${v.source} sh${Math.min(n, SHADE_MAX)}`,
      label: v.label,
      tokens: v.tokens,
    };
  });
  const toneOf = new Map(legend.map((l) => [l.key, l.tone]));
  const restTokens = ranked.slice(WEEK_MODEL_MAX).reduce((s, [, v]) => s + v.tokens, 0);
  if (restTokens > 0) {
    legend.push({ key: "rest", tone: "rest", label: "기타", tokens: restTokens });
  }

  return (
    <div className="weekbars-wrap">
      <div className="weekbars">
        {days.map((d) => {
          const total = totalOf(d.totals);
          // 조각도 순위 순으로 쌓아야 날마다 같은 층에 같은 모델이 온다.
          // 안 쓴 모델은 조각이 없을 뿐 순서는 유지된다.
          const models = (byDate.get(d.date) ?? []).slice();
          const segs = legend
            .map((l) => ({
              ...l,
              tokens:
                l.key === "rest"
                  ? models.filter((m) => !toneOf.has(keyOf(m))).reduce((s, m) => s + m.tokens, 0)
                  : models.filter((m) => keyOf(m) === l.key).reduce((s, m) => s + m.tokens, 0),
            }))
            .filter((s) => s.tokens > 0);
          const detail = segs.map((s) => `${s.label} ${fmtTokens(s.tokens)}`).join("\n");
          return (
            <div
              className="weekbar"
              key={d.date}
              title={`${d.date} · ${fmtTokens(total)} tokens · ${fmtCost(d.cost, false, currency)}${
                detail ? `\n${detail}` : ""
              }`}
            >
              <div className="weekbar-track">
                {/* 0 인 날도 바닥선이 보이도록 최소 높이를 준다 — 안 그러면 "데이터 없음"으로 읽힌다 */}
                <div
                  className={`weekbar-fill${segs.length === 0 ? " empty" : ""}`}
                  style={{ height: `${total > 0 ? Math.max(6, (total / max) * 100) : 2}%` }}
                >
                  {/* flex-grow 를 토큰 수로 주면 비율이 그대로 높이가 된다 (「오늘 모델」과 같은 방식) */}
                  {segs.map((s) => (
                    <span key={s.key} className={`weekbar-seg ${s.tone}`} style={{ flexGrow: s.tokens }} />
                  ))}
                </div>
              </div>
              <span className="weekbar-label">{labels[localDate(d.date).getDay()]}</span>
            </div>
          );
        })}
      </div>
      {legend.length > 0 && (
        <div className="mix-legend">
          {legend.map((l) => (
            <span className="mix-item" key={l.key}>
              <span className={`mix-dot ${l.tone}`} />
              {l.label} <b>{fmtTokens(l.tokens)}</b>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

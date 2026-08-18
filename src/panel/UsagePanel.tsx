import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ResizeGrips from "../components/ResizeGrips";
import VendorIcon from "../components/VendorIcon";
import { ModelMix, recordedDays, UsageHeatmap, WeekBars } from "./UsageCharts";
import { useCurrency, useLive, usePlans, useSummary, useThresholds } from "../hooks/useUsage";
import { useWindowPersist } from "../hooks/useWindowPersist";
import {
  fmtAgo,
  fmtCost,
  fmtRemaining,
  fmtTokens,
  meterLevel,
  planMeterThreshold,
  resetIsStale,
  shortModel,
  SOURCE_LABEL,
  totalOf,
  type AlertThresholds,
  type Currency,
} from "../format";
import type { ContextState, PlanMeter, PlanUsage, SourceStatus, SourceSummary } from "../types";
import "./panel.css";

function statusChip(status: SourceStatus) {
  switch (status.kind) {
    case "no_data":
      return <span className="chip muted">미감지</span>;
    default:
      return null;
  }
}

/** 게이지 한 줄: `라벨 | 바 | % | 리셋`. 컨텍스트와 한도가 같은 격자를 써야
 *  한 벤더의 소진율이 세로로 정렬돼 한눈에 비교된다. */
function MeterRow({
  label,
  pct,
  resetAt,
  stale,
  computed,
  title,
  danger,
}: {
  label: string;
  pct: number;
  /** 이 미터의 위험 한도 (%) — 설정(알림 탭)에서 온다 */
  danger: number;
  resetAt?: Date | null;
  /** 리셋 시각이 이미 지났다 = 캐시가 굳었고 계산으로도 못 메웠다 */
  stale?: boolean;
  /** 리셋 시각을 백엔드가 계산했다 (공식 캐시가 굳어서) */
  computed?: boolean;
  title?: string;
}) {
  return (
    <div className="vendor-row" title={title}>
      <span className="vendor-key">{label}</span>
      <div className="bar plan-bar">
        <div className={`bar-fill meter ${meterLevel(pct, danger)}`} style={{ width: `${pct}%` }} />
      </div>
      <span className={`plan-pct ${meterLevel(pct, danger)}`}>{pct}%</span>
      {/* 계산값이면 `~` 를 붙인다 — 숫자는 믿을 만하지만(실측에서 공식과 일치) 출처가
          공식이 아니라는 건 밝혀야 한다. 리셋을 못 구하면 "낡음" 으로 이유를 적는다:
          그냥 비우면 왜 사라졌는지 알 수 없고, "0분" 은 지금 막 리셋된다는 거짓말이다. */}
      <span className={`vendor-reset${stale ? " stale" : ""}`}>
        {resetAt ? `${computed ? "~" : ""}${fmtRemaining(resetAt)}` : stale ? "낡음" : ""}
      </span>
    </div>
  );
}

/** 미터의 리셋 시각 — 두 소스 모두 파일에서 기계가 읽는 형식으로 준다.
 *
 *  **이미 지난 시각은 값이 아니다** — 굳은 캐시의 잔해다(`resetIsStale`). Claude 5시간
 *  창은 백엔드가 계산값으로 갈아 끼워 주므로 여기까지 오는 일이 드물지만, 활동이 끊겨
 *  창이 닫혀 있으면(계산도 못 하면) 그대로 온다. */
function meterReset(m: PlanMeter): Date | null {
  if (!m.resets_at) return null;
  const d = new Date(m.resets_at);
  if (Number.isNaN(d.getTime()) || resetIsStale(d)) return null;
  return d;
}

/** 리셋 시각은 있는데 이미 지났다 = 그 미터가 굳었고 대체도 못 했다 */
function meterIsStale(m: PlanMeter): boolean {
  if (!m.resets_at) return false;
  const d = new Date(m.resets_at);
  return !Number.isNaN(d.getTime()) && resetIsStale(d);
}

/**
 * 벤더 카드 — 게이지가 벤더 하나만 보여주므로, 벤더 비교는 여기가 유일한 자리다.
 * 한 벤더에 대해 알 수 있는 것(모델·컨텍스트·공식 한도·리셋·오늘 사용량)을 한 덩어리로 모은다.
 */
function VendorCard({
  s,
  context,
  plan,
  active,
  busy,
  currency,
  thresholds,
}: {
  s: SourceSummary;
  context: ContextState | null;
  plan: PlanUsage | null;
  active: boolean;
  busy: boolean;
  currency: Currency;
  thresholds: AlertThresholds;
}) {
  const total = totalOf(s.today);
  const chip = statusChip(s.status);
  const pct = context ? Math.round(context.used_pct) : null;
  const meters = plan?.meters ?? [];
  return (
    <div className={`vendor-card ${active ? "active" : ""}`}>
      {/* 오늘 사용량을 머리줄 오른쪽에 붙여 카드에서 한 줄을 없앴다 —
          벤더 3개가 스크롤 없이 들어가야 "한눈에 비교"가 성립한다.
          컨텍스트 토큰 수(76K/200K)는 바와 % 가 이미 말하고 있어 툴팁으로 내렸다. */}
      <div className="vendor-head">
        <VendorIcon source={s.source} size={13} className={busy ? "busy" : ""} />
        <span className="source-label">{SOURCE_LABEL[s.source]}</span>
        {context?.model && <span className="vendor-model">{shortModel(context.model)}</span>}
        {busy && <span className="chip busy-chip">작업 중</span>}
        {chip}
        <span className="vendor-today">
          <b>{fmtTokens(total)}</b> {fmtCost(s.today_cost, s.cost_partial, currency)}
        </span>
      </div>
      {pct != null && context && (
        <MeterRow
          label="컨텍스트"
          pct={pct}
          danger={thresholds.context}
          title={`${context.tokens.toLocaleString()} / ${context.window.toLocaleString()} 토큰${
            context.interim ? " (정리 중)" : ""
          }`}
        />
      )}
      {/* 라벨은 백엔드가 두 소스 공통 어휘로 준다 ("5시간"·"주간") — 여기서 손보지 않는다 */}
      {meters.map((m, i) => (
        <MeterRow
          key={m.label}
          label={m.label}
          pct={m.used_pct}
          danger={planMeterThreshold(i, thresholds)}
          resetAt={meterReset(m)}
          stale={meterIsStale(m)}
          computed={m.resets_computed}
          title={
            m.resets_at
              ? meterIsStale(m)
                ? `${new Date(m.resets_at).toLocaleString()} 에 리셋됐어야 하는데 값이 그대로다` +
                  ` — CLI 가 캐시를 갱신하지 않았고, 활동이 끊겨 계산으로도 메울 수 없다` +
                  (plan ? ` (받아온 시각 ${new Date(plan.fetched_at).toLocaleString()})` : "")
                : m.resets_computed
                  ? `${new Date(m.resets_at).toLocaleString()} — 공식 캐시가 굳어(${
                      plan ? new Date(plan.fetched_at).toLocaleString() : "?"
                    } 이후 정지) 트랜스크립트에서 계산한 값이다. 왼쪽 %는 여전히 옛 창의 것이라 실제보다 높다`
                  : new Date(m.resets_at).toLocaleString()
              : ""
          }
        />
      ))}
      {/* 한도 미터가 없어도 아무 말도 안 한다. 예전엔 "공식 한도 없음" 을 적었는데,
          카드에 이미 벤더·모델·컨텍스트·오늘 사용량이 차 있어 빈 줄이 "로딩 중"으로
          읽히지 않는다. Antigravity 는 한도를 **영영** 안 주므로 그 줄이 상시 노이즈였고,
          Claude·Codex 는 값이 늦게 오는 것뿐이라 그 사이 "없음"은 오히려 거짓말이었다. */}
    </div>
  );
}



const PAGE_TITLES = ["현황", "통계·사용량", "최근 세션"];

/** 독립 창으로 뜨는 사용량 패널 — 펫 우클릭 또는 트레이 메뉴로 토글 */
export default function UsagePanel() {
  const summary = useSummary();
  const live = useLive();
  const plans = usePlans();
  const currency = useCurrency();
  const thresholds = useThresholds();
  const [page, setPage] = useState(0);
  const bodyRef = useRef<HTMLDivElement | null>(null);

  useWindowPersist("panel");

  // Esc로 닫기
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void getCurrentWindow().hide();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!summary) {
    return (
      <div className="panel-root">
        <ResizeGrips />
        <div className="card">
          <div className="loading">사용량 스캔 중…</div>
        </div>
      </div>
    );
  }

  const topModels = summary.models_today.slice(0, 5);
  const todayTotal = totalOf(summary.today);
  // 잔디가 덮는 기간(= summary.daily) 전체 합계. 백엔드에 따로 담지 않고 여기서 더한다 —
  // 일별 값이 이미 다 와 있어 서버 왕복을 늘릴 이유가 없다.
  const periodTotal = summary.daily.reduce((s, d) => s + totalOf(d.totals), 0);
  const periodCost = summary.daily.reduce((s, d) => s + d.cost, 0);
  // 격자 기간이 아니라 **기록이 있는 날 수**로 라벨을 단다 — 앞쪽이 통째로 기록 없는
  // 구간이면 "84일에 42.3M" 으로 읽혀 일평균을 잘못 계산하게 된다 (잔디 범례와 같은 값).
  const recorded = recordedDays(summary.daily, summary.first_event_ts);

  // 지금 돌고 있는 세션들 — 세 소스 다 파일에 적힌 턴 경계에서 온 값이다
  const running = live.sessions.filter((s) => s.status === "busy");
  const busySources = new Set(running.map((s) => s.source));
  // 최근 세션 목록에서 **그 줄**만 짚기 위한 키. id 를 못 알아낸 세션은 넣지 않는다 —
  // 빈 문자열을 넣으면 id 가 빈 다른 줄과 잘못 맞물린다.
  const runningKeys = new Set(running.filter((s) => s.id).map((s) => `${s.source}:${s.id}`));
  // 게이지가 보여주는 벤더와 같은 기준 — 작업 중 우선, 없으면 마지막으로 움직인 세션.
  // 게이지와 달리 여기선 깜빡임이 문제되지 않아 히스테리시스를 걸지 않는다.
  const latestContext = summary.contexts.length
    ? summary.contexts.reduce((a, b) => ((a.at ?? "") >= (b.at ?? "") ? a : b))
    : null;
  const activeSource = [...busySources][0] ?? latestContext?.source ?? null;

  const pageCount = PAGE_TITLES.length;
  const prev = () => setPage((p) => (p + pageCount - 1) % pageCount);
  const next = () => setPage((p) => (p + 1) % pageCount);

  // 휠은 우선 본문 스크롤에 양보하고, 더 스크롤할 곳이 없을 때만 페이지를 넘긴다.
  // (창을 작게 줄이면 본문이 넘치므로 스크롤이 먼저다)
  const onWheel = (e: React.WheelEvent) => {
    const el = bodyRef.current;
    if (el && el.scrollHeight > el.clientHeight + 1) {
      const atTop = el.scrollTop <= 0;
      const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
      if (e.deltaY > 0 ? !atBottom : !atTop) return;
    }
    if (e.deltaY > 0) next();
    else prev();
  };

  return (
    <div className="panel-root" onWheel={onWheel}>
      <ResizeGrips />
      <div className="card">
        {/* 닫기는 드래그 영역(.head) 밖 — 헤더 안에 두면 드래그와 클릭이 얽힌다 */}
        <button
          className="panel-close"
          title="닫기 (Esc)"
          onClick={() => void getCurrentWindow().hide()}
        >
          ✕
        </button>
        {/* 비용은 통계 페이지의 토큰 총합 옆으로 옮겼다 — 헤더에 두면 어느 페이지에서든
            떠 있어서 벤더별 비용과 헷갈린다 */}
        <div className="head" data-tauri-drag-region="deep">
          <span className="title">{PAGE_TITLES[page]}</span>
          <span className="date">{summary.today_date.slice(5).replace("-", "/")}</span>
        </div>

        <div className="page-body" ref={bodyRef}>
          {page === 0 && (
            /* 활성 벤더를 맨 위로 — 게이지가 보여주는 게 이것이라 시선이 먼저 닿아야 한다 */
            <div className="sources">
              {[...summary.sources]
                .sort(
                  (a, b) => Number(b.source === activeSource) - Number(a.source === activeSource),
                )
                .map((s) => (
                  <VendorCard
                    key={s.source}
                    s={s}
                    context={summary.contexts.find((c) => c.source === s.source) ?? null}
                    plan={plans.find((p) => p.source === s.source) ?? null}
                    active={s.source === activeSource}
                    busy={busySources.has(s.source)}
                    currency={currency}
                    thresholds={thresholds}
                  />
                ))}
            </div>
          )}

          {page === 1 && (
            <>
              {/* 오늘만 크게 두면 바로 아래 91일 잔디와 기간이 뒤섞여 읽힌다 —
                  큰 숫자를 전체 합계로 오해하기 딱 좋다. 둘을 나란히 놓아 기간을 못 박는다.
                  기간 합계는 통계 페이지에 원래 있어야 할 값이기도 하다
                  (지금까지는 잔디를 눈으로 훑어야 총량을 짐작할 수 있었다). */}
              <div className="totals">
                <div className="total-item">
                  <span className="total-label">오늘</span>
                  <span className="total-tokens">{fmtTokens(todayTotal)}</span>
                  <span className="total-cost">
                    {fmtCost(summary.today_cost, summary.cost_partial, currency)}
                  </span>
                </div>
                <div className="total-item">
                  {/* 격자 기간(84일)이 아니라 기록이 있는 날 수를 쓴다 —
                      기록 전 구간까지 포함한 숫자로 나누면 일평균이 어긋난다 */}
                  <span className="total-label">{recorded}일</span>
                  <span className="total-tokens">{fmtTokens(periodTotal)}</span>
                  <span className="total-cost">{fmtCost(periodCost, false, currency)}</span>
                </div>
              </div>
              {/* 어느 기간의 내역인지 앞에 못 박는다 (위 두 숫자와 헷갈리지 않게) */}
              <div className="grand-mini">
                오늘 · 입력 {fmtTokens(summary.today.input)} · 출력{" "}
                {fmtTokens(summary.today.output)} · 캐시{" "}
                {fmtTokens(summary.today.cache_write + summary.today.cache_read)}
              </div>

              <div className="chart-block">
                <div className="chart-title">일별 사용량</div>
                <UsageHeatmap daily={summary.daily} firstEvent={summary.first_event_ts} currency={currency} />
              </div>

              <div className="chart-block">
                <div className="chart-title">최근 7일</div>
                <WeekBars daily={summary.daily} weekModels={summary.week_models ?? []} currency={currency} />
              </div>

              {topModels.length > 0 && (
                <div className="chart-block">
                  <div className="chart-title">오늘 모델</div>
                  <ModelMix models={summary.models_today} />
                </div>
              )}
            </>
          )}

          {page === 2 && (
            <div className="sessions">
              {summary.sessions.length === 0 ? (
                <div className="empty-hint">최근 세션이 없습니다</div>
              ) : (
                summary.sessions.map((r) => (
                  <div className="session-row" key={`${r.source}:${r.id}`} title={r.cwd || r.id}>
                    {/* 벤더가 아니라 **이 세션**이 도는지로 판단한다 — 벤더로 보면
                        한 세션만 돌아도 그 벤더의 지난 세션까지 전부 깜빡였다 */}
                    <VendorIcon
                      source={r.source}
                      size={12}
                      className={runningKeys.has(`${r.source}:${r.id}`) ? "busy" : ""}
                    />
                    <span className="session-label">{r.label}</span>
                    <span className="session-ago">{fmtAgo(r.at)}</span>
                    <span className="session-meta">
                      {shortModel(r.model)}
                      {r.branch && ` · ${r.branch}`}
                    </span>
                    <span className="session-tokens">{fmtTokens(r.tokens)}</span>
                  </div>
                ))
              )}
            </div>
          )}
        </div>

        <div className="page-nav">
          <button className="nav-btn" onClick={prev} title="이전">
            ◀
          </button>
          <div className="dots">
            {PAGE_TITLES.map((t, i) => (
              <button
                key={t}
                className={`dot-btn ${i === page ? "on" : ""}`}
                onClick={() => setPage(i)}
                title={t}
              />
            ))}
          </div>
          <button className="nav-btn" onClick={next} title="다음">
            ▶
          </button>
        </div>
      </div>
    </div>
  );
}

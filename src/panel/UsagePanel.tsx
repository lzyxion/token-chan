import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ResizeGrips from "../components/ResizeGrips";
import VendorIcon from "../components/VendorIcon";
import {
  CostBreakdown,
  Efficiency,
  ModelMix,
  ModelToday,
  recordedDays,
  UsageHeatmap,
  VendorShare,
  WeekBars,
} from "./UsageCharts";
import {
  useCurrency,
  useLive,
  usePlans,
  useRetentionDays,
  useSummary,
  useThresholds,
} from "../hooks/useUsage";
import { useWindowPersist } from "../hooks/useWindowPersist";
import {
  fmtAgo,
  fmtCost,
  fmtRemaining,
  fmtTokens,
  meterLabel,
  meterLevel,
  resetIsStale,
  retentionLabel,
  RETENTION_OPTIONS,
  shortModel,
  showsHeatmap,
  SOURCE_LABEL,
  EMPTY_PARTS,
  totalOf,
  type AlertThresholds,
  type Currency,
} from "../format";
import { useI18n } from "../i18n";
import type {
  ContextState,
  DailyDetail,
  PlanMeter,
  PlanUsage,
  ProjectSessions,
  SessionRow,
  Source,
  SourceSummary,
} from "../types";
import "./panel.css";

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
  aside,
}: {
  label: string;
  pct: number;
  /** 이 미터의 위험 한도 (%) — 설정(알림 탭)에서 온다 */
  danger: number;
  /** 리셋이 **없는** 줄(컨텍스트)의 마지막 칸에 대신 넣을 값.
   *  같은 `span` 을 쓰므로 리셋 시간과 글꼴·정렬이 저절로 같아진다 */
  aside?: string;
  resetAt?: Date | null;
  /** 리셋 시각이 이미 지났다 = 캐시가 굳었고 계산으로도 못 메웠다 */
  stale?: boolean;
  /** 리셋 시각을 백엔드가 계산했다 (공식 캐시가 굳어서) */
  computed?: boolean;
  title?: string;
}) {
  const { language, t } = useI18n();
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
        {resetAt
          ? `${computed ? "~" : ""}${fmtRemaining(resetAt, language)}`
          : stale
            ? t("낡음", "Stale")
            : (aside ?? "")}
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
  const { language, t } = useI18n();
  const total = totalOf(s.today);
  const pct = context ? Math.round(context.used_pct) : null;
  const showsContext = pct != null && context != null;
  // 오늘 한 번도 안 쓴 벤더(연결 안 된 소스 포함)는 토큰 수를 적지 않는다 — "0" 은
  // 비용 옆에서 또 하나의 0 이 되어 무엇이 0 인지만 흐린다. 안 썼다는 사실은
  // 비용 0 이 이미 말하고, 연결 여부는 상태 칩이 말한다.
  const usedToday = total > 0;
  const meters = plan?.meters ?? [];
  const resetCredits = s.source === "codex" ? plan?.reset_credits : null;
  const resetExpiry = resetCredits ? resetCreditExpiry(resetCredits) : null;
  return (
    <div className={`vendor-card ${active ? "active" : ""}`}>
      {/* 오늘 누적은 머리줄, 컨텍스트는 그 행에서만 읽는다. 둘을 같은 자리에 놓으면
          `76K / 200K`를 오늘 합계로 오해하게 된다. */}
      <div className="vendor-head">
        <VendorIcon source={s.source} size={13} className={busy ? "busy" : ""} />
        <span className="source-label">{SOURCE_LABEL[s.source]}</span>
        {context?.model && <span className="vendor-model">{shortModel(context.model)}</span>}
        <span className="vendor-today">
          {usedToday && (
            <>
              <b>{fmtTokens(total)}</b>{" "}
            </>
          )}
          <span className="vendor-cost">{fmtCost(s.today_cost, s.cost_partial, currency)}</span>
        </span>
      </div>
      {showsContext && (
        <MeterRow
          label={t("컨텍스트", "Context")}
          pct={pct}
          danger={thresholds.context}
          aside={fmtTokens(context.tokens).replace(/\.0([KMB])$/, "$1")}
          title={`${context.tokens.toLocaleString()} / ${context.window.toLocaleString()} ${t("토큰", "tokens")}${
            context.interim ? t(" (정리 중)", " (finalizing)") : ""
          }`}
        />
      )}
      {/* 라벨은 백엔드가 두 소스 공통 어휘로 준다 ("5시간"·"주간") — 여기서 손보지 않는다 */}
      {meters.map((m) => (
        <MeterRow
          key={m.label}
          label={meterLabel(m.label, language)}
          pct={m.used_pct}
          danger={thresholds.plan}
          resetAt={meterReset(m)}
          stale={meterIsStale(m)}
          computed={m.resets_computed}
          title={
            m.resets_at
              ? meterIsStale(m)
                ? t(
                    `${new Date(m.resets_at).toLocaleString()} 에 리셋됐어야 하는데 값이 그대로다 — CLI 가 캐시를 갱신하지 않았고, 활동이 끊겨 계산으로도 메울 수 없다${plan ? ` (받아온 시각 ${new Date(plan.fetched_at).toLocaleString()})` : ""}`,
                    `This value should have reset at ${new Date(m.resets_at).toLocaleString()}, but the CLI cache was not refreshed and there is not enough recent activity to estimate it${plan ? ` (fetched ${new Date(plan.fetched_at).toLocaleString()})` : ""}`,
                  )
                : m.resets_computed
                  ? t(
                      `${new Date(m.resets_at).toLocaleString()} — 공식 캐시가 굳어(${plan ? new Date(plan.fetched_at).toLocaleString() : "?"} 이후 정지) 트랜스크립트에서 계산한 값이다. 왼쪽 %는 여전히 옛 창의 것이라 실제보다 높다`,
                      `${new Date(m.resets_at).toLocaleString()} — estimated from transcripts because the official cache stopped updating after ${plan ? new Date(plan.fetched_at).toLocaleString() : "?"}. The percentage still belongs to the previous window and may be too high.`,
                    )
                  : new Date(m.resets_at).toLocaleString()
              : ""
          }
        />
      ))}
      {resetCredits && resetCredits.available_count > 0 && (
        <div className={`vendor-reset-credits ${resetExpiry?.tone ?? "neutral"}`} title={resetCredits.expires_at ? t(`가장 이른 만료: ${new Date(resetCredits.expires_at).toLocaleString()}`, `Earliest expiry: ${new Date(resetCredits.expires_at).toLocaleString()}`) : t("만료 시각 정보 없음", "Expiry time unavailable")}>
          <span className="vendor-key">{t("리셋권", "Resets")}</span>
          <span className="reset-credit-track">
            <i style={{ width: `${resetExpiry?.pct ?? 0}%` }} />
          </span>
          <span className="reset-credit-days">{resetExpiry ? `D-${resetExpiry.days}` : "—"}</span>
          <b>{resetCredits.available_count}{t("개", "")}</b>
        </div>
      )}
      {/* 한도 미터가 없어도 아무 말도 안 한다. 예전엔 "공식 한도 없음" 을 적었는데,
          카드에 이미 벤더·모델·컨텍스트·오늘 사용량이 차 있어 빈 줄이 "로딩 중"으로
          읽히지 않는다. Antigravity 는 한도를 **영영** 안 주므로 그 줄이 상시 노이즈였고,
          Claude·Codex 는 값이 늦게 오는 것뿐이라 그 사이 "없음"은 오히려 거짓말이었다. */}
    </div>
  );
}



/**
 * 탭에 들어갈 짧은 이름. 정식 이름(`SOURCE_LABEL`)은 280px 폭에서 네 칸이 되면
 * 전부 말줄임이 되어 무슨 벤더인지 알 수 없다. 로고와 함께 세우므로 이름은
 * 구분에 필요한 최소만 남긴다 — 정식 이름은 `title` 로 붙는다.
 */
const SHORT_VENDOR: Record<Source, string> = {
  claude: "Claude",
  codex: "Codex",
  // Antigravity 의 약자라 대문자다 — CLI 실행 명령인 소문자 `agy` 와는 다른 것이다
  antigravity: "AGY",
};

const PAGE_TITLES = [
  ["현황", "Overview"],
  ["최근 세션", "Recent sessions"],
  ["통계·사용량", "Stats & usage"],
  ["사용 기록", "Activity history"],
] as const;
const PAGE = { STATUS: 0, SESSIONS: 1, STATS: 2, HISTORY: 3 } as const;

function localMonthDay(iso: string): string {
  const date = new Date(iso);
  return `${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function resetCreditExpiry(credit: NonNullable<PlanUsage["reset_credits"]>) {
  if (!credit.expires_at) return null;
  const end = new Date(credit.expires_at).getTime();
  const remaining = Math.max(0, end - Date.now());
  const days = Math.ceil(remaining / 86_400_000);
  const start = credit.granted_at ? new Date(credit.granted_at).getTime() : NaN;
  const pct = Number.isFinite(start) && end > start ? Math.min(100, (remaining / (end - start)) * 100) : 0;
  const tone = days > 14 ? "safe" : days > 7 ? "warn" : "danger";
  return { days, pct, tone };
}

const liveSessionKey = (session: Pick<SessionRow, "source" | "id">) =>
  `${session.source}:${session.id}`;

function ProjectSessionGroup({
  project,
  collapsed,
  runningKeys,
  retention,
  onToggle,
}: {
  project: ProjectSessions;
  collapsed: boolean;
  runningKeys: Set<string>;
  retention: number;
  onToggle: () => void;
}) {
  const { language, t } = useI18n();
  const activeCount = project.sessions.filter((session) =>
    runningKeys.has(liveSessionKey(session)),
  ).length;
  const active = activeCount > 0;
  const projectLabel = project.label || t("프로젝트 없음", "Unknown project");
  const countLabel = language === "en"
    ? `${project.sessionCount} ${project.sessionCount === 1 ? "session" : "sessions"}`
    : `${project.sessionCount}세션`;

  return (
    <section className="session-project">
      <button
        className={`project-row${active ? " active" : ""}${collapsed ? " collapsed" : ""}`}
        type="button"
        aria-expanded={!collapsed}
        title={`${project.cwd || projectLabel}\n${retentionLabel(retention, language)} ${t("합계", "total")}`}
        onClick={onToggle}
      >
        <span className="project-chevron" aria-hidden="true">{collapsed ? "▸" : "▾"}</span>
        <span className="project-label">{projectLabel}</span>
        {active && (
          <span className="project-active">
            {activeCount > 1 ? t(`작업 중 ${activeCount}`, `${activeCount} active`) : t("작업 중", "Active")}
          </span>
        )}
        <span className="project-total">
          <b>{fmtTokens(project.tokens)}</b>
          <i>{countLabel}</i>
        </span>
      </button>
      {!collapsed && (
        <div className="project-sessions">
          {project.sessions.map((session) => {
            const sessionActive = runningKeys.has(liveSessionKey(session));
            return (
              <div
                className={`session-row${sessionActive ? " active" : ""}`}
                key={liveSessionKey(session)}
                title={session.cwd || session.id}
              >
                {/* 프로젝트가 펼쳐졌을 때는 실제로 도는 세션만 기존 glow를 유지한다. */}
                <VendorIcon
                  source={session.source}
                  size={12}
                  className={sessionActive ? "busy" : ""}
                />
                <span className="session-label">{session.label}</span>
                <span className="session-ago">{fmtAgo(session.at, language)}</span>
                <span className="session-meta">
                  {shortModel(session.model)}
                  {session.branch && ` · ${session.branch}`}
                </span>
                <span className="session-tokens">{fmtTokens(session.tokens)}</span>
              </div>
            );
          })}
          {project.sessionCount > project.sessions.length && (
            <div className="project-more">
              {t(
                `최근 ${project.sessions.length}개 세션 표시`,
                `Showing ${project.sessions.length} most recent sessions`,
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
}

/** 독립 창으로 뜨는 사용량 패널 — 펫 우클릭 또는 트레이 메뉴로 토글 */
export default function UsagePanel() {
  const { language, t } = useI18n();
  const summary = useSummary();
  const live = useLive();
  const plans = usePlans();
  const currency = useCurrency();
  const thresholds = useThresholds();
  const retention = useRetentionDays();
  // 설정 파일은 사람이 고치는 JSON 이라 목록에 없는 값(예: 45일)이 들어 있을 수 있다.
  // 그때 목록만 그리면 select 가 엉뚱한 항목을 가리키고, 다른 걸 고르는 순간 그 값이
  // 조용히 사라진다 — 있는 그대로 한 칸을 내어 준다.
  const options: number[] = (RETENTION_OPTIONS as readonly number[]).includes(retention)
    ? [...RETENTION_OPTIONS]
    : [...RETENTION_OPTIONS, retention].sort((a, b) => (a === 0 ? 1 : b === 0 ? -1 : a - b));
  const [page, setPage] = useState<number>(PAGE.STATUS);
  // ⚠️ 훅은 전부 조기 반환(`if (!summary)`) **위**에 있어야 한다. 아래에 두면 summary 가
  // 도착하는 순간 훅 개수가 늘어나 React 가 던지고, 패널은 투명 창이라 "안 열린다"로 보인다.
  // "all" = 개요. 벤더를 고르면 그 벤더의 구성·효율만 본다 — 시간축(잔디·주간)은
  // 소스별로 나뉘어 오지 않으므로 개요에만 있다.
  const [tab, setTab] = useState<Source | "all">("all");
  const [selectedHistoryDate, setSelectedHistoryDate] = useState<string | null>(null);
  // 설정 저장 오류 배너와 같은 규칙: 닫은 소스는 같은 오류가 지속되는 동안 숨기고,
  // 정상으로 돌아오면 목록에서 빼 다음 오류 때 다시 보이게 한다.
  const [dismissedParserSources, setDismissedParserSources] = useState<Source[]>([]);
  // 기본은 모두 접힘. 라이브 상태가 바뀌어도 작업 중 프로젝트를 자동으로 펼치지 않는다.
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(() => new Set());
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

  useEffect(() => {
    if (!summary) return;
    const active = new Set(
      summary.sources.filter((s) => s.status.kind === "degraded").map((s) => s.source),
    );
    setDismissedParserSources((prev) => {
      const next = prev.filter((source) => active.has(source));
      return next.length === prev.length ? prev : next;
    });
  }, [summary]);

  if (!summary) {
    return (
      <div className="panel-root">
        <ResizeGrips />
        <div className="card">
          <div className="loading">{t("사용량 스캔 중…", "Scanning usage…")}</div>
        </div>
      </div>
    );
  }

  // 기록이 있는 벤더만 탭으로 세운다 (`period` 는 옛 백엔드엔 없다)
  const vendorTabs = summary.sources.filter((v) => (v.period ? totalOf(v.period) : 0) > 0);
  const picked = summary.sources.find((v) => v.source === tab) ?? null;
  const parserErrors = summary.sources.filter(
    (s) => s.status.kind === "degraded" && !dismissedParserSources.includes(s.source),
  );
  const parserErrorLabels = parserErrors.map((s) => s.label).join(", ");
  const parserErrorFailed = parserErrors.reduce(
    (sum, s) => sum + (s.status.kind === "degraded" ? s.status.failed : 0),
    0,
  );
  const shownModels = summary.models_today;
  const shownPeriodModels = picked
    ? summary.models_period.filter((m) => m.source === picked.source)
    : summary.models_period;
  // 잔디가 덮는 기간(= summary.daily) 전체 합계. 백엔드에 따로 담지 않고 여기서 더한다 —
  // 일별 값이 이미 다 와 있어 서버 왕복을 늘릴 이유가 없다.
  const periodTotal = summary.daily.reduce((s, d) => s + totalOf(d.totals), 0);
  const periodCost = summary.daily.reduce((s, d) => s + d.cost, 0);
  // 격자 기간이 아니라 **기록이 있는 날 수**로 라벨을 단다 — 앞쪽이 통째로 기록 없는
  // 구간이면 "84일에 42.3M" 으로 읽혀 일평균을 잘못 계산하게 된다 (잔디 범례와 같은 값).
  const recorded = recordedDays(summary.daily, summary.first_event_ts);
  const activeHistoryDays = summary.daily.filter((d) => totalOf(d.totals) > 0);
  const busiestHistoryDay = activeHistoryDays.reduce<(typeof summary.daily)[number] | null>(
    (top, day) => (!top || totalOf(day.totals) > totalOf(top.totals) ? day : top),
    null,
  );
  let currentStreak = 0;
  for (const day of [...summary.daily].reverse()) {
    if (totalOf(day.totals) === 0) break;
    currentStreak += 1;
  }
  const selectedHistoryDetail: DailyDetail | null =
    summary.daily_details?.find((d) => d.date === selectedHistoryDate) ??
    summary.daily_details?.[summary.daily_details.length - 1] ??
    null;
  const selectedHistoryTotal = selectedHistoryDetail
    ? summary.daily.find((d) => d.date === selectedHistoryDetail.date) ?? null
    : null;
  // 오늘과 비교할 평균은 실제 기록이 시작된 뒤의 직전 달력일만 쓴다. 설치 전 빈 날을
  // 0으로 넣으면 첫 며칠의 "평균 대비"가 부풀려진다.
  const firstRecordedDate = activeHistoryDays[0]?.date ?? null;
  const priorDays = summary.daily
    .filter((d) => d.date < summary.today_date && (!firstRecordedDate || d.date >= firstRecordedDate))
    .slice(-7);
  const priorAverage = priorDays.length
    ? priorDays.reduce((s, d) => s + totalOf(d.totals), 0) / priorDays.length
    : null;
  const priorAverageCost = priorDays.length
    ? priorDays.reduce((s, d) => s + d.cost, 0) / priorDays.length
    : null;
  const todayVsAverage = priorAverage && priorAverage > 0 ? totalOf(summary.today) / priorAverage : null;
  const todayDelta = todayVsAverage == null ? null : Math.round((todayVsAverage - 1) * 100);

  // 지금 돌고 있는 세션들 — 세 소스 다 파일에 적힌 턴 경계에서 온 값이다
  const running = live.sessions.filter((s) => s.status === "busy");
  const busySources = new Set(running.map((s) => s.source));
  // 현황 카드의 두 번째 우선순위는 마지막 세션 활동이다. 작업이 끝나도 방금 쓴
  // 벤더가 위에 남아야 카드 위치가 기본 순서로 갑자기 되돌아가지 않는다.
  const lastSessionAt = new Map<Source, number>();
  for (const session of summary.sessions) {
    const at = new Date(session.at).getTime();
    if (Number.isFinite(at) && at > (lastSessionAt.get(session.source) ?? 0)) {
      lastSessionAt.set(session.source, at);
    }
  }
  // 최근 세션 목록에서 **그 줄**만 짚기 위한 키. id 를 못 알아낸 세션은 넣지 않는다 —
  // 빈 문자열을 넣으면 id 가 빈 다른 줄과 잘못 맞물린다.
  const runningKeys = new Set(running.filter((s) => s.id).map(liveSessionKey));
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

  const periodSelect = (
    <select
      className="period-select"
      value={retention}
      onChange={(e) => void invoke("set_retention_days", { days: Number(e.currentTarget.value) })}
    >
      {options.map((d) => (
        <option key={d} value={d}>
          {retentionLabel(d, language)}
        </option>
      ))}
    </select>
  );

  return (
    <div className="panel-root" onWheel={onWheel}>
      <ResizeGrips />
      {parserErrors.length > 0 && (
        <div className="panel-source-error" role="alert">
          <span className="panel-source-error-text">
            {t(
              `⚠️ ${parserErrorLabels} 기록 ${parserErrorFailed}개를 읽지 못했습니다. CLI 로그 형식이 변경됐거나 파일이 손상됐을 수 있습니다.`,
              `⚠️ Could not read ${parserErrorFailed} ${parserErrorLabels} log records. The CLI log format may have changed or files may be damaged.`,
            )}
          </span>
          <button
            className="panel-source-error-close"
            title={t(
              "닫기 (오류가 복구될 때까지 다시 띄우지 않음)",
              "Dismiss until the error recovers",
            )}
            onClick={() =>
              setDismissedParserSources((prev) => [
                ...new Set([...prev, ...parserErrors.map((s) => s.source)]),
              ])
            }
          >
            ✕
          </button>
        </div>
      )}
      <div className="card">
        {/* 닫기는 드래그 영역(.head) 밖 — 헤더 안에 두면 드래그와 클릭이 얽힌다 */}
        <button
          className="panel-close"
          title={t("닫기 (Esc)", "Close (Esc)")}
          onClick={() => void getCurrentWindow().hide()}
        >
          ✕
        </button>
        {/* 비용은 통계 페이지의 토큰 총합 옆으로 옮겼다 — 헤더에 두면 어느 페이지에서든
            떠 있어서 벤더별 비용과 헷갈린다 */}
        <div className="head" data-tauri-drag-region="deep">
          <span className="title">{PAGE_TITLES[page][language === "en" ? 1 : 0]}</span>
          <span className="date">{summary.today_date.slice(5).replace("-", "/")}</span>
        </div>

        <div className="page-body" ref={bodyRef}>
          {page === PAGE.STATUS && (
            <>
              <div className="today-overview">
                <span className="today-overview-label">{t("오늘 누적", "Today")}</span>
                <div className="today-overview-values">
                  <span className="today-overview-tokens">{fmtTokens(totalOf(summary.today))}</span>
                  <span className="today-overview-cost">
                    {fmtCost(summary.today_cost, summary.cost_partial, currency)}
                  </span>
                </div>
                <span className="today-trend-label">
                  {t("최근 7일 토큰", "Tokens · last 7 days")}
                  {todayDelta != null && t(` · 평균 대비 ${todayDelta >= 0 ? "+" : ""}${todayDelta}%`, ` · ${todayDelta >= 0 ? "+" : ""}${todayDelta}% vs avg`)}
                </span>
                <ModelToday models={shownModels} />
              </div>
              {/* 작업 중 → 최근 세션 활동 → 기본 순서. 활성 표시는 작업 중인 세션만 근거로
                  하지만, 끝난 뒤 카드 위치는 마지막 활동 시각을 이어받는다. */}
              <div className="sources">
                {summary.sources
                  .filter((s) => s.status.kind !== "no_data")
                  .sort((a, b) => {
                    const busy = Number(busySources.has(b.source)) - Number(busySources.has(a.source));
                    if (busy) return busy;
                    return (lastSessionAt.get(b.source) ?? 0) - (lastSessionAt.get(a.source) ?? 0);
                  })
                  .map((s) => (
                    <VendorCard
                      key={s.source}
                      s={s}
                      context={summary.contexts.find((c) => c.source === s.source) ?? null}
                      plan={plans.find((p) => p.source === s.source) ?? null}
                      active={busySources.has(s.source)}
                      busy={busySources.has(s.source)}
                      currency={currency}
                      thresholds={thresholds}
                    />
                  ))}
              </div>
            </>
          )}

          {page === PAGE.STATS && (
            <>
              {/* 탭 줄과 판을 한 덩어리로 묶는다 — `.page-body` 의 gap 이 둘 사이에
                  들어가면 폴더가 끊겨 보인다 (margin -1px 로는 못 이긴다). */}
              <div className="vgroup">
              {/* 탭과 본문을 한 줄로 잇고, 본문은 탭에 이어 붙는 판(`.vpanel`)으로 감싼다. */}
              <div className="vtabs-row">
                <div className="vtabs" role="tablist">
                  <button
                    role="tab"
                    aria-selected={tab === "all"}
                    className={tab === "all" ? "on" : ""}
                    onClick={() => setTab("all")}
                    title={t("전체", "All")}
                  >
                    {/* 벤더 탭과 같은 요소로 감싼다 — 라벨 처리(말줄임)를 한 규칙이
                        맡게 하려면 맨 텍스트로 두면 안 된다. */}
                    <span className="vtab-name">{t("전체", "All")}</span>
                  </button>
                  {vendorTabs.map((v) => (
                    <button
                      role="tab"
                      key={v.source}
                      aria-selected={tab === v.source}
                      className={tab === v.source ? "on" : ""}
                      onClick={() => setTab(v.source)}
                      title={SOURCE_LABEL[v.source]}
                    >
                      <VendorIcon source={v.source} size={11} />
                      <span className="vtab-name">{SHORT_VENDOR[v.source]}</span>
                    </button>
                  ))}
                </div>
              </div>
              <div className="vpanel">
              {tab === "all" ? (
                <div className="overall-total">
                  <div className="overall-total-head">
                    <span className="overall-total-label">{t("전체 사용량", "Total usage")}</span>
                    {periodSelect}
                  </div>
                  <div className="overall-total-values">
                    <span className="overall-total-tokens">{fmtTokens(periodTotal)}</span>
                    <span className="overall-total-cost">{fmtCost(periodCost, false, currency)}</span>
                  </div>
                  <span className="overall-total-recorded">{t(`기록 ${recorded}일`, `${recorded} recorded days`)}</span>
                </div>
              ) : picked ? (
                <div className="overall-total">
                  <div className="overall-total-head">
                    <span className="overall-total-label">{SOURCE_LABEL[picked.source]} {t("사용량", "usage")}</span>
                    {periodSelect}
                  </div>
                  <div className="overall-total-values">
                    <span className="overall-total-tokens">{fmtTokens(totalOf(picked.period))}</span>
                    <span className="overall-total-cost">{fmtCost(picked.period_cost, false, currency)}</span>
                  </div>
                  <span className="overall-total-recorded">{t(`기록 ${recorded}일`, `${recorded} recorded days`)}</span>
                </div>
              ) : null}

              {tab === "all" && priorAverageCost != null && (
                <div className="chart-block">
                  <div className="chart-title">{t("비용 페이스", "Cost pace")}</div>
                  <div className="cost-pace">
                    <div className="cost-pace-item">
                      <span>{t(`최근 ${priorDays.length}일 일평균`, `${priorDays.length}-day daily average`)}</span>
                      <b>{fmtCost(priorAverageCost, false, currency)}</b>
                    </div>
                    <div className="cost-pace-item">
                      <span>{t("30일 환산", "30-day projection")}</span>
                      <b>{fmtCost(priorAverageCost * 30, false, currency)}</b>
                    </div>
                  </div>
                </div>
              )}

              {/* 개요에서 기간 전체를 어떤 벤더가 차지했는지 먼저 보고, 아래 시간축으로
                  내려간다. 이 줄은 벤더 상세로 들어가는 문이기도 하다. */}
              {tab === "all" && vendorTabs.length > 1 && (
                <div className="chart-block">
                  <div className="chart-title">{t(`벤더별 비용 · ${recorded}일`, `Cost by provider · ${recorded} days`)}</div>
                  <VendorShare
                    sources={vendorTabs}
                    currency={currency}
                    basis="cost"
                    onPick={setTab}
                  />
                </div>
              )}

              {tab !== "all" && picked && (
                <>
                  {shownPeriodModels.some((m) => m.cost_known) && (
                    <div className="chart-block">
                      <div className="chart-title">{t(`모델별 비용 · ${recorded}일`, `Cost by model · ${recorded} days`)}</div>
                      <ModelMix models={shownPeriodModels} basis="cost" currency={currency} />
                    </div>
                  )}
                  <div className="cost-analysis">
                    <div className="chart-title">{t(`비용 분석 · ${recorded}일`, `Cost analysis · ${recorded} days`)}</div>
                    <div className="cost-analysis-section">
                      <span className="cost-analysis-title">{t("항목별 비용", "Cost by token type")}</span>
                      <CostBreakdown
                        totals={picked.period}
                        parts={picked.period_parts ?? EMPTY_PARTS}
                        currency={currency}
                      />
                    </div>
                    <div className="cost-analysis-section">
                      <span className="cost-analysis-title">{t("캐시 효율", "Cache efficiency")}</span>
                      <Efficiency
                        totals={picked.period}
                        parts={picked.period_parts ?? EMPTY_PARTS}
                        currency={currency}
                      />
                    </div>
                  </div>
                </>
              )}

              {tab === "all" && (
              <div className="chart-block">
                <div className="chart-title">{t("모델별 토큰 · 최근 7일", "Tokens by model · last 7 days")}</div>
                <WeekBars
                  daily={summary.daily}
                  weekModels={summary.week_models ?? []}
                  currency={currency}
                  basis="tokens"
                />
              </div>
              )}

              </div>
              </div>
            </>
          )}

          {page === PAGE.SESSIONS && (
            <div className="sessions">
              {(summary.projects ?? []).length === 0 ? (
                <div className="empty-hint">{t("최근 세션이 없습니다", "No recent sessions")}</div>
              ) : (
                <>
                  <div className="session-period">
                    {t("프로젝트 합계", "Project totals")} · {retentionLabel(retention, language)}
                  </div>
                  {(summary.projects ?? []).map((project) => (
                    <ProjectSessionGroup
                      key={project.key}
                      project={project}
                      collapsed={!expandedProjects.has(project.key)}
                      runningKeys={runningKeys}
                      retention={retention}
                      onToggle={() => setExpandedProjects((previous) => {
                        const next = new Set(previous);
                        if (next.has(project.key)) next.delete(project.key);
                        else next.add(project.key);
                        return next;
                      })}
                    />
                  ))}
                </>
              )}
            </div>
          )}

          {page === PAGE.HISTORY && (
            <div className="history-page">
              <div className="history-summary">
                <div className="history-summary-head">
                  <span className="history-summary-label">{t("활동 요약", "Activity summary")}</span>
                  <div className="history-period">
                    {periodSelect}
                  </div>
                </div>
                <div className="history-summary-items">
                  <span>
                    <i>{t("기록 시작", "First record")}</i>
                    <b>{summary.first_event_ts ? localMonthDay(summary.first_event_ts) : "—"}</b>
                  </span>
                  <span>
                    <i>{t("활동일", "Active days")}</i>
                    <b>{activeHistoryDays.length}{t("일", "")}</b>
                  </span>
                  <span>
                    <i>{t("연속", "Streak")}</i>
                    <b>{currentStreak}{t("일", " days")}</b>
                  </span>
                  <span>
                    <i>{t("최다 사용", "Busiest")}</i>
                    <b>{busiestHistoryDay ? busiestHistoryDay.date.slice(5) : "—"}</b>
                  </span>
                </div>
              </div>
              {showsHeatmap(retention) ? (
                <>
                  <UsageHeatmap
                    daily={summary.daily}
                    firstEvent={summary.first_event_ts}
                    currency={currency}
                    basis="tokens"
                    selectedDate={selectedHistoryDetail?.date}
                    onSelect={setSelectedHistoryDate}
                  />
                  {selectedHistoryDetail && selectedHistoryTotal && (
                    <div className="history-day-detail">
                      <div className="history-day-head">
                        <span>{t("선택한 날짜", "Selected date")}</span>
                        <b>{selectedHistoryDetail.date}</b>
                      </div>
                      <div className="history-day-total">
                        <b>{fmtTokens(totalOf(selectedHistoryTotal.totals))}</b>
                        <span>{fmtCost(selectedHistoryTotal.cost, false, currency)}</span>
                      </div>
                      {selectedHistoryDetail.sources.length > 0 && (
                        <div className="history-day-rows">
                          {selectedHistoryDetail.sources.map((source) => (
                            <div className="history-day-row" key={source.source}>
                              <VendorIcon source={source.source} size={11} />
                              <span>{SOURCE_LABEL[source.source]}</span>
                              <b>{fmtTokens(totalOf(source.totals))}</b>
                              {source.cost_known && <i>{fmtCost(source.cost, false, currency)}</i>}
                            </div>
                          ))}
                        </div>
                      )}
                      {selectedHistoryDetail.models.length > 0 && (
                        <div className="history-day-models">
                          <span className="history-day-models-title">{t("모델별 토큰", "Tokens by model")}</span>
                          <ModelMix models={selectedHistoryDetail.models} basis="tokens" currency={currency} />
                        </div>
                      )}
                    </div>
                  )}
                </>
              ) : (
                <div className="empty-hint">{t("사용 기록은 최근 28일 이상에서 볼 수 있습니다", "Activity history requires a period of at least 28 days")}</div>
              )}
            </div>
          )}
        </div>

        <div className="page-nav">
          <button className="nav-btn" onClick={prev} title={t("이전", "Previous")}>
            ◀
          </button>
          <div className="dots">
            {PAGE_TITLES.map((title, i) => (
              <button
                key={title[0]}
                className={`dot-btn ${i === page ? "on" : ""}`}
                onClick={() => setPage(i)}
                title={title[language === "en" ? 1 : 0]}
              />
            ))}
          </div>
          <button className="nav-btn" onClick={next} title={t("다음", "Next")}>
            ▶
          </button>
        </div>
      </div>
    </div>
  );
}

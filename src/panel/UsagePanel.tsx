import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ResizeGrips from "../components/ResizeGrips";
import { useLive, usePlans, useSummary } from "../hooks/useUsage";
import { fmtCost, fmtMinutes, fmtTokens, parseResetTime, shortModel, totalOf } from "../format";
import type { PlanUsage, Source, SourceStatus, SourceSummary } from "../types";
import "./panel.css";

function meterClass(pct: number): string {
  if (pct >= 85) return "danger";
  if (pct >= 60) return "warn";
  return "ok";
}

function statusChip(status: SourceStatus) {
  switch (status.kind) {
    case "no_data":
      return <span className="chip muted">미감지</span>;
    default:
      return null;
  }
}

function SourceRow({ s }: { s: SourceSummary }) {
  const total = totalOf(s.today);
  const chip = statusChip(s.status);
  return (
    <div className="source-row">
      <span className={`dot ${s.source}`} />
      <span className="source-label">{s.label}</span>
      {chip ?? (
        <>
          <span className="source-tokens">{fmtTokens(total)}</span>
          <span className="source-cost">{fmtCost(s.today_cost, s.cost_partial)}</span>
        </>
      )}
    </div>
  );
}

const SOURCE_LABEL: Record<Source, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "AGY",
};

/** 첫 미터의 리셋 안내. Codex 는 정확한 시각(`resets_at`)을, Claude 는 원문을 준다. */
function resetHint(p: PlanUsage): string {
  const m = p.meters[0];
  if (!m) return "";
  if (m.resets_at) {
    const d = new Date(m.resets_at);
    const day = `${d.getMonth() + 1}/${d.getDate()}`;
    const time = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    return `${m.label} 리셋 ${day} ${time}`;
  }
  return m.resets ? `세션 리셋 ${m.resets}` : "";
}

const PAGE_TITLES = ["요약", "소스·블록", "모델·세션"];

/** 독립 창으로 뜨는 사용량 패널 — 펫 우클릭 또는 트레이 메뉴로 토글 */
export default function UsagePanel() {
  const summary = useSummary();
  const live = useLive();
  const plans = usePlans();
  const [page, setPage] = useState(0);
  const bodyRef = useRef<HTMLDivElement | null>(null);

  // 사용자가 옮긴 위치·조절한 크기 기억 (연속 이벤트 debounce)
  useEffect(() => {
    const win = getCurrentWindow();
    let moveTimer: ReturnType<typeof setTimeout> | undefined;
    let sizeTimer: ReturnType<typeof setTimeout> | undefined;
    const unMoved = win.onMoved(({ payload }) => {
      clearTimeout(moveTimer);
      moveTimer = setTimeout(() => {
        void invoke("save_panel_position", { x: payload.x, y: payload.y });
      }, 500);
    });
    const unResized = win.onResized(({ payload }) => {
      clearTimeout(sizeTimer);
      sizeTimer = setTimeout(() => {
        void invoke("save_window_size", { label: "panel", width: payload.width, height: payload.height });
      }, 500);
    });
    return () => {
      clearTimeout(moveTimer);
      clearTimeout(sizeTimer);
      unMoved.then((f) => f());
      unResized.then((f) => f());
    };
  }, []);

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
  const maxModel = topModels.length > 0 ? totalOf(topModels[0].totals) : 0;

  // 5시간 블록 표시는 Claude 세션 리셋을 기준으로 삼는다 (블록 개념 자체가 Claude 것)
  const claudePlan = plans.find((p) => p.source === "claude") ?? null;
  const officialReset = claudePlan?.meters?.[0]?.resets
    ? parseResetTime(claudePlan.meters[0].resets)
    : null;
  const blockRemain = officialReset
    ? Math.max(0, Math.round((officialReset.getTime() - Date.now()) / 60000))
    : (summary.block?.remaining_minutes ?? 0);
  const blockRatio = officialReset
    ? Math.min(1, Math.max(0, 1 - blockRemain / 300))
    : (summary.block?.time_ratio ?? 0);

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
        <div className="head" data-tauri-drag-region="deep">
          <span className="title">{PAGE_TITLES[page]}</span>
          <span className="date">{summary.today_date.slice(5).replace("-", "/")}</span>
          <span className="cost">{fmtCost(summary.today_cost, summary.cost_partial)}</span>
        </div>

        <div className="page-body" ref={bodyRef}>
          {page === 0 && (
            <>
              <div className="grand">
                <span className="grand-tokens">{fmtTokens(todayTotal)}</span>
                <span className="grand-unit">tokens</span>
                <div className="grand-mini">
                  입력 {fmtTokens(summary.today.input)} · 출력 {fmtTokens(summary.today.output)} ·
                  캐시 {fmtTokens(summary.today.cache_write + summary.today.cache_read)}
                </div>
              </div>

              {/* 소스마다 한도를 얻는 경로가 달라 한 덩어리로 못 묶는다 —
                  Claude 는 CLI 조회, Codex 는 rollout 에 실려 온다. agy 는 아예 없다. */}
              {plans.map((p) => (
                <div className="plan" key={p.source}>
                  <div className="plan-title">
                    {SOURCE_LABEL[p.source]} 한도 (공식)
                    {p.detail && <span className="plan-detail"> · {p.detail}</span>}
                  </div>
                  {p.meters.map((m) => (
                    <div className="plan-row" key={m.label}>
                      <span className="plan-label">
                        {m.label
                          .replace("Current session", "세션(5h)")
                          .replace("Current week", "주간")}
                      </span>
                      <div className="bar plan-bar">
                        <div
                          className={`bar-fill meter ${meterClass(m.used_pct)}`}
                          style={{ width: `${m.used_pct}%` }}
                        />
                      </div>
                      <span className={`plan-pct ${meterClass(m.used_pct)}`}>{m.used_pct}%</span>
                    </div>
                  ))}
                  {/* 리셋까지 남은 시간도 같은 게이지로 — 다만 소진율이 아니라
                      "블록이 얼마나 지났나"라서 단계 색 대신 파란 시간 색을 쓴다 */}
                  {p.source === "claude" && officialReset && (
                    <div className="plan-row">
                      <span className="plan-label">리셋까지</span>
                      <div className="bar plan-bar">
                        <div className="bar-fill time" style={{ width: `${blockRatio * 100}%` }} />
                      </div>
                      <span className="plan-pct time">{fmtMinutes(blockRemain)}</span>
                    </div>
                  )}
                  <div className="plan-reset">{resetHint(p)}</div>
                </div>
              ))}
            </>
          )}

          {page === 1 && (
            <>
              <div className="sources">
                {summary.sources.map((s) => (
                  <SourceRow key={s.source} s={s} />
                ))}
              </div>

              {summary.block && (
                <div className="block">
                  <div className="block-head">
                    <span>{claudePlan ? "이번 블록 상세 (로컬)" : "5시간 블록"}</span>
                    <span className="block-remain">
                      {fmtMinutes(blockRemain)} 남음{officialReset ? " (공식)" : ""}
                    </span>
                  </div>
                  <div className="bar">
                    <div className="bar-fill time" style={{ width: `${blockRatio * 100}%` }} />
                  </div>
                  <div className="block-foot">
                    <span>{fmtTokens(totalOf(summary.block.totals))} tokens</span>
                    <span>{fmtCost(summary.block.cost, summary.block.cost_partial)}</span>
                    {summary.block.token_ratio != null && !claudePlan && (
                      <span
                        className={summary.block.token_ratio >= 0.8 ? "ratio warn-text" : "ratio"}
                      >
                        최대 블록 대비 {Math.round(summary.block.token_ratio * 100)}%
                      </span>
                    )}
                  </div>
                </div>
              )}

            </>
          )}

          {page === 2 && (
            <>
              {topModels.length > 0 ? (
                <div className="models">
                  {topModels.map((m) => (
                    <div className="model-row" key={`${m.source}-${m.model}`}>
                      <span className="model-name">{shortModel(m.model)}</span>
                      <div className="model-bar">
                        <div
                          className={`model-fill ${m.source}`}
                          style={{
                            width: `${maxModel > 0 ? (totalOf(m.totals) / maxModel) * 100 : 0}%`,
                          }}
                        />
                      </div>
                      <span className="model-tokens">{fmtTokens(totalOf(m.totals))}</span>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="empty-hint">오늘 사용된 모델이 없습니다</div>
              )}

              {summary.last_model && (
                <div className="active-model">
                  활성 모델: <b>{shortModel(summary.last_model)}</b>
                </div>
              )}

              <div className={`live ${live.busy ? "busy" : ""}`}>
                {live.busy
                  ? `⚙ 작업 중 세션 ${live.busy_count}개`
                  : live.sessions.length > 0
                    ? `세션 ${live.sessions.length}개 대기 중`
                    : "실행 중인 세션 없음"}
                {/* 어느 CLI 가 움직이는지가 정보의 핵심 — 이제 셋이 섞이므로 벤더를 밝힌다.
                    `active` 는 파일 신선도로 유도한 것이라 `~` 로 구분 표시한다. */}
                {live.sessions.length > 0 && (
                  <span className="live-sources">
                    {live.sessions
                      .map((s) => `${SOURCE_LABEL[s.source]}${s.status === "active" ? "~" : ""}`)
                      .filter((v, i, a) => a.indexOf(v) === i)
                      .join(" · ")}
                  </span>
                )}
              </div>
            </>
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

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useLive, usePlan, useSummary } from "../hooks/useUsage";
import { fmtCost, fmtMinutes, fmtTokens, parseResetTime, shortModel, totalOf } from "../format";
import type { SourceStatus, SourceSummary } from "../types";
import "./bubble.css";

function meterClass(pct: number): string {
  if (pct >= 85) return "danger";
  if (pct >= 60) return "warn";
  return "ok";
}

function statusChip(status: SourceStatus) {
  switch (status.kind) {
    case "no_data":
      return <span className="chip muted">미감지</span>;
    case "needs_setup":
      return <span className="chip warn">설정 필요</span>;
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

const PAGE_TITLES = ["요약", "소스·블록", "모델·세션"];

export default function Bubble() {
  const summary = useSummary();
  const live = useLive();
  const plan = usePlan();
  const [page, setPage] = useState(0);
  const [tailPos, setTailPos] = useState<"bottom" | "top">("bottom");
  const [pinned, setPinned] = useState(false);

  // 꼬리 방향(펫 기준 배치: 위=bottom 꼬리, 아래=top 꼬리) + 고정 상태 동기화
  useEffect(() => {
    const unTail = listen<string>("bubble-tail", (e) => {
      setTailPos(e.payload === "top" ? "top" : "bottom");
    });
    const unPin = listen<boolean>("bubble-pinned", (e) => setPinned(e.payload));
    return () => {
      unTail.then((f) => f());
      unPin.then((f) => f());
    };
  }, []);

  // 펫 ↔ 말풍선 간 호버 이동 허용 (백엔드 hover 플래그)
  const onEnter = () => void invoke("set_hover", { zone: "bubble", hovering: true });
  const onLeave = () => {
    void invoke("set_hover", { zone: "bubble", hovering: false });
    window.setTimeout(() => void invoke("hide_bubble"), 250);
  };

  if (!summary) {
    return (
      <div className="bubble-root" onMouseEnter={onEnter} onMouseLeave={onLeave}>
        <div className="card">
          <div className={`tail ${tailPos}`} />
          <div className="loading">사용량 스캔 중…</div>
        </div>
      </div>
    );
  }

  const needsSetup = summary.sources.find((s) => s.status.kind === "needs_setup");
  const topModels = summary.models_today.slice(0, 5);
  const todayTotal = totalOf(summary.today);
  const maxModel = topModels.length > 0 ? totalOf(topModels[0].totals) : 0;

  const officialReset = plan?.meters?.[0]?.resets ? parseResetTime(plan.meters[0].resets) : null;
  const blockRemain = officialReset
    ? Math.max(0, Math.round((officialReset.getTime() - Date.now()) / 60000))
    : (summary.block?.remaining_minutes ?? 0);
  const blockRatio = officialReset
    ? Math.min(1, Math.max(0, 1 - blockRemain / 300))
    : (summary.block?.time_ratio ?? 0);

  const pageCount = PAGE_TITLES.length;
  const prev = () => setPage((p) => (p + pageCount - 1) % pageCount);
  const next = () => setPage((p) => (p + 1) % pageCount);

  return (
    <div
      className="bubble-root"
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      onWheel={(e) => (e.deltaY > 0 ? next() : prev())}
    >
      <div className="card">
        <div className={`tail ${tailPos}`} />

        <div className="head">
          <span className="title">{PAGE_TITLES[page]}</span>
          <span className="date">{summary.today_date.slice(5).replace("-", "/")}</span>
          <span className="cost">{fmtCost(summary.today_cost, summary.cost_partial)}</span>
          <button
            className={`pin-btn ${pinned ? "on" : ""}`}
            title={pinned ? "고정 해제" : "말풍선 고정"}
            onClick={() => void invoke("toggle_bubble_pin")}
          >
            📌
          </button>
        </div>

        <div className="page-body">
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

              {plan && plan.meters.length > 0 && (
                <div className="plan">
                  <div className="plan-title">플랜 한도 (공식)</div>
                  {plan.meters.map((m) => (
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
                  {plan.meters[0]?.resets && (
                    <div className="plan-reset">세션 리셋 {plan.meters[0].resets}</div>
                  )}
                </div>
              )}
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
                    <span>{plan ? "이번 블록 상세 (로컬)" : "5시간 블록"}</span>
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
                    {summary.block.token_ratio != null && !plan && (
                      <span
                        className={summary.block.token_ratio >= 0.8 ? "ratio warn-text" : "ratio"}
                      >
                        최대 블록 대비 {Math.round(summary.block.token_ratio * 100)}%
                      </span>
                    )}
                  </div>
                </div>
              )}

              {needsSetup && needsSetup.status.kind === "needs_setup" && (
                <div className="setup-hint">
                  {needsSetup.label}: ~/.gemini/settings.json에 telemetry(local) 설정 필요
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

import { useRef } from "react";
import { DEFAULT_LINES } from "../pet/speech";

/** 문구에서 쓸 수 있는 변수 — 리셋 임박은 백엔드가 채우는 {분}·{시각}만 지원한다 */
const VARS_DEFAULT = ["오늘토큰", "오늘비용", "세션", "주간", "컨텍스트", "리셋", "리셋시각", "모델", "벤더"];
const VARS_RESET = ["분", "시각"];

interface FieldProps {
  /** 상황 키 (speech.ts DEFAULT_LINES · speech.json 과 동일) */
  situationKey: string;
  label: string;
  /** 편집 중인 문구 (상황 키 → 줄 목록) */
  lines: Record<string, string[]>;
  /** 팩 편집 시 폴백되는 기본 문구 — placeholder(흐린 글씨)로 보여준다 */
  baseLines?: Record<string, string[]> | null;
  onChange: (key: string, raw: string) => void;
  /** ▶ 테스트 — 지금 실제로 나갈 문구 하나를 골라 펫이 말하게 한다 */
  onTest?: (key: string, template: string) => void;
}

const nonBlankLines = (raw: string): string[] =>
  raw.split("\n").filter((l) => l.trim());

/** 상황 하나의 문구 편집 칸 — 캐릭터 스튜디오가 상태 카드마다 끼워 넣는다 */
export function SpeechField({
  situationKey: key,
  label,
  lines,
  baseLines,
  onChange,
  onTest,
}: FieldProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const value = lines[key]?.join("\n") ?? "";
  const placeholder = (() => {
    if (baseLines?.[key]?.some((l) => l.trim())) {
      return (baseLines[key] ?? []).join("\n");
    }
    return (DEFAULT_LINES[key] ?? []).join("\n");
  })();
  /** 지금 실제로 쓰이는 후보 — 런타임 linesFor 와 같은 판정 (편집값 → 폴백) */
  const pool = nonBlankLines(value).length
    ? nonBlankLines(value)
    : nonBlankLines(placeholder);

  /** 변수 칩 클릭 → 커서 위치에 {변수} 삽입 */
  const insertVar = (name: string) => {
    const el = ref.current;
    const token = `{${name}}`;
    const start = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? start;
    onChange(key, value.slice(0, start) + token + value.slice(end));
    if (el) {
      requestAnimationFrame(() => {
        el.focus();
        el.setSelectionRange(start + token.length, start + token.length);
      });
    }
  };

  const vars = key === "resetNotify" ? VARS_RESET : VARS_DEFAULT;

  return (
    <div className="settings-speech-item">
      <div className="settings-speech-head">
        <span className="settings-hint">{label}</span>
        <span className="settings-hint">
          {pool.length > 1 ? `${pool.length}개 중 무작위 ` : ""}
          {onTest && pool.length > 0 && (
            <button
              className="speech-test"
              title="펫이 이 대사를 실제로 말해봅니다 (무작위 한 줄)"
              onClick={() =>
                onTest(key, pool[Math.floor(Math.random() * pool.length)])
              }
            >
              ▶ 테스트
            </button>
          )}
        </span>
      </div>
      <textarea
        ref={ref}
        className="settings-textarea"
        rows={2}
        spellCheck={false}
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(key, e.currentTarget.value)}
      />
      {/* 변수 칩 — 칸에 포커스가 있을 때만 (CSS :focus-within).
          mousedown 을 막아야 클릭 순간 textarea 가 포커스를 잃으며 칩이 사라지지 않는다 */}
      <div className="speech-vars chips">
        {vars.map((v) => (
          <button
            key={v}
            className="chip-btn"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => insertVar(v)}
            title="커서 위치에 삽입"
          >
            {`{${v}}`}
          </button>
        ))}
      </div>
    </div>
  );
}

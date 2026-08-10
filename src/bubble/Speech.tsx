import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./bubble.css";

interface SpeechEvent {
  text: string;
  tail: "bottom" | "top";
}

/**
 * 캐릭터 대사 말풍선 — 상황 이벤트(한도 경고·블록 리셋·작업 시작/종료·잠자기 등)에만
 * 잠깐 떴다 사라진다. 표시/숨김과 위치는 백엔드(commands::show_speech)가 결정하고,
 * 이 창은 전달받은 문구를 그리기만 한다.
 */
export default function Speech() {
  const [text, setText] = useState("");
  const [tail, setTail] = useState<"bottom" | "top">("bottom");
  // 문구가 바뀔 때마다 등장 애니메이션을 다시 태우기 위한 키
  const [seq, setSeq] = useState(0);

  useEffect(() => {
    const un = listen<SpeechEvent>("speech", (e) => {
      setText(e.payload.text);
      setTail(e.payload.tail === "top" ? "top" : "bottom");
      setSeq((n) => n + 1);
    });
    // 펫을 드래그하다 화면 위/아래 경계를 넘으면 말풍선이 뒤집힌다.
    // seq 는 건드리지 않는다 — 올리면 이동 중에 등장 애니메이션이 다시 돈다.
    const unTail = listen<"bottom" | "top">("speech-tail", (e) => {
      setTail(e.payload === "top" ? "top" : "bottom");
    });
    return () => {
      un.then((f) => f());
      unTail.then((f) => f());
    };
  }, []);

  return (
    <div className={`speech-root tail-${tail}`}>
      {/* 창은 백엔드가 먼저 띄우고 문구는 이벤트로 도착한다 —
          그 사이 한 프레임 동안 빈 말풍선이 보이지 않도록 문구가 있을 때만 그린다 */}
      {text && (
        <div className="speech-card" key={seq}>
          <div className={`tail ${tail}`} />
          <span className="speech-text">{text}</span>
        </div>
      )}
    </div>
  );
}

import { useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./resize.css";

/** 창 테두리(투명 패딩 8px) 위에 얹는 리사이즈 손잡이. 대각선 포함 여덟 방향. */
const GRIPS = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];

/**
 * 장식 없는 창이라 OS 테두리가 없다 → 가장자리를 직접 잡아 크기를 조절한다.
 * 사용량 패널·설정 창이 공유하고, 백엔드는 현재 창 라벨로 대상을 구분한다.
 *
 * 펫 드래그와 같은 이유로 네이티브 리사이즈(`startResizeDragging`)를 쓰지 않는다:
 * 그러려면 창이 resizable 이어야 하는데 그때 Windows 가 붙이는 WS_THICKFRAME 이
 * 투명 창 둘레에 흐릿한 프레임 그림자를 만든다. 실제 계산은 백엔드가 물리 좌표로 한다 —
 * 여기서 screenX(CSS px)를 환산하면 배율 다른 모니터에서 커서와 창이 어긋난다.
 */
export default function ResizeGrips() {
  const active = useRef(false);
  const label = getCurrentWindow().label;
  return (
    <>
      {GRIPS.map((dir) => (
        <div
          key={dir}
          className={`resize-grip ${dir}`}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            // 포인터 캡처로 커서가 창 밖으로 나가도 이벤트를 계속 받는다
            e.currentTarget.setPointerCapture(e.pointerId);
            active.current = true;
            void invoke("start_window_resize", { label, dir });
          }}
          onPointerMove={(e) => {
            if (!active.current) return;
            // Esc·✕ 로 창을 닫으면 pointerup 을 못 받고 끝날 수 있다. 버튼이 떼어져
            // 있으면 끝난 드래그로 보고 정리한다 — 안 그러면 다시 열었을 때 손잡이에
            // 커서만 스쳐도 창이 끌려간다.
            if (e.buttons === 0) {
              active.current = false;
              void invoke("end_window_resize");
              return;
            }
            void invoke("resize_window");
          }}
          onPointerUp={(e) => {
            if (!active.current) return;
            active.current = false;
            if (e.currentTarget.hasPointerCapture(e.pointerId)) {
              e.currentTarget.releasePointerCapture(e.pointerId);
            }
            void invoke("end_window_resize");
          }}
          onLostPointerCapture={() => {
            // 캡처가 끊겨도(창 전환 등) 리사이즈 상태가 남지 않게
            if (!active.current) return;
            active.current = false;
            void invoke("end_window_resize");
          }}
        />
      ))}
    </>
  );
}

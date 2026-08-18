import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 기하를 기억하는 창 — 백엔드 `window::WINDOWS` 표와 같은 라벨이어야 한다 */
export type PersistedWindow = "panel" | "settings" | "studio";

/** 이벤트가 몰아쳐도 쓰기는 한 번만 — 드래그·리사이즈는 초당 수십 번 발생한다 */
const DEBOUNCE_MS = 500;

/**
 * 창의 위치·크기를 설정 파일에 기억시킨다.
 *
 * 세 창(패널·설정·스튜디오)이 **같은 규칙**을 쓴다 — 자리를 옮기거나 크기를 조절하면
 * 다음에 열 때 그대로 복원된다. 예전엔 이 24줄이 세 컴포넌트에 그대로 복사돼 있었고,
 * 디바운스 시간이나 정리(cleanup) 하나만 어긋나도 한 창에서만 나는 버그가 됐다.
 *
 * `onResized` 는 사용자가 잡아 끌 때뿐 아니라 **창을 열며 백엔드가 크기를 복원할 때도**
 * 울린다. 그래서 여는 순간 같은 값이 한 번 다시 저장되는데, 값이 같으니 무해하다.
 */
export function useWindowPersist(label: PersistedWindow) {
  useEffect(() => {
    const win = getCurrentWindow();
    let moveTimer: ReturnType<typeof setTimeout> | undefined;
    let sizeTimer: ReturnType<typeof setTimeout> | undefined;
    const unMoved = win.onMoved(({ payload }) => {
      clearTimeout(moveTimer);
      moveTimer = setTimeout(() => {
        void invoke("save_window_position", { label, x: payload.x, y: payload.y });
      }, DEBOUNCE_MS);
    });
    const unResized = win.onResized(({ payload }) => {
      clearTimeout(sizeTimer);
      sizeTimer = setTimeout(() => {
        void invoke("save_window_size", {
          label,
          width: payload.width,
          height: payload.height,
        });
      }, DEBOUNCE_MS);
    });
    return () => {
      clearTimeout(moveTimer);
      clearTimeout(sizeTimer);
      unMoved.then((f) => f());
      unResized.then((f) => f());
    };
  }, [label]);
}

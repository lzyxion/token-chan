import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "../i18n";

/** 여러 파일이 실패하면 하나를 닫은 뒤 다음 미확인 오류를 보여준다. */
export function useLoadErrors() {
  const { t } = useI18n();
  const [errors, setErrors] = useState<string[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);

  useEffect(() => {
    let alive = true;
    let revision = 0;
    const apply = (values: string[]) => {
      if (!alive) return;
      setErrors(values);
      // 복구된 오류의 닫기 기록은 버린다. 같은 파일이 다시 실패하면 새 알림이다.
      setDismissed((old) => old.filter((error) => values.includes(error)));
    };
    const un = listen<string[]>("settings-load-errors", ({ payload }) => {
      ++revision;
      apply(payload);
    });
    // 먼저 구독하고 조회한다. 조회 중 들어온 새 이벤트를 과거 응답으로 덮지 않는다.
    void un.then(async () => {
      if (!alive) return;
      const before = revision;
      const values = await invoke<string[]>("get_load_errors");
      if (revision === before) apply(values);
    }).catch(() => {});
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);

  const error = errors.find((value) => !dismissed.includes(value));
  const file = error?.split(": ")[0].split(/[\\/]/).slice(-2).join("/");
  return {
    message: error ? t(
      `⚠️ ${file} 읽기 실패 — 해당 설정은 기본값으로 불러왔습니다. 파일 내용·권한을 확인한 뒤 ${file?.endsWith("/settings.json") ? "앱을 재시작하세요" : "↻로 다시 불러오세요"}.`,
      `⚠️ Failed to load ${file} — defaults were used. Check the file and permissions, then ${file?.endsWith("/settings.json") ? "restart the app" : "click ↻ to reload"}.`,
    ) : null,
    detail: error,
    dismiss: () => {
      if (error) setDismissed((old) => [...old, error]);
    },
  };
}

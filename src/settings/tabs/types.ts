import type { AppSettings } from "../../types";

/** 모든 탭이 공유하는 것 — 현재 설정과 "부분 수정" 한 가지.
 *  탭이 직접 저장하지 않는다: 저장·경합 방지는 껍데기(SettingsPanel)가 맡는다. */
export interface TabProps {
  s: AppSettings;
  update: (patch: Partial<AppSettings>) => void;
}

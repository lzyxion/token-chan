import { createContext, type ReactNode, useContext, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppSettings } from "./types";

export type Language = "ko" | "en";

export function languageOf(value: unknown): Language {
  return value === "ko" ? "ko" : "en";
}

export function tr(language: Language, ko: string, en: string): string {
  return language === "en" ? en : ko;
}

const LanguageContext = createContext<Language>("en");

export function I18nProvider({ children }: { children: ReactNode }) {
  const [language, setLanguage] = useState<Language>("en");

  useEffect(() => {
    let alive = true;
    const apply = (settings: AppSettings | null) => {
      if (!alive || !settings) return;
      setLanguage(languageOf(settings.language));
    };
    invoke<AppSettings>("get_settings").then(apply).catch(() => {});
    const unlisten = listen<AppSettings>("settings-changed", (event) => apply(event.payload));
    return () => {
      alive = false;
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  return <LanguageContext.Provider value={language}>{children}</LanguageContext.Provider>;
}

export function useI18n() {
  const language = useContext(LanguageContext);
  return {
    language,
    t: (ko: string, en: string) => tr(language, ko, en),
  };
}

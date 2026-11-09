import { useEffect } from "react";
import { useStore } from "../state/store";

/** Stamp the theme on the root: light/dark force it, system defers to prefers-color-scheme. */
export function useApplyTheme(): void {
  const theme = useStore((s) => s.settings?.theme ?? "system");
  const locale = useStore((s) => s.settings?.locale ?? "en");
  useEffect(() => {
    if (theme === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = theme;
  }, [theme]);
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
}

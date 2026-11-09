/*
 * The i18n runtime: the locale lives in the settings. `useT` binds a typed `t` to the active
 * dictionary; leaves are picked by accessor, `{name}` tokens interpolate, and a Plural leaf
 * resolves by `n`.
 */

import { useStore } from "../state/store";
import { en } from "./en";
import { fr } from "./fr";
import type { Dict, Plural } from "./en";
import type { Locale } from "../lib/ipc";

const DICTS: Record<Locale, Dict> = { en, fr };

type Vars = Record<string, string | number>;

export interface Translate {
  (pick: (d: Dict) => string, vars?: Vars): string;
  (pick: (d: Dict) => Plural, vars: Vars & { n: number }): string;
}

function interpolate(text: string, vars?: Vars): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in vars ? String(vars[name]) : whole,
  );
}

export function createT(locale: Locale): Translate {
  const dict = DICTS[locale] ?? en;
  const t = (pick: (d: Dict) => string | Plural, vars?: Vars): string => {
    const node = pick(dict);
    const text = typeof node === "string" ? node : vars?.n === 1 ? node.one : node.other;
    return interpolate(text, vars);
  };
  return t as Translate;
}

export function useLocale(): Locale {
  return useStore((s) => s.settings?.locale ?? "en");
}

export function useT(): Translate {
  return createT(useLocale());
}

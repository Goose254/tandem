/*
 * The updater store: what the last check found, whether the prompt is up, and the actions behind
 * the automatic check, the Settings button and the prompt.
 *
 * Rules that matter: a failed check is an error and never "up to date" (only a null answer is);
 * an automatic check stays silent when it fails (offline at launch is not worth a word); a
 * skipped version stays quiet until a newer one ships; a running download is never interrupted.
 */

import { useEffect } from "react";
import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { checkForUpdate, relaunchApp } from "../lib/updater";
import type { FoundUpdate } from "../lib/updater";
import { useStore } from "./store";

export type UpdateInfo = { version: string; notes: string };

export type UpdaterState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate" }
  | { kind: "available"; update: UpdateInfo }
  // `progress` runs 0..1, or null while the download size is unknown.
  | { kind: "downloading"; update: UpdateInfo; progress: number | null }
  | { kind: "ready"; update: UpdateInfo }
  // `update` is set when the install failed, so the prompt can offer a retry on it.
  | { kind: "error"; message: string; update: UpdateInfo | null };

/** Where a check came from: the quiet automatic one, or the Settings button. */
export type CheckSource = "auto" | "manual";

type UpdaterStore = {
  state: UpdaterState;
  promptOpen: boolean;
  check: (source: CheckSource) => Promise<void>;
  install: () => Promise<void>;
  later: () => void;
  skip: () => void;
  openPrompt: () => void;
};

/** The update a state carries, if any. */
export function updateOf(state: UpdaterState): UpdateInfo | null {
  return "update" in state ? state.update : null;
}

function reason(error: unknown): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message) return error.message;
  return "unknown error";
}

/** The plugin's handle is a backend resource: it lives beside the store, not in it. */
let held: FoundUpdate | null = null;

function hold(next: FoundUpdate | null) {
  if (held && held !== next) held.release();
  held = next;
}

const busy = (s: UpdaterState) => s.kind === "checking" || s.kind === "downloading" || s.kind === "ready";

export const useUpdater = create<UpdaterStore>((set, get) => ({
  state: { kind: "idle" },
  promptOpen: false,

  check: async (source) => {
    if (busy(get().state)) return;
    // An automatic check never wipes (or flickers) an update that is already on offer.
    const before = get().state;
    if (source === "manual" || !updateOf(before)) set({ state: { kind: "checking" } });
    let found: FoundUpdate | null;
    try {
      found = await checkForUpdate();
    } catch (error) {
      set({
        state: source === "auto" ? before : { kind: "error", message: reason(error), update: null },
      });
      return;
    }
    hold(found);
    void ipc.setUpdateNotice(found?.version ?? null).catch(() => {});
    if (!found) {
      set({ state: { kind: "upToDate" }, promptOpen: false });
      return;
    }
    const skipped = useStore.getState().settings?.skippedUpdate;
    set((s) => ({
      state: { kind: "available", update: { version: found.version, notes: found.notes } },
      promptOpen: s.promptOpen || source === "manual" || found.version !== skipped,
    }));
  },

  install: async () => {
    const found = held;
    const update = updateOf(get().state);
    if (!found || !update || busy(get().state)) return;
    set({ state: { kind: "downloading", update, progress: null }, promptOpen: true });
    try {
      await found.install((progress) =>
        set((s) => (s.state.kind === "downloading" ? { state: { ...s.state, progress } } : {})),
      );
      set({ state: { kind: "ready", update } });
      await relaunchApp();
    } catch (error) {
      set({ state: { kind: "error", message: reason(error), update } });
    }
  },

  // Closes the prompt for now; nothing is stored, so the next check brings it back.
  later: () => {
    if (!busy(get().state)) set({ promptOpen: false });
  },

  // Closes the prompt and stores the version, so it stays quiet until a newer one ships.
  skip: () => {
    const update = updateOf(get().state);
    if (busy(get().state)) return;
    if (update) void ipc.setPreferences({ skippedUpdate: update.version }).catch(() => {});
    set({ promptOpen: false });
  },

  openPrompt: () => {
    if (updateOf(get().state)) set({ promptOpen: true });
  },
}));

/** After launch, wait past startup so the request never competes with opening the devices. */
const FIRST_CHECK_MS = 5_000;
/** Tandem lives in the tray for days: keep looking, a few times a day. */
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

/**
 * The automatic checks, for the main window only: one shortly after launch, then one every few
 * hours, while "check automatically" is on. Nothing shows unless an update exists.
 */
export function useAutoUpdateCheck(): void {
  const armed = useStore((s) => s.ready && s.settings?.autoCheckUpdates === true);
  useEffect(() => {
    if (!armed) return;
    const check = () => void useUpdater.getState().check("auto");
    const first = window.setTimeout(check, FIRST_CHECK_MS);
    const repeat = window.setInterval(check, CHECK_EVERY_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(repeat);
    };
  }, [armed]);
}

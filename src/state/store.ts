/*
 * The app store. It hydrates from the Rust side once, then follows its events: `status` (the
 * supervisor's live snapshot, a few times per second), `settings` (after any change, from any
 * window) and `calibration` (auto-sync progress). Devices are polled while a window is open.
 */

import { create } from "zustand";
import { ipc, on, simulated } from "../lib/ipc";
import type { CalibrationEvent, Devices, Levels, OutputSetting, Settings, Snapshot } from "../lib/ipc";

export type View = "outputs" | "sync" | "settings";

type Store = {
  ready: boolean;
  settings: Settings | null;
  status: Snapshot | null;
  devices: Devices;
  autostart: boolean;
  calibration: Record<string, CalibrationEvent>;
  view: View;
  syncTarget: string | null;
  /** The output removed last, kept for a few seconds so the removal can be undone. */
  removed: OutputSetting | null;
  removeOutput: (output: OutputSetting) => void;
  undoRemove: () => void;
  setView: (view: View) => void;
  openSync: (deviceId: string) => void;
  setSyncTarget: (deviceId: string) => void;
  setSettings: (settings: Settings) => void;
  setAutostart: (on: boolean) => void;
  refreshDevices: () => Promise<void>;
};

const UNDO_WINDOW_MS = 8000;
let undoTimer: number | undefined;

export const useStore = create<Store>((set, get) => ({
  ready: false,
  settings: null,
  status: null,
  devices: { outputs: [], inputs: [] },
  autostart: false,
  calibration: {},
  view: "outputs",
  syncTarget: null,
  removed: null,
  removeOutput: (output) => {
    window.clearTimeout(undoTimer);
    set({ removed: output });
    undoTimer = window.setTimeout(() => set({ removed: null }), UNDO_WINDOW_MS);
    void ipc.removeOutput(output.device.id);
  },
  undoRemove: () => {
    const output = get().removed;
    window.clearTimeout(undoTimer);
    set({ removed: null });
    if (output) void ipc.restoreOutput(output);
  },
  setView: (view) => set({ view }),
  openSync: (deviceId) => set({ view: "sync", syncTarget: deviceId }),
  setSyncTarget: (deviceId) => set({ syncTarget: deviceId }),
  setSettings: (settings) => set({ settings }),
  setAutostart: (autostart) => set({ autostart }),
  refreshDevices: async () => {
    try {
      set({ devices: await ipc.listDevices() });
    } catch {
      /* no shell: keep the last list */
    }
  },
}));

let started = false;

/** Hydrate once per window and subscribe to the Rust events. */
export async function startSync(): Promise<void> {
  if (started) return;
  started = true;
  const { setState, getState } = useStore;
  try {
    const [settings, status, devices, autostart] = await Promise.all([
      ipc.getSettings(),
      ipc.getStatus(),
      ipc.listDevices(),
      ipc.getAutostart(),
    ]);
    setState({ settings, status, devices, autostart, ready: true });
  } catch {
    setState({ ready: true });
  }
  await on<Snapshot>("status", (status) => setState({ status }));
  await on<Levels>("levels", (levels) =>
    setState((s) =>
      s.status
        ? {
            status: {
              ...s.status,
              source: { ...s.status.source, level: levels.source },
              outputs: s.status.outputs.map((o) =>
                o.deviceId in levels.outputs ? { ...o, level: levels.outputs[o.deviceId] } : o,
              ),
            },
          }
        : {},
    ),
  );
  await on<Settings>("settings", (settings) => setState({ settings }));
  await on<CalibrationEvent>("calibration", (ev) =>
    setState((s) => ({ calibration: { ...s.calibration, [ev.deviceId]: ev } })),
  );
  // The simulated backend can open the preview on a given screen and state (`?view=`,
  // `?sync=`, ...): it is told once everything above is listening.
  if (simulated) void simulated.then((mock) => mock.previewReady((patch) => setState(patch)));
  window.setInterval(() => {
    if (document.visibilityState === "visible") void getState().refreshDevices();
  }, 2000);
}

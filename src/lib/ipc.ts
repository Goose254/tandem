/*
 * The typed bridge to the Rust side. Types mirror the serde structs (camelCase); each command
 * is a thin invoke wrapper. Outside the desktop shell (a plain browser preview) calls reject,
 * and callers treat that as "no data".
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { mockInvoke, mockListen } from "./mock";

/** True inside the desktop shell; false in a plain browser preview. */
export const inShell = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return inShell ? tauriInvoke<T>(cmd, args) : mockInvoke<T>(cmd, args);
}

export type DeviceRef = { id: string; name: string };
export type Device = { id: string; name: string; isDefault: boolean };
export type Devices = { outputs: Device[]; inputs: Device[] };

export type OutputSetting = {
  device: DeviceRef;
  enabled: boolean;
  gainDb: number;
  defaultDelayMs: number;
  delays: Record<string, number>;
};

export type ThemeChoice = "system" | "light" | "dark";
export type Locale = "en" | "fr";

export type Settings = {
  enabled: boolean;
  source: { followDefault: boolean; device: DeviceRef | null };
  outputs: OutputSetting[];
  mic: DeviceRef | null;
  resyncOnReconnect: boolean;
  /** Auto-sync against this output (device id) instead of the source. */
  syncReference: string | null;
  followSourceVolume: boolean;
  pauseWhenSilent: boolean;
  theme: ThemeChoice;
  locale: Locale;
  closeToTray: boolean;
  startMinimized: boolean;
};

export type SourceState = "off" | "running" | "waiting" | "error" | "paused";
export type OutputState =
  | "disabled"
  | "buffering"
  | "playing"
  | "waiting"
  | "isSource"
  | "idle"
  | "error";

/** A coarse reason for a device error; the raw message stays in `error`. */
export type ErrorKind = "inUse" | "invalidated" | "stalled" | "other";

/** Meter levels, sent more often than the full snapshot. */
export type Levels = { source: number; outputs: Record<string, number> };

export type OutputStatus = {
  deviceId: string;
  state: OutputState;
  delayMs: number;
  bufferedMs: number;
  correctionPpm: number;
  underruns: number;
  level: number;
  error: string | null;
  errorKind: ErrorKind | null;
  /** How far behind the source this output plays at 0 ms (its minimum path). */
  minLagMs: number | null;
  sampleRate: number | null;
};

export type Snapshot = {
  enabled: boolean;
  source: {
    state: SourceState;
    deviceId: string | null;
    deviceName: string | null;
    level: number;
    error: string | null;
    errorKind: ErrorKind | null;
    sampleRate: number | null;
    /** The source applies its volume itself, so following it makes a difference. */
    hardwareVolume: boolean;
  };
  outputs: OutputStatus[];
  clicks: boolean;
  calibrating: string | null;
};

export type Failure =
  | "noMic"
  | "micMissing"
  | "micFailed"
  | "notRunning"
  | "referenceNotRunning"
  | "sourceNotHeard"
  | "targetNotHeard"
  | "tooShort"
  | "busy";

export type CalibrationEvent = {
  deviceId: string;
  /** "limited": set to 0 ms and still `lagMs` late, so the source is too fast for it. */
  phase: "listening" | "done" | "limited" | "failed";
  pass: number;
  automatic: boolean;
  offsetMs: number | null;
  previousDelayMs: number | null;
  delayMs: number | null;
  lagMs: number | null;
  /** Device id of the output used in place of the source, if any. */
  reference: string | null;
  /** Set when the reference had to be delayed to line up: its new delay. */
  referenceDelayMs: number | null;
  failure: Failure | null;
};

export type PreferencesPatch = Partial<{
  theme: ThemeChoice;
  locale: Locale;
  closeToTray: boolean;
  startMinimized: boolean;
  resyncOnReconnect: boolean;
  mic: DeviceRef | null;
  syncReference: string | null;
  followSourceVolume: boolean;
  pauseWhenSilent: boolean;
}>;

export const ipc = {
  getSettings: () => invoke<Settings>("get_settings"),
  getStatus: () => invoke<Snapshot>("get_status"),
  listDevices: () => invoke<Devices>("list_devices"),
  setEnabled: (enabled: boolean) => invoke<void>("set_enabled", { enabled }),
  setSource: (followDefault: boolean, device: DeviceRef | null) =>
    invoke<void>("set_source", { followDefault, device }),
  addOutput: (device: DeviceRef) => invoke<void>("add_output", { device }),
  removeOutput: (deviceId: string) => invoke<void>("remove_output", { deviceId }),
  restoreOutput: (output: OutputSetting) => invoke<void>("restore_output", { output }),
  setOutputEnabled: (deviceId: string, enabled: boolean) =>
    invoke<void>("set_output_enabled", { deviceId, enabled }),
  setOutputDelay: (deviceId: string, delayMs: number) =>
    invoke<void>("set_output_delay", { deviceId, delayMs: Math.round(delayMs) }),
  setOutputGain: (deviceId: string, gainDb: number) =>
    invoke<void>("set_output_gain", { deviceId, gainDb }),
  setClicks: (on: boolean) => invoke<void>("set_clicks", { on }),
  setPreferences: (patch: PreferencesPatch) => invoke<Settings>("set_preferences", { patch }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (on: boolean) => invoke<boolean>("set_autostart", { on }),
  startCalibration: (deviceId: string) => invoke<void>("start_calibration", { deviceId }),
  openSoundSettings: () => invoke<void>("open_sound_settings"),
  showMainWindow: () => invoke<void>("show_main_window"),
  hideTrayPopup: () => invoke<void>("hide_tray_popup"),
  quit: () => invoke<void>("quit_app"),
};

export async function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!inShell) return mockListen(event, (p) => handler(p as T));
  try {
    return await listen<T>(event, (e) => handler(e.payload));
  } catch {
    return () => {};
  }
}

/** Delays sit on top of each output's minimum path, so 0 means "as early as it can play". */
export const MIN_DELAY_MS = 0;
export const MAX_DELAY_MS = 2000;

/** The delay an output uses with the given source: remembered per source, else its default. */
export function delayFor(output: OutputSetting, sourceId: string | null | undefined): number {
  return (sourceId != null ? output.delays[sourceId] : undefined) ?? output.defaultDelayMs;
}

/* eslint-disable @typescript-eslint/no-explicit-any */
/*
 * Browser-preview backend: when the UI runs outside the desktop shell (`npm run dev` in a
 * browser), commands and events are served from this in-memory model with animated levels,
 * so the interface can be designed and reviewed without Windows audio.
 */

import type { CalibrationEvent, Devices, Settings, Snapshot } from "./ipc";

type Handler = (payload: unknown) => void;
const listeners = new Map<string, Set<Handler>>();

const devices: Devices = {
  outputs: [
    { id: "yamaha", name: "Yamaha R-N303D (Bluetooth)", isDefault: true },
    { id: "ka6", name: "Main Output (Komplete Audio 6)", isDefault: false },
    { id: "hdmi", name: "Acer KG241Q (HDMI)", isDefault: false },
    { id: "phones", name: "Headphones (Komplete Audio 6)", isDefault: false },
  ],
  inputs: [
    { id: "mic", name: "Microphone (USB Audio)", isDefault: true },
    { id: "ka6in", name: "Input 1/2 (Komplete Audio 6)", isDefault: false },
  ],
};

const settings: Settings = {
  enabled: true,
  source: { followDefault: true, device: null },
  outputs: [
    { device: { id: "ka6", name: devices.outputs[1].name }, enabled: true, gainDb: 0, defaultDelayMs: 214, delays: { yamaha: 214 } },
    { device: { id: "hdmi", name: devices.outputs[2].name }, enabled: false, gainDb: -3, defaultDelayMs: 180, delays: {} },
  ],
  mic: { id: "mic", name: "Microphone (USB Audio)" },
  resyncOnReconnect: true,
  syncReference: null,
  followSourceVolume: true,
  pauseWhenSilent: false,
  theme: "system",
  locale: (new URLSearchParams(location.search).get("lang") as "en" | "fr") ?? "en",
  closeToTray: true,
  startMinimized: true,
};
if (settings.locale !== "fr") settings.locale = "en";

let clicks = false;
let calibrating: string | null = null;
let autostart = true;

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((h) => h(payload));
}

function snapshot(): Snapshot {
  const now = performance.now() / 1000;
  const level = settings.enabled ? 0.25 + 0.2 * Math.abs(Math.sin(now * 2.3)) + 0.1 * Math.random() : 0;
  return {
    enabled: settings.enabled,
    source: {
      state: settings.enabled ? "running" : "off",
      deviceId: "yamaha",
      deviceName: devices.outputs[0].name,
      level,
      error: null,
      errorKind: null,
      sampleRate: settings.enabled ? 48000 : null,
      hardwareVolume: true,
    },
    outputs: settings.outputs.map((o) => {
      const delay = o.delays.yamaha ?? o.defaultDelayMs;
      const playing = settings.enabled && o.enabled;
      return {
        deviceId: o.device.id,
        state: !o.enabled ? "disabled" : settings.enabled ? "playing" : "idle",
        delayMs: delay,
        bufferedMs: playing ? delay + 0.2 * Math.sin(now) : 0,
        correctionPpm: playing ? -7 + 2 * Math.sin(now / 3) : 0,
        underruns: 0,
        level: playing ? level * Math.pow(10, o.gainDb / 20) : 0,
        error: null,
        errorKind: null,
        minLagMs: playing ? 24.3 : null,
        sampleRate: playing ? (o.device.id === "hdmi" ? 44100 : 48000) : null,
      };
    }),
    clicks,
    calibrating,
  };
}

function changed() {
  emit("settings", structuredClone(settings));
}

function output(id: string) {
  return settings.outputs.find((o) => o.device.id === id);
}

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const a = args as any;
  const r = ((): unknown => {
    switch (cmd) {
      case "get_settings":
        return structuredClone(settings);
      case "get_status":
        return snapshot();
      case "list_devices":
        return devices;
      case "get_autostart":
        return autostart;
      case "set_autostart":
        autostart = a.on;
        return autostart;
      case "set_enabled":
        settings.enabled = a.enabled;
        break;
      case "set_source":
        settings.source = { followDefault: a.followDefault, device: a.device ?? settings.source.device };
        break;
      case "add_output":
        if (!output(a.device.id))
          settings.outputs.push({ device: a.device, enabled: true, gainDb: 0, defaultDelayMs: 200, delays: {} });
        break;
      case "remove_output":
        settings.outputs = settings.outputs.filter((o) => o.device.id !== a.deviceId);
        break;
      case "restore_output":
        if (!output(a.output.device.id)) settings.outputs.push(a.output);
        break;
      case "set_output_enabled":
        output(a.deviceId)!.enabled = a.enabled;
        break;
      case "set_output_delay": {
        const o = output(a.deviceId)!;
        o.delays.yamaha = a.delayMs;
        o.defaultDelayMs = a.delayMs;
        break;
      }
      case "set_output_gain":
        output(a.deviceId)!.gainDb = Math.max(-40, Math.min(12, a.gainDb));
        break;
      case "open_sound_settings":
        return undefined;
      case "set_clicks":
        clicks = a.on;
        return undefined;
      case "set_preferences":
        Object.assign(settings, a.patch);
        changed();
        return structuredClone(settings);
      case "start_calibration": {
        calibrating = a.deviceId;
        const o = output(a.deviceId)!;
        const before = o.delays.yamaha ?? o.defaultDelayMs;
        const base = { deviceId: a.deviceId, automatic: false, offsetMs: null, previousDelayMs: before, delayMs: null, lagMs: null, reference: settings.syncReference, referenceDelayMs: null, failure: null };
        emit("calibration", { ...base, phase: "listening", pass: 1 } satisfies CalibrationEvent);
        setTimeout(() => emit("calibration", { ...base, phase: "listening", pass: 2 }), 2500);
        setTimeout(() => {
          calibrating = null;
          // Demo both outcomes: a normal correction, or (at 0 ms already) a limited one.
          if (before === 0) {
            emit("calibration", { ...base, phase: "limited", pass: 1, offsetMs: 8.4, delayMs: 0, lagMs: 8.4 });
          } else {
            o.delays.yamaha = before - 12;
            changed();
            emit("calibration", { ...base, phase: "done", pass: 2, offsetMs: 12.3, delayMs: before - 12 });
          }
        }, 5000);
        return undefined;
      }
      default:
        return undefined;
    }
    changed();
    return undefined;
  })();
  return r as T;
}

export function mockListen(event: string, handler: Handler): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(handler);
  return () => listeners.get(event)?.delete(handler);
}

setInterval(() => emit("status", snapshot()), 120);

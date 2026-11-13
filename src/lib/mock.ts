/* eslint-disable @typescript-eslint/no-explicit-any */
/*
 * Simulated backend: when the UI runs outside the desktop shell (`npm run dev` in a browser, or
 * the demo build), commands and events are served from this in-memory model with animated
 * levels, so the interface can be designed, reviewed and shown without Windows audio.
 *
 * `ipc.ts` loads this module on demand, and only there. It must never be imported statically:
 * that would put it (and the timer at the bottom) back into the app that ships.
 *
 * Page switches (all optional, combinable):
 *
 *   ?lang=fr                 French
 *   ?theme=dark | light      force a theme (default: follow the system)
 *   ?view=outputs|sync|settings   open on that screen
 *   ?sync=<state>            open the Sync screen in one state of auto-sync (see SYNC_STATES)
 *   ?outputs=<state>         open the Outputs screen in one state (see OUTPUT_STATES)
 *   ?autostart=fail          Windows refuses the launch-at-login switch
 *   ?firstRun=1              a new installation: no output, the guide at its first step
 *   ?firstRun=noDevices      ... on a machine with no other playback device
 *   ?firstRun=noMic          the guide on a machine with no microphone (outputs already added)
 *   ?guide=<0..4>            the guide at that step (with ?firstRun=, or with the usual outputs)
 *   ?autorun=1               start an auto-sync by itself shortly after loading
 *   ?update=1.2.3 | fail     an update is available / the update check fails
 */

import type {
  CalibrationEvent,
  Devices,
  ErrorKind,
  Failure,
  OutputStatus,
  Settings,
  Snapshot,
  SyncStep,
} from "./ipc";
import type { FoundUpdate } from "./updater";
import { en } from "../i18n/en";
import { fr } from "../i18n/fr";

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

const params = new URLSearchParams(location.search);
const wantedTheme = params.get("theme");

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
  autoCheckUpdates: true,
  skippedUpdate: null,
  theme: wantedTheme === "dark" || wantedTheme === "light" ? wantedTheme : "system",
  locale: params.get("lang") === "fr" ? "fr" : "en",
  closeToTray: true,
  startMinimized: true,
  guideStep: null,
};

let clicks = false;
let calibrating: string | null = null;
let updateNotice: string | null = null;
let autostart = true;

// -- States on demand --------------------------------------------------------------------

/** The output every simulated auto-sync state is about, and the one it can be lined up with. */
const TARGET = "ka6";
const OTHER = "hdmi";
const DELAY_BEFORE = 214;

const FAILURES: Failure[] = [
  "noMic",
  "micMissing",
  "micFailed",
  "notRunning",
  "referenceNotRunning",
  "interrupted",
  "otherAudio",
  "sourceNotHeard",
  "targetNotHeard",
  "tooShort",
  "busy",
];

/**
 * Every state `?sync=` understands: a running pass at each step (with the source, then with
 * another output as the reference), each way a run can end, each failure, and each reason the
 * Run button can be unavailable.
 */
export const SYNC_STATES = [
  "preparing",
  "first",
  "second",
  "analysing",
  "pass2",
  "refFirst",
  "refSecond",
  "later",
  "earlier",
  "same",
  "referenceMoved",
  "limited",
  "automatic",
  ...FAILURES,
  "blockedNoMic",
  "blockedMicMissing",
  "blockedNotPlaying",
  "blockedBusy",
] as const;

/** Every state `?outputs=` understands. */
export const OUTPUT_STATES = [
  "sourceWaiting",
  "sourceInUse",
  "sourceInvalidated",
  "sourceStalled",
  "sourceOther",
  "outputInUse",
  "outputInvalidated",
  "outputStalled",
  "outputOther",
  "outputWaiting",
  "isSource",
  "dropouts",
  "empty",
  "autoResynced",
  "autoFailed",
] as const;

const syncState = params.get("sync");
const outputsState = params.get("outputs");

function output(id: string) {
  return settings.outputs.find((o) => o.device.id === id);
}

/** States that involve a "Line up with" output need one that is on. */
const withReference = ["refFirst", "refSecond", "referenceMoved", "referenceNotRunning"].includes(syncState ?? "");
if (withReference || syncState === "blockedBusy") output(OTHER)!.enabled = true;
if (withReference) settings.syncReference = OTHER;
if (syncState === "blockedNoMic") settings.mic = null;
if (syncState === "blockedMicMissing") devices.inputs = [];
if (syncState === "blockedNotPlaying") output(TARGET)!.enabled = false;
if (syncState === "blockedBusy") calibrating = OTHER;
if (["preparing", "first", "second", "analysing", "pass2", "refFirst", "refSecond"].includes(syncState ?? "")) {
  calibrating = TARGET;
}
if (outputsState === "empty") settings.outputs = [];

// -- A new installation, and the guide -----------------------------------------------------
const firstRun = params.get("firstRun");
const guideAt = params.get("guide");
if (firstRun === "1" || firstRun === "noDevices") {
  settings.outputs = [];
  settings.mic = null;
  settings.guideStep = 0;
}
if (firstRun === "noDevices") devices.outputs = devices.outputs.slice(0, 1);
if (firstRun === "noMic") {
  devices.inputs = [];
  settings.mic = null;
  settings.guideStep = 3;
}
if (guideAt != null) settings.guideStep = Math.max(0, Math.min(4, Number(guideAt) || 0));

function setDelay(id: string, ms: number) {
  const o = output(id);
  if (!o) return;
  o.delays.yamaha = ms;
  o.defaultDelayMs = ms;
}

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((h) => h(payload));
}

function changed() {
  emit("settings", structuredClone(settings));
}

/** One auto-sync event for the target output, from the few fields that vary. */
function calibration(patch: Partial<CalibrationEvent>): CalibrationEvent {
  return {
    deviceId: TARGET,
    phase: "listening",
    pass: 1,
    step: null,
    automatic: false,
    offsetMs: null,
    previousDelayMs: DELAY_BEFORE,
    delayMs: null,
    lagMs: null,
    reference: settings.syncReference,
    referenceDelayMs: null,
    failure: null,
    ...patch,
  };
}

/** How a simulated run ends, by name. Also moves the delay it reports. */
function ending(name: string | null, before: number): CalibrationEvent {
  const done = (delayMs: number, more: Partial<CalibrationEvent> = {}) => {
    setDelay(TARGET, delayMs);
    return calibration({ phase: "done", pass: 2, previousDelayMs: before, delayMs, offsetMs: 0.4, ...more });
  };
  if (name && (FAILURES as string[]).includes(name)) {
    return calibration({
      phase: "failed",
      failure: name as Failure,
      previousDelayMs: before,
      // Only an automatic run gives up because of another program: a run by hand mutes it.
      automatic: name === "otherAudio",
    });
  }
  switch (name) {
    case "later":
      return done(before + 12);
    case "same":
      return done(before, { pass: 1 });
    case "automatic":
      return done(before + 7, { automatic: true });
    case "referenceMoved":
      setDelay(OTHER, 31);
      return done(0, { previousDelayMs: 9, referenceDelayMs: 31 });
    case "limited":
      setDelay(TARGET, 0);
      return calibration({ phase: "limited", previousDelayMs: before, delayMs: 0, lagMs: 8.4, offsetMs: 8.4 });
    default:
      // "earlier", and what a run started by hand in the preview does.
      return done(Math.max(0, before - 12));
  }
}

/** The event a `?sync=` state opens on, if it shows one. */
function openingEvent(): CalibrationEvent | null {
  const step = (s: SyncStep | null, pass = 1) => calibration({ pass, step: s });
  switch (syncState) {
    case null:
    case "blockedNoMic":
    case "blockedMicMissing":
    case "blockedNotPlaying":
    case "blockedBusy":
      return null;
    case "preparing":
      return step(null);
    case "first":
    case "refFirst":
      return step("first");
    case "second":
    case "refSecond":
      return step("second");
    case "analysing":
      return step("analysing");
    case "pass2":
      return step("first", 2);
    default:
      return ending(syncState, DELAY_BEFORE);
  }
}

const SOURCE_ERRORS: Record<string, ErrorKind> = {
  sourceInUse: "inUse",
  sourceInvalidated: "invalidated",
  sourceStalled: "stalled",
  sourceOther: "other",
};
const OUTPUT_ERRORS: Record<string, ErrorKind> = {
  outputInUse: "inUse",
  outputInvalidated: "invalidated",
  outputStalled: "stalled",
  outputOther: "other",
};
const RAW_ERROR = "opening the device: AUDCLNT_E_DEVICE_IN_USE (0x8889000A) (preview)";

function snapshot(): Snapshot {
  const now = performance.now() / 1000;
  const sourceError = SOURCE_ERRORS[outputsState ?? ""];
  const sourceWaiting = outputsState === "sourceWaiting";
  const running = settings.enabled && !sourceError && !sourceWaiting;
  const level = running ? 0.25 + 0.2 * Math.abs(Math.sin(now * 2.3)) + 0.1 * Math.random() : 0;
  return {
    enabled: settings.enabled,
    source: {
      state: !settings.enabled ? "off" : sourceError ? "error" : sourceWaiting ? "waiting" : "running",
      deviceId: "yamaha",
      deviceName: devices.outputs[0].name,
      level,
      error: sourceError ? RAW_ERROR : null,
      errorKind: sourceError ?? null,
      sampleRate: running ? 48000 : null,
      hardwareVolume: true,
    },
    outputs: settings.outputs.map((o): OutputStatus => {
      const delay = o.delays.yamaha ?? o.defaultDelayMs;
      const target = o.device.id === TARGET;
      const error = target ? OUTPUT_ERRORS[outputsState ?? ""] : undefined;
      const state: OutputStatus["state"] = !o.enabled
        ? "disabled"
        : !running
          ? "idle"
          : error
            ? "error"
            : target && outputsState === "outputWaiting"
              ? "waiting"
              : target && outputsState === "isSource"
                ? "isSource"
                : "playing";
      const playing = state === "playing";
      return {
        deviceId: o.device.id,
        state,
        delayMs: delay,
        bufferedMs: playing ? delay + 0.2 * Math.sin(now) : 0,
        correctionPpm: playing ? -7 + 2 * Math.sin(now / 3) : 0,
        underruns: target && outputsState === "dropouts" ? 3 : 0,
        level: playing ? level * Math.pow(10, o.gainDb / 20) : 0,
        error: error ? RAW_ERROR : null,
        errorKind: error ?? null,
        minLagMs: playing ? 24.3 : null,
        sampleRate: playing ? (o.device.id === "hdmi" ? 44100 : 48000) : null,
      };
    }),
    clicks,
    calibrating,
    update: updateNotice,
  };
}

/** A whole simulated run: both passes, step by step, then the ending `?sync=` names. */
function simulateRun(deviceId: string) {
  calibrating = deviceId;
  const before = output(deviceId)?.delays.yamaha ?? output(deviceId)?.defaultDelayMs ?? DELAY_BEFORE;
  const steps: [number, number, SyncStep | null][] = [
    [0, 1, null],
    [500, 1, "first"],
    [1900, 1, "second"],
    [3300, 1, "analysing"],
    [3900, 2, null],
    [4300, 2, "first"],
    [5500, 2, "second"],
    [6700, 2, "analysing"],
  ];
  const failing = syncState != null && (FAILURES as string[]).includes(syncState);
  // A failure shows at the end of the first pass, as the real ones do.
  const end = failing ? 3900 : 7300;
  for (const [at, pass, step] of steps) {
    if (at < end) setTimeout(() => emit("calibration", { ...calibration({ pass, step }), deviceId }), at);
  }
  setTimeout(() => {
    calibrating = null;
    // At 0 ms already, a run can only report what is left: the "limited" ending.
    const last = ending(before === 0 ? "limited" : syncState, before);
    changed();
    emit("calibration", { ...last, deviceId });
  }, end);
}

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const a = args as any;
  if (cmd === "set_autostart" && params.get("autostart") === "fail") {
    throw new Error("the launch-at-login entry could not be changed (preview)");
  }
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
      case "set_output_delay":
        setDelay(a.deviceId, a.delayMs);
        break;
      case "set_output_gain":
        output(a.deviceId)!.gainDb = Math.max(-40, Math.min(12, a.gainDb));
        break;
      case "open_sound_settings":
        return undefined;
      case "set_update_notice":
        updateNotice = a.version ?? null;
        return undefined;
      case "set_clicks":
        clicks = a.on;
        return undefined;
      case "set_preferences":
        Object.assign(settings, a.patch);
        changed();
        return structuredClone(settings);
      case "start_calibration":
        simulateRun(a.deviceId);
        return undefined;
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

type Screen = "outputs" | "sync" | "settings" | "guide";

/**
 * Called by the store once it listens to every event: opens the screen the page asked for and
 * puts it in the state it asked for.
 */
export function previewReady(apply: (patch: { view?: Screen; syncTarget?: string }) => void): void {
  const asked = params.get("view");
  const view: Screen | null =
    asked === "outputs" || asked === "sync" || asked === "settings" || asked === "guide"
      ? asked
      : settings.guideStep != null
        ? "guide"
        : syncState
          ? "sync"
          : outputsState
            ? "outputs"
            : null;
  if (view) apply({ view, syncTarget: TARGET });

  const opening = openingEvent();
  if (opening) {
    changed();
    emit("calibration", opening);
  }
  if (outputsState === "autoResynced") {
    emit("calibration", ending("automatic", DELAY_BEFORE));
    changed();
  }
  if (outputsState === "autoFailed") {
    emit("calibration", { ...ending("otherAudio", DELAY_BEFORE), automatic: true });
  }
  if (params.get("autorun")) setTimeout(() => simulateRun(TARGET), 700);

  // In a small window the part a `?sync=` state is about can sit below the fold: bring it
  // into view, as someone looking for it would.
  if (syncState) {
    setTimeout(() => {
      const part = document.querySelector("[data-sync-outcome]") ?? document.querySelector("[data-sync-run]");
      part?.scrollIntoView({ block: "center" });
    }, 300);
  }
  if (outputsState) {
    setTimeout(() => {
      const notices = document.querySelectorAll("[data-notice]");
      notices[notices.length - 1]?.scrollIntoView({ block: "center" });
    }, 300);
  }
  // The launch-at-login warning only shows after an attempt: make one.
  if (params.get("autostart") === "fail") {
    setTimeout(() => {
      const label = AUTOSTART_LABEL[settings.locale];
      document.querySelector<HTMLElement>(`button[role="switch"][aria-label="${label}"]`)?.click();
    }, 300);
    setTimeout(() => document.querySelector('[data-notice="warn"]')?.scrollIntoView({ block: "center" }), 700);
  }
}

/** The launch-at-login switch's label, to find it on the Settings screen. */
const AUTOSTART_LABEL: Record<Settings["locale"], string> = {
  en: en.settings.autostart,
  fr: fr.settings.autostart,
};

/*
 * Made-up release notes for the preview's update card. Real notes come from the published
 * release; these follow the preview's language so a French demo reads as one piece. They are
 * sample data of the simulation, which is why they live here and not in the i18n dictionaries
 * (those ship with the app).
 */
const PREVIEW_NOTES: Record<Settings["locale"], string> = {
  en: "- Faster reconnection after a Bluetooth drop\n- Fixes a click when an output starts\n- French wording tweaks",
  fr: "- Reconnexion plus rapide après une coupure Bluetooth\n- Corrige un clic au démarrage d'une sortie\n- Retouches du texte français",
};

/**
 * The update check of the preview, answered from the page's `?update=` parameter:
 * `?update=1.2.3` offers that version, `?update=fail` fails the check, nothing means up to date.
 */
export async function previewUpdate(): Promise<FoundUpdate | null> {
  const wanted = params.get("update");
  await new Promise((r) => setTimeout(r, 600));
  if (wanted === "fail") throw new Error("release feed unreachable (preview)");
  if (!wanted) return null;
  return {
    version: wanted,
    notes: PREVIEW_NOTES[settings.locale],
    install: async (onProgress) => {
      for (let i = 0; i <= 20; i++) {
        onProgress(i / 20);
        await new Promise((r) => setTimeout(r, 150));
      }
    },
    release: () => {},
  };
}

setInterval(() => emit("status", snapshot()), 120);

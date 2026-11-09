/*
 * Derived, display-ready state shared by the title bar, the outputs view and the tray.
 */

import type { Translate } from "../i18n";
import type { OutputState, Snapshot } from "./ipc";
import type { Tone } from "../components/common/Controls";

/** One line for the whole app: "2 outputs in sync", "Waiting for Yamaha", "Paused". */
export function summarize(status: Snapshot | null, t: Translate): { tone: Tone; text: string } {
  if (!status) return { tone: "idle", text: "" };
  const { source } = status;
  switch (source.state) {
    case "off":
      return { tone: "idle", text: t((d) => d.summary.off) };
    case "waiting":
      return {
        tone: "warn",
        text: source.deviceName
          ? t((d) => d.summary.waiting, { name: source.deviceName })
          : t((d) => d.summary.waitingAny),
      };
    case "error":
      return { tone: "danger", text: t((d) => d.summary.error) };
    case "running": {
      const n = status.outputs.filter((o) => o.state === "playing").length;
      if (n === 0) {
        const buffering = status.outputs.some((o) => o.state === "buffering");
        return { tone: buffering ? "busy" : "idle", text: t((d) => d.summary.noOutput) };
      }
      return { tone: "good", text: t((d) => d.summary.inSync, { n }) };
    }
  }
}

export function outputTone(state: OutputState | undefined): Tone {
  switch (state) {
    case "playing":
      return "good";
    case "buffering":
      return "busy";
    case "waiting":
      return "warn";
    case "error":
      return "danger";
    default:
      return "idle";
  }
}

export function formatPpm(ppm: number): string {
  const r = Math.round(ppm);
  return `${r > 0 ? "+" : ""}${r}`;
}

export function formatDb(db: number): string {
  const r = Math.round(db * 10) / 10;
  return `${r > 0 ? "+" : ""}${r.toFixed(1)}`;
}

/** 48000 -> "48 kHz", 44100 -> "44.1 kHz" (with a decimal comma in French). */
export function formatRate(hz: number, locale: string): string {
  const khz = hz / 1000;
  return `${khz.toLocaleString(locale, { maximumFractionDigits: 1 })} kHz`;
}

export function formatMs(ms: number, locale: string): string {
  return ms.toLocaleString(locale, { maximumFractionDigits: 0 });
}

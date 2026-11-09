/*
 * The updater and relaunch calls. The plugins load lazily, so their code stays out of the first
 * chunk. Outside the desktop shell (the browser preview) a check answers from the page's
 * `?update=` parameter, so the prompt can be designed and reviewed without a real release:
 * `?update=1.2.3` offers that version, `?update=fail` fails the check, nothing means up to date.
 */

import { inShell } from "./ipc";

/** A found update: what to show, and how to install or drop it. */
export type FoundUpdate = {
  version: string;
  notes: string;
  /** Downloads and installs. `onProgress` gets 0..1, or null while the size is unknown. */
  install: (onProgress: (progress: number | null) => void) => Promise<void>;
  /** Releases an update the user did not install. */
  release: () => void;
};

/** Asks the release feed for a newer build: the update, or null when this build is current. */
export async function checkForUpdate(): Promise<FoundUpdate | null> {
  if (!inShell) return previewUpdate();
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    notes: update.body ?? "",
    install: async (onProgress) => {
      let total: number | null = null;
      let received = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
          onProgress(total ? 0 : null);
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          onProgress(total ? Math.min(1, received / total) : null);
        } else {
          onProgress(1);
        }
      });
    },
    release: () => void update.close().catch(() => {}),
  };
}

/** Restarts the app onto the installed build. On Windows the installer usually does it first. */
export async function relaunchApp(): Promise<void> {
  if (!inShell) return;
  try {
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch {
    /* the installer already restarts the app */
  }
}

async function previewUpdate(): Promise<FoundUpdate | null> {
  const wanted = new URLSearchParams(location.search).get("update");
  await new Promise((r) => setTimeout(r, 600));
  if (wanted === "fail") throw new Error("release feed unreachable (preview)");
  if (!wanted) return null;
  return {
    version: wanted,
    notes: "- Faster reconnection after a Bluetooth drop\n- Fixes a click when an output starts\n- French wording tweaks",
    install: async (onProgress) => {
      for (let i = 0; i <= 20; i++) {
        onProgress(i / 20);
        await new Promise((r) => setTimeout(r, 150));
      }
    },
    release: () => {},
  };
}

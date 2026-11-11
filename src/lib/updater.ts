/*
 * The updater and relaunch calls. The plugins load lazily, so their code stays out of the first
 * chunk. Outside the desktop shell a check is answered by the simulated backend (`mock.ts`, in
 * the browser preview and the demo build only) from the page's `?update=` parameter, so the
 * prompt can be designed and reviewed without a real release.
 */

import { inShell, simulated } from "./ipc";

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
  if (!inShell) {
    if (!simulated) throw new Error("no release feed: not running in the desktop shell");
    return (await simulated).previewUpdate();
  }
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

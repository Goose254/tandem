/*
 * Window controls, guarded so the UI still renders in a plain browser preview where the
 * desktop runtime is missing.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Window } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";

function appWindow(): Window | null {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

export function minimizeWindow(): void {
  void appWindow()?.minimize();
}

export function toggleMaximizeWindow(): void {
  void appWindow()?.toggleMaximize();
}

/** Close the main window: the Rust side decides between hiding to the tray and quitting. */
export function closeWindow(): void {
  void appWindow()?.close();
}

export async function isWindowMaximized(): Promise<boolean> {
  try {
    return (await appWindow()?.isMaximized()) ?? false;
  } catch {
    return false;
  }
}

export async function onWindowResized(handler: () => void): Promise<UnlistenFn> {
  const w = appWindow();
  if (!w) return () => {};
  try {
    return await w.onResized(() => handler());
  } catch {
    return () => {};
  }
}

import { openUrl } from "@tauri-apps/plugin-opener";
import { inShell } from "./ipc";

/** Where Tandem's source code lives (the AGPL asks that users can find it). */
export const REPO_URL = "https://github.com/Goose254/tandem";

/** Open a link in the default browser: through the shell in the app, a new tab in the preview. */
export function openExternal(url: string): void {
  if (inShell) void openUrl(url).catch(() => {});
  else window.open(url, "_blank", "noopener");
}

import { useEffect } from "react";
import { TitleBar } from "./components/shell/TitleBar";
import { Sidebar } from "./components/shell/Sidebar";
import { OutputsView } from "./components/outputs/OutputsView";
import { SyncView } from "./components/sync/SyncView";
import { SettingsView } from "./components/settings/SettingsView";
import { GuideView } from "./components/guide/GuideView";
import { SyncSpinner } from "./components/common/Layout";
import { startSync, useStore } from "./state/store";
import { useApplyTheme } from "./lib/theme";
import { UpdatePrompt } from "./components/updater/UpdatePrompt";
import { useAutoUpdateCheck, useUpdater } from "./state/updater";
import { on } from "./lib/ipc";
import styles from "./App.module.css";

export default function App() {
  const ready = useStore((s) => s.ready);
  const view = useStore((s) => s.view);
  useApplyTheme();
  useAutoUpdateCheck();

  useEffect(() => {
    void startSync();
    // The tray popup's "Update…" asks the main window to show its prompt.
    const stop = on("show-update", () => useUpdater.getState().openPrompt());
    // ... and its "Finish setting up" asks for the guide.
    const stopGuide = on("show-guide", () => useStore.getState().setView("guide"));
    return () => {
      void stop.then((off) => off());
      void stopGuide.then((off) => off());
    };
  }, []);

  return (
    <div className={styles.frame}>
      <TitleBar />
      <div className={styles.shell}>
        <Sidebar />
        <main className={styles.main}>
          {!ready ? (
            <div className={styles.loading}>
              <SyncSpinner size={32} />
            </div>
          ) : (
            <div className={styles.view} key={view}>
              {view === "outputs" ? <OutputsView /> : null}
              {view === "sync" ? <SyncView /> : null}
              {view === "settings" ? <SettingsView /> : null}
              {view === "guide" ? <GuideView /> : null}
            </div>
          )}
        </main>
      </div>
      <UpdatePrompt />
    </div>
  );
}

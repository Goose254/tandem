import { useEffect } from "react";
import { TitleBar } from "./components/shell/TitleBar";
import { Sidebar } from "./components/shell/Sidebar";
import { OutputsView } from "./components/outputs/OutputsView";
import { SyncView } from "./components/sync/SyncView";
import { SettingsView } from "./components/settings/SettingsView";
import { SyncSpinner } from "./components/common/Layout";
import { startSync, useStore } from "./state/store";
import { useApplyTheme } from "./lib/theme";
import styles from "./App.module.css";

export default function App() {
  const ready = useStore((s) => s.ready);
  const view = useStore((s) => s.view);
  useApplyTheme();

  useEffect(() => {
    void startSync();
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
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

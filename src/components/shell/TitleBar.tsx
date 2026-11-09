import { useEffect, useState } from "react";
import { Copy, Minus, Square, X } from "lucide-react";
import { Logo } from "../common/Layout";
import { StatusDot } from "../common/Controls";
import { closeWindow, isWindowMaximized, minimizeWindow, onWindowResized, toggleMaximizeWindow } from "../../lib/appWindow";
import { summarize } from "../../lib/summary";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";
import styles from "./TitleBar.module.css";

/**
 * The custom window chrome: brand and live status on the left, a drag surface in the middle,
 * minimize / maximize / close on the right. Ambient ground, no divider.
 */
export function TitleBar() {
  const t = useT();
  const status = useStore((s) => s.status);
  const [maximized, setMaximized] = useState(false);
  const summary = summarize(status, t);

  useEffect(() => {
    let alive = true;
    let unlisten = () => {};
    const sync = () => void isWindowMaximized().then((v) => alive && setMaximized(v));
    sync();
    void onWindowResized(sync).then((fn) => (alive ? (unlisten = fn) : fn()));
    return () => {
      alive = false;
      unlisten();
    };
  }, []);

  return (
    <div className={styles.bar} data-tauri-drag-region>
      <div className={styles.brand} data-tauri-drag-region>
        <Logo size={26} />
        <span className={styles.name} data-tauri-drag-region>
          {t((d) => d.app.name)}
        </span>
      </div>
      {summary.text ? (
        <div className={styles.status} data-tauri-drag-region>
          <StatusDot tone={summary.tone} />
          <span data-tauri-drag-region>{summary.text}</span>
        </div>
      ) : null}
      <div className={styles.controls}>
        <button type="button" className={styles.control} onClick={minimizeWindow} aria-label={t((d) => d.window.minimize)}>
          <Minus size={16} strokeWidth={1.3} />
        </button>
        <button
          type="button"
          className={styles.control}
          onClick={toggleMaximizeWindow}
          aria-label={maximized ? t((d) => d.window.restore) : t((d) => d.window.maximize)}
        >
          {maximized ? <Copy size={15} strokeWidth={1.3} /> : <Square size={14} strokeWidth={1.3} />}
        </button>
        <button
          type="button"
          className={`${styles.control} ${styles.close}`}
          onClick={closeWindow}
          aria-label={t((d) => d.window.close)}
        >
          <X size={16} strokeWidth={1.3} />
        </button>
      </div>
    </div>
  );
}

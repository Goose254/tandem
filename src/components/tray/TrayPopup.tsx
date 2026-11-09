import { useEffect } from "react";
import { Logo } from "../common/Layout";
import { PrimaryButton, QuietButton } from "../common/Buttons";
import { Meter, StatusDot, Switch } from "../common/Controls";
import { delayFor, ipc } from "../../lib/ipc";
import { outputTone, summarize } from "../../lib/summary";
import { startSync, useStore } from "../../state/store";
import { useApplyTheme } from "../../lib/theme";
import { useT } from "../../i18n";
import styles from "./TrayPopup.module.css";

/**
 * The tray popup: a small solid card in its own window. The master switch, a one-line summary,
 * the source level, every output with its state and delay, then Open and Quit.
 */
export function TrayPopup() {
  const t = useT();
  const settings = useStore((s) => s.settings);
  const status = useStore((s) => s.status);
  useApplyTheme();

  useEffect(() => {
    void startSync();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && void ipc.hideTrayPopup();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const summary = summarize(status, t);
  const enabled = settings?.enabled ?? false;

  return (
    <div className={styles.popup}>
      <div className={styles.head}>
        <Logo size={22} />
        <span className={styles.name}>{t((d) => d.app.name)}</span>
        {settings ? (
          <Switch checked={enabled} onChange={(on) => void ipc.setEnabled(on)} label={t((d) => d.outputs.mirroring)} />
        ) : null}
      </div>

      <div className={styles.summary}>
        <StatusDot tone={summary.tone} />
        <span>{summary.text}</span>
      </div>

      <div className={styles.source}>
        <span className={styles.sourceLabel}>
          {t((d) => d.tray.source)} · {status?.source.deviceName ?? "—"}
        </span>
        <Meter level={status?.source.level ?? 0} muted={status?.source.state !== "running"} />
      </div>

      <ul className={styles.outputs}>
        {settings && settings.outputs.length === 0 ? (
          <li className={styles.empty}>{t((d) => d.tray.noOutputs)}</li>
        ) : null}
        {settings?.outputs.map((o) => {
          const st = status?.outputs.find((s) => s.deviceId === o.device.id);
          return (
            <li key={o.device.id} className={styles.output}>
              <StatusDot tone={outputTone(st?.state)} />
              <span className={styles.outputName} title={o.device.name}>
                {o.device.name}
              </span>
              <span className={`${styles.delay} tabular`}>
                {delayFor(o, status?.source.deviceId)} {t((d) => d.units.ms)}
              </span>
            </li>
          );
        })}
      </ul>

      <div className={styles.actions}>
        <PrimaryButton onClick={() => void ipc.showMainWindow()}>{t((d) => d.tray.open)}</PrimaryButton>
        <QuietButton onClick={() => void ipc.quit()}>{t((d) => d.tray.quit)}</QuietButton>
      </div>
    </div>
  );
}

import type { ReactNode } from "react";
import { AudioWaveform, Compass, Settings, Speaker } from "lucide-react";
import { Meter, Switch } from "../common/Controls";
import { ipc } from "../../lib/ipc";
import { useStore } from "../../state/store";
import type { View } from "../../state/store";
import { useT } from "../../i18n";
import styles from "./Sidebar.module.css";

function NavItem({
  icon,
  label,
  count,
  active,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`${styles.item} ${active ? styles.active : ""}`}
      aria-current={active ? "page" : undefined}
      onClick={onClick}
    >
      {icon}
      <span className={styles.txt}>{label}</span>
      {count != null ? <span className={styles.count}>{count}</span> : null}
    </button>
  );
}

/** Transparent rail on the ambient ground: nav, then the mirroring switch pinned at the foot. */
export function Sidebar() {
  const t = useT();
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);
  const settings = useStore((s) => s.settings);
  const status = useStore((s) => s.status);
  const go = (v: View) => () => setView(v);
  const enabled = settings?.enabled ?? false;

  return (
    <nav className={styles.side}>
      <div className={styles.navgroup}>
        <div className={styles.navlabel}>{t((d) => d.nav.group)}</div>
        {/* The way back into the guide, for as long as it is pending or open. */}
        {settings?.guideStep != null || view === "guide" ? (
          <NavItem
            icon={<Compass size={17} />}
            label={t((d) => d.nav.guide)}
            active={view === "guide"}
            onClick={go("guide")}
          />
        ) : null}
        <NavItem
          icon={<Speaker size={17} />}
          label={t((d) => d.nav.outputs)}
          count={settings?.outputs.length}
          active={view === "outputs"}
          onClick={go("outputs")}
        />
        <NavItem
          icon={<AudioWaveform size={17} />}
          label={t((d) => d.nav.sync)}
          active={view === "sync"}
          onClick={go("sync")}
        />
      </div>
      <div className={styles.spacer} />
      <div className={styles.footgroup}>
        <div className={styles.master}>
          <div className={styles.masterText}>
            <span className={styles.masterLabel}>{t((d) => d.outputs.mirroring)}</span>
            <Meter level={status?.source.level ?? 0} muted={!enabled} />
          </div>
          {settings ? (
            <Switch checked={enabled} onChange={(on) => void ipc.setEnabled(on)} label={t((d) => d.outputs.mirroring)} />
          ) : null}
        </div>
        <NavItem
          icon={<Settings size={17} />}
          label={t((d) => d.nav.settings)}
          active={view === "settings"}
          onClick={go("settings")}
        />
      </div>
    </nav>
  );
}

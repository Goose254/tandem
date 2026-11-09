import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ExternalLink } from "lucide-react";
import { QuietButton } from "../common/Buttons";
import { openExternal, REPO_URL } from "../../lib/links";
import { Logo, Row, Section, ViewHeader } from "../common/Layout";
import { SegmentedControl, Switch } from "../common/Controls";
import { inShell, ipc } from "../../lib/ipc";
import type { Locale, ThemeChoice } from "../../lib/ipc";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";
import styles from "./SettingsView.module.css";

export function SettingsView() {
  const t = useT();
  const settings = useStore((s) => s.settings)!;
  const autostart = useStore((s) => s.autostart);
  const setAutostart = useStore((s) => s.setAutostart);
  const [version, setVersion] = useState("");

  useEffect(() => {
    if (inShell) void getVersion().then(setVersion).catch(() => setVersion("dev"));
    else setVersion("preview");
  }, []);

  return (
    <div className={styles.view}>
      <ViewHeader title={t((d) => d.settings.title)} sub={t((d) => d.settings.sub)} />
      <div className={styles.sections}>
        <Section label={t((d) => d.settings.appearance)}>
          <Row label={t((d) => d.settings.theme)}>
            <SegmentedControl<ThemeChoice>
              value={settings.theme}
              ariaLabel={t((d) => d.settings.theme)}
              onChange={(theme) => void ipc.setPreferences({ theme })}
              segments={[
                { value: "system", label: t((d) => d.settings.themeSystem) },
                { value: "light", label: t((d) => d.settings.themeLight) },
                { value: "dark", label: t((d) => d.settings.themeDark) },
              ]}
            />
          </Row>
          <Row label={t((d) => d.settings.language)}>
            <SegmentedControl<Locale>
              value={settings.locale}
              ariaLabel={t((d) => d.settings.language)}
              onChange={(locale) => void ipc.setPreferences({ locale })}
              segments={[
                { value: "en", label: "English" },
                { value: "fr", label: "Français" },
              ]}
            />
          </Row>
        </Section>

        <Section label={t((d) => d.settings.audio)}>
          <Row label={t((d) => d.settings.followVolume)} helper={t((d) => d.settings.followVolumeHelp)}>
            <Switch
              checked={settings.followSourceVolume}
              label={t((d) => d.settings.followVolume)}
              onChange={(on) => void ipc.setPreferences({ followSourceVolume: on })}
            />
          </Row>
          <Row label={t((d) => d.settings.pauseSilent)} helper={t((d) => d.settings.pauseSilentHelp)}>
            <Switch
              checked={settings.pauseWhenSilent}
              label={t((d) => d.settings.pauseSilent)}
              onChange={(on) => void ipc.setPreferences({ pauseWhenSilent: on })}
            />
          </Row>
        </Section>

        <Section label={t((d) => d.settings.startup)}>
          <Row label={t((d) => d.settings.autostart)} helper={t((d) => d.settings.autostartHelp)}>
            <Switch
              checked={autostart}
              label={t((d) => d.settings.autostart)}
              onChange={(on) =>
                void ipc
                  .setAutostart(on)
                  .then(setAutostart)
                  .catch(() => {})
              }
            />
          </Row>
          <Row label={t((d) => d.settings.startMinimized)} helper={t((d) => d.settings.startMinimizedHelp)}>
            <Switch
              checked={settings.startMinimized}
              disabled={!autostart}
              label={t((d) => d.settings.startMinimized)}
              onChange={(on) => void ipc.setPreferences({ startMinimized: on })}
            />
          </Row>
          <Row label={t((d) => d.settings.closeToTray)} helper={t((d) => d.settings.closeToTrayHelp)}>
            <Switch
              checked={settings.closeToTray}
              label={t((d) => d.settings.closeToTray)}
              onChange={(on) => void ipc.setPreferences({ closeToTray: on })}
            />
          </Row>
        </Section>

        <Section label={t((d) => d.settings.about)}>
          <div className={styles.about}>
            <Logo size={40} />
            <div>
              <p className={styles.aboutName}>
                Tandem <span className="tabular">{t((d) => d.settings.version, { version })}</span>
              </p>
              <p className={styles.aboutLine}>{t((d) => d.settings.aboutLine)}</p>
              <p className={styles.aboutLine}>{t((d) => d.settings.license)}</p>
              <QuietButton className={styles.sourceLink} onClick={() => openExternal(REPO_URL)}>
                <ExternalLink size={14} />
                {t((d) => d.settings.source)}
              </QuietButton>
            </div>
          </div>
        </Section>
      </div>
    </div>
  );
}

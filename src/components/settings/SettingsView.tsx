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
import { updateOf, useUpdater } from "../../state/updater";
import styles from "./SettingsView.module.css";

export function SettingsView() {
  const t = useT();
  const settings = useStore((s) => s.settings)!;
  const autostart = useStore((s) => s.autostart);
  const setAutostart = useStore((s) => s.setAutostart);
  // Outside the desktop shell there is no app to ask: the preview says so from the first render.
  const [version, setVersion] = useState(inShell ? "" : "preview");

  useEffect(() => {
    if (inShell) void getVersion().then(setVersion).catch(() => setVersion("dev"));
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

        <UpdatesSection version={version} />

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

/**
 * Updates: the running version beside "Check for updates", one line for the last result, and
 * the automatic-check switch. A found update opens the prompt; the line keeps a link back to it.
 */
function UpdatesSection({ version }: { version: string }) {
  const t = useT();
  const settings = useStore((s) => s.settings)!;
  const state = useUpdater((s) => s.state);
  const check = useUpdater((s) => s.check);
  const openPrompt = useUpdater((s) => s.openPrompt);
  const update = updateOf(state);
  const working = state.kind === "checking" || state.kind === "downloading" || state.kind === "ready";

  return (
    <Section label={t((d) => d.updater.section)}>
      <Row
        label={<span className="tabular">{t((d) => d.updater.version, { version })}</span>}
        helper={
          state.kind === "checking"
            ? t((d) => d.updater.checking)
            : state.kind === "upToDate"
              ? t((d) => d.updater.upToDate)
              : undefined
        }
      >
        <QuietButton onClick={() => void check("manual")} disabled={working}>
          {t((d) => d.updater.check)}
        </QuietButton>
      </Row>
      {state.kind === "error" ? (
        <p className={styles.updateFailed}>
          {state.update
            ? t((d) => d.updater.installFailed, { reason: state.message })
            : t((d) => d.updater.checkFailed, { reason: state.message })}
        </p>
      ) : update ? (
        <p className={`${styles.updateLine} tabular`}>
          {settings.skippedUpdate === update.version
            ? t((d) => d.updater.skippedLine, { version: update.version })
            : t((d) => d.updater.availableLine, { version: update.version })}{" "}
          <button type="button" className={styles.updateShow} onClick={openPrompt}>
            {t((d) => d.updater.show)}
          </button>
        </p>
      ) : null}
      <Row label={t((d) => d.updater.autoCheck)} helper={t((d) => d.updater.autoCheckHelp)}>
        <Switch
          checked={settings.autoCheckUpdates}
          label={t((d) => d.updater.autoCheck)}
          onChange={(on) => void ipc.setPreferences({ autoCheckUpdates: on })}
        />
      </Row>
    </Section>
  );
}

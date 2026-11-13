import { useEffect } from "react";
import { Ear, Mic } from "lucide-react";
import { Card, EmptyState, Notice, Row, ViewHeader } from "../common/Layout";
import { PrimaryButton, QuietButton } from "../common/Buttons";
import { SegmentedControl, Select, Switch } from "../common/Controls";
import { DelayControl } from "../common/DelayControl";
import { delayFor, ipc } from "../../lib/ipc";
import { useStore } from "../../state/store";
import { useLocale, useT } from "../../i18n";
import { formatMs } from "../../lib/summary";
import { fixedOnOutputs, OutcomePanel, outcomeText, outcomeVisual, stepLine } from "./SyncOutcome";
import styles from "./SyncView.module.css";

/** Select value for "line up with the source". */
const SOURCE = "";

export function SyncView() {
  const t = useT();
  const setView = useStore((s) => s.setView);
  return (
    <div className={styles.view}>
      <ViewHeader title={t((d) => d.sync.title)} sub={t((d) => d.sync.sub)} />
      <SyncPanel onNeedOutputs={() => setView("outputs")} />
    </div>
  );
}

/**
 * Lining an output up, by ear or with a microphone: the output picker and the two cards.
 * The Sync screen is this under a header; the guide shows the same thing as one of its steps.
 * `onNeedOutputs` is where "there is no output yet" sends the user.
 */
export function SyncPanel({ onNeedOutputs }: { onNeedOutputs: () => void }) {
  const t = useT();
  const locale = useLocale();
  const settings = useStore((s) => s.settings)!;
  const status = useStore((s) => s.status);
  const syncTarget = useStore((s) => s.syncTarget);
  const setSyncTarget = useStore((s) => s.setSyncTarget);
  const clicks = status?.clicks ?? false;

  // Never leave the clicks running once the user walks away from this part.
  useEffect(() => () => void ipc.setClicks(false).catch(() => {}), []);

  if (settings.outputs.length === 0) {
    return (
      <EmptyState
        title={t((d) => d.sync.emptyTitle)}
        line={t((d) => d.sync.emptyLine)}
        action={<PrimaryButton onClick={onNeedOutputs}>{t((d) => d.sync.goOutputs)}</PrimaryButton>}
      />
    );
  }

  const target = settings.outputs.find((o) => o.device.id === syncTarget) ?? settings.outputs[0];
  const id = target.device.id;
  const outStatus = status?.outputs.find((o) => o.deviceId === id);
  const delay = delayFor(target, status?.source.deviceId);

  return (
    <>
      {settings.outputs.length > 1 ? (
        <SegmentedControl
          value={id}
          ariaLabel={t((d) => d.sync.pickOutput)}
          segments={settings.outputs.map((o) => ({ value: o.device.id, label: o.device.name, title: o.device.name }))}
          onChange={setSyncTarget}
        />
      ) : null}
      <div className={styles.grid}>
        <Card>
          <h3 className={styles.cardTitle}>
            <Ear size={17} />
            {t((d) => d.sync.earTitle)}
          </h3>
          <p className={styles.line}>{t((d) => d.sync.earLine)}</p>
          <Row label={t((d) => d.sync.clicks)}>
            <Switch checked={clicks} onChange={(on) => void ipc.setClicks(on)} label={t((d) => d.sync.clicks)} />
          </Row>
          <div className={styles.dial}>
            <DelayControl key={id} large value={delay} onChange={(ms) => void ipc.setOutputDelay(id, ms)} />
          </div>
          {delay === 0 && outStatus?.minLagMs != null ? (
            <Notice>{t((d) => d.outputs.atZero, { lag: formatMs(outStatus.minLagMs, locale) })}</Notice>
          ) : (
            <p className={styles.tip}>{t((d) => d.sync.earTip)}</p>
          )}
        </Card>
        <MicCard deviceId={id} playing={outStatus?.state === "playing"} />
      </div>
    </>
  );
}

/** Why auto-sync cannot be started right now, if it cannot. */
type Blocked = "busyElsewhere" | "noMic" | "micMissing" | "notPlaying";

function MicCard({ deviceId, playing }: { deviceId: string; playing: boolean }) {
  const t = useT();
  const locale = useLocale();
  const settings = useStore((s) => s.settings)!;
  const devices = useStore((s) => s.devices);
  const refresh = useStore((s) => s.refreshDevices);
  const setView = useStore((s) => s.setView);
  const calibrating = useStore((s) => s.status?.calibrating ?? null);
  const event = useStore((s) => s.calibration[deviceId]);
  const busyHere = calibrating === deviceId;
  const mic = settings.mic;
  const micConnected = mic != null && devices.inputs.some((d) => d.id === mic.id || d.name === mic.name);

  const options = devices.inputs.map((d) => ({ value: d.id, label: d.name }));
  if (mic && !devices.inputs.some((d) => d.id === mic.id)) options.push({ value: mic.id, label: mic.name });

  // What this output is lined up with: the source, or another output standing in for it.
  const others = settings.outputs.filter((o) => o.device.id !== deviceId && o.enabled);
  const reference = others.find((o) => o.device.id === settings.syncReference) ?? null;
  const referenceOptions = [
    { value: SOURCE, label: t((d) => d.sync.referenceSource) },
    ...others.map((o) => ({ value: o.device.id, label: o.device.name })),
  ];

  const blocked: Blocked | null =
    calibrating != null
      ? busyHere
        ? null
        : "busyElsewhere"
      : !mic
        ? "noMic"
        : !micConnected
          ? "micMissing"
          : !playing
            ? "notPlaying"
            : null;
  const blockedLine =
    blocked === "busyElsewhere"
      ? t((d) => d.sync.blockedBusy, {
          name: settings.outputs.find((o) => o.device.id === calibrating)?.device.name ?? "",
        })
      : blocked === "noMic"
        ? t((d) => d.sync.blockedNoMic)
        : blocked === "micMissing"
          ? t((d) => d.sync.blockedMicMissing, { name: mic?.name ?? "" })
          : blocked === "notPlaying"
            ? t((d) => d.sync.blockedNotPlaying)
            : null;

  // A finished run keeps the name of the output it was lined up with, even if the choice
  // above has changed since; a running one uses the current choice.
  const finished = !busyHere && event && event.phase !== "listening" ? event : undefined;
  const referenceName =
    (finished ? settings.outputs.find((o) => o.device.id === finished.reference)?.device.name : reference?.device.name) ??
    null;
  const visual = outcomeVisual(event, busyHere, referenceName, t, locale);
  const sentence = finished ? outcomeText(finished, referenceName, t, locale) : "";

  return (
    <Card>
      <h3 className={styles.cardTitle}>
        <Mic size={17} />
        {t((d) => d.sync.micTitle)}
      </h3>
      <p className={styles.line}>{t((d) => d.sync.micLine)}</p>
      <Row label={t((d) => d.sync.mic)}>
        <div className={styles.pull}>
          <Select
            value={mic?.id ?? null}
            options={options}
            placeholder={t((d) => d.sync.micNone)}
            onOpen={() => void refresh()}
            onChange={(id) => {
              const d = devices.inputs.find((x) => x.id === id);
              if (d) void ipc.setPreferences({ mic: { id: d.id, name: d.name } });
            }}
            ariaLabel={t((d) => d.sync.mic)}
          />
        </div>
      </Row>
      {others.length > 0 ? (
        <Row label={t((d) => d.sync.reference)} helper={t((d) => d.sync.referenceHelp)}>
          <div className={styles.pull}>
            <Select
              value={reference?.device.id ?? SOURCE}
              options={referenceOptions}
              onChange={(id) => void ipc.setPreferences({ syncReference: id === SOURCE ? null : id })}
              ariaLabel={t((d) => d.sync.reference)}
            />
          </div>
        </Row>
      ) : null}
      <div className={styles.run} data-sync-run>
        <PrimaryButton
          disabled={blocked != null || busyHere}
          onClick={() => void ipc.startCalibration(deviceId)}
          aria-describedby={blockedLine ? `${deviceId}-blocked` : undefined}
        >
          {t((d) => d.sync.run)}
        </PrimaryButton>
        {blockedLine ? (
          <span className={styles.blocked} id={`${deviceId}-blocked`}>
            {blockedLine}
          </span>
        ) : (
          <span className={styles.hint}>{t((d) => d.sync.pauseHint)}</span>
        )}
      </div>
      {busyHere ? (
        <OutcomePanel visual={visual}>
          <p className={styles.outcomeTitle}>
            {t((d) => d.sync.running, { pass: event?.phase === "listening" ? event.pass : 1 })}
          </p>
          <p className={styles.outcomeLine}>{stepLine(event, referenceName, t)}</p>
        </OutcomePanel>
      ) : finished ? (
        <OutcomePanel
          visual={visual}
          action={
            fixedOnOutputs(finished) ? (
              <QuietButton onClick={() => setView("outputs")}>{t((d) => d.sync.goOutputs)}</QuietButton>
            ) : undefined
          }
        >
          <p className={finished.phase === "done" ? styles.outcomeGood : styles.outcomeWarn}>{sentence}</p>
        </OutcomePanel>
      ) : null}
      {/*
        An outcome is spoken once, when it arrives: this region is empty while a run is in
        progress, so the same result after a second run is announced again. The visible
        sentence above is not a live region, so nothing is read twice, and the steps of a
        running measurement are not narrated.
      */}
      <p className="sr-only" role="status" aria-live="polite">
        {sentence}
      </p>
      <Row label={t((d) => d.sync.resync)} helper={t((d) => d.sync.resyncHelp)}>
        <Switch
          checked={settings.resyncOnReconnect}
          onChange={(on) => void ipc.setPreferences({ resyncOnReconnect: on })}
          label={t((d) => d.sync.resync)}
        />
      </Row>
    </Card>
  );
}

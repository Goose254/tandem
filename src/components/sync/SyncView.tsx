import { useEffect } from "react";
import { Ear, Mic } from "lucide-react";
import { Card, EmptyState, Notice, Row, SyncSpinner, ViewHeader } from "../common/Layout";
import { PrimaryButton } from "../common/Buttons";
import { SegmentedControl, Select, Switch } from "../common/Controls";
import { DelayControl } from "../common/DelayControl";
import { delayFor, ipc } from "../../lib/ipc";
import type { CalibrationEvent } from "../../lib/ipc";
import { useStore } from "../../state/store";
import { useLocale, useT } from "../../i18n";
import { formatMs } from "../../lib/summary";
import type { Translate } from "../../i18n";
import styles from "./SyncView.module.css";

/** Select value for "line up with the source". */
const SOURCE = "";

export function SyncView() {
  const t = useT();
  const locale = useLocale();
  const settings = useStore((s) => s.settings)!;
  const status = useStore((s) => s.status);
  const syncTarget = useStore((s) => s.syncTarget);
  const setSyncTarget = useStore((s) => s.setSyncTarget);
  const setView = useStore((s) => s.setView);
  const clicks = status?.clicks ?? false;

  // Never leave the clicks running once the user walks away from this view.
  useEffect(() => () => void ipc.setClicks(false).catch(() => {}), []);

  if (settings.outputs.length === 0) {
    return (
      <div className={styles.view}>
        <ViewHeader title={t((d) => d.sync.title)} sub={t((d) => d.sync.sub)} />
        <EmptyState
          title={t((d) => d.sync.emptyTitle)}
          line={t((d) => d.sync.emptyLine)}
          action={<PrimaryButton onClick={() => setView("outputs")}>{t((d) => d.sync.goOutputs)}</PrimaryButton>}
        />
      </div>
    );
  }

  const target = settings.outputs.find((o) => o.device.id === syncTarget) ?? settings.outputs[0];
  const id = target.device.id;
  const outStatus = status?.outputs.find((o) => o.deviceId === id);
  const delay = delayFor(target, status?.source.deviceId);

  return (
    <div className={styles.view}>
      <ViewHeader title={t((d) => d.sync.title)} sub={t((d) => d.sync.sub)} />
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
    </div>
  );
}

function MicCard({ deviceId, playing }: { deviceId: string; playing: boolean }) {
  const t = useT();
  const locale = useLocale();
  const settings = useStore((s) => s.settings)!;
  const devices = useStore((s) => s.devices);
  const refresh = useStore((s) => s.refreshDevices);
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
      <div className={styles.run}>
        <PrimaryButton
          disabled={!micConnected || !playing || calibrating != null}
          onClick={() => void ipc.startCalibration(deviceId)}
        >
          {t((d) => d.sync.run)}
        </PrimaryButton>
        {busyHere ? (
          <span className={styles.progress}>
            <SyncSpinner size={20} />
            {t((d) => d.sync.running, { pass: event?.phase === "listening" ? event.pass : 1 })}
          </span>
        ) : event ? (
          <Result
            event={event}
            t={t}
            locale={locale}
            referenceName={settings.outputs.find((o) => o.device.id === event.reference)?.device.name}
          />
        ) : (
          <span className={styles.hint}>{t((d) => d.sync.pauseHint)}</span>
        )}
      </div>
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

function Result({
  event,
  t,
  locale,
  referenceName,
}: {
  event: CalibrationEvent;
  t: Translate;
  locale: string;
  referenceName?: string;
}) {
  if (event.phase === "failed" && event.failure) {
    return <span className={styles.failure}>{t((d) => d.sync.failure[event.failure!])}</span>;
  }
  if (event.phase === "limited" && event.lagMs != null) {
    return (
      <span className={styles.failure}>{t((d) => d.sync.resultLimited, { lag: formatMs(event.lagMs!, locale) })}</span>
    );
  }
  if (event.phase !== "done" || event.delayMs == null) return null;
  if (event.referenceDelayMs != null) {
    return (
      <span className={styles.success}>
        {t((d) => d.sync.resultReferenceMoved, {
          delay: event.delayMs,
          name: referenceName ?? t((d) => d.sync.reference),
          refDelay: event.referenceDelayMs,
        })}
      </span>
    );
  }
  const delta = event.delayMs - (event.previousDelayMs ?? event.delayMs);
  const delay = event.delayMs;
  const text =
    Math.abs(delta) < 1
      ? t((d) => d.sync.resultSame, { delay })
      : delta > 0
        ? t((d) => d.sync.resultLater, { delta, delay })
        : t((d) => d.sync.resultEarlier, { delta: -delta, delay });
  return (
    <span className={styles.success}>
      {text}
      {event.automatic ? ` ${t((d) => d.sync.automatic)}` : ""}
    </span>
  );
}

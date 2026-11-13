import { useRef, useState } from "react";
import { AudioWaveform, CircleAlert, Lock, Minus, Plus, TimerOff, Trash2, Unplug } from "lucide-react";
import { Card, EmptyState, Notice, Section, ViewHeader } from "../common/Layout";
import { IconButton, QuietButton } from "../common/Buttons";
import { Meter, Select, StatusDot, Switch } from "../common/Controls";
import type { SelectOption, Tone } from "../common/Controls";
import { DelayControl } from "../common/DelayControl";
import { delayFor, ipc } from "../../lib/ipc";
import type { ErrorKind, OutputSetting, OutputStatus, Snapshot } from "../../lib/ipc";
import { outcomeText } from "../sync/SyncOutcome";
import { formatDb, formatMs, formatPpm, formatRate, outputTone } from "../../lib/summary";
import { useStore } from "../../state/store";
import { useLocale, useT } from "../../i18n";
import styles from "./OutputsView.module.css";

const DEFAULT = "__default__";

/** A drawing for why Windows would not open a device, shown before the sentence that says it. */
function CauseIcon({ kind }: { kind: ErrorKind }) {
  switch (kind) {
    case "inUse":
      return <Lock size={14} />;
    case "invalidated":
      return <Unplug size={14} />;
    case "stalled":
      return <TimerOff size={14} />;
    default:
      return <CircleAlert size={14} />;
  }
}

export function OutputsView() {
  const t = useT();
  const settings = useStore((s) => s.settings);
  const status = useStore((s) => s.status);
  const removed = useStore((s) => s.removed);
  const undoRemove = useStore((s) => s.undoRemove);
  if (!settings) return null;

  return (
    <div className={styles.view}>
      <ViewHeader
        title={t((d) => d.outputs.title)}
        sub={t((d) => d.outputs.sub)}
        trailing={
          <>
            <span className={styles.masterLabel}>{t((d) => d.outputs.mirroring)}</span>
            <Switch
              checked={settings.enabled}
              onChange={(on) => void ipc.setEnabled(on)}
              label={t((d) => d.outputs.mirroring)}
            />
          </>
        }
      />
      <Section label={t((d) => d.outputs.sourceLabel)}>
        <SourceCard />
      </Section>
      <Section label={t((d) => d.outputs.outputsLabel)} trailing={<AddOutput />}>
        {removed ? (
          <Notice action={<QuietButton onClick={undoRemove}>{t((d) => d.outputs.undo)}</QuietButton>}>
            {t((d) => d.outputs.removed, { name: removed.device.name })}
          </Notice>
        ) : null}
        {settings.outputs.length === 0 && removed ? null : settings.outputs.length === 0 ? (
          <EmptyState
            title={t((d) => d.outputs.emptyTitle)}
            line={t((d) => d.outputs.emptyLine)}
            action={<AddOutput primary />}
          />
        ) : (
          <div className={styles.list}>
            {settings.outputs.map((o) => (
              <OutputCard
                key={o.device.id}
                output={o}
                status={status?.outputs.find((s) => s.deviceId === o.device.id)}
                snapshot={status}
              />
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}

function SourceCard() {
  const t = useT();
  const locale = useLocale();
  const settings = useStore((s) => s.settings)!;
  const status = useStore((s) => s.status);
  const devices = useStore((s) => s.devices);
  const refresh = useStore((s) => s.refreshDevices);
  const defaultDevice = devices.outputs.find((d) => d.isDefault);

  const options: SelectOption[] = [
    {
      value: DEFAULT,
      label: t((d) => d.outputs.followDefault),
      hint: defaultDevice ? t((d) => d.outputs.followDefaultNow, { name: defaultDevice.name }) : undefined,
    },
    ...devices.outputs.map((d) => ({ value: d.id, label: d.name })),
  ];
  // A pinned device that is not connected still shows by name.
  const pinned = settings.source.device;
  if (!settings.source.followDefault && pinned && !devices.outputs.some((d) => d.id === pinned.id)) {
    options.push({ value: pinned.id, label: pinned.name });
  }

  const onChange = (value: string) => {
    if (value === DEFAULT) {
      void ipc.setSource(true, null);
    } else {
      const d = devices.outputs.find((x) => x.id === value);
      if (d) void ipc.setSource(false, { id: d.id, name: d.name });
    }
  };

  const src = status?.source;
  const tone: Tone =
    src?.state === "running" ? "good" : src?.state === "waiting" ? "warn" : src?.state === "error" ? "danger" : "idle";
  const stateText =
    src?.state === "running"
      ? t((d) => d.outputs.sourceRunning)
      : src?.state === "waiting"
        ? t((d) => d.outputs.sourceWaiting)
        : src?.state === "error"
          ? t((d) => d.outputs.sourceError)
          : src?.state === "paused"
            ? t((d) => d.outputs.sourcePaused)
            : t((d) => d.outputs.sourceOff);

  return (
    <Card dim={!settings.enabled}>
      <div className={styles.cardHead}>
        <div className={styles.sourcePick}>
          <Select
            value={settings.source.followDefault ? DEFAULT : (pinned?.id ?? DEFAULT)}
            options={options}
            onChange={onChange}
            onOpen={() => void refresh()}
            ariaLabel={t((d) => d.outputs.sourceLabel)}
            align="start"
            wide
          />
        </div>
        <div className={styles.state}>
          <StatusDot tone={tone} />
          <span>{stateText}</span>
          {src?.state === "running" && src.sampleRate ? (
            <span className={`${styles.rate} tabular`}>{formatRate(src.sampleRate, locale)}</span>
          ) : null}
        </div>
      </div>
      <Meter level={src?.level ?? 0} muted={src?.state !== "running"} />
      {src?.state === "error" && src.errorKind ? (
        <Notice tone="warn" icon={<CauseIcon kind={src.errorKind} />}>
          <span title={src.error ?? undefined}>{t((d) => d.outputs.errors[src.errorKind!])}</span>
        </Notice>
      ) : null}
      {src?.state === "waiting" ? <Notice>{t((d) => d.outputs.sourceWaitingHelp)}</Notice> : null}
      <p className={styles.helper}>{t((d) => d.outputs.sourceHelp)}</p>
    </Card>
  );
}

function AddOutput({ primary = false }: { primary?: boolean }) {
  const t = useT();
  const settings = useStore((s) => s.settings)!;
  const devices = useStore((s) => s.devices);
  const refresh = useStore((s) => s.refreshDevices);
  const used = new Set(settings.outputs.map((o) => o.device.id));
  const options: SelectOption[] = devices.outputs
    .filter((d) => !used.has(d.id))
    .map((d) => ({ value: d.id, label: d.name }));

  return (
    <Select
      value={null}
      options={options.length ? options : [{ value: "", label: t((d) => d.outputs.addNone) }]}
      onOpen={() => void refresh()}
      onChange={(id) => {
        const d = devices.outputs.find((x) => x.id === id);
        if (d) void ipc.addOutput({ id: d.id, name: d.name });
      }}
      ariaLabel={t((d) => d.outputs.add)}
      align={primary ? "start" : "end"}
      trigger={() =>
        primary ? (
          <span className={styles.addPrimary}>
            <Plus size={16} />
            {t((d) => d.outputs.add)}
          </span>
        ) : (
          <QuietButton>
            <Plus size={15} />
            {t((d) => d.outputs.add)}
          </QuietButton>
        )
      }
    />
  );
}

function OutputCard({
  output,
  status,
  snapshot,
}: {
  output: OutputSetting;
  status: OutputStatus | undefined;
  snapshot: Snapshot | null;
}) {
  const t = useT();
  const locale = useLocale();
  const openSync = useStore((s) => s.openSync);
  const removeOutput = useStore((s) => s.removeOutput);
  const settings = useStore((s) => s.settings);
  const id = output.device.id;
  // The last automatic re-sync of this output, until a later run replaces it: it happened
  // with nobody watching, so its result is shown here and not only on the Sync screen.
  const auto = useStore((s) => s.calibration[id]);
  const autoDone = auto?.automatic && auto.phase !== "listening" ? auto : undefined;
  const autoReference =
    settings?.outputs.find((o) => o.device.id === autoDone?.reference)?.device.name ?? null;
  const delay = delayFor(output, snapshot?.source.deviceId);
  const state = status?.state ?? "idle";
  const sourceRate = snapshot?.source.sampleRate ?? null;
  const live = state === "playing" || state === "buffering";
  const stateText = t((d) => d.outputs.state[state]);

  return (
    <Card dim={!output.enabled || state === "isSource" || state === "waiting"}>
      <div className={styles.cardHead}>
        <div className={styles.name}>
          <StatusDot tone={outputTone(state)} />
          <span className={styles.deviceName} title={output.device.name}>
            {output.device.name}
          </span>
        </div>
        <div className={styles.headActions}>
          <span className={styles.stateText}>{stateText}</span>
          <Switch
            checked={output.enabled}
            onChange={(on) => void ipc.setOutputEnabled(id, on)}
            label={t((d) => d.outputs.enable)}
          />
          <IconButton label={t((d) => d.outputs.remove)} small onClick={() => removeOutput(output)}>
            <Trash2 size={15} />
          </IconButton>
        </div>
      </div>

      <div className={styles.controls}>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t((d) => d.outputs.delay)}</span>
          <DelayControl value={delay} onChange={(ms) => void ipc.setOutputDelay(id, ms)} />
        </div>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t((d) => d.outputs.gain)}</span>
          <GainControl value={output.gainDb} onChange={(db) => void ipc.setOutputGain(id, db)} />
        </div>
      </div>

      <div className={styles.foot}>
        <Meter level={status?.level ?? 0} muted={!live} />
        <span className={`${styles.stats} tabular`}>
          {live && status
            ? t((d) => d.outputs.stats, {
                rate: status.sampleRate ? formatRate(status.sampleRate, locale) : "—",
                ppm: formatPpm(status.correctionPpm),
                lag: status.minLagMs != null ? formatMs(status.minLagMs, locale) : "—",
              })
            : ""}
          {status && status.underruns > 0 ? (
            <span className={styles.dropouts}>
              {live ? " · " : ""}
              {t((d) => d.outputs.underruns, { n: status.underruns })}
            </span>
          ) : null}
        </span>
        <QuietButton onClick={() => openSync(id)}>
          <AudioWaveform size={15} />
          {t((d) => d.outputs.sync)}
        </QuietButton>
      </div>

      {state === "error" && status?.errorKind ? (
        <Notice tone="warn" icon={<CauseIcon kind={status.errorKind} />}>
          <span title={status.error ?? undefined}>{t((d) => d.outputs.errors[status.errorKind!])}</span>
        </Notice>
      ) : null}
      {state === "isSource" ? <Notice>{t((d) => d.outputs.isSourceHelp)}</Notice> : null}
      {autoDone ? (
        <Notice tone={autoDone.phase === "done" ? "info" : "warn"}>
          {autoDone.phase === "done" && autoDone.delayMs != null
            ? t((d) => d.outputs.autoResynced, { delay: autoDone.delayMs })
            : autoDone.failure === "otherAudio"
              ? // This one already says it was the automatic re-sync that stopped.
                outcomeText(autoDone, autoReference, t, locale)
              : t((d) => d.outputs.autoFailed, { reason: outcomeText(autoDone, autoReference, t, locale) })}
        </Notice>
      ) : null}
      {live && status?.sampleRate && sourceRate && status.sampleRate !== sourceRate ? (
        <Notice
          tone="warn"
          action={
            <QuietButton onClick={() => void ipc.openSoundSettings()}>{t((d) => d.outputs.openSound)}</QuietButton>
          }
        >
          {t((d) => d.outputs.rateMismatch, {
            out: formatRate(status.sampleRate, locale),
            src: formatRate(sourceRate, locale),
          })}
        </Notice>
      ) : null}
      {live && delay === 0 && status?.minLagMs != null ? (
        <Notice>{t((d) => d.outputs.atZero, { lag: formatMs(status.minLagMs, locale) })}</Notice>
      ) : null}
    </Card>
  );
}

const MIN_GAIN_DB = -40;
const MAX_GAIN_DB = 12;

/**
 * Volume in dB: buttons step by 1, the arrow keys by 0.5 (0.1 with Shift), and the readout
 * takes any typed value.
 */
function GainControl({ value, onChange }: { value: number; onChange: (db: number) => void }) {
  const t = useT();
  const [editing, setEditing] = useState<string | null>(null);
  const cancelled = useRef(false);
  const label = t((d) => d.outputs.gain);

  const set = (db: number) => {
    const clamped = Math.round(Math.min(MAX_GAIN_DB, Math.max(MIN_GAIN_DB, db)) * 10) / 10;
    if (clamped !== value) onChange(clamped);
  };
  const commit = () => {
    if (editing == null) return;
    const n = Number(editing.replace(",", ".").replace("−", "-"));
    setEditing(null);
    if (cancelled.current) cancelled.current = false;
    else if (editing.trim() !== "" && Number.isFinite(n)) set(n);
  };

  return (
    <div className={styles.gain}>
      <IconButton label="-1 dB" small onClick={() => set(value - 1)}>
        <Minus size={13} />
      </IconButton>
      <label className={styles.gainValue}>
        <input
          className={`${styles.gainInput} tabular`}
          value={editing ?? formatDb(value)}
          aria-label={label}
          inputMode="decimal"
          onFocus={(e) => {
            setEditing(formatDb(value));
            e.currentTarget.select();
          }}
          onChange={(e) => setEditing(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              cancelled.current = true;
              e.currentTarget.blur();
            }
            if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              const step = (e.shiftKey ? 0.1 : 0.5) * (e.key === "ArrowUp" ? 1 : -1);
              setEditing(null);
              set(value + step);
            }
          }}
        />
        <small>{t((d) => d.units.db)}</small>
      </label>
      <IconButton label="+1 dB" small onClick={() => set(value + 1)}>
        <Plus size={13} />
      </IconButton>
    </div>
  );
}

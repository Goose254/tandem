import { Plus, Trash2, Minus, AudioWaveform } from "lucide-react";
import { Card, EmptyState, Section, ViewHeader } from "../common/Layout";
import { IconButton, QuietButton } from "../common/Buttons";
import { Meter, Select, StatusDot, Switch } from "../common/Controls";
import type { SelectOption, Tone } from "../common/Controls";
import { DelayControl } from "../common/DelayControl";
import { delayFor, ipc } from "../../lib/ipc";
import type { OutputSetting, OutputStatus, Snapshot } from "../../lib/ipc";
import { formatDb, formatPpm, outputTone } from "../../lib/summary";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";
import styles from "./OutputsView.module.css";

const DEFAULT = "__default__";

export function OutputsView() {
  const t = useT();
  const settings = useStore((s) => s.settings);
  const status = useStore((s) => s.status);
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
        {settings.outputs.length === 0 ? (
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
        </div>
      </div>
      <Meter level={src?.level ?? 0} muted={src?.state !== "running"} />
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
  const openSync = useStore((s) => s.openSync);
  const id = output.device.id;
  const delay = delayFor(output, snapshot?.source.deviceId);
  const state = status?.state ?? "idle";
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
          <IconButton label={t((d) => d.outputs.remove)} small onClick={() => void ipc.removeOutput(id)}>
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
          <div className={styles.gain}>
            <IconButton label="-1 dB" small onClick={() => void ipc.setOutputGain(id, output.gainDb - 1)}>
              <Minus size={13} />
            </IconButton>
            <span className={`${styles.gainValue} tabular`}>
              {formatDb(output.gainDb)} <small>{t((d) => d.units.db)}</small>
            </span>
            <IconButton label="+1 dB" small onClick={() => void ipc.setOutputGain(id, output.gainDb + 1)}>
              <Plus size={13} />
            </IconButton>
          </div>
        </div>
      </div>

      <div className={styles.foot}>
        <Meter level={status?.level ?? 0} muted={!live} />
        <span className={`${styles.stats} tabular`}>
          {live && status
            ? t((d) => d.outputs.stats, {
                buffer: status.bufferedMs.toFixed(1),
                ppm: formatPpm(status.correctionPpm),
              })
            : status?.error ?? ""}
          {status && status.underruns > 0
            ? ` · ${t((d) => d.outputs.underruns, { n: status.underruns })}`
            : ""}
        </span>
        <QuietButton onClick={() => openSync(id)}>
          <AudioWaveform size={15} />
          {t((d) => d.outputs.sync)}
        </QuietButton>
      </div>
    </Card>
  );
}

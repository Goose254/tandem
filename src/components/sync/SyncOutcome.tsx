/*
 * What auto-sync is doing, or how it ended, for one output: a drawing beside the sentence
 * that says it. The sentence is the source of truth (it is what a screen reader gets, once,
 * through the live region in SyncView); the drawing shows the same thing at a glance, and for
 * a failure it points at the part to fix.
 */

import type { ReactNode } from "react";
import type { CalibrationEvent, Failure } from "../../lib/ipc";
import type { Translate } from "../../i18n";
import { formatMs } from "../../lib/summary";
import { Scene, Timeline } from "./SyncVisuals";
import type { MicState, SceneSpeaker, SpeakerState } from "./SyncVisuals";
import styles from "./SyncView.module.css";

/**
 * The name of the output this one is lined up with ("Line up with"), or null when it is
 * lined up with the source.
 */
export type Reference = string | null;

/** Longest label that fits above the time axis. */
const AXIS_LABEL = 26;

function short(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

/** The sentence for a finished run. A running pass has its own line (see `stepLine`). */
export function outcomeText(
  event: CalibrationEvent,
  reference: Reference,
  t: Translate,
  locale: string,
): string {
  if (event.phase === "failed" && event.failure) return t((d) => d.sync.failure[event.failure!]);
  if (event.phase === "limited" && event.lagMs != null) {
    return t((d) => d.sync.resultLimited, { lag: formatMs(event.lagMs, locale) });
  }
  if (event.phase !== "done" || event.delayMs == null) return "";
  const automatic = event.automatic ? ` ${t((d) => d.sync.automatic)}` : "";
  if (event.referenceDelayMs != null) {
    return (
      t((d) => d.sync.resultReferenceMoved, {
        delay: event.delayMs,
        name: reference ?? t((d) => d.sync.reference),
        refDelay: event.referenceDelayMs,
      }) + automatic
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
  return text + automatic;
}

/** What is happening right now in a running pass. */
export function stepLine(event: CalibrationEvent | undefined, reference: Reference, t: Translate): string {
  switch (event?.step) {
    case "first":
      return reference
        ? t((d) => d.sync.stepFirstOutput, { name: reference })
        : t((d) => d.sync.stepFirstSource);
    case "second":
      return reference
        ? t((d) => d.sync.stepSecondOutput, { name: reference })
        : t((d) => d.sync.stepSecondSource);
    case "analysing":
      return t((d) => d.sync.stepAnalysing);
    default:
      return t((d) => d.sync.stepPreparing);
  }
}

/** Who is at fault for each failure, as states of the scene. `null`: nothing worth drawing. */
const FAULTS: Record<Failure, { anchor: SpeakerState; output: SpeakerState; mic: MicState; intruder?: true } | null> = {
  noMic: { anchor: "idle", output: "idle", mic: "none" },
  micMissing: { anchor: "idle", output: "idle", mic: "unplugged" },
  micFailed: { anchor: "idle", output: "idle", mic: "blocked" },
  notRunning: { anchor: "idle", output: "off", mic: "idle" },
  referenceNotRunning: { anchor: "off", output: "idle", mic: "idle" },
  sourceNotHeard: { anchor: "unheard", output: "idle", mic: "idle" },
  targetNotHeard: { anchor: "heard", output: "unheard", mic: "idle" },
  tooShort: { anchor: "idle", output: "idle", mic: "cut" },
  interrupted: { anchor: "stuttering", output: "stuttering", mic: "idle" },
  otherAudio: { anchor: "idle", output: "idle", mic: "idle", intruder: true },
  busy: null,
};

/** The drawing for an event, or null when the sentence alone says it all. */
export function outcomeVisual(
  event: CalibrationEvent | undefined,
  busy: boolean,
  reference: Reference,
  t: Translate,
  locale: string,
): ReactNode {
  const anchor = reference ?? t((d) => d.sync.vizSource);
  const scene = (anchorState: SpeakerState, outputState: SpeakerState, mic: MicState, intruder = false) => {
    const speakers: SceneSpeaker[] = [
      { label: anchor, state: anchorState },
      { label: t((d) => d.sync.vizOutput), state: outputState },
    ];
    return (
      <Scene
        speakers={speakers}
        mic={mic}
        micLabel={t((d) => d.sync.mic)}
        intruder={intruder ? t((d) => d.sync.vizOther) : undefined}
      />
    );
  };

  if (busy) {
    switch (event?.step) {
      case "first":
        return scene("playing", "idle", "listening");
      case "second":
        return scene("playing", "playing", "listening");
      case "analysing":
        return scene("idle", "idle", "thinking");
      default:
        return scene("idle", "idle", "listening");
    }
  }
  if (!event) return null;

  if (event.phase === "failed" && event.failure) {
    const fault = FAULTS[event.failure];
    return fault ? scene(fault.anchor, fault.output, fault.mic, fault.intruder) : null;
  }

  const anchorLabel = short(anchor, AXIS_LABEL);
  if (event.phase === "limited" && event.lagMs != null) {
    return (
      <Timeline
        kind="stuck"
        lagMs={event.lagMs}
        anchorLabel={anchorLabel}
        caption={t((d) => d.sync.vizStillLate, { lag: formatMs(event.lagMs, locale) })}
      />
    );
  }
  if (event.phase !== "done" || event.delayMs == null) return null;
  if (event.referenceDelayMs != null) {
    // How far the reference moved is not in the event: the drawing shows that it did.
    return <Timeline kind="anchorMoved" movedMs={40} anchorLabel={anchorLabel} caption={t((d) => d.sync.vizWaits)} />;
  }
  const delta = event.delayMs - (event.previousDelayMs ?? event.delayMs);
  if (Math.abs(delta) < 1) {
    return <Timeline kind="same" anchorLabel={anchorLabel} caption={t((d) => d.sync.vizTogether)} />;
  }
  return (
    <Timeline
      kind="moved"
      movedMs={delta}
      anchorLabel={anchorLabel}
      caption={delta > 0 ? t((d) => d.sync.vizLater, { delta }) : t((d) => d.sync.vizEarlier, { delta: -delta })}
    />
  );
}

/** Failures the user fixes on the Outputs screen: the panel offers the way there. */
export function fixedOnOutputs(event: CalibrationEvent): boolean {
  return (
    event.phase === "limited" ||
    (event.phase === "failed" && (event.failure === "notRunning" || event.failure === "referenceNotRunning"))
  );
}

/** The panel under the Run button: drawing on the left, words on the right. */
export function OutcomePanel({
  visual,
  children,
  action,
}: {
  visual: ReactNode;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className={styles.outcome} data-sync-outcome>
      {visual}
      <div className={styles.outcomeText}>
        {children}
        {action ? <div className={styles.outcomeAction}>{action}</div> : null}
      </div>
    </div>
  );
}

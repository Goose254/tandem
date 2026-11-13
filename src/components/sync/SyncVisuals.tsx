/*
 * The drawings behind every auto-sync state. They carry no information the text beside them
 * does not: the text is the source of truth, and each drawing is hidden from assistive tech.
 *
 * Two drawings cover everything:
 *
 *  - `Scene`: who plays and who listens. The speakers a measurement uses on the left, the
 *    microphone on the right, sound between them. While listening it shows which speaker is
 *    playing now; after a failure it shows which part to fix.
 *  - `Timeline`: when each speaker's sound arrived. After a success it shows the output
 *    moving onto the speaker it was lined up with, once, then stays still.
 */

import type { CSSProperties, ReactNode } from "react";
import { Check, Mic, MicOff, Music, Speaker, TimerOff, Unplug, VolumeX } from "lucide-react";
import { SyncSpinner } from "../common/Layout";
import styles from "./SyncVisuals.module.css";

// -- Scene -------------------------------------------------------------------------------

/**
 * What one speaker is doing. `heard` is a speaker the microphone picked up; `unheard`, `off`
 * and `stuttering` mark the part to fix.
 */
export type SpeakerState = "idle" | "playing" | "heard" | "unheard" | "off" | "stuttering";
/** `none`, `unplugged`, `blocked` and `cut` mark the microphone as the part to fix. */
export type MicState = "idle" | "listening" | "thinking" | "none" | "unplugged" | "blocked" | "cut";

export type SceneSpeaker = { label: string; state: SpeakerState };

function Waves({ state }: { state: SpeakerState }) {
  if (state === "idle" || state === "off") return <span className={styles.waves} />;
  return (
    <span className={styles.waves} data-state={state}>
      <svg viewBox="0 0 44 20" width="44" height="20">
        <path d="M6 5 A7 7 0 0 1 6 15" />
        <path d="M17 2 A11 11 0 0 1 17 18" />
        <path d="M28 0.5 A14 14 0 0 1 28 19.5" />
        {state === "unheard" ? <path className={styles.cross} d="M36 6 L42 14 M42 6 L36 14" /> : null}
      </svg>
    </span>
  );
}

function SpeakerRow({ label, state }: SceneSpeaker) {
  const fault = state === "unheard" || state === "off" || state === "stuttering";
  return (
    <>
      <span className={styles.node} data-fault={fault || undefined} data-live={state === "playing" || undefined}>
        <span className={styles.chip}>
          {state === "off" ? <VolumeX size={15} /> : <Speaker size={15} />}
          {state === "heard" ? (
            <i className={styles.badge}>
              <Check size={9} strokeWidth={3} />
            </i>
          ) : null}
        </span>
        <span className={styles.nodeLabel}>{label}</span>
      </span>
      <Waves state={state} />
    </>
  );
}

const MIC_ICON: Record<MicState, ReactNode> = {
  idle: <Mic size={16} />,
  listening: <Mic size={16} />,
  thinking: <SyncSpinner size={18} />,
  none: <span className={styles.question}>?</span>,
  unplugged: <Unplug size={16} />,
  blocked: <MicOff size={16} />,
  cut: <TimerOff size={16} />,
};

/**
 * Speakers on the left, each with the sound it sends; the microphone on the right. An extra
 * row can show another program playing over the measurement.
 */
export function Scene({
  speakers,
  mic,
  micLabel,
  intruder,
}: {
  speakers: SceneSpeaker[];
  mic: MicState;
  micLabel: string;
  /** Label of another program heard during the measurement, if that is the problem. */
  intruder?: string;
}) {
  const micFault = mic === "none" || mic === "unplugged" || mic === "blocked" || mic === "cut";
  return (
    <div className={styles.scene} aria-hidden="true">
      <div className={styles.speakers}>
        {speakers.map((s, i) => (
          <SpeakerRow key={i} {...s} />
        ))}
        {intruder ? (
          <>
            <span className={styles.node} data-fault>
              <span className={styles.chip}>
                <Music size={15} />
              </span>
              <span className={styles.nodeLabel}>{intruder}</span>
            </span>
            <Waves state="playing" />
          </>
        ) : null}
      </div>
      <span className={styles.mic} data-fault={micFault || undefined} data-live={mic === "listening" || undefined}>
        <span className={styles.chip}>{MIC_ICON[mic]}</span>
        <span className={styles.nodeLabel}>{micLabel}</span>
      </span>
    </div>
  );
}

// -- Timeline ----------------------------------------------------------------------------

/** Pixels for a gap of `ms`: grows with the square root, so 4 ms shows and 300 ms still fits. */
function gapPx(ms: number): number {
  return Math.max(18, Math.min(92, 10 + 8 * Math.sqrt(Math.abs(ms))));
}

const WIDTH = 240;
const MID = WIDTH / 2;
const LINE_Y = 30;

export type TimelineProps = {
  /** Above the tick: the speaker this output is lined up with. */
  anchorLabel: string;
  /** Below the axis: the gap that was closed, or the one that remains, or "this output". */
  caption: string;
} & (
  | {
      /** The output moved by `movedMs` (negative = earlier) and now sits on the anchor. */
      kind: "moved";
      movedMs: number;
    }
  | { kind: "same" }
  | {
      /** The output could not come earlier: the anchor was moved later, by `movedMs`, onto it. */
      kind: "anchorMoved";
      movedMs: number;
    }
  | {
      /** Still `lagMs` late at 0 ms: nothing can close this gap. */
      kind: "stuck";
      lagMs: number;
    }
);

/**
 * A time axis running left to right (later is further right). The anchor is a tick, this
 * output a dot. Whatever moved starts where it was, drawn as a ghost, and slides to where it
 * is now, once.
 */
export function Timeline(props: TimelineProps) {
  let anchorX = MID;
  let outputX = MID;
  // Where the moving part came from, relative to where it ends.
  let from = 0;
  let moving: "output" | "anchor" | null = null;

  if (props.kind === "moved") {
    // Moved earlier means it was late, to the right; moved later means it was to the left.
    from = (props.movedMs < 0 ? 1 : -1) * gapPx(props.movedMs);
    moving = "output";
  } else if (props.kind === "anchorMoved") {
    from = -gapPx(props.movedMs);
    moving = "anchor";
  } else if (props.kind === "stuck") {
    const px = gapPx(props.lagMs);
    anchorX = MID - px / 2;
    outputX = MID + px / 2;
  }

  const slide = (who: "output" | "anchor") =>
    moving === who ? { className: styles.slide, style: { "--from": `${from}px` } as CSSProperties } : {};
  const span =
    props.kind === "stuck"
      ? ([anchorX, outputX] as const)
      : moving
        ? ([Math.min(MID, MID + from), Math.max(MID, MID + from)] as const)
        : null;

  return (
    <div className={styles.timeline} aria-hidden="true">
      <svg viewBox={`0 0 ${WIDTH} 62`} width={WIDTH} height="62">
        <line className={styles.axis} x1="10" y1={LINE_Y} x2="224" y2={LINE_Y} />
        <path className={styles.axisHead} d={`M221 ${LINE_Y - 4} L229 ${LINE_Y} L221 ${LINE_Y + 4}`} />
        {span ? (
          <line
            className={props.kind === "stuck" ? styles.gapWarn : styles.gap}
            x1={span[0]}
            y1={LINE_Y}
            x2={span[1]}
            y2={LINE_Y}
          />
        ) : null}
        {moving ? <circle className={styles.ghost} cx={MID + from} cy={LINE_Y} r="5" /> : null}
        <g {...slide("anchor")}>
          <line className={styles.tick} x1={anchorX} y1={LINE_Y - 9} x2={anchorX} y2={LINE_Y + 9} />
        </g>
        <g {...slide("output")}>
          <circle className={props.kind === "stuck" ? styles.dotWarn : styles.dot} cx={outputX} cy={LINE_Y} r="6" />
        </g>
        <text className={styles.label} x={anchorX} y="12" textAnchor="middle">
          {props.anchorLabel}
        </text>
        <text
          className={props.kind === "stuck" ? styles.captionWarn : styles.caption}
          x={MID}
          y={LINE_Y + 26}
          textAnchor="middle"
        >
          {props.caption}
        </text>
      </svg>
    </div>
  );
}

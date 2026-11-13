/*
 * The picture behind the whole app: every speaker plays the same sound, but not at the same
 * moment. A slow device (Bluetooth) plays late; the others are early, so they are made to
 * wait until everything lands together.
 *
 * Time runs left to right. Each row is a speaker and the dot is when its sound comes out. The
 * dotted stretch is the wait Tandem adds to an output. Same vocabulary as the auto-sync time
 * line (`SyncVisuals.tsx`): a tick for the speaker everything is lined up with, dots that
 * slide once and then stay.
 *
 * Decorative: the text beside it says the same thing, so it is hidden from assistive tech.
 */

import type { CSSProperties } from "react";
import styles from "./Lineup.module.css";

const AXIS_FROM = 132;
const AXIS_TO = 440;
/** Where the source's sound comes out: late, because it is the slow device. */
const SOURCE_X = 372;
/** Where each output's sound would come out on its own: early, and not together. */
const OUTPUT_X = [214, 268];
const ROWS = [26, 66, 106];

export function Lineup({
  stage,
  sourceLabel,
  sourceTag,
  outputLabel,
  waitLabel,
}: {
  /** `apart`: as the devices are; `together`: once delayed; `play`: one becoming the other. */
  stage: "apart" | "together" | "play";
  sourceLabel: string;
  /** A word under the source's name: why it is the source. */
  sourceTag: string;
  outputLabel: string;
  /** A word on the dotted stretch: what the output does there. */
  waitLabel: string;
}) {
  const lined = stage !== "apart";
  return (
    <div className={styles.lineup} aria-hidden="true">
      <svg viewBox="0 0 452 132" width="452" height="132">
        {/* The moment everything is lined up on. */}
        <line className={lined ? styles.mark : styles.markFaint} x1={SOURCE_X} y1="8" x2={SOURCE_X} y2="124" />

        {ROWS.map((y, row) => {
          const source = row === 0;
          const own = source ? SOURCE_X : OUTPUT_X[row - 1];
          const at = source || lined ? SOURCE_X : own;
          const slide =
            !source && stage === "play"
              ? { className: styles.slide, style: { "--from": `${own - SOURCE_X}px` } as CSSProperties }
              : {};
          return (
            <g key={row}>
              <text className={source ? styles.nameStrong : styles.name} x="0" y={y + 4}>
                {source ? sourceLabel : outputLabel}
              </text>
              {source ? (
                <text className={styles.tag} x="0" y={y + 17}>
                  {sourceTag}
                </text>
              ) : null}
              <line className={styles.axis} x1={AXIS_FROM} y1={y} x2={AXIS_TO} y2={y} />
              {!source ? (
                <>
                  {/* Where it would play on its own, and the wait that brings it to the mark. */}
                  <circle className={styles.ghost} cx={own} cy={y} r="5" />
                  <line className={styles.wait} x1={own + 7} y1={y} x2={SOURCE_X - 8} y2={y} />
                  {row === 1 ? (
                    <text className={styles.waitLabel} x={(own + SOURCE_X) / 2} y={y - 8} textAnchor="middle">
                      {waitLabel}
                    </text>
                  ) : null}
                </>
              ) : null}
              <g {...slide}>
                <circle className={source ? styles.dotSource : styles.dot} cx={at} cy={y} r="6.5" />
              </g>
            </g>
          );
        })}
      </svg>
    </div>
  );
}

/*
 * The first-run guide: five steps that take a new installation to a source, at least one
 * output and a delay that sounds right, and say why along the way.
 *
 * It owns no logic of its own. Each step shows the very parts of the Outputs, Sync and
 * Settings screens (`SourceSection`, `OutputsSection`, `SyncPanel`, `AutostartRow`), driving
 * the same store and the same commands, so nothing set up here can differ from what those
 * screens would have produced. What the guide adds is the order, the explanations and one
 * picture.
 *
 * Where it is (`settings.guideStep`) is saved with the settings: leaving the guide for another
 * screen, or closing Tandem, and coming back resumes at the same step. `null` means it has been
 * finished or skipped. Nothing here plays a sound: clicks and sweeps only start from the
 * switch and the button of the Sync part, as everywhere else.
 */

import type { ReactNode } from "react";
import { Notice } from "../common/Layout";
import { PrimaryButton, QuietButton } from "../common/Buttons";
import { OutputsSection, SourceSection } from "../outputs/OutputsView";
import { SyncPanel } from "../sync/SyncView";
import { AutostartRow } from "../settings/SettingsView";
import { ipc } from "../../lib/ipc";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";
import { Lineup } from "./Lineup";
import styles from "./GuideView.module.css";

export const GUIDE_STEPS = ["intro", "source", "outputs", "sync", "done"] as const;
type StepName = (typeof GUIDE_STEPS)[number];
const LAST = GUIDE_STEPS.length - 1;

export function GuideView() {
  const t = useT();
  const settings = useStore((s) => s.settings)!;
  const status = useStore((s) => s.status);
  const devices = useStore((s) => s.devices);
  const setView = useStore((s) => s.setView);

  // Reopened from Settings after it was finished: start again from the top.
  const index = Math.min(LAST, Math.max(0, settings.guideStep ?? 0));
  const step: StepName = GUIDE_STEPS[index];
  const go = (to: number) => void ipc.setPreferences({ guideStep: Math.min(LAST, Math.max(0, to)) });
  const finish = () => {
    void ipc.setPreferences({ guideStep: null });
    setView("outputs");
  };

  const hasOutputs = settings.outputs.length > 0;
  // Devices that could still be added: not the source, not already an output.
  const addable = devices.outputs.filter(
    (d) => d.id !== status?.source.deviceId && !settings.outputs.some((o) => o.device.id === d.id),
  );
  // One solid button per screen: where the step shows its own (the empty list's "Add
  // output", the Sync part's "Run auto-sync"), moving on is the quiet one.
  const ownsAccent = (step === "outputs" && !hasOutputs) || step === "sync";
  // A step whose point has not been done yet says so on the way out.
  const skipping = step === "outputs" && !hasOutputs;
  const forward =
    step === "done"
      ? t((d) => d.guide.finish)
      : step === "intro"
        ? t((d) => d.guide.start)
        : skipping
          ? t((d) => d.guide.skipStep)
          : t((d) => d.guide.next);
  const onForward = step === "done" ? finish : () => go(index + 1);
  const Forward = ownsAccent ? QuietButton : PrimaryButton;

  const picture = (stage: "apart" | "together" | "play") => (
    <Lineup
      stage={stage}
      sourceLabel={t((d) => d.guide.vizSource)}
      sourceTag={t((d) => d.guide.vizSlowest)}
      outputLabel={t((d) => d.guide.vizOutput)}
      waitLabel={t((d) => d.guide.vizWaits)}
    />
  );

  const body: Record<StepName, ReactNode> = {
    intro: (
      <>
        {/* Keyed so that coming back to this step plays the picture again. */}
        <div key="intro">{picture("play")}</div>
        <Prose>
          <p>{t((d) => d.guide.intro.p1)}</p>
          <p>{t((d) => d.guide.intro.p2)}</p>
          <p>{t((d) => d.guide.intro.p3)}</p>
        </Prose>
      </>
    ),
    source: (
      <>
        {picture("apart")}
        <Prose>
          <p>{t((d) => d.guide.source.p1)}</p>
          <p>{t((d) => d.guide.source.p2)}</p>
        </Prose>
        <SourceSection />
      </>
    ),
    outputs: (
      <>
        <Prose>
          <p>{t((d) => d.guide.outputs.p1)}</p>
        </Prose>
        {!hasOutputs && addable.length === 0 ? (
          <Notice
            tone="warn"
            action={
              <QuietButton onClick={() => void ipc.openSoundSettings()}>{t((d) => d.outputs.openSound)}</QuietButton>
            }
          >
            {t((d) => d.guide.outputs.noDevices)}
          </Notice>
        ) : null}
        <OutputsSection />
      </>
    ),
    sync: (
      <>
        <Prose>
          <p>{t((d) => d.guide.sync.p1)}</p>
        </Prose>
        {hasOutputs && devices.inputs.length === 0 ? <Notice>{t((d) => d.guide.sync.noMic)}</Notice> : null}
        <SyncPanel onNeedOutputs={() => go(GUIDE_STEPS.indexOf("outputs"))} />
      </>
    ),
    done: (
      <>
        {picture("together")}
        <Prose>
          <p>{hasOutputs ? t((d) => d.guide.done.p1) : t((d) => d.guide.done.p1Empty)}</p>
          <p>{t((d) => d.guide.done.p2)}</p>
        </Prose>
        <div className={styles.rows}>
          <AutostartRow />
        </div>
      </>
    ),
  };

  return (
    <div className={styles.view}>
      <header className={styles.head}>
        <div className={styles.headText}>
          <p className={styles.eyebrow}>{t((d) => d.guide.eyebrow, { n: index + 1, total: GUIDE_STEPS.length })}</p>
          <h1 className={styles.title}>{t((d) => d.guide[step].title)}</h1>
        </div>
        <QuietButton onClick={finish}>{t((d) => d.guide.skipAll)}</QuietButton>
      </header>

      <ol className={styles.steps}>
        {GUIDE_STEPS.map((name, i) => (
          <li key={name}>
            <button
              type="button"
              className={styles.step}
              data-state={i === index ? "current" : i < index ? "past" : "ahead"}
              aria-current={i === index ? "step" : undefined}
              onClick={() => go(i)}
            >
              <i aria-hidden="true">{i + 1}</i>
              {t((d) => d.guide.steps[name])}
            </button>
          </li>
        ))}
      </ol>

      {/* Keyed by step so each one comes in the same way, and the Sync part resets with it. */}
      <div className={styles.body} key={step}>
        {body[step]}
      </div>

      <footer className={styles.foot}>
        {index > 0 ? <QuietButton onClick={() => go(index - 1)}>{t((d) => d.guide.back)}</QuietButton> : <span />}
        <Forward onClick={onForward}>{forward}</Forward>
      </footer>
    </div>
  );
}

/** Explanatory text of a step: a reading column, not the full width of the window. */
function Prose({ children }: { children: ReactNode }) {
  return <div className={styles.prose}>{children}</div>;
}

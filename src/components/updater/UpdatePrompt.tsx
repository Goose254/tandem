import { PrimaryButton, QuietButton } from "../common/Buttons";
import { useStore } from "../../state/store";
import { updateOf, useUpdater } from "../../state/updater";
import { useT } from "../../i18n";
import styles from "./UpdatePrompt.module.css";

const EXCERPT_LINES = 4;
const EXCERPT_CHARS = 280;

/** The first few lines of the release notes as plain text: no headings, links or list marks. */
export function notesExcerpt(notes: string): string[] {
  const lines = notes
    .split(/\r?\n/)
    .map((line) =>
      line
        .replace(/^\s*(#{1,6}|[-*+]|\d+\.)\s+/, "")
        .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
        .replace(/[*_`]/g, "")
        .trim(),
    )
    .filter((line) => line && !/^(what's changed|full changelog)/i.test(line));
  const out: string[] = [];
  let budget = EXCERPT_CHARS;
  for (const line of lines.slice(0, EXCERPT_LINES)) {
    if (budget <= 0) break;
    out.push(line.length > budget ? `${line.slice(0, budget).trimEnd()}…` : line);
    budget -= line.length;
  }
  return out;
}

/**
 * The update prompt: a card in the window's lower corner that never blocks the app. It names
 * the new version over a short excerpt of its notes, with Later, Skip this version and Update
 * now. Update now turns the card into a progress bar until the installer takes over and Tandem
 * restarts. A failed install shows its reason with a retry. Mounted once, in the main window.
 */
export function UpdatePrompt() {
  const t = useT();
  const state = useUpdater((s) => s.state);
  const open = useUpdater((s) => s.promptOpen);
  const install = useUpdater((s) => s.install);
  const later = useUpdater((s) => s.later);
  const skip = useUpdater((s) => s.skip);
  const skipped = useStore((s) => s.settings?.skippedUpdate ?? null);

  const update = updateOf(state);
  if (!open || !update) return null;
  const notes = notesExcerpt(update.notes);

  return (
    <div className={styles.card} role="status" aria-live="polite">
      {state.kind === "downloading" || state.kind === "ready" ? (
        <>
          <p className={styles.title}>{t((d) => d.updater.downloading, { version: update.version })}</p>
          <Progress value={state.kind === "ready" ? 1 : state.progress} />
          <p className={styles.note}>{t((d) => d.updater.restartNote)}</p>
        </>
      ) : state.kind === "error" ? (
        <>
          <p className={styles.title}>{t((d) => d.updater.failedTitle, { version: update.version })}</p>
          <p className={styles.error}>{state.message}</p>
          <div className={styles.actions}>
            <QuietButton onClick={later}>{t((d) => d.updater.later)}</QuietButton>
            <PrimaryButton onClick={() => void install()}>{t((d) => d.updater.retry)}</PrimaryButton>
          </div>
        </>
      ) : (
        <>
          <p className={styles.title}>{t((d) => d.updater.title, { version: update.version })}</p>
          {notes.length > 0 ? (
            <ul className={styles.notes}>
              {notes.map((line, i) => (
                <li key={i}>{line}</li>
              ))}
            </ul>
          ) : (
            <p className={styles.note}>{t((d) => d.updater.noNotes)}</p>
          )}
          <div className={styles.actions}>
            <QuietButton onClick={later}>{t((d) => d.updater.later)}</QuietButton>
            {skipped === update.version ? null : (
              <QuietButton onClick={skip}>{t((d) => d.updater.skip)}</QuietButton>
            )}
            <PrimaryButton onClick={() => void install()}>{t((d) => d.updater.install)}</PrimaryButton>
          </div>
        </>
      )}
    </div>
  );
}

/** A thin progress bar; `null` shows an indeterminate sweep. */
function Progress({ value }: { value: number | null }) {
  return (
    <div
      className={styles.progress}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value == null ? undefined : Math.round(value * 100)}
      data-indeterminate={value == null || undefined}
    >
      <i style={value == null ? undefined : { width: `${Math.round(value * 100)}%` }} />
    </div>
  );
}

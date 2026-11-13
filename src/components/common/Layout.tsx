import type { ReactNode } from "react";
import { AlertTriangle, Info } from "lucide-react";
import mark from "../../assets/tandem-mark.svg";
import styles from "./Layout.module.css";

/** The Tandem mark. Decorative: the wordmark or surrounding label carries the name. */
export function Logo({ size = 28 }: { size?: number }) {
  return <img src={mark} alt="" aria-hidden="true" width={size} height={size} className={styles.logo} />;
}

/** A view's titled head, with an optional trailing control. */
export function ViewHeader({ title, sub, trailing }: { title: string; sub?: string; trailing?: ReactNode }) {
  return (
    <header className={styles.head}>
      <div className={styles.headText}>
        <h1 className={styles.title}>{title}</h1>
        {sub ? <p className={styles.sub}>{sub}</p> : null}
      </div>
      {trailing ? <div className={styles.headTrailing}>{trailing}</div> : null}
    </header>
  );
}

/** A section: a dimmed uppercase micro-label over its content, parted by space alone. */
export function Section({ label, trailing, children }: { label: string; trailing?: ReactNode; children: ReactNode }) {
  return (
    <section className={styles.section}>
      <div className={styles.sectionHead}>
        <h2 className={styles.label}>{label}</h2>
        {trailing}
      </div>
      {children}
    </section>
  );
}

/** One preference: label leading, control trailing, an optional helper beneath. */
export function Row({ label, helper, children }: { label: ReactNode; helper?: string; children: ReactNode }) {
  return (
    <div className={styles.pref}>
      <div className={styles.row}>
        <span className={styles.rowLabel}>{label}</span>
        <div className={styles.control}>{children}</div>
      </div>
      {helper ? <p className={styles.helper}>{helper}</p> : null}
    </div>
  );
}

/** A soft recess gathering one live device's controls (the source, each output). */
export function Card({ children, dim = false }: { children: ReactNode; dim?: boolean }) {
  return <div className={`${styles.card} ${dim ? styles.cardDim : ""}`}>{children}</div>;
}

/** A centered terminal state: a tone dot, a title, a line, an optional action. */
export function EmptyState({ title, line, action }: { title: string; line: string; action?: ReactNode }) {
  return (
    <div className={styles.empty}>
      <Logo size={44} />
      <p className={styles.emptyTitle}>{title}</p>
      <p className={styles.emptyLine}>{line}</p>
      {action ? <div className={styles.emptyAction}>{action}</div> : null}
    </div>
  );
}

/** The three note colours of the mark, in the order they sit round the ring. */
const NOTE_COLORS = ["#1e9cc0", "#54b24c", "#7a56d0"];

/**
 * The app's busy indicator: the mark, its three notes stepping round the centre point a third
 * of a turn at a time. Same geometry as `src/assets/tandem-mark.svg` (one note drawn three
 * times), so it must follow the mark if the mark ever changes. It only shows while something
 * is in progress; with reduced motion it is the still mark.
 */
export function SyncSpinner({ size = 22 }: { size?: number }) {
  return (
    <svg className={styles.spinner} viewBox="0 0 256 256" width={size} height={size} aria-hidden="true">
      <g className={styles.spinnerTurn}>
        {NOTE_COLORS.map((color, i) => (
          <g key={color} transform={`rotate(${i * 120} 128 128)`} stroke={color} fill={color}>
            <path d="M110 28 A102 102 0 0 1 229 114" fill="none" strokeWidth="18" strokeLinecap="round" />
            <path d="M110 28 L110 100" fill="none" strokeWidth="18" strokeLinecap="round" />
            <ellipse cx="91" cy="104" rx="27" ry="20" transform="rotate(-24 91 104)" stroke="none" />
          </g>
        ))}
      </g>
      <circle cx="128" cy="128" r="13" fill="#f2a81e" />
    </svg>
  );
}

/**
 * A quiet inline note: an icon and a line of text, no box. Warn tone for something to fix.
 * `icon` replaces the default one when a drawing says what kind of thing it is.
 */
export function Notice({
  tone = "info",
  icon,
  children,
  action,
}: {
  tone?: "info" | "warn";
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className={`${styles.notice} ${tone === "warn" ? styles.noticeWarn : ""}`} data-notice={tone}>
      {icon ?? (tone === "warn" ? <AlertTriangle size={14} /> : <Info size={14} />)}
      <p>{children}</p>
      {action ? <div className={styles.noticeAction}>{action}</div> : null}
    </div>
  );
}

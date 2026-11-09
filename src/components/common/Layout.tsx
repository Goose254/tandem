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

/** The mark's three bars sliding into alignment: the app's busy indicator. */
export function SyncSpinner({ size = 22 }: { size?: number }) {
  return (
    <span className={styles.spinner} style={{ width: size, height: size }} aria-hidden="true">
      <i />
      <i />
      <i />
    </span>
  );
}

/** A quiet inline note: an icon and a line of text, no box. Warn tone for something to fix. */
export function Notice({ tone = "info", children, action }: { tone?: "info" | "warn"; children: ReactNode; action?: ReactNode }) {
  return (
    <div className={`${styles.notice} ${tone === "warn" ? styles.noticeWarn : ""}`}>
      {tone === "warn" ? <AlertTriangle size={14} /> : <Info size={14} />}
      <p>{children}</p>
      {action ? <div className={styles.noticeAction}>{action}</div> : null}
    </div>
  );
}

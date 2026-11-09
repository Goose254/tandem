import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Check, ChevronDown } from "lucide-react";
import styles from "./Controls.module.css";

// -- SegmentedControl --------------------------------------------------------------------

export type Segment<T extends string> = { value: T; label: ReactNode; title?: string };

/** A soft recess track holding chips; the active chip lifts onto the surface, never the accent. */
export function SegmentedControl<T extends string>({
  value,
  segments,
  onChange,
  ariaLabel,
}: {
  value: T;
  segments: Segment<T>[];
  onChange: (value: T) => void;
  ariaLabel: string;
}) {
  return (
    <div className={styles.segmented} role="radiogroup" aria-label={ariaLabel}>
      {segments.map((s) => (
        <button
          key={s.value}
          type="button"
          role="radio"
          aria-checked={s.value === value}
          title={s.title}
          className={`${styles.segment} ${s.value === value ? styles.segmentActive : ""}`}
          onClick={() => onChange(s.value)}
        >
          <span className={styles.segmentLabel}>{s.label}</span>
        </button>
      ))}
    </div>
  );
}

// -- Switch ------------------------------------------------------------------------------

export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={label}
      disabled={disabled}
      className={styles.switch}
      onClick={() => onChange(!checked)}
    >
      <span className={styles.thumb} />
    </button>
  );
}

// -- Select ------------------------------------------------------------------------------

export type SelectOption = { value: string; label: string; hint?: string };

/** A single-select that reads as a quiet field over a floating listbox. */
export function Select({
  value,
  options,
  onChange,
  ariaLabel,
  placeholder,
  onOpen,
  trigger,
  align = "end",
  wide = false,
}: {
  value: string | null;
  options: SelectOption[];
  onChange: (value: string) => void;
  ariaLabel: string;
  placeholder?: string;
  onOpen?: () => void;
  /** Replace the default field trigger (e.g. an "Add output" button). */
  trigger?: (open: boolean) => ReactNode;
  align?: "start" | "end";
  /** Let the field grow to show a long label with its hint. */
  wide?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const current = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", key);
    };
  }, [open]);

  const toggle = () => {
    if (!open) onOpen?.();
    setOpen(!open);
  };

  return (
    <div className={styles.selectWrap} ref={wrap}>
      {trigger ? (
        <span onClick={toggle} aria-haspopup="listbox" aria-expanded={open}>
          {trigger(open)}
        </span>
      ) : (
        <button
          type="button"
          className={`${styles.trigger} ${wide ? styles.wide : ""}`}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-label={ariaLabel}
          onClick={toggle}
        >
          <span className={styles.current}>
            <span className={current ? styles.value : `${styles.value} ${styles.placeholder}`}>
              {current?.label ?? placeholder ?? ""}
            </span>
            {current?.hint ? <span className={styles.hint}>{current.hint}</span> : null}
          </span>
          <ChevronDown size={15} className={`${styles.chevron} ${open ? styles.chevronOpen : ""}`} />
        </button>
      )}
      {open ? (
        <div className={`${styles.menu} ${align === "start" ? styles.menuStart : ""}`}>
          <ul className={styles.list} role="listbox" aria-label={ariaLabel}>
            {options.map((o) => (
              <li key={o.value}>
                <button
                  type="button"
                  role="option"
                  aria-selected={o.value === value}
                  className={`${styles.option} ${o.value === value ? styles.optionActive : ""}`}
                  onClick={() => {
                    setOpen(false);
                    onChange(o.value);
                  }}
                >
                  <span className={styles.optionLabel}>{o.label}</span>
                  {o.hint ? <span className={styles.hint}>{o.hint}</span> : null}
                  {o.value === value ? <Check size={14} className={styles.check} /> : null}
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

// -- Slider ------------------------------------------------------------------------------

export function Slider({
  value,
  min,
  max,
  step = 1,
  onChange,
  ariaLabel,
  disabled = false,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  ariaLabel: string;
  disabled?: boolean;
}) {
  const pct = ((Math.min(Math.max(value, min), max) - min) / (max - min)) * 100;
  return (
    <input
      type="range"
      className={styles.slider}
      min={min}
      max={max}
      step={step}
      value={value}
      disabled={disabled}
      aria-label={ariaLabel}
      style={{ ["--fill" as string]: `${pct}%` }}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}

// -- Meter -------------------------------------------------------------------------------

/** A thin level bar. `level` is linear peak 0..1, shown on a -60..0 dB scale. */
export function Meter({ level, muted = false }: { level: number; muted?: boolean }) {
  const db = level > 0 ? 20 * Math.log10(level) : -100;
  const pct = Math.max(0, Math.min(1, (db + 60) / 60)) * 100;
  return (
    <span className={styles.meter} data-muted={muted || undefined} aria-hidden="true">
      <i style={{ width: `${pct}%` }} />
    </span>
  );
}

// -- StatusDot ---------------------------------------------------------------------------

export type Tone = "good" | "busy" | "warn" | "danger" | "idle";

export function StatusDot({ tone }: { tone: Tone }) {
  return <span className={`${styles.dot} ${styles[tone]}`} aria-hidden="true" />;
}

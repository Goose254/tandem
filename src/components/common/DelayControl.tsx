import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from "lucide-react";
import { IconButton } from "./Buttons";
import { Slider } from "./Controls";
import { MAX_DELAY_MS, MIN_DELAY_MS } from "../../lib/ipc";
import { useT } from "../../i18n";
import styles from "./DelayControl.module.css";

const SLIDER_MAX = 800;

/**
 * The delay control: ±10 / ±1 steppers around an editable readout, over a slider. It holds a
 * local value while the user drags so the readout never lags, and sends at most ~25 updates
 * per second; the engine applies each one immediately.
 */
export function DelayControl({
  value,
  onChange,
  large = false,
  disabled = false,
}: {
  value: number;
  onChange: (ms: number) => void;
  large?: boolean;
  disabled?: boolean;
}) {
  const t = useT();
  const [local, setLocal] = useState(value);
  const [editing, setEditing] = useState<string | null>(null);
  const pending = useRef<number | null>(null);
  const timer = useRef<number | null>(null);
  const lastSent = useRef(value);

  // Follow the persisted value unless a local edit is in flight.
  useEffect(() => {
    if (pending.current == null) setLocal(value);
  }, [value]);

  const send = (ms: number) => {
    const clamped = Math.round(Math.min(MAX_DELAY_MS, Math.max(MIN_DELAY_MS, ms)));
    setLocal(clamped);
    pending.current = clamped;
    if (timer.current != null) return;
    timer.current = window.setTimeout(() => {
      timer.current = null;
      const v = pending.current;
      pending.current = null;
      if (v != null && v !== lastSent.current) {
        lastSent.current = v;
        onChange(v);
      }
    }, 40);
  };

  const commitEdit = () => {
    if (editing == null) return;
    const n = Number(editing.replace(",", "."));
    setEditing(null);
    if (Number.isFinite(n)) send(n);
  };

  const label = t((d) => d.outputs.delay);
  return (
    <div className={`${styles.wrap} ${large ? styles.large : ""}`}>
      <div className={styles.stepper}>
        <IconButton label="-10 ms" small onClick={() => send(local - 10)} disabled={disabled}>
          <ChevronsLeft size={large ? 18 : 15} />
        </IconButton>
        <IconButton label="-1 ms" small onClick={() => send(local - 1)} disabled={disabled}>
          <ChevronLeft size={large ? 18 : 15} />
        </IconButton>
        <label className={styles.readout}>
          <input
            className={`${styles.input} tabular`}
            value={editing ?? String(local)}
            aria-label={label}
            inputMode="numeric"
            disabled={disabled}
            onFocus={(e) => {
              setEditing(String(local));
              e.currentTarget.select();
            }}
            onChange={(e) => setEditing(e.target.value)}
            onBlur={commitEdit}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") {
                setEditing(null);
                e.currentTarget.blur();
              }
              if (e.key === "ArrowUp" || e.key === "ArrowDown") {
                e.preventDefault();
                const step = (e.shiftKey ? 10 : 1) * (e.key === "ArrowUp" ? 1 : -1);
                setEditing(null);
                send(local + step);
              }
            }}
          />
          <span className={styles.unit}>{t((d) => d.units.ms)}</span>
        </label>
        <IconButton label="+1 ms" small onClick={() => send(local + 1)} disabled={disabled}>
          <ChevronRight size={large ? 18 : 15} />
        </IconButton>
        <IconButton label="+10 ms" small onClick={() => send(local + 10)} disabled={disabled}>
          <ChevronsRight size={large ? 18 : 15} />
        </IconButton>
      </div>
      <div className={styles.slider}>
        <Slider
          value={Math.min(local, SLIDER_MAX)}
          min={MIN_DELAY_MS}
          max={SLIDER_MAX}
          onChange={send}
          ariaLabel={label}
          disabled={disabled}
        />
      </div>
    </div>
  );
}

import type { ButtonHTMLAttributes, ReactNode } from "react";
import styles from "./Buttons.module.css";

type Props = ButtonHTMLAttributes<HTMLButtonElement> & { children?: ReactNode };

/** The single solid-accent action. One per screen. */
export function PrimaryButton({ className = "", ...rest }: Props) {
  return <button type="button" className={`${styles.primary} ${className}`} {...rest} />;
}

/** A control that dissolves until touched: transparent at rest, a veil on hover. */
export function QuietButton({
  destructive = false,
  className = "",
  ...rest
}: Props & { destructive?: boolean }) {
  return (
    <button
      type="button"
      className={`${styles.quiet} ${destructive ? styles.destructive : ""} ${className}`}
      {...rest}
    />
  );
}

/** Icon-only quiet control. Always labelled for assistive tech. */
export function IconButton({
  label,
  small = false,
  className = "",
  ...rest
}: Props & { label: string; small?: boolean }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={`${styles.icon} ${small ? styles.small : ""} ${className}`}
      {...rest}
    />
  );
}

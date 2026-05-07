import { useState, type ReactNode } from "react";

interface RevealOnHoldProps {
  /** The sensitive value to reveal on hold (e.g. a passphrase). */
  value: string;
  /** Optional renderer for the masked form. By default we keep word
   *  separators (`-`, ` `) intact and replace every other char with `*` so
   *  the user can see the structure (e.g. how many words) without the
   *  content. */
  mask?: (value: string) => string;
  /** Optional class for the container `<span>`. */
  className?: string;
  /** Render-prop for the visible content ; receives the displayed string
   *  (masked or revealed) so the caller can apply its own styling around it.
   *  Defaults to wrapping it in a plain span. */
  children?: (display: string, revealed: boolean) => ReactNode;
}

function defaultMask(value: string): string {
  return value
    .split("")
    .map((c) => (c === "-" || c === " " ? c : "*"))
    .join("");
}

/** Hide a sensitive value behind asterisks ; reveal while the eye button
 *  is held down (mouse, touch, or keyboard space/enter). Auto-hides as
 *  soon as the press is released — there's no toggle, so the secret can
 *  never be left visible by accident.
 *
 *  Used for the room passphrase ; can be reused for any other low-stakes
 *  shared secret displayed in the UI. */
export function RevealOnHold({
  value,
  mask = defaultMask,
  className = "",
  children,
}: RevealOnHoldProps) {
  const [revealed, setRevealed] = useState(false);
  const display = revealed ? value : mask(value);

  const start = () => setRevealed(true);
  const stop = () => setRevealed(false);

  return (
    <span className={`reveal-on-hold ${className}`}>
      {children ? (
        children(display, revealed)
      ) : (
        <span className="reveal-on-hold__value">{display}</span>
      )}
      <button
        type="button"
        className="reveal-on-hold__btn"
        title="Maintenez pour afficher"
        aria-label={revealed ? "Cacher la valeur" : "Maintenir pour afficher la valeur"}
        aria-pressed={revealed}
        onMouseDown={start}
        onMouseUp={stop}
        onMouseLeave={stop}
        onTouchStart={(e) => {
          e.preventDefault(); // suppress the synthesised mousedown that follows
          start();
        }}
        onTouchEnd={stop}
        onTouchCancel={stop}
        onKeyDown={(e) => {
          if (e.key === " " || e.key === "Enter") {
            e.preventDefault();
            start();
          }
        }}
        onKeyUp={(e) => {
          if (e.key === " " || e.key === "Enter") {
            e.preventDefault();
            stop();
          }
        }}
        onBlur={stop}
      >
        {revealed ? <EyeOff /> : <EyeOn />}
      </button>
    </span>
  );
}

function EyeOn() {
  // Outline eye, visible when the value is currently masked (i.e. press to reveal)
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function EyeOff() {
  // Eye with a slash, visible while the value is being revealed (release to hide)
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M17.94 17.94A10.94 10.94 0 0 1 12 20c-7 0-11-8-11-8a19.78 19.78 0 0 1 4.22-5.36" />
      <path d="M9.9 4.24A10.94 10.94 0 0 1 12 4c7 0 11 8 11 8a19.86 19.86 0 0 1-3.16 4.19" />
      <path d="M14.12 14.12A3 3 0 1 1 9.88 9.88" />
      <line x1="1" y1="1" x2="23" y2="23" />
    </svg>
  );
}

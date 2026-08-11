// Custom pointer-driven faders (WKWebView's native vertical range inputs are
// unreliable, so these are plain divs).

import { useCallback, useRef } from "react";

interface FaderProps {
  value: number; // 0..1
  onChange: (v: number) => void;
  vertical?: boolean;
  className?: string;
  accent?: string;
  /** Double-click resets to this value if set. */
  resetTo?: number;
}

export default function Fader({
  value,
  onChange,
  vertical = false,
  className = "",
  accent = "#8ab4f8",
  resetTo,
}: FaderProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  const valueFromEvent = useCallback(
    (e: PointerEvent | React.PointerEvent) => {
      const track = trackRef.current;
      if (!track) return null;
      const rect = track.getBoundingClientRect();
      const frac = vertical
        ? 1 - (e.clientY - rect.top) / rect.height
        : (e.clientX - rect.left) / rect.width;
      return Math.max(0, Math.min(1, frac));
    },
    [vertical],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const v = valueFromEvent(e);
    if (v !== null) onChange(v);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const v = valueFromEvent(e);
    if (v !== null) onChange(v);
  };

  const pct = Math.max(0, Math.min(1, value)) * 100;

  return (
    <div
      ref={trackRef}
      className={`fader ${vertical ? "fader-v" : "fader-h"} ${className}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onDoubleClick={() => resetTo !== undefined && onChange(resetTo)}
    >
      <div className="fader-track" />
      <div
        className="fader-thumb"
        style={
          vertical
            ? { bottom: `calc(${pct}% - 9px)`, borderColor: accent }
            : { left: `calc(${pct}% - 9px)`, borderColor: accent }
        }
      />
    </div>
  );
}

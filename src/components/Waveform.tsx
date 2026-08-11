// Canvas waveform views. Both draw in a requestAnimationFrame loop using the
// extrapolated playhead so motion is smooth between 30 Hz engine updates.

import { useEffect, useRef } from "react";
import { deckPosition, getEngineSnapshot } from "../engine";

export function useCanvas(
  draw: (ctx: CanvasRenderingContext2D, w: number, h: number) => void,
) {
  const ref = useRef<HTMLCanvasElement>(null);
  const drawRef = useRef(draw);
  drawRef.current = draw;

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    let raf = 0;
    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const { width, height } = canvas.getBoundingClientRect();
      const w = Math.max(1, Math.round(width * dpr));
      const h = Math.max(1, Math.round(height * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
    };
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    const loop = () => {
      resize();
      drawRef.current(ctx, canvas.width, canvas.height);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, []);

  return ref;
}

const CUE_COLORS = ["#ffd54f", "#81c784", "#ba68c8", "#4dd0e1"];

function drawCues(
  ctx: CanvasRenderingContext2D,
  cues: (number | null)[],
  toX: (secs: number) => number | null,
  h: number,
  labels?: (string | null)[],
) {
  cues.forEach((cue, i) => {
    if (cue === null) return;
    const x = toX(cue);
    if (x === null) return;
    ctx.fillStyle = CUE_COLORS[i % CUE_COLORS.length];
    ctx.fillRect(x - 1, 0, 2, h);
    ctx.fillRect(x - 1, 0, 8, 8);
    const label = labels?.[i];
    if (label) {
      ctx.font = `${Math.round(h * 0.16 + 6)}px sans-serif`;
      ctx.textBaseline = "top";
      ctx.fillText(label, x + 9, 1, 120);
    }
  });
}

/// `toX` maps seconds to an x clamped into [0, w]; a zero-width result means
/// the loop is outside the view.
function drawLoop(
  ctx: CanvasRenderingContext2D,
  loop: [number, number] | null,
  toX: (secs: number) => number | null,
  w: number,
  h: number,
) {
  if (!loop) return;
  const clamp = (secs: number) => {
    const x = toX(secs);
    return x === null ? null : Math.max(0, Math.min(w, x));
  };
  const a = clamp(loop[0]);
  const b = clamp(loop[1]);
  if (a === null || b === null || b - a < 1) return;
  ctx.fillStyle = "rgba(126, 226, 168, 0.18)";
  ctx.fillRect(a, 0, b - a, h);
  ctx.fillStyle = "#7ee2a8";
  ctx.fillRect(a, 0, 2, h);
  ctx.fillRect(b - 2, 0, 2, h);
}

interface OverviewProps {
  deck: number;
  peaks: Float32Array | null;
  duration: number;
  cues: (number | null)[];
  cueLabels: (string | null)[];
  loop: [number, number] | null;
  accent: string;
  onSeek: (seconds: number) => void;
}

export function OverviewWaveform({
  deck,
  peaks,
  duration,
  cues,
  cueLabels,
  loop,
  accent,
  onSeek,
}: OverviewProps) {
  const ref = useCanvas((ctx, w, h) => {
    ctx.clearRect(0, 0, w, h);
    if (!peaks || duration <= 0) return;
    const pos = Math.min(deckPosition(deck), duration);
    const playedX = (pos / duration) * w;
    const mid = h / 2;
    const bins = peaks.length;
    for (let x = 0; x < w; x++) {
      const amp = peaks[Math.min(bins - 1, Math.floor((x / w) * bins))];
      const half = Math.max(1, amp * mid * 0.95);
      ctx.fillStyle = x <= playedX ? accent : "rgba(255,255,255,0.28)";
      ctx.fillRect(x, mid - half, 1, half * 2);
    }
    const toX = (secs: number) => (secs / duration) * w;
    drawLoop(ctx, loop, toX, w, h);
    drawCues(ctx, cues, toX, h, cueLabels);
    // Playhead
    ctx.fillStyle = "#fff";
    ctx.fillRect(playedX - 1, 0, 2, h);
  });

  return (
    <canvas
      ref={ref}
      className="wave-overview"
      onPointerDown={(e) => {
        if (!peaks || duration <= 0) return;
        const rect = e.currentTarget.getBoundingClientRect();
        const frac = (e.clientX - rect.left) / rect.width;
        onSeek(Math.max(0, Math.min(1, frac)) * duration);
      }}
    />
  );
}

interface ZoomProps {
  deck: number;
  detail: Float32Array | null;
  binsPerSec: number;
  duration: number;
  cues: (number | null)[];
  cueLabels: (string | null)[];
  loop: [number, number] | null;
  accent: string;
}

const WINDOW_SECS = 8;

export function ZoomWaveform({
  deck,
  detail,
  binsPerSec,
  duration,
  cues,
  cueLabels,
  loop,
  accent,
}: ZoomProps) {
  const ref = useCanvas((ctx, w, h) => {
    ctx.clearRect(0, 0, w, h);
    if (!detail || duration <= 0) return;
    const pos = Math.min(deckPosition(deck), duration);
    const mid = h / 2;
    const pxPerSec = w / WINDOW_SECS;
    const t0 = pos - WINDOW_SECS / 2;
    const playing = getEngineSnapshot().decks[deck]?.playing;

    for (let x = 0; x < w; x++) {
      const t = t0 + x / pxPerSec;
      if (t < 0 || t > duration) continue;
      const bin = Math.floor(t * binsPerSec);
      if (bin < 0 || bin >= detail.length) continue;
      const amp = detail[bin];
      const half = Math.max(1, amp * mid * 0.95);
      const behind = t <= pos;
      ctx.fillStyle = behind ? accent : "rgba(255,255,255,0.35)";
      ctx.fillRect(x, mid - half, 1, half * 2);
    }

    // Second markers along the bottom
    ctx.fillStyle = "rgba(255,255,255,0.25)";
    for (let s = Math.ceil(t0); s <= t0 + WINDOW_SECS; s++) {
      if (s < 0 || s > duration) continue;
      const x = (s - t0) * pxPerSec;
      ctx.fillRect(x, h - 6, 1, 6);
    }

    const toXVisible = (secs: number) =>
      secs >= t0 && secs <= t0 + WINDOW_SECS ? (secs - t0) * pxPerSec : null;
    // Loop uses unclamped mapping so a region spanning past the window edges
    // still fills the visible slice.
    drawLoop(ctx, loop, (secs) => (secs - t0) * pxPerSec, w, h);
    drawCues(ctx, cues, toXVisible, h, cueLabels);

    // Center playhead
    ctx.fillStyle = playing ? "#fff" : "rgba(255,255,255,0.8)";
    ctx.fillRect(w / 2 - 1, 0, 2, h);
  });

  return <canvas ref={ref} className="wave-zoom" />;
}

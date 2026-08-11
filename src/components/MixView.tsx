// The mix view: both decks' waveforms stacked on a shared time axis with
// their beat grids drawn as interlocking ticks, so phrase alignment is
// visible before it's audible. Also home of the quantize control and the
// phrase-position readout ("bar.beat" within the current 32-beat phrase).
//
// The horizontal axis is *output* time around now (the center line), so each
// lane's track time is scaled by that deck's playback rate — when two decks
// are tempo-synced their grids scroll at the same speed and the tick columns
// visibly lock together.

import { Analysis, deckPosition, getEngineSnapshot } from "../engine";
import type { DeckData } from "../App";
import { useCanvas } from "./Waveform";

interface Props {
  decks: [DeckData, DeckData];
  accents: string[];
  quant: number;
  onQuant: (q: number) => void;
}

/** Output-time window: seconds shown behind and ahead of the now line.
 * Ahead is deeper than behind because that's where the next phrase boundary
 * lives (a full 32-beat phrase is 16 s at 120 BPM). */
const BACK_SECS = 6;
const AHEAD_SECS = 18;
const WINDOW_SECS = BACK_SECS + AHEAD_SECS;

const QUANT_CHOICES: { label: string; beats: number; title: string }[] = [
  { label: "OFF", beats: 0, title: "Play starts immediately" },
  { label: "BEAT", beats: 1, title: "Play lands on the other deck's next beat" },
  { label: "BAR", beats: 4, title: "Play lands on the other deck's next bar (4 beats)" },
  {
    label: "PHRASE",
    beats: 32,
    title: "Play lands on the other deck's next 8-bar phrase boundary",
  },
];

/** Beats since the grid anchor, in track time. */
function beatsAt(t: number, a: Analysis): number {
  return ((t - a.beatOffset) * a.bpm) / 60;
}

/** "bar.beat" position within the current 32-beat phrase, 1-based — the
 * count a DJ keeps in their head ("3.2" = bar 3 of 8, beat 2). */
function phraseCount(t: number, a: Analysis): string {
  const beats = beatsAt(t, a);
  const inPhrase = ((beats % 32) + 32) % 32;
  return `${Math.floor(inPhrase / 4) + 1}.${(Math.floor(inPhrase) % 4) + 1}`;
}

export default function MixView({ decks, accents, quant, onQuant }: Props) {
  const ref = useCanvas((ctx, w, h) => {
    ctx.clearRect(0, 0, w, h);
    const snap = getEngineSnapshot();
    const laneH = h / 2;
    const nowX = (BACK_SECS / WINDOW_SECS) * w;
    const pxPerOutSec = w / WINDOW_SECS;
    const font = Math.max(10, Math.round(h * 0.11));

    for (let i = 0; i < 2; i++) {
      const d = decks[i];
      const laneY = i * laneH;
      const mid = laneY + laneH / 2;
      const accent = accents[i];
      if (!d.detail || !d.load) continue;
      const duration = d.load.durationSecs;
      const binsPerSec = d.load.detailBinsPerSec;
      const pos = Math.min(deckPosition(i), duration);
      const rate = d.rate;

      // Waveform, output-time scaled.
      for (let x = 0; x < w; x++) {
        const t = pos + ((x - nowX) / pxPerOutSec) * rate;
        if (t < 0 || t > duration) continue;
        const bin = Math.floor(t * binsPerSec);
        if (bin < 0 || bin >= d.detail.length) continue;
        const half = Math.max(1, d.detail[bin] * (laneH / 2) * 0.92);
        ctx.fillStyle = t <= pos ? accent : "rgba(255,255,255,0.30)";
        ctx.fillRect(x, mid - half, 1, half * 2);
      }

      const a = d.analysis;
      if (a && !a.fluid && a.bpm > 0) {
        // Beat grid: minor tick per beat, taller per bar, full-height line
        // per 32-beat phrase. Anchored to the analyser's grid.
        const beatLen = 60 / a.bpm;
        const t0 = Math.max(0, pos - BACK_SECS * rate);
        const t1 = Math.min(duration, pos + AHEAD_SECS * rate);
        const k0 = Math.ceil((t0 - a.beatOffset) / beatLen);
        const k1 = Math.floor((t1 - a.beatOffset) / beatLen);
        for (let k = k0; k <= k1; k++) {
          const t = a.beatOffset + k * beatLen;
          const x = nowX + ((t - pos) / rate) * pxPerOutSec;
          const km = ((k % 32) + 32) % 32;
          if (km === 0) {
            ctx.fillStyle = "rgba(126, 226, 168, 0.85)";
            ctx.fillRect(x - 1, laneY, 2, laneH);
          } else if (km % 4 === 0) {
            ctx.fillStyle = "rgba(255,255,255,0.55)";
            ctx.fillRect(x, laneY, 1, laneH * 0.45);
          } else {
            ctx.fillStyle = "rgba(255,255,255,0.22)";
            ctx.fillRect(x, laneY, 1, laneH * 0.2);
          }
        }

        // Phrase-count readout: where you are in the 8-bar phrase.
        ctx.font = `${font}px ui-monospace, monospace`;
        ctx.textBaseline = "top";
        ctx.fillStyle = accent;
        ctx.fillText(phraseCount(pos, a), 6, laneY + 4);
      } else if (a?.fluid) {
        ctx.font = `${font}px sans-serif`;
        ctx.textBaseline = "top";
        ctx.fillStyle = "rgba(255,255,255,0.4)";
        ctx.fillText("fluid tempo — no grid", 6, laneY + 4);
      }

      // Cue markers with their notes ("sax rip") for aiming an entry.
      d.cues.forEach((cue, ci) => {
        if (cue === null) return;
        const x = nowX + ((cue - pos) / rate) * pxPerOutSec;
        if (x < 0 || x > w) return;
        ctx.fillStyle = ["#ffd54f", "#81c784", "#ba68c8", "#4dd0e1"][ci];
        ctx.fillRect(x - 1, laneY, 2, laneH);
        const label = d.cueLabels[ci];
        if (label) {
          ctx.font = `${font - 1}px sans-serif`;
          ctx.textBaseline = "bottom";
          ctx.fillText(label, x + 4, laneY + laneH - 3, 110);
        }
      });
    }

    // Armed-start marker: the moment the waiting deck will enter, drawn
    // across both lanes at the master's target boundary.
    for (let i = 0; i < 2; i++) {
      const ds = snap.decks[i];
      if (!ds?.armed) continue;
      const master = 1 - i;
      const md = decks[master];
      if (!md.load) continue;
      const mPos = deckPosition(master);
      const x =
        nowX + ((ds.armedMasterSecs - mPos) / md.rate) * pxPerOutSec;
      if (x < 0 || x > w) continue;
      ctx.fillStyle = "#7ee2a8";
      ctx.fillRect(x - 1, 0, 2, h);
      ctx.font = `${font}px sans-serif`;
      ctx.textBaseline = "top";
      ctx.fillText(`${i === 0 ? "A" : "B"} IN`, x + 4, 2);
    }

    // Lane divider + now line.
    ctx.fillStyle = "rgba(255,255,255,0.15)";
    ctx.fillRect(0, laneH - 0.5, w, 1);
    ctx.fillStyle = "#fff";
    ctx.fillRect(nowX - 1, 0, 2, h);
  });

  return (
    <div className="mix-view">
      <div className="mix-head">
        <span className="mix-title">MIX</span>
        <span
          className="mix-q-label"
          title="Quantize: pressing play while the other deck runs waits for its next grid boundary, and both decks enter aligned"
        >
          Q
        </span>
        {QUANT_CHOICES.map((c) => (
          <button
            key={c.label}
            className={`btn btn-quant ${quant === c.beats ? "active" : ""}`}
            onClick={() => onQuant(c.beats)}
            title={c.title}
          >
            {c.label}
          </button>
        ))}
      </div>
      <canvas ref={ref} className="mix-canvas" />
    </div>
  );
}

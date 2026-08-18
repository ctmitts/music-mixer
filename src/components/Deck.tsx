import { useSyncExternalStore } from "react";
import {
  Analysis,
  clearLoop,
  formatTime,
  getEngineSnapshot,
  pause,
  quantize,
  seek,
  setLoop,
  shiftCamelot,
  subscribeEngine,
  TrackStats,
} from "../engine";
import type { DeckData } from "../App";
import Fader from "./Fader";
import { OverviewWaveform, ZoomWaveform } from "./Waveform";

interface Props {
  index: number;
  accent: string;
  data: DeckData;
  otherAnalysis: Analysis | null;
  stats: TrackStats | undefined;
  onTaste: (loved: boolean, banned: boolean) => void;
  onSetCue: (slot: number, seconds: number | null) => void;
  onSetCueLabel: (slot: number, label: string | null) => void;
  onPlay: () => void;
  onRate: (rate: number) => void;
  onSync: () => void;
  onLoop: (loop: [number, number] | null, loopIn: number | null) => void;
  onKeyLock: (enabled: boolean) => void;
  onFx: (fx: { filter: number; delay: number; reverb: number }) => void;
  onAgToggle: () => void;
  onVocal: (amount: number, isolate: boolean) => void;
  onKeyShift: (semitones: number) => void;
}

const CUE_COLORS = ["#ffd54f", "#81c784", "#ba68c8", "#4dd0e1"];

export default function Deck({
  index,
  accent,
  data,
  otherAnalysis,
  stats,
  onTaste,
  onSetCue,
  onSetCueLabel,
  onPlay,
  onRate,
  onSync,
  onLoop,
  onKeyLock,
  onFx,
  onAgToggle,
  onVocal,
  onKeyShift,
}: Props) {
  const snapshot = useSyncExternalStore(subscribeEngine, getEngineSnapshot);
  const deckSnap = snapshot.decks[index];
  const loaded = data.load !== null;
  const duration = data.load?.durationSecs ?? 0;
  const position = Math.min(deckSnap?.positionSecs ?? 0, duration);
  const playing = deckSnap?.playing ?? false;
  const name = index === 0 ? "A" : "B";
  const a = data.analysis;
  const effectiveBpm = a && !a.fluid ? a.bpm * data.rate : null;
  const canSync =
    loaded && a && !a.fluid && otherAnalysis && !otherAnalysis.fluid;

  const armed = deckSnap?.armed ?? false;

  const togglePlay = () => {
    if (!loaded) return;
    // Pausing an armed deck cancels the pending quantized start.
    if (playing || armed) pause(index);
    else onPlay();
  };

  const cueClick = (slot: number, e: React.MouseEvent) => {
    if (!loaded) return;
    if (e.shiftKey) {
      onSetCue(slot, null);
    } else if (e.altKey && data.cues[slot] !== null) {
      // Alt-click: name the cue ("sax rip", "vocals enter").
      const label = window.prompt(
        "Cue note (empty to clear):",
        data.cueLabels[slot] ?? "",
      );
      if (label !== null) onSetCueLabel(slot, label);
    } else if (data.cues[slot] === null) {
      onSetCue(slot, position);
    } else {
      seek(index, data.cues[slot]!);
    }
  };

  // Beat-based moves need a trustworthy grid; FLUID tracks don't have one.
  const beatLen = a && !a.fluid && a.bpm > 0 ? 60 / a.bpm : 0;
  const canBeat = loaded && beatLen > 0;

  /** Jump `beats` forward (or back), landing on the grid. */
  const beatJump = (beats: number) => {
    if (!canBeat) return;
    seek(index, Math.max(0, quantize(position, a) + beats * beatLen));
  };

  /** One-press loop of `beats` starting at the current beat. */
  const autoLoop = (beats: number) => {
    if (!canBeat) return;
    const start = quantize(position, a);
    const end = start + beats * beatLen;
    setLoop(index, start, end);
    onLoop([start, end], start);
  };

  const loopIn = () => loaded && onLoop(data.loop, position);
  const loopOut = () => {
    if (!loaded || data.loopIn === null || position <= data.loopIn) return;
    setLoop(index, data.loopIn, position);
    onLoop([data.loopIn, position], data.loopIn);
  };
  const loopExit = () => {
    clearLoop(index);
    onLoop(null, null);
  };

  return (
    <section className={`deck deck-${name.toLowerCase()}`}>
      <header className="deck-header">
        <div className="deck-badge" style={{ background: accent }}>
          {name}
        </div>
        {data.artwork ? (
          <img className="deck-art" src={data.artwork} alt="" />
        ) : (
          <div className="deck-art deck-art-empty" />
        )}
        <div className="deck-titles">
          <div className="deck-title">
            {data.meta?.title ?? (data.loading ? "Loading…" : "No track loaded")}
          </div>
          <div className="deck-artist">{data.meta?.artist ?? ""}</div>
          {loaded && (
            <div className="deck-taste">
              <button
                className={`btn btn-taste ${stats?.loved ? "loved" : ""}`}
                onClick={() => onTaste(!stats?.loved, false)}
                title={stats?.loved ? "Loved — click to unset" : "Mark as loved"}
              >
                {stats?.loved ? "♥" : "♡"}
              </button>
              <button
                className={`btn btn-taste ${stats?.banned ? "banned" : ""}`}
                onClick={() => onTaste(false, !stats?.banned)}
                title={
                  stats?.banned
                    ? "Banned from suggestions — click to unset"
                    : "Never suggest this track"
                }
              >
                ⊘
              </button>
              {(stats?.playCount ?? 0) > 0 && (
                <span
                  className="play-count"
                  title={`Played ${stats!.playCount} time${stats!.playCount === 1 ? "" : "s"}`}
                >
                  ×{stats!.playCount}
                </span>
              )}
            </div>
          )}
          <div className="deck-format">
            {data.meta
              ? [
                  data.meta.format,
                  data.load ? `${(data.load.sourceRate / 1000).toFixed(1)} kHz` : "",
                ]
                  .filter(Boolean)
                  .join(" · ")
              : ""}
          </div>
        </div>
        {a && (
          <div className="deck-analysis">
            <span className="key-badge" title={a.keyName}>
              {a.camelot}
            </span>
            {data.keyShift !== 0 && (
              <span
                className="keyshift-badge"
                title={`Transposed ${data.keyShift > 0 ? "+" : ""}${data.keyShift} semitone${
                  Math.abs(data.keyShift) === 1 ? "" : "s"
                } — sounds in ${shiftCamelot(a.camelot, data.keyShift)}`}
              >
                →{shiftCamelot(a.camelot, data.keyShift)}
              </span>
            )}
            {a.fluid ? (
              <span className="fluid-badge" title={`Tempo drift ${(a.tempoDrift * 100).toFixed(1)}% — beat sync disabled; use key + long fades`}>
                FLUID
              </span>
            ) : (
              <span className="bpm-badge">
                {effectiveBpm!.toFixed(1)} BPM
                {data.rate !== 1 && (
                  <em>
                    {" "}
                    {((data.rate - 1) * 100).toFixed(1)}%
                  </em>
                )}
              </span>
            )}
          </div>
        )}
        <div className="deck-time">
          <div className="deck-elapsed">{formatTime(position)}</div>
          <div className="deck-remaining">-{formatTime(duration - position)}</div>
        </div>
      </header>

      <ZoomWaveform
        deck={index}
        detail={data.detail}
        binsPerSec={data.load?.detailBinsPerSec ?? 100}
        duration={duration}
        cues={data.cues}
        cueLabels={data.cueLabels}
        loop={data.loop}
        accent={accent}
      />
      <OverviewWaveform
        deck={index}
        peaks={data.overview}
        duration={duration}
        cues={data.cues}
        cueLabels={data.cueLabels}
        loop={data.loop}
        accent={accent}
        onSeek={(secs) => seek(index, secs)}
      />

      <div className="deck-transport">
        <button
          className={`btn btn-play ${playing ? "active" : ""} ${armed ? "armed" : ""}`}
          disabled={!loaded}
          onClick={togglePlay}
          title={armed ? "Armed — starts on the other deck's next boundary (click to cancel)" : undefined}
        >
          {armed ? "⧖" : playing ? "⏸" : "▶"}
        </button>
        {[0, 1, 2, 3].map((slot) => (
          <button
            key={slot}
            className={`btn btn-hotcue ${data.cues[slot] !== null ? "set" : ""}`}
            style={
              data.cues[slot] !== null
                ? { borderColor: CUE_COLORS[slot], color: CUE_COLORS[slot] }
                : undefined
            }
            disabled={!loaded}
            onClick={(e) => cueClick(slot, e)}
            title={
              data.cues[slot] !== null
                ? `${data.cueLabels[slot] ? `"${data.cueLabels[slot]}" — ` : ""}Jump to ${formatTime(
                    data.cues[slot]!,
                  )} (alt-click to name, shift-click to clear)`
                : "Set cue at playhead"
            }
          >
            {slot + 1}
          </button>
        ))}
        <span className="transport-sep" />
        <button
          className="btn btn-sync"
          disabled={!canSync}
          onClick={onSync}
          title={
            canSync
              ? "Match tempo and align beats to the other deck"
              : "Sync needs steady-tempo analysis on both decks"
          }
        >
          Sync
        </button>
        <div className="pitch-wrap" title="Pitch (playback rate ±8%) — double-click to reset">
          <Fader
            value={(data.rate - 0.92) / 0.16}
            onChange={(v) => onRate(0.92 + v * 0.16)}
            accent={accent}
            resetTo={0.5}
            className="fader-pitch"
          />
        </div>
      </div>

      {/* Second transport row: beat moves, loops, and key tools. Splitting
          these off keeps the row from overflowing the deck's width. */}
      <div className="deck-transport deck-transport-2">
        <button
          className="btn btn-beatjump"
          disabled={!canBeat}
          onClick={() => beatJump(-8)}
          title="Jump back 8 beats (2 bars)"
        >
          ⏪8
        </button>
        <button
          className="btn btn-beatjump"
          disabled={!canBeat}
          onClick={() => beatJump(-4)}
          title="Jump back 4 beats (1 bar)"
        >
          ⏪4
        </button>
        <button
          className="btn btn-beatjump"
          disabled={!canBeat}
          onClick={() => beatJump(4)}
          title="Jump forward 4 beats (1 bar)"
        >
          4⏩
        </button>
        <button
          className="btn btn-beatjump"
          disabled={!canBeat}
          onClick={() => beatJump(8)}
          title="Jump forward 8 beats (2 bars)"
        >
          8⏩
        </button>
        <span className="transport-sep" />
        {[1, 2, 4, 8].map((n) => (
          <button
            key={n}
            className={`btn btn-autoloop ${
              data.loop && beatLen > 0 &&
              Math.abs(data.loop[1] - data.loop[0] - n * beatLen) < 0.02
                ? "active"
                : ""
            }`}
            disabled={!canBeat}
            onClick={() => autoLoop(n)}
            title={`Loop ${n} beat${n > 1 ? "s" : ""} from here, snapped to the grid`}
          >
            {n}
          </button>
        ))}
        <span className="transport-sep" />
        <button
          className={`btn ${data.loopIn !== null && !data.loop ? "active" : ""}`}
          disabled={!loaded}
          onClick={loopIn}
          title="Mark loop start at playhead"
        >
          In
        </button>
        <button
          className="btn"
          disabled={!loaded || data.loopIn === null}
          onClick={loopOut}
          title="Mark loop end and start looping"
        >
          Out
        </button>
        <button
          className={`btn ${data.loop ? "btn-loop-active" : ""}`}
          disabled={!data.loop}
          onClick={loopExit}
          title="Exit loop"
        >
          {data.loop ? "⟳ Exit" : "Loop"}
        </button>
        <span className="transport-sep" />
        <button
          className={`btn ${data.keyLock ? "btn-keylock-on" : ""}`}
          disabled={!loaded}
          onClick={() => onKeyLock(!data.keyLock)}
          title="Key lock: tempo changes no longer shift pitch (time-stretch)"
        >
          ♪⌖
        </button>
        <div className="keyshift" title="Transpose without changing tempo — force a harmonic match">
          <button
            className="btn btn-keyshift"
            disabled={!loaded}
            onClick={() => onKeyShift(Math.max(-12, data.keyShift - 1))}
          >
            ♭
          </button>
          <span
            className={`keyshift-value ${data.keyShift !== 0 ? "active" : ""}`}
            onClick={() => onKeyShift(0)}
            title="Click to reset to original key"
          >
            {data.keyShift > 0 ? `+${data.keyShift}` : data.keyShift}
          </span>
          <button
            className="btn btn-keyshift"
            disabled={!loaded}
            onClick={() => onKeyShift(Math.min(12, data.keyShift + 1))}
          >
            ♯
          </button>
        </div>
      </div>

      <div className="deck-fx">
        {(
          [
            ["FLT", "filter", "Filter sweep: left = low-pass, right = high-pass"],
            ["DLY", "delay", "Delay send (time follows track tempo)"],
            ["RVB", "reverb", "Reverb send"],
          ] as const
        ).map(([label, key, tip]) => (
          <div className="fx-slot" key={key} title={tip}>
            <span className="eq-label">{label}</span>
            <Fader
              value={key === "filter" ? (data.fx.filter + 1) / 2 : data.fx[key]}
              onChange={(v) =>
                onFx({
                  ...data.fx,
                  [key]: key === "filter" ? v * 2 - 1 : v,
                })
              }
              accent={accent}
              resetTo={key === "filter" ? 0.5 : 0}
              className="fader-fx"
            />
          </div>
        ))}
        <button
          className={`btn btn-ag ${data.agOn ? "active" : ""}`}
          disabled={!loaded}
          onClick={onAgToggle}
          title={`Loudness normalization to -14 LUFS (${data.autoGainDb >= 0 ? "+" : ""}${data.autoGainDb.toFixed(1)} dB) — click to bypass`}
        >
          AG {data.agOn ? `${data.autoGainDb >= 0 ? "+" : ""}${data.autoGainDb.toFixed(1)}` : "off"}
        </button>
        <div className="fx-slot" title="Vocal remove / isolate (center-channel)">
          <button
            className={`btn btn-vox ${data.vocalIsolate ? "isolate" : ""}`}
            disabled={!loaded}
            onClick={() => onVocal(data.vocalAmount, !data.vocalIsolate)}
            title={
              data.vocalIsolate
                ? "Isolate vocals — click for Remove"
                : "Remove vocals — click for Isolate"
            }
          >
            {data.vocalIsolate ? "VOX+" : "VOX−"}
          </button>
          <Fader
            value={data.vocalAmount}
            onChange={(v) => onVocal(v, data.vocalIsolate)}
            accent={accent}
            resetTo={0}
            className="fader-fx"
          />
        </div>
        <button
          className="btn btn-fx-reset"
          disabled={!loaded}
          onClick={() => {
            onFx({ filter: 0, delay: 0, reverb: 0 });
            onVocal(0, false);
            if (!data.agOn) onAgToggle();
          }}
          title="Reset filter, delay, reverb, and vocals to neutral; re-enable auto-gain"
        >
          ⌫
        </button>
      </div>
    </section>
  );
}

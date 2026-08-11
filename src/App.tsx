import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import "./App.css";
import Deck from "./components/Deck";
import Mixer from "./components/Mixer";
import Library from "./components/Library";
import Suggest from "./components/Suggest";
import MixView from "./components/MixView";
import {
  Analysis,
  deckPosition,
  getArtwork,
  getEngineSnapshot,
  LoadResult,
  loadTrack,
  nextGridBoundary,
  onAnalysisProgress,
  play,
  playQuantized,
  quantizeGrid,
  restoreDecks,
  seek,
  setAutoGain,
  setCueLabel,
  setCuePoint,
  setFx,
  setKeyLock,
  setKeyShift,
  setRate,
  setVocal,
  subscribeEngine,
  TrackMeta,
} from "./engine";

const ACCENTS = ["#4fc3f7", "#ff8a65"];

export interface DeckData {
  meta: TrackMeta | null;
  load: LoadResult | null;
  overview: Float32Array | null;
  detail: Float32Array | null;
  artwork: string | null;
  analysis: Analysis | null;
  cues: (number | null)[];
  cueLabels: (string | null)[];
  loopIn: number | null;
  loop: [number, number] | null;
  rate: number;
  keyLock: boolean;
  fx: { filter: number; delay: number; reverb: number };
  autoGainDb: number;
  agOn: boolean;
  vocalAmount: number;
  vocalIsolate: boolean;
  keyShift: number;
  loading: boolean;
}

const emptyDeck = (): DeckData => ({
  meta: null,
  load: null,
  overview: null,
  detail: null,
  artwork: null,
  analysis: null,
  cues: [null, null, null, null],
  cueLabels: [null, null, null, null],
  loopIn: null,
  loop: null,
  rate: 1.0,
  keyLock: false,
  fx: { filter: 0, delay: 0, reverb: 0 },
  autoGainDb: 0,
  agOn: true,
  vocalAmount: 0,
  vocalIsolate: false,
  keyShift: 0,
  loading: false,
});

export default function App() {
  const [decks, setDecks] = useState<[DeckData, DeckData]>([
    emptyDeck(),
    emptyDeck(),
  ]);
  const [loadingDeck, setLoadingDeck] = useState<number | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [analysisMap, setAnalysisMap] = useState<Map<string, Analysis>>(
    new Map(),
  );
  const [libraryTracks, setLibraryTracks] = useState<TrackMeta[]>([]);
  // Quantize for deck starts: 0 = off, else beats per grid line (1 beat,
  // 4 = bar, 32 = phrase). Default to bar — the setting that makes a
  // just-pressed play land musically without being as strict as phrase.
  const [quant, setQuant] = useState<number>(() => {
    const saved = Number(localStorage.getItem("quantize"));
    return [0, 1, 4, 32].includes(saved) ? saved : 4;
  });
  const decksRef = useRef(decks);
  decksRef.current = decks;

  const handleQuant = (q: number) => {
    setQuant(q);
    localStorage.setItem("quantize", String(q));
  };

  const trackByPath = useMemo(
    () => new Map(libraryTracks.map((t) => [t.path, t])),
    [libraryTracks],
  );
  const libraryPaths = useMemo(
    () => libraryTracks.map((t) => t.path),
    [libraryTracks],
  );

  useEffect(
    () =>
      onAnalysisProgress((p) => {
        if (p.analysis) {
          setAnalysisMap((prev) => {
            const next = new Map(prev);
            next.set(p.path, p.analysis!);
            return next;
          });
        }
      }),
    [],
  );

  // Rehydrate deck UI after a webview reload — the Rust engine keeps playing
  // through reloads, so we only restore display state, never re-load audio.
  useEffect(() => {
    restoreDecks()
      .then((restored) => {
        restored.forEach((r, i) => {
          if (!r) return;
          patchDeck(i, {
            meta: r.meta,
            load: r.result,
            overview: Float32Array.from(r.result.overview),
            detail: Float32Array.from(r.result.detail),
            analysis: r.result.analysis,
            cues: [...r.result.cues],
            cueLabels: [...r.result.cueLabels],
            autoGainDb: r.result.autoGainDb,
            agOn: Math.abs(r.autoGain - 1.0) > 0.001 || r.result.autoGainDb === 0,
            rate: r.rate,
            keyLock: r.keyLock,
            fx: { filter: r.fx[0], delay: r.fx[1], reverb: r.fx[2] },
            vocalAmount: r.vocal[0],
            vocalIsolate: r.vocal[1],
            keyShift: r.keyShift,
            loop: r.loopRegion,
            loading: false,
          });
          getArtwork(r.meta.path).then((art) => patchDeck(i, { artwork: art }));
        });
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patchDeck = (index: number, patch: Partial<DeckData>) => {
    setDecks((prev) => {
      const next = [...prev] as [DeckData, DeckData];
      next[index] = { ...next[index], ...patch };
      return next;
    });
  };

  const handleLoad = async (deck: number, meta: TrackMeta) => {
    if (loadingDeck !== null) return;
    setLoadingDeck(deck);
    setLoadError(null);
    patchDeck(deck, { ...emptyDeck(), loading: true, meta });
    try {
      const result = await loadTrack(deck, meta.path);
      const prev = decksRef.current[deck];
      patchDeck(deck, {
        load: result,
        overview: Float32Array.from(result.overview),
        detail: Float32Array.from(result.detail),
        analysis: result.analysis,
        cues: [...result.cues],
        cueLabels: [...result.cueLabels],
        autoGainDb: result.autoGainDb,
        agOn: true,
        keyLock: prev.keyLock, // persists across loads; engine keeps stretcher
        loading: false,
      });
      setRate(deck, 1.0);
      if (result.analysis) {
        setAnalysisMap((prev) => {
          const next = new Map(prev);
          next.set(meta.path, result.analysis!);
          return next;
        });
      }
      getArtwork(meta.path).then((art) => patchDeck(deck, { artwork: art }));
    } catch (e) {
      setLoadError(`Failed to load "${meta.title}": ${e}`);
      patchDeck(deck, emptyDeck());
    } finally {
      setLoadingDeck(null);
    }
  };

  const handleSetCue = (deck: number, slot: number, seconds: number | null) => {
    const d = decksRef.current[deck];
    if (!d.meta) return;
    const cues = [...d.cues];
    cues[slot] = seconds;
    // Clearing a cue clears its note too (the DB row is deleted).
    const cueLabels = [...d.cueLabels];
    if (seconds === null) cueLabels[slot] = null;
    patchDeck(deck, { cues, cueLabels });
    setCuePoint(d.meta.path, slot, seconds);
  };

  const handleSetCueLabel = (deck: number, slot: number, label: string | null) => {
    const d = decksRef.current[deck];
    if (!d.meta || d.cues[slot] === null) return;
    const cueLabels = [...d.cueLabels];
    cueLabels[slot] = label && label.trim() ? label.trim() : null;
    patchDeck(deck, { cueLabels });
    setCueLabel(d.meta.path, slot, cueLabels[slot]);
  };

  /**
   * Play with quantized entry: when the other deck is running and both grids
   * are trustworthy, snap this deck to its nearest grid line and arm it to
   * fire exactly as the other deck crosses its own next boundary — so both
   * tracks enter aligned at the beat/bar/phrase level. Falls back to a plain
   * play in every case the grid can't support.
   */
  const handlePlay = (deck: number) => {
    const d = decksRef.current[deck];
    const other = decksRef.current[1 - deck];
    const snap = getEngineSnapshot();
    const otherPlaying = snap.decks[1 - deck]?.playing;
    const a = d.analysis;
    const oa = other.analysis;
    if (
      !quant ||
      !otherPlaying ||
      !a ||
      a.fluid ||
      a.bpm <= 0 ||
      !oa ||
      oa.fluid ||
      oa.bpm <= 0
    ) {
      play(deck);
      return;
    }
    // Margin covers IPC latency plus the 30 Hz snapshot staleness; a missed
    // boundary is still recovered engine-side via overshoot compensation.
    const target = nextGridBoundary(
      deckPosition(1 - deck),
      oa,
      quant,
      0.08,
      other.load?.durationSecs ?? 0,
    );
    if (target === null) {
      play(deck);
      return;
    }
    seek(deck, quantizeGrid(deckPosition(deck), a, quant));
    playQuantized(deck, 1 - deck, target * snap.sampleRate);
  };

  const handleRate = (deck: number, rate: number) => {
    patchDeck(deck, { rate });
    setRate(deck, rate);
  };

  const handleKeyLock = (deck: number, enabled: boolean) => {
    patchDeck(deck, { keyLock: enabled });
    setKeyLock(deck, enabled);
  };

  const handleFx = (
    deck: number,
    fx: { filter: number; delay: number; reverb: number },
  ) => {
    patchDeck(deck, { fx });
    setFx(deck, fx.filter, fx.delay, fx.reverb);
  };

  const handleVocal = (deck: number, amount: number, isolate: boolean) => {
    patchDeck(deck, { vocalAmount: amount, vocalIsolate: isolate });
    setVocal(deck, amount, isolate);
  };

  const handleKeyShift = (deck: number, semitones: number) => {
    // The engine turns key lock on for a non-zero shift; mirror that here so
    // the toggle's appearance matches what's actually running.
    patchDeck(deck, {
      keyShift: semitones,
      keyLock: semitones !== 0 ? true : decksRef.current[deck].keyLock,
    });
    setKeyShift(deck, semitones);
  };

  const handleAgToggle = (deck: number) => {
    const d = decksRef.current[deck];
    const agOn = !d.agOn;
    patchDeck(deck, { agOn });
    setAutoGain(deck, agOn ? Math.pow(10, d.autoGainDb / 20) : 1.0);
  };

  /** Beat-sync `deck` to the other deck: match effective tempo, align phase. */
  const handleSync = (deck: number) => {
    const me = decksRef.current[deck];
    const other = decksRef.current[1 - deck];
    if (!me.analysis || !other.analysis) return;
    if (me.analysis.fluid || other.analysis.fluid) return;
    const otherBpm = other.analysis.bpm * other.rate;
    const myBpm = me.analysis.bpm;
    // Fold half/double-time relationships to the nearest octave.
    const targetBpm = [otherBpm, otherBpm * 2, otherBpm / 2].reduce((best, c) =>
      Math.abs(c - myBpm) < Math.abs(best - myBpm) ? c : best,
    );
    const rate = Math.max(0.92, Math.min(1.08, targetBpm / myBpm));
    handleRate(deck, rate);

    // Phase align: nudge this deck so its grid phase matches the other's.
    // With quantize at bar or phrase, align whole bars (nearest equivalent
    // position mod 4 beats — up to a 2-beat jump); otherwise nearest beat.
    const snap = getEngineSnapshot();
    if (snap.decks[deck]?.playing && snap.decks[1 - deck]?.playing) {
      const n = quant >= 4 ? 4 : 1;
      // Grid phase in track time is rate-independent.
      const phase = (pos: number, a: Analysis) => {
        const beats = ((pos - a.beatOffset) * a.bpm) / 60;
        return ((beats % n) + n) % n;
      };
      const posMe = deckPosition(deck);
      const pMe = phase(posMe, me.analysis);
      const pOther = phase(deckPosition(1 - deck), other.analysis);
      let delta = pOther - pMe;
      if (delta > n / 2) delta -= n;
      if (delta < -n / 2) delta += n;
      // Nudge in this track's own time: one beat = 60/bpm track-seconds.
      seek(deck, posMe + (delta * 60) / me.analysis.bpm);
    }
  };

  const snapshot = useSyncExternalStore(subscribeEngine, getEngineSnapshot);
  const playingDeck = snapshot.decks.findIndex((d) => d.playing);

  return (
    <div className="app">
      <div className="decks">
        {[0, 1].map((i) => (
          <Deck
            key={i}
            index={i}
            accent={ACCENTS[i]}
            data={decks[i]}
            otherAnalysis={decks[1 - i].analysis}
            onSetCue={(slot, secs) => handleSetCue(i, slot, secs)}
            onSetCueLabel={(slot, label) => handleSetCueLabel(i, slot, label)}
            onPlay={() => handlePlay(i)}
            onRate={(r) => handleRate(i, r)}
            onSync={() => handleSync(i)}
            onLoop={(loop, loopIn) => patchDeck(i, { loop, loopIn })}
            onKeyLock={(on) => handleKeyLock(i, on)}
            onFx={(fx) => handleFx(i, fx)}
            onAgToggle={() => handleAgToggle(i)}
            onVocal={(amount, isolate) => handleVocal(i, amount, isolate)}
            onKeyShift={(s) => handleKeyShift(i, s)}
          />
        ))}
      </div>
      <MixView
        decks={decks}
        accents={ACCENTS}
        quant={quant}
        onQuant={handleQuant}
      />
      <Mixer />
      {loadError && <div className="load-error">{loadError}</div>}
      <Suggest
        paths={libraryPaths}
        trackByPath={trackByPath}
        decks={decks}
        playingDeck={playingDeck}
        onLoad={handleLoad}
        loadingDeck={loadingDeck}
      />
      <Library
        onLoad={handleLoad}
        loadingDeck={loadingDeck}
        analysisMap={analysisMap}
        setAnalysisMap={setAnalysisMap}
        decks={decks}
        onTracksChange={setLibraryTracks}
      />
    </div>
  );
}

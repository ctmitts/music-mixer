// Typed bridge to the Rust engine: IPC commands + the 30 Hz engine-state
// event, exposed as a subscribable store with position extrapolation.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface TrackMeta {
  path: string;
  title: string;
  artist: string;
  album: string;
  genre: string;
  durationSecs: number;
  sampleRate: number;
  bitDepth: number | null;
  format: string;
}

export interface Analysis {
  bpm: number;
  beatOffset: number;
  tempoDrift: number;
  fluid: boolean;
  keyName: string;
  camelot: string;
  lufs: number;
  energy: number;
  brightness: number;
  beatStrength: number;
}

export interface LoadResult {
  durationSecs: number;
  sourceRate: number;
  engineRate: number;
  overview: number[];
  detail: number[];
  detailBinsPerSec: number;
  analysis: Analysis | null;
  cues: (number | null)[];
  cueLabels: (string | null)[];
  autoGainDb: number;
}

export interface AnalysisProgress {
  done: number;
  total: number;
  path: string;
  analysis: Analysis | null;
}

export interface DeckSnapshot {
  positionSecs: number;
  playing: boolean;
  peak: number;
  trackId: number;
  /** A quantized start is waiting on the other deck's playhead. */
  armed: boolean;
  /** Master-deck time the pending start fires at (secs; valid while armed). */
  armedMasterSecs: number;
}

export interface EngineSnapshot {
  decks: DeckSnapshot[];
  masterPeak: [number, number];
  sampleRate: number;
}

export interface DeviceList {
  devices: string[];
  defaultDevice: string | null;
  current: string | null;
  sampleRate: number;
}

const emptyDeck = (): DeckSnapshot => ({
  positionSecs: 0,
  playing: false,
  peak: 0,
  trackId: 0,
  armed: false,
  armedMasterSecs: 0,
});

let snapshot: EngineSnapshot = {
  decks: [emptyDeck(), emptyDeck()],
  masterPeak: [0, 0],
  sampleRate: 44100,
};
let receivedAt = performance.now();
const listeners = new Set<() => void>();

listen<EngineSnapshot>("engine-state", (event) => {
  snapshot = event.payload;
  receivedAt = performance.now();
  listeners.forEach((l) => l());
});

export function subscribeEngine(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getEngineSnapshot(): EngineSnapshot {
  return snapshot;
}

/** Playhead position extrapolated between 30 Hz updates for smooth drawing. */
export function deckPosition(deck: number): number {
  const d = snapshot.decks[deck];
  if (!d) return 0;
  if (!d.playing) return d.positionSecs;
  return d.positionSecs + (performance.now() - receivedAt) / 1000;
}

// --- commands --------------------------------------------------------------

export const scanLibrary = (folder: string) =>
  invoke<TrackMeta[]>("scan_library", { folder });

export const getArtwork = (path: string) =>
  invoke<string | null>("get_artwork", { path });

export const loadTrack = (deck: number, path: string) =>
  invoke<LoadResult>("load_track", { deck, path });

export const play = (deck: number) => invoke("play", { deck });
export const pause = (deck: number) => invoke("pause", { deck });
/** Arm `deck` to start when `master`'s playhead reaches `masterFrame`. */
export const playQuantized = (
  deck: number,
  master: number,
  masterFrame: number,
) => invoke("play_quantized", { deck, master, masterFrame });
export const seek = (deck: number, seconds: number) =>
  invoke("seek", { deck, seconds });

export const setRate = (deck: number, value: number) =>
  invoke("set_rate", { deck, value });
export const setKeyLock = (deck: number, enabled: boolean) =>
  invoke("set_key_lock", { deck, enabled });
export const setFx = (
  deck: number,
  filter: number,
  delayMix: number,
  reverbMix: number,
) => invoke("set_fx", { deck, filter, delayMix, reverbMix });
export const setAutoGain = (deck: number, value: number) =>
  invoke("set_auto_gain", { deck, value });
export const setVocal = (deck: number, amount: number, isolate: boolean) =>
  invoke("set_vocal", { deck, amount, isolate });
export const setKeyShift = (deck: number, semitones: number) =>
  invoke("set_key_shift", { deck, semitones });

/** EQ band count and labels, mirroring NUM_EQ_BANDS in the engine. */
export const EQ_BANDS = ["LOW", "L-MID", "H-MID", "HIGH"] as const;

/** Snap a time to the nearest beat using the analysed grid. */
export function quantize(seconds: number, a: Analysis | null): number {
  if (!a || a.fluid || a.bpm <= 0) return seconds;
  const beat = 60 / a.bpm;
  const n = Math.round((seconds - a.beatOffset) / beat);
  return Math.max(0, a.beatOffset + n * beat);
}

/** Snap a time to the nearest N-beat grid line (1 = beat, 4 = bar,
 * 32 = phrase). The bar/phrase anchor is the analyser's first detected beat,
 * so it's consistent within a track but arbitrary against the "true" musical
 * downbeat. */
export function quantizeGrid(
  seconds: number,
  a: Analysis | null,
  nBeats: number,
): number {
  if (!a || a.fluid || a.bpm <= 0 || nBeats <= 0) return seconds;
  const step = (60 / a.bpm) * nBeats;
  const n = Math.round((seconds - a.beatOffset) / step);
  return Math.max(0, a.beatOffset + n * step);
}

/** The next N-beat grid line strictly after `seconds + marginSecs`, or null
 * if the grid is unusable or the boundary would fall past `duration`. */
export function nextGridBoundary(
  seconds: number,
  a: Analysis | null,
  nBeats: number,
  marginSecs: number,
  duration: number,
): number | null {
  if (!a || a.fluid || a.bpm <= 0 || nBeats <= 0) return null;
  const step = (60 / a.bpm) * nBeats;
  const n = Math.ceil((seconds + marginSecs - a.beatOffset) / step + 1e-6);
  const t = a.beatOffset + n * step;
  return t < duration ? t : null;
}

/** Camelot code after transposing by `semitones`. Each semitone is seven
 * steps around the wheel (the circle of fifths); the letter is unchanged. */
export function shiftCamelot(camelot: string, semitones: number): string {
  const m = /^(\d+)([AB])$/.exec(camelot.trim());
  if (!m || !semitones) return camelot;
  const n = parseInt(m[1], 10);
  const shifted = (((n - 1 + semitones * 7) % 12) + 12) % 12;
  return `${shifted + 1}${m[2]}`;
}
export const startRecording = () => invoke<string>("start_recording");
export const stopRecording = () => invoke("stop_recording");
/** Open the spectral visualizer in its own window (drag it to a TV/projector). */
export const openVisualizerWindow = () => invoke("open_visualizer_window");

export interface DeckRestore {
  meta: TrackMeta;
  result: LoadResult;
  rate: number;
  keyLock: boolean;
  fx: [number, number, number];
  autoGain: number;
  vocal: [number, boolean];
  keyShift: number;
  eqs: number[];
  loopRegion: [number, number] | null;
}

export const restoreDecks = () =>
  invoke<(DeckRestore | null)[]>("restore_decks");

// --- recommendations -------------------------------------------------------

export interface ScoredTrack {
  path: string;
  title: string;
  artist: string;
  genre: string;
  bpm: number;
  camelot: string;
  keyName: string;
  fluid: boolean;
  energy: number;
  durationSecs: number;
  score: number;
  reason: string;
}

export interface RecommendResult {
  tracks: ScoredTrack[];
  usedModel: boolean;
  notice: string | null;
}

export interface RecommendArgs {
  paths: string[];
  referencePath: string | null;
  description: string;
  exclude: string[];
  limit: number;
  useModel: boolean;
}

export const recommendTracks = (args: RecommendArgs) =>
  invoke<RecommendResult>("recommend_tracks", { args });

export const setApiKey = (key: string) => invoke("set_api_key", { key });
export const hasApiKey = () => invoke<boolean>("has_api_key");

export interface PlayedTrack {
  path: string;
  title: string;
  artist: string;
  genre: string;
  camelot: string;
  bpm: number;
  fluid: boolean;
  energy: number;
}

export const getSessionHistory = () =>
  invoke<PlayedTrack[]>("get_session_history");
export const clearSessionHistory = () => invoke("clear_session_history");
export const setLoop = (deck: number, startSecs: number, endSecs: number) =>
  invoke("set_loop", { deck, startSecs, endSecs });
export const clearLoop = (deck: number) => invoke("clear_loop", { deck });

export const getAnalysis = (paths: string[]) =>
  invoke<{ path: string; analysis: Analysis }[]>("get_analysis", { paths });
export const startLibraryAnalysis = (paths: string[]) =>
  invoke("start_library_analysis", { paths });
export const cancelLibraryAnalysis = () => invoke("cancel_library_analysis");
export const setCuePoint = (
  path: string,
  slot: number,
  seconds: number | null,
) => invoke("set_cue_point", { path, slot, seconds });
export const setCueLabel = (path: string, slot: number, label: string | null) =>
  invoke("set_cue_label", { path, slot, label });

export function onAnalysisProgress(
  handler: (p: AnalysisProgress) => void,
): () => void {
  const un = listen<AnalysisProgress>("analysis-progress", (e) =>
    handler(e.payload),
  );
  return () => {
    un.then((f) => f());
  };
}

/** Camelot compatibility: same slot, ±1 same letter, or same number other
 * letter. */
export function camelotCompatible(a: string, b: string): boolean {
  const parse = (s: string) => {
    const m = /^(\d+)([AB])$/.exec(s.trim());
    return m ? { n: parseInt(m[1], 10), l: m[2] } : null;
  };
  const pa = parse(a);
  const pb = parse(b);
  if (!pa || !pb) return false;
  if (pa.l === pb.l) {
    const diff = Math.abs(pa.n - pb.n);
    return diff === 0 || diff === 1 || diff === 11;
  }
  return pa.n === pb.n;
}

/** Tempos mixable within ±8%, allowing half/double time. */
export function bpmCompatible(a: number, b: number): boolean {
  if (a <= 0 || b <= 0) return false;
  return [b, b * 2, b / 2].some((t) => Math.abs(a - t) / t <= 0.08);
}

export const setGain = (deck: number, value: number) =>
  invoke("set_gain", { deck, value });
export const setEq = (deck: number, band: number, value: number) =>
  invoke("set_eq", { deck, band, value });
export const setCrossfader = (value: number) =>
  invoke("set_crossfader", { value });
export const setMasterGain = (value: number) =>
  invoke("set_master_gain", { value });

export const listOutputDevices = () =>
  invoke<DeviceList>("list_output_devices");
export const setOutputDevice = (device: string | null) =>
  invoke<number>("set_output_device", { device });

// --- helpers ---------------------------------------------------------------

export function formatTime(secs: number): string {
  if (!isFinite(secs) || secs < 0) secs = 0;
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Map a 0..1 fader position to a DJ-style gain: 0 = kill, 0.5 = unity,
 * 1 = +6 dB. */
export function faderToGain(s: number): number {
  if (s <= 0) return 0;
  if (s <= 0.5) return Math.pow(s / 0.5, 2);
  return Math.pow(10, ((s - 0.5) / 0.5) * 6 / 20);
}

export function peakToDb(peak: number): number {
  return peak <= 0 ? -Infinity : 20 * Math.log10(peak);
}

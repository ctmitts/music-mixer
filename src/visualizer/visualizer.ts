import { StftAnalyzer, type StftOptions } from "./core/stft";
import { HISTORY as HISTORY_ROWS, Renderer, type Mode, type RenderParams } from "./render/renderer";
import type { AudioSource } from "./sources/types";

export interface VisualizerOptions extends Partial<RenderParams> {
  fftSize?: number;
  hopSize?: number;
  window?: StftOptions["window"];
  mode?: Mode;
  detail?: DetailName;
}

/**
 * Time/frequency resolution presets.
 *
 * These are the uncertainty principle made into a control. A short FFT
 * localizes events in time but smears them in frequency; a long one does the
 * reverse. No setting is "best" — `ultra` resolves individual drum hits but
 * cannot tell a bass note from its neighbour, while `coarse` nails pitch and
 * turns transients to mush.
 *
 * The hop is kept at fftSize/8 (87.5% overlap) throughout, which is what makes
 * the instantaneous-frequency estimate stable enough to drive hue.
 *
 * `windowMs` is the analysis window length at 48 kHz — the true floor on time
 * resolution. Asking for a 0.125 s display window while running `coarse`
 * (171 ms) shows fewer than one window's worth of independent data, so the
 * picture blurs no matter how small the hop gets.
 */
export const DETAIL_PRESETS = {
  coarse: { fftSize: 8192, hopSize: 1024, windowMs: 171 },
  balanced: { fftSize: 4096, hopSize: 512, windowMs: 85 },
  fine: { fftSize: 2048, hopSize: 256, windowMs: 43 },
  ultra: { fftSize: 1024, hopSize: 128, windowMs: 21 },
} as const;

export type DetailName = keyof typeof DETAIL_PRESETS;

/**
 * Ties a sample source to the analyzer to the GPU.
 *
 * Analysis is driven by sample arrival (so frames land on exact hop
 * boundaries, and the picture stays sample-accurate regardless of display
 * refresh); drawing is driven by requestAnimationFrame. The two rates are
 * deliberately independent — the history texture is the buffer between them.
 */
export class Visualizer {
  readonly renderer: Renderer;
  private analyzer: StftAnalyzer | null = null;
  private source: AudioSource | null = null;

  private ring: Float32Array | null = null;
  private sinceHop = 0;
  private rafId = 0;
  private lastDraw = 0;
  private running = false;

  private readonly opts: VisualizerOptions;
  private detailName: DetailName;
  /** Explicit fftSize/hopSize override the preset until `setDetail` is used. */
  private customFft?: number;
  private customHop?: number;

  constructor(canvas: HTMLCanvasElement, opts: VisualizerOptions = {}) {
    this.opts = opts;
    this.detailName = opts.detail ?? "balanced";
    this.customFft = opts.fftSize;
    this.customHop = opts.hopSize;
    this.renderer = new Renderer(canvas);
    Object.assign(this.renderer.params, opts);
    if (opts.mode) this.renderer.setMode(opts.mode);
  }

  get detail(): DetailName {
    return this.detailName;
  }

  /** Analysis window length in ms — the hard floor on time resolution. */
  get analysisWindowMs(): number {
    return this.analyzer
      ? (this.analyzer.fftSize / this.analyzer.sampleRate) * 1000
      : DETAIL_PRESETS[this.detailName].windowMs;
  }

  /** Frames of history currently available, in seconds. */
  get historySeconds(): number {
    return this.analyzer ? HISTORY_ROWS / this.analyzer.framesPerSecond : 0;
  }

  /**
   * Change time/frequency resolution live. Rebuilds the analyzer but leaves
   * the source and the GL context alone, so audio never drops. Rows already in
   * the history were captured at the old frame rate, so the oldest part of the
   * picture is briefly stretched until they scroll out.
   */
  setDetail(name: DetailName): void {
    this.detailName = name;
    this.customFft = undefined;
    this.customHop = undefined;
    if (this.source) this.rebuildAnalyzer(this.source.sampleRate);
  }

  private rebuildAnalyzer(sampleRate: number): void {
    const preset = DETAIL_PRESETS[this.detailName];
    const fftSize = this.customFft ?? preset.fftSize;
    const hopSize = this.customHop ?? preset.hopSize;

    this.analyzer = new StftAnalyzer({
      sampleRate,
      fftSize,
      hopSize,
      window: this.opts.window,
    });
    this.renderer.setFrameRate(this.analyzer.framesPerSecond);
    this.ring = new Float32Array(fftSize);
    this.sinceHop = 0;
  }

  async attach(source: AudioSource): Promise<void> {
    this.detach();
    this.source = source;
    this.rebuildAnalyzer(source.sampleRate);
    await source.start((chunk) => this.consume(chunk));
    this.start();
  }

  detach(): void {
    this.source?.stop();
    this.source = null;
    this.analyzer = null;
    this.ring = null;
  }

  /**
   * Slide the analysis window forward and fire an analysis on every hop
   * boundary. Worklet chunks (128 samples) are far smaller than a hop, but the
   * loop handles the other case too rather than silently dropping frames.
   */
  private consume(chunk: Float32Array): void {
    const analyzer = this.analyzer;
    const ring = this.ring;
    if (!analyzer || !ring) return;

    const hop = analyzer.hopSize;
    const n = ring.length;
    let offset = 0;

    while (offset < chunk.length) {
      const take = Math.min(hop - this.sinceHop, chunk.length - offset);
      ring.copyWithin(0, take);
      ring.set(chunk.subarray(offset, offset + take), n - take);
      this.sinceHop += take;
      offset += take;

      if (this.sinceHop >= hop) {
        this.sinceHop = 0;
        const stats = analyzer.analyze(ring);
        this.renderer.pushFrame(analyzer.pixels, stats);
      }
    }
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.lastDraw = performance.now();
    const loop = () => {
      if (!this.running) return;
      const now = performance.now();
      const dt = Math.min(0.1, (now - this.lastDraw) / 1000);
      this.lastDraw = now;
      this.renderer.draw(dt);
      this.rafId = requestAnimationFrame(loop);
    };
    this.rafId = requestAnimationFrame(loop);
  }

  stop(): void {
    this.running = false;
    cancelAnimationFrame(this.rafId);
  }

  setMode(mode: Mode): void {
    this.renderer.setMode(mode);
  }

  get params(): RenderParams {
    return this.renderer.params;
  }

  dispose(): void {
    this.stop();
    this.detach();
  }
}

export { MODES, type Mode, type RenderParams } from "./render/renderer";
export { WebAudioSource } from "./sources/webaudio";
export type { AudioSource } from "./sources/types";

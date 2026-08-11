import { StftAnalyzer, type StftOptions } from "./core/stft";
import { Renderer, type Mode, type RenderParams } from "./render/renderer";
import type { AudioSource } from "./sources/types";

export interface VisualizerOptions extends Partial<RenderParams> {
  fftSize?: number;
  hopSize?: number;
  window?: StftOptions["window"];
  mode?: Mode;
}

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

  constructor(canvas: HTMLCanvasElement, opts: VisualizerOptions = {}) {
    this.opts = opts;
    this.renderer = new Renderer(canvas);
    Object.assign(this.renderer.params, opts);
    if (opts.mode) this.renderer.setMode(opts.mode);
  }

  async attach(source: AudioSource): Promise<void> {
    this.detach();
    this.source = source;

    const fftSize = this.opts.fftSize ?? 4096;
    const hopSize = this.opts.hopSize ?? 1024;

    this.analyzer = new StftAnalyzer({
      sampleRate: source.sampleRate,
      fftSize,
      hopSize,
      window: this.opts.window,
    });
    this.renderer.setFrameRate(this.analyzer.framesPerSecond);

    this.ring = new Float32Array(fftSize);
    this.sinceHop = 0;

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

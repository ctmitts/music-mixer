import { FRAG, VERT } from "./shaders";
import { BINS_PER_OCTAVE, N_BINS } from "../core/stft";
import type { FrameStats } from "../core/stft";

export const MODES = ["waterfall", "mandala", "flow", "helix"] as const;
export type Mode = (typeof MODES)[number];

/**
 * Rows of spectral history kept on the GPU.
 *
 * Sized so that even the finest hop still buys ~20 s of scrollback: at hop 256
 * (187 frames/sec) this is 21.8 s, and at hop 1024 it is 87 s. Costs
 * 324 × 4096 × 4 B ≈ 5.3 MB of texture, which is nothing.
 */
export const HISTORY = 4096;

/** How fast the overall level envelope tracks, in seconds. */
const LEVEL_TAU = 0.1;
/** How fast the onset bloom decays, in seconds. */
const FLUX_TAU = 0.16;

export interface RenderParams {
  /** Seconds of history on screen — your 5–10 s slider. */
  windowSeconds: number;
  /** Seconds a mode crossfade takes. */
  fadeSeconds: number;
  /** Kaleidoscope fold count in mandala mode. */
  symmetry: number;
  /** Psychedelic hue drift, revolutions per minute. */
  hueDriftRpm: number;
  gain: number;
  saturation: number;
  /**
   * Delay the picture to match how long audio takes to actually reach the
   * listener. HDMI needs ~20–50 ms; AirPlay to an Apple TV needs ~1500–2000 ms
   * and looks badly out of sync without it.
   */
  syncOffsetMs: number;
}

export const DEFAULT_PARAMS: RenderParams = {
  windowSeconds: 8,
  fadeSeconds: 1.2,
  symmetry: 6,
  hueDriftRpm: 1.5,
  gain: 1.0,
  saturation: 1.0,
  syncOffsetMs: 0,
};

export class Renderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly texture: WebGLTexture;
  private readonly uniforms: Record<string, WebGLUniformLocation | null> = {};

  private cursor = 0;
  private framesPerSecond = 47;

  private modeA: Mode = "mandala";
  private modeB: Mode = "mandala";
  private blend = 0;

  private hueRotate = 0;
  /** Smoothed so the bloom breathes instead of strobing. */
  private smoothLevel = 0;
  private smoothFlux = 0;

  params: RenderParams = { ...DEFAULT_PARAMS };

  constructor(private readonly canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      // The visuals are heavily temporal; letting the compositor keep the
      // previous frame avoids a black flash on resize.
      preserveDrawingBuffer: false,
      powerPreference: "high-performance",
    });
    if (!gl) throw new Error("WebGL2 is required for the visualizer");
    this.gl = gl;

    this.program = link(gl, VERT, FRAG);
    gl.useProgram(this.program);

    for (const name of [
      "uHistory", "uResolution", "uBins", "uBinsPerOctave", "uHistoryLen", "uCursor",
      "uWindowFrac", "uTime", "uModeA", "uModeB", "uBlend", "uLevel",
      "uFlux", "uHueRotate", "uSymmetry", "uGain", "uSaturation", "uAgeOffset",
    ]) {
      this.uniforms[name] = gl.getUniformLocation(this.program, name);
    }

    // Full-screen triangle pair.
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, -1, 3, -1, -1, 3]),
      gl.STATIC_DRAW,
    );
    const loc = gl.getAttribLocation(this.program, "aPos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

    // Ring-buffer history texture. NEAREST because the shader does its own
    // bilinear filtering with correct wrap handling at the cursor seam.
    this.texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.texImage2D(
      gl.TEXTURE_2D, 0, gl.RGBA8, N_BINS, HISTORY, 0,
      gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(N_BINS * HISTORY * 4),
    );
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.uniform1i(this.uniforms.uHistory, 0);
  }

  setFrameRate(fps: number): void {
    this.framesPerSecond = fps;
  }

  /** Switch modes with a crossfade. Re-selecting the current mode is a no-op. */
  setMode(mode: Mode): void {
    if (mode === this.modeB) return;
    // If a fade is already running, collapse it so we always blend from
    // whatever is actually on screen rather than snapping back to modeA.
    this.modeA = this.blend > 0.5 ? this.modeB : this.modeA;
    this.modeB = mode;
    this.blend = 0;
  }

  get mode(): Mode {
    return this.blend > 0.5 ? this.modeB : this.modeA;
  }

  /** Push one analyzed frame into the history ring. */
  pushFrame(pixels: Uint8Array, stats: FrameStats): void {
    const gl = this.gl;
    this.cursor = (this.cursor + 1) % HISTORY;
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.texSubImage2D(
      gl.TEXTURE_2D, 0, 0, this.cursor, N_BINS, 1,
      gl.RGBA, gl.UNSIGNED_BYTE, pixels,
    );

    // Attack fast, release slow: onsets should pop, then settle.
    //
    // Derived from time constants, not written as per-frame numbers. Frames
    // arrive anywhere from 47/sec (coarse) to 375/sec (ultra), so a fixed
    // coefficient would make the bloom breathe eight times faster at the
    // finest Detail setting — which reads as the whole picture racing.
    const dt = 1 / this.framesPerSecond;
    this.smoothLevel +=
      (stats.level - this.smoothLevel) * (1 - Math.exp(-dt / LEVEL_TAU));
    this.smoothFlux = Math.max(
      this.smoothFlux * Math.exp(-dt / FLUX_TAU),
      stats.flux,
    );
  }

  draw(dtSeconds: number): void {
    const gl = this.gl;
    const { canvas } = this;

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.round(canvas.clientWidth * dpr);
    const h = Math.round(canvas.clientHeight * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    gl.viewport(0, 0, canvas.width, canvas.height);

    if (this.blend < 1 && this.modeA !== this.modeB) {
      this.blend = Math.min(1, this.blend + dtSeconds / Math.max(0.01, this.params.fadeSeconds));
      if (this.blend >= 1) {
        this.modeA = this.modeB;
        this.blend = 0;
      }
    }

    this.hueRotate = (this.hueRotate + (dtSeconds * this.params.hueDriftRpm) / 60) % 1;

    const framesOnScreen = this.params.windowSeconds * this.framesPerSecond;

    gl.useProgram(this.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);

    const u = this.uniforms;
    gl.uniform2f(u.uResolution, canvas.width, canvas.height);
    gl.uniform1i(u.uBins, N_BINS);
    gl.uniform1i(u.uBinsPerOctave, BINS_PER_OCTAVE);
    gl.uniform1i(u.uHistoryLen, HISTORY);
    gl.uniform1i(u.uCursor, this.cursor);
    gl.uniform1f(u.uWindowFrac, Math.min(1, framesOnScreen / HISTORY));
    gl.uniform1f(u.uTime, performance.now() / 1000);
    gl.uniform1i(u.uModeA, MODES.indexOf(this.modeA));
    gl.uniform1i(u.uModeB, MODES.indexOf(this.modeB));
    gl.uniform1f(u.uBlend, this.blend);
    gl.uniform1f(u.uLevel, this.smoothLevel);
    gl.uniform1f(u.uFlux, this.smoothFlux);
    gl.uniform1f(u.uHueRotate, this.hueRotate);
    gl.uniform1f(u.uSymmetry, this.params.symmetry);
    gl.uniform1f(u.uGain, this.params.gain);
    gl.uniform1f(u.uSaturation, this.params.saturation);
    gl.uniform1f(
      u.uAgeOffset,
      (this.params.syncOffsetMs / 1000) * this.framesPerSecond,
    );

    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const compile = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(s);
      throw new Error(`shader compile failed:\n${log}`);
    }
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(`program link failed:\n${gl.getProgramInfoLog(p)}`);
  }
  return p;
}

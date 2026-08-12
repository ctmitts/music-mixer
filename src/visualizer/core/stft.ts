import { FFT, princarg } from "./fft";
import { makeWindow, type WindowKind } from "./window";

/**
 * Short-time Fourier analysis with phase-derivative feature extraction.
 *
 * The forward STFT does the decomposition (the *inverse* transform is what
 * would rebuild the waveform — we never need it here). What makes this more
 * than a spectrogram is that raw per-bin phase is discarded and its
 * derivatives are kept instead:
 *
 *   ∂φ/∂t  → instantaneous frequency. Recovers the true partial frequency to
 *            far finer than one bin, so a 4096-point FFT resolves bass
 *            intervals it has no business resolving. Drives HUE, by way of
 *            pitch class.
 *   ∂f/∂t  → frame-to-frame stability of that frequency. Steady partials hold
 *            still; noise scribbles. Drives SATURATION.
 *   ∂|X|/∂t → half-wave-rectified spectral flux. Drives TRANSIENT bloom.
 *
 * Output is resampled onto a log-frequency (constant-Q-ish) axis so that
 * octaves are equally spaced, then packed straight into RGBA8 bytes for
 * texture upload. Every renderer reads that one texture; they differ only in
 * how they map its coordinates onto the screen.
 */

/** A0. Nine octaves from here reaches 14080 Hz, which covers everything that
 *  carries pitch. */
export const F_MIN = 27.5;
export const OCTAVES = 9;
export const BINS_PER_OCTAVE = 36; // 3 per semitone
export const N_BINS = OCTAVES * BINS_PER_OCTAVE; // 324

export interface StftOptions {
  sampleRate: number;
  /** FFT size. 4096 @ 48k = 85 ms window, 11.7 Hz bins. */
  fftSize?: number;
  /** Hop in samples. 1024 = 75% overlap = ~47 frames/sec @ 48k. */
  hopSize?: number;
  window?: WindowKind;
  /** Noise floor in dB below the running peak. */
  dynamicRange?: number;
}

/** Per-frame scalars for global/whole-screen effects. */
export interface FrameStats {
  /** Broadband RMS, 0..1 after AGC. */
  level: number;
  /** Spectral centroid in normalized log-frequency, 0..1. Brightness. */
  centroid: number;
  /** Total positive spectral flux, 0..1. Onset strength. */
  flux: number;
  /** Energy-weighted mean pitch class, 0..1 around the chroma circle. */
  chroma: number;
  /** 12-bin chroma vector (pitch-class profile), each 0..1. */
  chromaVector: Float32Array;
}

export class StftAnalyzer {
  readonly fftSize: number;
  readonly hopSize: number;
  readonly sampleRate: number;
  readonly framesPerSecond: number;

  private readonly fft: FFT;
  private readonly win: Float32Array;
  private readonly re: Float32Array;
  private readonly im: Float32Array;

  private readonly prevPhase: Float32Array;
  private readonly prevLogFreq: Float32Array; // per log-bin, in semitones
  private readonly prevLogMag: Float32Array; // per log-bin, in dB
  private readonly hasPrev: { value: boolean };

  /** Precomputed linear-bin span for each log bin. */
  private readonly binLo: Int32Array;
  private readonly binHi: Int32Array;

  private readonly dynamicRange: number;
  /** Slowly-decaying peak level for auto-gain, in dB. */
  private agcPeak = -60;
  private readonly agcRelease: number;
  /**
   * Smoothing coefficients derived from time constants rather than written as
   * per-frame numbers. A fixed per-frame coefficient means the picture's
   * dynamics change whenever the hop changes — `ultra` runs at 375 frames/sec
   * against `coarse`'s 47, so the same constant would settle eight times
   * faster and the whole image would visibly twitch. These make every Detail
   * preset behave identically in wall-clock terms.
   */
  private readonly coherenceAlpha: number;
  private readonly agcAttackAlpha: number;
  /** Per-bin smoothed coherence. Raw coherence is a frame-to-frame comparison,
   *  so on any band that is borderline tonal it flickers hard enough to strobe. */
  private readonly coherenceEma: Float32Array;

  /** RGBA8, one texel per log bin. R=magnitude G=pitchclass B=coherence A=transient */
  readonly pixels: Uint8Array;
  private readonly chromaVector = new Float32Array(12);

  constructor(opts: StftOptions) {
    this.sampleRate = opts.sampleRate;
    this.fftSize = opts.fftSize ?? 4096;
    this.hopSize = opts.hopSize ?? 1024;
    this.dynamicRange = opts.dynamicRange ?? 70;
    this.framesPerSecond = this.sampleRate / this.hopSize;

    this.fft = new FFT(this.fftSize);
    this.win = makeWindow(opts.window ?? "hann", this.fftSize);
    this.re = new Float32Array(this.fftSize);
    this.im = new Float32Array(this.fftSize);

    const nyquistBins = this.fftSize / 2;
    this.prevPhase = new Float32Array(nyquistBins);
    this.prevLogFreq = new Float32Array(N_BINS);
    this.prevLogMag = new Float32Array(N_BINS).fill(-200);
    this.coherenceEma = new Float32Array(N_BINS);
    this.hasPrev = { value: false };
    // ~8 dB/second, so the picture re-normalizes over about a second instead
    // of pumping visibly on every transient.
    this.agcRelease = 8 / this.framesPerSecond;
    const perFrame = (tauSeconds: number) =>
      1 - Math.exp(-1 / (this.framesPerSecond * tauSeconds));
    this.coherenceAlpha = perFrame(COHERENCE_TAU);
    this.agcAttackAlpha = perFrame(AGC_ATTACK_TAU);

    this.pixels = new Uint8Array(N_BINS * 4);

    // Map each log bin to the linear FFT bins that fall inside its band.
    this.binLo = new Int32Array(N_BINS);
    this.binHi = new Int32Array(N_BINS);
    const hzPerBin = this.sampleRate / this.fftSize;
    for (let i = 0; i < N_BINS; i++) {
      const fLo = F_MIN * Math.pow(2, (i - 0.5) / BINS_PER_OCTAVE);
      const fHi = F_MIN * Math.pow(2, (i + 0.5) / BINS_PER_OCTAVE);
      let lo = Math.floor(fLo / hzPerBin);
      let hi = Math.ceil(fHi / hzPerBin);
      lo = Math.max(1, Math.min(nyquistBins - 1, lo));
      hi = Math.max(lo + 1, Math.min(nyquistBins, hi));
      this.binLo[i] = lo;
      this.binHi[i] = hi;
    }
  }

  /**
   * Analyze one frame. `block` must be `fftSize` mono samples; the caller is
   * responsible for hopping it by `hopSize`. Fills `this.pixels` and returns
   * per-frame scalars.
   */
  analyze(block: Float32Array): FrameStats {
    const { fftSize, hopSize, sampleRate, re, im, win } = this;

    for (let i = 0; i < fftSize; i++) {
      re[i] = block[i] * win[i];
      im[i] = 0;
    }
    this.fft.forward(re, im);

    // Expected phase advance per hop for bin k is 2πkH/N; whatever the signal
    // does beyond that is the partial's offset from the bin centre.
    const binsPerRadian = fftSize / (2 * Math.PI * hopSize);
    const hzPerBin = sampleRate / fftSize;

    let energySum = 0;
    let centroidSum = 0;
    let fluxSum = 0;
    let chromaX = 0;
    let chromaY = 0;
    this.chromaVector.fill(0);

    // Running peak decays ~1.5 dB/frame so the picture re-normalizes over a
    // second or so rather than pumping on every transient.
    let framePeakDb = -200;

    for (let i = 0; i < N_BINS; i++) {
      const lo = this.binLo[i];
      const hi = this.binHi[i];

      let bandEnergy = 0;
      let peakMag = 0;
      let peakBin = lo;
      let peakDev = 0;

      for (let k = lo; k < hi; k++) {
        const mr = re[k];
        const mi = im[k];
        const mag2 = mr * mr + mi * mi;
        bandEnergy += mag2;

        if (mag2 > peakMag) {
          const phase = Math.atan2(mi, mr);
          const expected = (2 * Math.PI * k * hopSize) / fftSize;
          const dev = princarg(phase - this.prevPhase[k] - expected);
          peakMag = mag2;
          peakBin = k;
          peakDev = dev;
        }
      }

      // Refresh the phase memory for every bin in the band, not just the peak
      // — next frame's estimate for a bin is only valid if we stored its phase
      // this frame.
      for (let k = lo; k < hi; k++) {
        this.prevPhase[k] = Math.atan2(im[k], re[k]);
      }

      // Sub-bin true frequency of the dominant partial in this band.
      const instFreq = (peakBin + peakDev * binsPerRadian) * hzPerBin;

      // dB, normalized per-band by bandwidth so log bins are comparable.
      const rms = Math.sqrt(bandEnergy / (hi - lo)) / (fftSize * 0.25);
      const db = 20 * Math.log10(rms + 1e-12);
      if (db > framePeakDb) framePeakDb = db;

      // --- magnitude → luminance -------------------------------------------
      // Soft-knee gate above the floor. Without it, room tone and the decoder's
      // own dither sit at a low but nonzero level across every band and wash
      // the whole field to grey, which buries the actual music in haze.
      const raw = clamp01((db - (this.agcPeak - this.dynamicRange)) / this.dynamicRange);
      const norm = smoothstep(NOISE_GATE, 1, raw);

      // --- instantaneous frequency → pitch class → hue ----------------------
      const semitones =
        instFreq > 0 ? 12 * Math.log2(instFreq / 440) + 69 : 0;
      let pitchClass = semitones % 12;
      if (pitchClass < 0) pitchClass += 12;

      // --- frequency stability → saturation ---------------------------------
      // A held partial keeps the same instantaneous frequency frame to frame;
      // broadband noise picks a different "peak" every time.
      let coherence = 0;
      if (this.hasPrev.value && norm > 0.02) {
        const drift = Math.abs(semitones - this.prevLogFreq[i]);
        coherence = Math.exp(-drift / 0.35);
      }
      // Fast attack, slow release: a partial should colour up the instant it
      // arrives, but a single unstable frame must not strobe it back to grey.
      const prevCoh = this.coherenceEma[i];
      coherence =
        coherence > prevCoh
          ? coherence
          : prevCoh + (coherence - prevCoh) * this.coherenceAlpha;
      this.coherenceEma[i] = coherence;

      // --- spectral flux → transient bloom -----------------------------------
      const rise = db - this.prevLogMag[i];
      const transient = clamp01(rise / 12);
      fluxSum += Math.max(0, rise) * norm;

      this.prevLogFreq[i] = semitones;
      this.prevLogMag[i] = db;

      const o = i * 4;
      this.pixels[o] = (norm * 255) | 0;
      this.pixels[o + 1] = ((pitchClass / 12) * 255) | 0;
      this.pixels[o + 2] = (coherence * 255) | 0;
      this.pixels[o + 3] = (transient * 255) | 0;

      // Weighted aggregates for the global stats.
      const w = norm * norm;
      energySum += w;
      centroidSum += w * (i / N_BINS);
      const ang = (pitchClass / 12) * 2 * Math.PI;
      chromaX += w * coherence * Math.cos(ang);
      chromaY += w * coherence * Math.sin(ang);
      this.chromaVector[Math.floor(pitchClass) % 12] += w * coherence;
    }

    this.hasPrev.value = true;

    // AGC. Snapping straight to each frame's peak modulates the whole
    // picture's brightness frame to frame, which reads as vertical banding
    // across the waterfall; ease into rises and fall back at a fixed dB/sec.
    if (framePeakDb > this.agcPeak) {
      this.agcPeak += (framePeakDb - this.agcPeak) * this.agcAttackAlpha;
    } else {
      this.agcPeak = Math.max(-60, this.agcPeak - this.agcRelease);
    }

    let maxChroma = 0;
    for (let i = 0; i < 12; i++) maxChroma = Math.max(maxChroma, this.chromaVector[i]);
    if (maxChroma > 0) {
      for (let i = 0; i < 12; i++) this.chromaVector[i] /= maxChroma;
    }

    let chroma = Math.atan2(chromaY, chromaX) / (2 * Math.PI);
    if (chroma < 0) chroma += 1;

    return {
      level: clamp01(Math.sqrt(energySum / N_BINS)),
      centroid: energySum > 0 ? clamp01(centroidSum / energySum) : 0,
      flux: clamp01(fluxSum / (N_BINS * 0.5)),
      chroma,
      chromaVector: this.chromaVector,
    };
  }
}

/** Normalized level below which a band is treated as silence. */
const NOISE_GATE = 0.18;

/** Release time for per-bin tonal coherence, in seconds. */
const COHERENCE_TAU = 0.15;
/** How fast auto-gain climbs toward a louder peak, in seconds. */
const AGC_ATTACK_TAU = 0.075;

function clamp01(x: number): number {
  return x < 0 ? 0 : x > 1 ? 1 : x;
}

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = clamp01((x - edge0) / (edge1 - edge0));
  return t * t * (3 - 2 * t);
}

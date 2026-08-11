/**
 * Analysis windows.
 *
 * `gaussian` makes this a literal Gabor transform: the Gaussian uniquely
 * minimizes the time-bandwidth uncertainty product, so it is the sharpest
 * possible joint time-frequency view. `hann` is the default anyway, because
 * the instantaneous-frequency estimator in `stft.ts` assumes a window whose
 * spectral leakage falls off fast and symmetrically around each peak — Hann's
 * -18 dB/octave sidelobe rolloff keeps a partial's phase from being polluted
 * by its neighbours, which the Gaussian's fatter skirts do not.
 *
 * Use gaussian when you want the prettiest picture, hann when you want the
 * phase math to be trustworthy.
 */
export type WindowKind = "hann" | "gaussian" | "blackman-harris";

export function makeWindow(kind: WindowKind, size: number): Float32Array {
  const w = new Float32Array(size);
  switch (kind) {
    case "hann":
      for (let i = 0; i < size; i++) {
        w[i] = 0.5 * (1 - Math.cos((2 * Math.PI * i) / size));
      }
      break;

    case "gaussian": {
      // σ = 0.4 of the half-width: wide enough that the window reaches ~0.04
      // at the edges, so truncation discontinuity stays negligible.
      const sigma = 0.4;
      const mid = (size - 1) / 2;
      for (let i = 0; i < size; i++) {
        const x = (i - mid) / (sigma * mid);
        w[i] = Math.exp(-0.5 * x * x);
      }
      break;
    }

    case "blackman-harris": {
      const a = [0.35875, 0.48829, 0.14128, 0.01168];
      for (let i = 0; i < size; i++) {
        const t = (2 * Math.PI * i) / size;
        w[i] =
          a[0] - a[1] * Math.cos(t) + a[2] * Math.cos(2 * t) - a[3] * Math.cos(3 * t);
      }
      break;
    }
  }
  return w;
}

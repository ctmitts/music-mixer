/**
 * Iterative in-place radix-2 complex FFT.
 *
 * Allocation-free after construction: `forward()` mutates the caller's arrays.
 * The analysis loop runs this ~47x/second, so the twiddle factors and the
 * bit-reversal permutation are precomputed once per size.
 */
export class FFT {
  readonly size: number;
  private readonly cos: Float32Array;
  private readonly sin: Float32Array;
  private readonly rev: Uint32Array;

  constructor(size: number) {
    if (size < 2 || (size & (size - 1)) !== 0) {
      throw new Error(`FFT size must be a power of two, got ${size}`);
    }
    this.size = size;
    this.cos = new Float32Array(size / 2);
    this.sin = new Float32Array(size / 2);
    for (let i = 0; i < size / 2; i++) {
      this.cos[i] = Math.cos((-2 * Math.PI * i) / size);
      this.sin[i] = Math.sin((-2 * Math.PI * i) / size);
    }

    // Bit-reversal permutation table.
    this.rev = new Uint32Array(size);
    const bits = Math.log2(size);
    for (let i = 0; i < size; i++) {
      let r = 0;
      for (let b = 0; b < bits; b++) {
        r = (r << 1) | ((i >>> b) & 1);
      }
      this.rev[i] = r;
    }
  }

  /** In-place forward transform. `re`/`im` must both be `size` long. */
  forward(re: Float32Array, im: Float32Array): void {
    const n = this.size;

    // Reorder into bit-reversed index order. Guarding on i < r swaps each
    // pair exactly once.
    for (let i = 0; i < n; i++) {
      const r = this.rev[i];
      if (i < r) {
        let t = re[i];
        re[i] = re[r];
        re[r] = t;
        t = im[i];
        im[i] = im[r];
        im[r] = t;
      }
    }

    // Danielson-Lanczos butterflies, doubling the block size each pass.
    for (let len = 2; len <= n; len <<= 1) {
      const half = len >> 1;
      const step = n / len;
      for (let i = 0; i < n; i += len) {
        for (let j = 0, k = 0; j < half; j++, k += step) {
          const wr = this.cos[k];
          const wi = this.sin[k];
          const a = i + j;
          const b = a + half;
          const xr = re[b] * wr - im[b] * wi;
          const xi = re[b] * wi + im[b] * wr;
          re[b] = re[a] - xr;
          im[b] = im[a] - xi;
          re[a] += xr;
          im[a] += xi;
        }
      }
    }
  }
}

/** Wrap a phase difference into (-π, π]. The phase vocoder's workhorse. */
export function princarg(phase: number): number {
  const TWO_PI = 2 * Math.PI;
  let p = phase - TWO_PI * Math.round(phase / TWO_PI);
  // Round-half-to-even can leave p exactly at -π; normalize to +π.
  if (p <= -Math.PI) p += TWO_PI;
  return p;
}

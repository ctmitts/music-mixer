/**
 * Where samples come from.
 *
 * The renderer never knows. In the browser this is an AudioWorklet tapping a
 * Web Audio graph; inside Mix Table it is a Tauri channel fed by a ring buffer
 * that the Rust audio callback writes to. Both hand over the same thing: mono
 * f32 at a known sample rate.
 */
export interface AudioSource {
  /** Device/engine sample rate. Valid once `start()` resolves. */
  readonly sampleRate: number;

  /** Begin delivering samples to `onSamples`. */
  start(onSamples: (chunk: Float32Array) => void): Promise<void>;

  stop(): void;
}

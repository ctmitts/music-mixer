import { Channel, invoke } from "@tauri-apps/api/core";
import type { AudioSource } from "./types";

/**
 * Taps the Mix Table Rust engine.
 *
 * Mix Table's audio never enters the webview — symphonia decodes, the cpal
 * callback mixes, and the result goes straight to the device. So there is no
 * AudioNode to observe. Instead the Rust audio callback pushes post-limiter
 * frames into an `rtrb` ring (exactly as `rec_tx` already does for recording),
 * a drain thread pulls them off, and they arrive here over a Tauri channel.
 *
 * Mono f32 at 48 kHz is ~192 kB/s over IPC, which is negligible. If the main
 * thread ever gets tight, the next step is to move the STFT itself into that
 * Rust drain thread and ship packed RGBA rows instead — ~61 kB/s and no FFT on
 * the UI thread at all. See docs/mixer-integration.md.
 *
 * The import of `@tauri-apps/api/core` is deliberately **static**. An earlier
 * version hid it behind a dynamic import with a non-literal specifier, so that
 * this file would still type-check in the standalone browser build where the
 * package is not installed. That works for tsc and breaks at runtime: a bundler
 * cannot statically analyse a computed specifier, so Vite emits the bare string
 * untouched and the browser fails with "does not resolve to a valid URL". In
 * the standalone repo the package is supplied as a types-only shim instead
 * (`src/tauri-shim.d.ts`), and this module is never imported there anyway.
 */

export class TauriSource implements AudioSource {
  private _sampleRate = 48000;
  private stopFn: (() => void) | null = null;

  get sampleRate(): number {
    return this._sampleRate;
  }

  /** Reads the engine's actual device rate before analysis is configured. */
  async prepare(): Promise<void> {
    this._sampleRate = await invoke<number>("viz_sample_rate");
  }

  async start(onSamples: (chunk: Float32Array) => void): Promise<void> {
    const channel = new Channel<ArrayBuffer | number[]>();
    channel.onmessage = (msg: ArrayBuffer | number[]) => {
      // Tauri hands raw responses back as an ArrayBuffer where supported and
      // a plain number array otherwise.
      const bytes =
        msg instanceof ArrayBuffer ? new Uint8Array(msg) : new Uint8Array(msg);
      onSamples(
        new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4),
      );
    };

    await invoke("start_visualizer", { channel });
    this.stopFn = () => {
      void invoke("stop_visualizer");
    };
  }

  stop(): void {
    this.stopFn?.();
    this.stopFn = null;
  }
}

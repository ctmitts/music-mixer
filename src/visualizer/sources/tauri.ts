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
 * `@tauri-apps/api` is resolved through a non-literal specifier so this module
 * stays importable — and type-checkable — in a plain browser build where the
 * package is not installed and the class is simply never constructed.
 */

/** The slice of `@tauri-apps/api/core` this file uses. */
interface TauriCore {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  Channel: new <T>() => { onmessage: (msg: T) => void };
}

const TAURI_CORE = "@tauri-apps/api/core";

async function loadTauri(): Promise<TauriCore> {
  return (await import(/* @vite-ignore */ TAURI_CORE)) as unknown as TauriCore;
}

export class TauriSource implements AudioSource {
  private _sampleRate = 48000;
  private stopFn: (() => void) | null = null;

  get sampleRate(): number {
    return this._sampleRate;
  }

  /** Reads the engine's actual device rate before analysis is configured. */
  async prepare(): Promise<void> {
    const { invoke } = await loadTauri();
    this._sampleRate = await invoke<number>("viz_sample_rate");
  }

  async start(onSamples: (chunk: Float32Array) => void): Promise<void> {
    const { invoke, Channel } = await loadTauri();

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

import type { AudioSource } from "./types";

/**
 * Taps any Web Audio node. Used by the standalone app (file playback or mic)
 * and by any React host whose audio actually lives in the browser.
 *
 * The worklet is inlined as a blob so this package stays a single import with
 * no build-config requirements on the host app.
 */

const WORKLET_SOURCE = `
class TapProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    const left = input[0];
    if (!left) return true;
    const right = input.length > 1 ? input[1] : null;

    // Downmix to mono up front: the analysis is mono, and copying one array
    // across the thread boundary rather than two halves the traffic.
    const out = new Float32Array(left.length);
    if (right) {
      for (let i = 0; i < left.length; i++) out[i] = (left[i] + right[i]) * 0.5;
    } else {
      out.set(left);
    }
    this.port.postMessage(out, [out.buffer]);
    return true;
  }
}
registerProcessor('viz-tap', TapProcessor);
`;

export class WebAudioSource implements AudioSource {
  private node: AudioWorkletNode | null = null;
  private blobUrl: string | null = null;

  constructor(
    private readonly context: AudioContext,
    /** The node to observe — typically your master gain. */
    private readonly input: AudioNode,
  ) {}

  get sampleRate(): number {
    return this.context.sampleRate;
  }

  async start(onSamples: (chunk: Float32Array) => void): Promise<void> {
    const blob = new Blob([WORKLET_SOURCE], { type: "application/javascript" });
    this.blobUrl = URL.createObjectURL(blob);
    await this.context.audioWorklet.addModule(this.blobUrl);

    this.node = new AudioWorkletNode(this.context, "viz-tap", {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    this.node.port.onmessage = (e) => onSamples(e.data as Float32Array);

    this.input.connect(this.node);
    // A worklet with no downstream connection may be culled by the graph, but
    // routing it to the speakers would double the audio. Park it on a muted
    // gain so it stays alive and silent.
    const sink = this.context.createGain();
    sink.gain.value = 0;
    this.node.connect(sink).connect(this.context.destination);
  }

  stop(): void {
    if (this.node) {
      this.node.port.onmessage = null;
      try {
        this.input.disconnect(this.node);
      } catch {
        /* already torn down */
      }
      this.node.disconnect();
      this.node = null;
    }
    if (this.blobUrl) {
      URL.revokeObjectURL(this.blobUrl);
      this.blobUrl = null;
    }
  }
}

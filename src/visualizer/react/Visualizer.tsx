import { useEffect, useRef } from "react";
import { Visualizer, type VisualizerOptions } from "../visualizer";
import type { AudioSource } from "../sources/types";
import type { Mode } from "../render/renderer";

export interface VisualizerViewProps extends VisualizerOptions {
  /** Built once by the host — a WebAudioSource or a TauriSource. */
  source: AudioSource;
  mode?: Mode;
  className?: string;
  style?: React.CSSProperties;
}

/**
 * Drop-in React surface.
 *
 * The engine is created once per `source` and kept in a ref; parameter changes
 * are pushed onto the live instance rather than remounting it, because tearing
 * down the GL context would drop the spectral history and visibly restart the
 * picture.
 */
export function VisualizerView({
  source,
  mode = "mandala",
  className,
  style,
  ...options
}: VisualizerViewProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const vizRef = useRef<Visualizer | null>(null);

  useEffect(() => {
    if (!canvasRef.current) return;
    const viz = new Visualizer(canvasRef.current, { ...options, mode });
    vizRef.current = viz;
    let cancelled = false;

    void viz.attach(source).catch((err) => {
      if (!cancelled) console.error("visualizer failed to attach", err);
    });

    return () => {
      cancelled = true;
      viz.dispose();
      vizRef.current = null;
    };
    // Only the source identity should rebuild the pipeline.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source]);

  useEffect(() => {
    vizRef.current?.setMode(mode);
  }, [mode]);

  useEffect(() => {
    const viz = vizRef.current;
    if (!viz) return;
    Object.assign(viz.params, options);
  });

  return (
    <canvas
      ref={canvasRef}
      className={className}
      style={{ display: "block", width: "100%", height: "100%", ...style }}
    />
  );
}

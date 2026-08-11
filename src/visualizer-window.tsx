/**
 * The visualizer window.
 *
 * Runs in its own webview so it can be dragged onto a TV or projector and
 * fullscreened there while the mixer UI stays on the main display. It taps the
 * Rust engine directly — Mix Table's audio never enters the webview, so there
 * is nothing in Web Audio to observe.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { Visualizer } from "./visualizer/visualizer";
import { TauriSource } from "./visualizer/sources/tauri";
import { MODES, type Mode } from "./visualizer/render/renderer";

/** Latency presets for the usual ways of getting sound to a TV. */
const SYNC_PRESETS: { label: string; ms: number }[] = [
  { label: "Built-in / HDMI", ms: 30 },
  { label: "AV receiver", ms: 120 },
  { label: "AirPlay", ms: 1800 },
];

function VisualizerWindow() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const vizRef = useRef<Visualizer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("mandala");
  const [windowSeconds, setWindowSeconds] = useState(10);
  const [syncMs, setSyncMs] = useState(30);
  const [chromeVisible, setChromeVisible] = useState(true);

  useEffect(() => {
    if (!canvasRef.current) return;
    let viz: Visualizer;
    try {
      viz = new Visualizer(canvasRef.current, { mode: "mandala" });
    } catch (e) {
      setError(String(e));
      return;
    }
    vizRef.current = viz;

    const source = new TauriSource();
    let cancelled = false;

    void (async () => {
      try {
        // The analyzer sizes itself from the device rate, so this has to
        // resolve before attaching.
        await source.prepare();
        if (cancelled) return;
        await viz.attach(source);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();

    return () => {
      cancelled = true;
      viz.dispose();
      vizRef.current = null;
    };
  }, []);

  useEffect(() => {
    vizRef.current?.setMode(mode);
  }, [mode]);

  useEffect(() => {
    if (vizRef.current) vizRef.current.params.windowSeconds = windowSeconds;
  }, [windowSeconds]);

  useEffect(() => {
    if (vizRef.current) vizRef.current.params.syncOffsetMs = syncMs;
  }, [syncMs]);

  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key >= "1" && e.key <= "4") setMode(MODES[+e.key - 1]);
      else if (e.key.toLowerCase() === "f") toggleFullscreen();
      else if (e.key.toLowerCase() === "h") setChromeVisible((v) => !v);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleFullscreen]);

  // Idle-hide: on a TV the controls should disappear on their own.
  useEffect(() => {
    let timer: number;
    const wake = () => {
      setChromeVisible(true);
      clearTimeout(timer);
      timer = window.setTimeout(() => setChromeVisible(false), 3500);
    };
    wake();
    window.addEventListener("mousemove", wake);
    return () => {
      window.removeEventListener("mousemove", wake);
      clearTimeout(timer);
    };
  }, []);

  return (
    <div style={{ position: "fixed", inset: 0, background: "#000" }}>
      <canvas ref={canvasRef} style={{ display: "block", width: "100%", height: "100%" }} />

      {error && (
        <div style={{ position: "fixed", top: 20, left: 20, color: "#ff8a65", maxWidth: 520 }}>
          {error}
        </div>
      )}

      <div
        style={{
          position: "fixed",
          bottom: 18,
          left: "50%",
          transform: "translateX(-50%)",
          display: "flex",
          gap: 14,
          alignItems: "center",
          padding: "10px 16px",
          borderRadius: 12,
          background: "rgba(10,12,20,0.72)",
          backdropFilter: "blur(18px)",
          border: "1px solid rgba(255,255,255,0.09)",
          opacity: chromeVisible ? 1 : 0,
          transition: "opacity 0.5s ease",
          pointerEvents: chromeVisible ? "auto" : "none",
        }}
      >
        <div style={{ display: "flex", gap: 5 }}>
          {MODES.map((m) => (
            <button
              key={m}
              onClick={() => setMode(m)}
              style={{
                font: "inherit",
                fontSize: 11.5,
                padding: "6px 11px",
                borderRadius: 7,
                cursor: "pointer",
                textTransform: "capitalize",
                color: mode === m ? "#fff" : "#b9c0d6",
                border: "1px solid " + (mode === m ? "transparent" : "rgba(255,255,255,0.1)"),
                background:
                  mode === m ? "linear-gradient(135deg,#7b4dff,#d33ba8)" : "rgba(255,255,255,0.05)",
              }}
            >
              {m}
            </button>
          ))}
        </div>

        <label style={{ fontSize: 11, color: "#8a92ad", display: "flex", gap: 7, alignItems: "center" }}>
          Window
          <input
            type="range" min={2} max={20} step={0.5} value={windowSeconds}
            onChange={(e) => setWindowSeconds(+e.target.value)}
            style={{ width: 90, accentColor: "#a05cff" }}
          />
          <span style={{ color: "#cfd4e4", width: 38 }}>{windowSeconds.toFixed(1)}s</span>
        </label>

        <label
          style={{ fontSize: 11, color: "#8a92ad", display: "flex", gap: 7, alignItems: "center" }}
          title="The tap is pre-device, so the picture runs ahead of what you hear by the output latency. AirPlay needs ~1.8 s."
        >
          A/V sync
          <select
            value={SYNC_PRESETS.some((p) => p.ms === syncMs) ? syncMs : "custom"}
            onChange={(e) => setSyncMs(+e.target.value)}
            style={{
              font: "inherit", fontSize: 11, padding: "4px 6px", borderRadius: 6,
              background: "rgba(255,255,255,0.06)", color: "#cfd4e4",
              border: "1px solid rgba(255,255,255,0.1)",
            }}
          >
            {SYNC_PRESETS.map((p) => (
              <option key={p.ms} value={p.ms}>{p.label}</option>
            ))}
          </select>
          <input
            type="range" min={0} max={2500} step={10} value={syncMs}
            onChange={(e) => setSyncMs(+e.target.value)}
            style={{ width: 90, accentColor: "#a05cff" }}
          />
          <span style={{ color: "#cfd4e4", width: 46 }}>{syncMs} ms</span>
        </label>

        <button
          onClick={toggleFullscreen}
          style={{
            font: "inherit", fontSize: 11.5, padding: "6px 11px", borderRadius: 7,
            cursor: "pointer", color: "#b9c0d6",
            border: "1px solid rgba(255,255,255,0.1)", background: "rgba(255,255,255,0.05)",
          }}
        >
          Fullscreen
        </button>
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(<VisualizerWindow />);

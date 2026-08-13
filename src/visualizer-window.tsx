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
import { Visualizer } from "spectral-visualizer";
import { TauriSource } from "spectral-visualizer/sources/tauri";
import { MODES, type Mode } from "spectral-visualizer/render";

/**
 * Delay presets, NOT output destinations.
 *
 * These only shift the picture later to match how long audio takes to reach
 * the listener. Naming them after devices made it look like a routing control,
 * which it has never been — routing is done in macOS Control Center.
 */
const SYNC_PRESETS: { label: string; ms: number }[] = [
  { label: "30 ms \u00b7 built-in / HDMI", ms: 30 },
  { label: "120 ms \u00b7 AV receiver", ms: 120 },
  { label: "1800 ms \u00b7 AirPlay", ms: 1800 },
];

// The window spans 0.5 s to 20 s — a 40x range, so the slider is
// logarithmic. Linear steps would make everything below 2 s unreachable.
const WIN_MIN = 0.5;
const WIN_MAX = 20;
const winFromSlider = (t: number) =>
  WIN_MIN * Math.pow(WIN_MAX / WIN_MIN, t / 1000);
const sliderFromWin = (s: number) =>
  (1000 * Math.log(s / WIN_MIN)) / Math.log(WIN_MAX / WIN_MIN);
const fmtWin = (s: number) =>
  s < 1 ? `${Math.round(s * 1000)} ms` : `${s.toFixed(1)} s`;

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
      // Bounded by MODES.length, not a hardcoded digit, so new modes in the
      // package get a key without this file changing.
      const modeIdx = +e.key - 1;
      if (e.key >= "1" && e.key <= "9" && modeIdx < MODES.length)
        setMode(MODES[modeIdx]);
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
            type="range" min={0} max={1000} step={1}
            value={sliderFromWin(windowSeconds)}
            onChange={(e) => setWindowSeconds(winFromSlider(+e.target.value))}
            style={{ width: 90, accentColor: "#a05cff" }}
          />
          <span style={{ color: "#cfd4e4", width: 46 }}>{fmtWin(windowSeconds)}</span>
        </label>

        <label
          style={{ fontSize: 11, color: "#8a92ad", display: "flex", gap: 7, alignItems: "center" }}
          title="Delays the PICTURE only — it does not route audio anywhere. Choose the output device in macOS Control Center; this just compensates for that path's latency."
        >
          Video delay
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

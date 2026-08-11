import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  DeviceList,
  faderToGain,
  formatTime,
  getEngineSnapshot,
  listOutputDevices,
  peakToDb,
  setCrossfader,
  setEq,
  setGain,
  setMasterGain,
  setOutputDevice,
  startRecording,
  stopRecording,
  subscribeEngine,
} from "../engine";
import Fader from "./Fader";

function RecordButton() {
  const [recording, setRecording] = useState<{
    path: string;
    started: number;
  } | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [lastFile, setLastFile] = useState<string | null>(null);
  const timer = useRef<number | null>(null);

  const toggle = async () => {
    if (recording) {
      await stopRecording();
      if (timer.current !== null) window.clearInterval(timer.current);
      timer.current = null;
      setLastFile(recording.path.split("/").pop() ?? recording.path);
      setRecording(null);
    } else {
      try {
        const path = await startRecording();
        setRecording({ path, started: Date.now() });
        setElapsed(0);
        timer.current = window.setInterval(
          () => setElapsed((e) => e + 1),
          1000,
        );
      } catch (e) {
        setLastFile(`record failed: ${e}`);
      }
    }
  };

  return (
    <div className="record-row">
      <button
        className={`btn btn-record ${recording ? "recording" : ""}`}
        onClick={toggle}
        title="Record the master mix to a WAV in ~/Music/Mix Table Recordings"
      >
        {recording ? `■ ${formatTime(elapsed)}` : "● Rec"}
      </button>
      {lastFile && !recording && (
        <span className="record-file" title={lastFile}>
          {lastFile}
        </span>
      )}
    </div>
  );
}

const ACCENTS = ["#4fc3f7", "#ff8a65"];
const BAND_NAMES = ["LOW", "L-MID", "H-MID", "HIGH"];
const BAND_HINTS = [
  "Below 200 Hz — kick and bass",
  "200–800 Hz — where two tracks turn to mud; cut here to make room",
  "800 Hz–3 kHz — vocals and presence",
  "Above 3 kHz — air and cymbals",
];

function Meter({ peak, className = "" }: { peak: number; className?: string }) {
  const db = peakToDb(peak);
  const pct = Math.max(0, Math.min(1, (db + 60) / 60)) * 100;
  const hot = db > -3;
  return (
    <div className={`meter ${className}`}>
      <div
        className={`meter-fill ${hot ? "meter-hot" : ""}`}
        style={{ height: `${pct}%` }}
      />
    </div>
  );
}

interface StripProps {
  deck: number;
}

function ChannelStrip({ deck }: StripProps) {
  const snapshot = useSyncExternalStore(subscribeEngine, getEngineSnapshot);
  const accent = ACCENTS[deck];
  const [fader, setFader] = useState(1.0);
  // EQ sliders: 0.5 = unity. One per band, plus latching kill buttons.
  const [eq, setEqState] = useState(BAND_NAMES.map(() => 0.5));
  const [kills, setKills] = useState(BAND_NAMES.map(() => false));
  // Ganged bands move together. Off by default — during a transition you
  // want single-band moves; ganging is for shaping a track's tone.
  const [linked, setLinked] = useState<number[]>([]);

  const applyEq = (band: number, slider: number, killed: boolean) => {
    setEq(deck, band, killed ? 0 : faderToGain(slider));
  };

  const toggleLink = (band: number) =>
    setLinked((prev) =>
      prev.includes(band) ? prev.filter((b) => b !== band) : [...prev, band],
    );

  const onEqChange = (band: number, v: number) => {
    // Dragging a linked band carries the others with it, preserving the
    // offsets between them rather than snapping them all to one value.
    const move = linked.includes(band) ? linked : [band];
    setEqState((prev) => {
      const delta = v - prev[band];
      const next = [...prev];
      for (const b of move) {
        next[b] = Math.max(0, Math.min(1, prev[b] + delta));
        applyEq(b, next[b], kills[b]);
      }
      return next;
    });
  };

  const onKill = (band: number) => {
    setKills((prev) => {
      const next = [...prev];
      next[band] = !next[band];
      applyEq(band, eq[band], next[band]);
      return next;
    });
  };

  const onFader = (v: number) => {
    setFader(v);
    // Channel fader: audio-taper, unity at the top.
    setGain(deck, v * v);
  };

  return (
    <div className="strip">
      <div className="strip-label" style={{ color: accent }}>
        {deck === 0 ? "A" : "B"}
      </div>
      <div className="strip-eq">
        {[3, 2, 1, 0].map((band) => (
          <div className="eq-band" key={band}>
            <span
              className={`eq-label eq-label-click ${
                linked.includes(band) ? "linked" : ""
              }`}
              onClick={() => toggleLink(band)}
              title={`${BAND_HINTS[band]}\nClick the label to link this band — linked bands move together.`}
            >
              {linked.includes(band) ? "⛓" : ""}
              {BAND_NAMES[band]}
            </span>
            <Fader
              value={eq[band]}
              onChange={(v) => onEqChange(band, v)}
              vertical
              accent={linked.includes(band) ? "#b9a8f0" : accent}
              resetTo={0.5}
              className="fader-eq"
            />
            <button
              className={`btn btn-kill ${kills[band] ? "active" : ""}`}
              onClick={() => onKill(band)}
            >
              KILL
            </button>
          </div>
        ))}
      </div>
      <div className="strip-fader-row">
        <Fader
          value={fader}
          onChange={onFader}
          vertical
          accent={accent}
          resetTo={1}
          className="fader-channel"
        />
        <Meter peak={snapshot.decks[deck]?.peak ?? 0} />
      </div>
    </div>
  );
}

function DevicePicker() {
  const [list, setList] = useState<DeviceList | null>(null);
  const [switching, setSwitching] = useState(false);

  const refresh = () => listOutputDevices().then(setList).catch(() => {});
  useEffect(() => {
    refresh();
  }, []);

  const onChange = async (name: string) => {
    setSwitching(true);
    try {
      await setOutputDevice(name === "__default__" ? null : name);
    } finally {
      setSwitching(false);
      refresh();
    }
  };

  return (
    <div className="device-picker">
      <label>Output</label>
      <select
        disabled={switching}
        value={list?.current ?? "__default__"}
        onChange={(e) => onChange(e.target.value)}
        onFocus={refresh}
      >
        <option value="__default__">
          System default
          {list?.defaultDevice ? ` (${list.defaultDevice})` : ""}
        </option>
        {(list?.devices ?? []).map((d) => (
          <option key={d} value={d}>
            {d}
          </option>
        ))}
      </select>
      <span className="device-rate">
        {list ? `${(list.sampleRate / 1000).toFixed(1)} kHz` : ""}
      </span>
    </div>
  );
}

export default function Mixer() {
  const snapshot = useSyncExternalStore(subscribeEngine, getEngineSnapshot);
  const [xf, setXf] = useState(0.5);
  const [master, setMaster] = useState(0.8);

  return (
    <section className="mixer">
      <ChannelStrip deck={0} />
      <div className="mixer-center">
        <div className="master-row">
          <div className="master-meters">
            <Meter peak={snapshot.masterPeak[0]} />
            <Meter peak={snapshot.masterPeak[1]} />
          </div>
          <div className="master-gain">
            <span className="eq-label">MASTER</span>
            <Fader
              value={master}
              onChange={(v) => {
                setMaster(v);
                setMasterGain(faderToGain(v) * 1.25);
              }}
              vertical
              accent="#e0e0e0"
              resetTo={0.8}
              className="fader-master"
            />
          </div>
        </div>
        <RecordButton />
        <div className="xfader-row">
          <span className="xf-label" style={{ color: ACCENTS[0] }}>
            A
          </span>
          <Fader
            value={xf}
            onChange={(v) => {
              setXf(v);
              setCrossfader(v);
            }}
            accent="#e0e0e0"
            resetTo={0.5}
            className="fader-xf"
          />
          <span className="xf-label" style={{ color: ACCENTS[1] }}>
            B
          </span>
        </div>
        <DevicePicker />
      </div>
      <ChannelStrip deck={1} />
    </section>
  );
}

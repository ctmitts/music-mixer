import { useEffect, useState } from "react";
import {
  clearSessionHistory,
  formatTime,
  getSessionHistory,
  hasApiKey,
  PlayedTrack,
  recommendTracks,
  ScoredTrack,
  setApiKey,
  subscribeEngine,
  TrackMeta,
} from "../engine";
import type { DeckData } from "../App";

interface Props {
  /** Every scanned library path — the search space. */
  paths: string[];
  /** Lookup so a pick can be loaded onto a deck. */
  trackByPath: Map<string, TrackMeta>;
  decks: [DeckData, DeckData];
  /** Index of the deck currently playing, if any. */
  playingDeck: number | null;
  onLoad: (deck: number, meta: TrackMeta) => void;
  loadingDeck: number | null;
}

export default function Suggest({
  paths,
  trackByPath,
  decks,
  playingDeck,
  onLoad,
  loadingDeck,
}: Props) {
  const [description, setDescription] = useState("");
  const [results, setResults] = useState<ScoredTrack[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [usedModel, setUsedModel] = useState(false);
  const [keyPresent, setKeyPresent] = useState(false);
  const [showKeyField, setShowKeyField] = useState(false);
  const [keyDraft, setKeyDraft] = useState("");
  const [history, setHistory] = useState<PlayedTrack[]>([]);
  // Collapsing hands the vertical space back to the library browser, so you
  // can search and scroll the full list without losing the suggestions.
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    hasApiKey().then(setKeyPresent).catch(() => {});
  }, []);

  // History is recorded Rust-side when a deck starts playing, so poll it on
  // every engine tick rather than trying to mirror the rule in the UI.
  useEffect(() => {
    const refresh = () => getSessionHistory().then(setHistory).catch(() => {});
    refresh();
    return subscribeEngine(refresh);
  }, []);

  // Mix out of the playing deck; fall back to whichever deck has a track.
  const referenceDeck =
    playingDeck !== null && decks[playingDeck]?.meta
      ? playingDeck
      : decks[0]?.meta
        ? 0
        : decks[1]?.meta
          ? 1
          : null;
  const reference = referenceDeck !== null ? decks[referenceDeck] : null;

  const run = async (useModel: boolean) => {
    if (paths.length === 0) {
      setError("Scan a library folder first.");
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const res = await recommendTracks({
        paths,
        referencePath: reference?.meta?.path ?? null,
        description,
        exclude: decks.map((d) => d.meta?.path).filter((p): p is string => !!p),
        limit: 5,
        useModel,
      });
      setResults(res.tracks);
      setUsedModel(res.usedModel);
      setNotice(res.notice);
      setCollapsed(false); // new results are worth showing
      if (res.tracks.length === 0) {
        setError("No compatible tracks found — try loosening the description.");
      }
    } catch (e) {
      setError(String(e));
      setResults(null);
    } finally {
      setBusy(false);
    }
  };

  const saveKey = async () => {
    await setApiKey(keyDraft);
    setKeyDraft("");
    setShowKeyField(false);
    setKeyPresent(await hasApiKey());
  };

  // Whichever deck isn't the reference — highlighted as the natural landing
  // spot, but either deck is always clickable.
  const suggestedDeck =
    referenceDeck === 0 ? 1 : referenceDeck === 1 ? 0 : decks[0]?.meta ? 1 : 0;

  return (
    <section className={`suggest ${collapsed ? "suggest-collapsed" : ""}`}>
      <div className="suggest-bar">
        <button
          className="btn btn-collapse"
          onClick={() => setCollapsed((v) => !v)}
          title={
            collapsed
              ? "Show suggestions"
              : "Hide suggestions and give the space back to the library"
          }
        >
          {collapsed ? "▸" : "▾"}
        </button>
        <span className="suggest-title">
          Next track
          {collapsed && results && results.length > 0
            ? ` (${results.length})`
            : ""}
        </span>
        <input
          className="suggest-input"
          placeholder='Describe the sound — "dark and driving around 120, jazzy but danceable"'
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !busy) run(keyPresent);
          }}
        />
        <button
          className="btn btn-suggest"
          disabled={busy}
          onClick={() => run(keyPresent)}
          title={
            keyPresent
              ? "Shortlist locally, then let Claude pick and explain"
              : "Rank by key, tempo, and energy against the playing deck"
          }
        >
          {busy ? "Thinking…" : keyPresent ? "✦ Suggest" : "Suggest"}
        </button>
        {keyPresent && (
          <button
            className="btn"
            disabled={busy}
            onClick={() => run(false)}
            title="Skip the model and use local scoring only"
          >
            Local
          </button>
        )}
        <button
          className="btn"
          disabled={busy || history.length === 0}
          onClick={() => {
            setDescription("");
            run(keyPresent);
          }}
          title="Suggest what continues the set you've been playing"
        >
          Same feel
        </button>
        <button
          className="btn btn-keycfg"
          onClick={() => setShowKeyField((v) => !v)}
          title={
            keyPresent
              ? "Anthropic API key is set — click to replace"
              : "Add an Anthropic API key to enable Claude-written suggestions"
          }
        >
          {keyPresent ? "🔑" : "🔑 Set key"}
        </button>
      </div>

      {showKeyField && (
        <div className="suggest-keyrow">
          <input
            className="suggest-input"
            type="password"
            placeholder="Paste your Anthropic API key (stored locally, never leaves this Mac except to api.anthropic.com)"
            value={keyDraft}
            onChange={(e) => setKeyDraft(e.target.value)}
          />
          <button className="btn" onClick={saveKey} disabled={!keyDraft.trim()}>
            Save
          </button>
        </div>
      )}

      {history.length > 0 && !collapsed && (
        <div className="suggest-history-row">
          <span className="suggest-history-label">Set so far</span>
          <div className="suggest-history">
            {history.map((h, i, arr) => {
              const meta = trackByPath.get(h.path);
              return (
                <span
                  className={`suggest-history-item ${
                    i === arr.length - 1 ? "current" : ""
                  }`}
                  key={`${h.path}-${i}`}
                  title={`${h.artist || "Unknown artist"}${
                    h.camelot ? ` · ${h.camelot}` : ""
                  }${h.fluid ? " · fluid" : h.bpm ? ` · ${h.bpm.toFixed(0)} BPM` : ""}${
                    meta ? "\nClick A or B to replay" : "\nNot in the current library folders"
                  }`}
                >
                  <span className="history-title">{h.title}</span>
                  {meta && (
                    <span className="history-decks">
                      {[0, 1].map((d) => (
                        <button
                          key={d}
                          className="history-deck-btn"
                          disabled={loadingDeck !== null}
                          onClick={() => onLoad(d, meta)}
                          title={`Replay on deck ${d === 0 ? "A" : "B"}`}
                        >
                          {d === 0 ? "A" : "B"}
                        </button>
                      ))}
                    </span>
                  )}
                </span>
              );
            })}
          </div>
          <button
            className="btn btn-history-clear"
            onClick={() => clearSessionHistory().then(() => setHistory([]))}
            title="Clear the session set list"
          >
            ✕
          </button>
        </div>
      )}

      {(error || notice) && (
        <div className={`suggest-note ${error ? "suggest-error" : ""}`}>
          {error ?? notice}
        </div>
      )}

      {results && results.length > 0 && !collapsed && (
        <div className="suggest-results">
          <div className="suggest-context">
            {reference?.meta
              ? `Mixing out of “${reference.meta.title}”`
              : "Opening track"}
            {usedModel && <span className="suggest-badge">✦ Claude</span>}
          </div>
          {results.map((t) => {
            const meta = trackByPath.get(t.path);
            return (
              <div className="suggest-card" key={t.path}>
                <div className="suggest-card-main">
                  <div className="suggest-card-title">{t.title}</div>
                  <div className="suggest-card-artist">
                    {t.artist || "Unknown artist"}
                    {t.genre ? ` · ${t.genre}` : ""}
                  </div>
                  <div className="suggest-card-reason">{t.reason}</div>
                </div>
                <div className="suggest-card-facts">
                  <span
                    className={`key-chip key-${t.camelot.endsWith("A") ? "minor" : "major"}`}
                    title={t.keyName}
                  >
                    {t.camelot}
                  </span>
                  <span className="suggest-bpm">
                    {t.fluid ? "fluid" : `${t.bpm.toFixed(0)} BPM`}
                  </span>
                  <span className="suggest-dur">
                    {formatTime(t.durationSecs)}
                  </span>
                </div>
                <div className="suggest-card-load">
                  {[0, 1].map((deck) => (
                    <button
                      key={deck}
                      className={`btn btn-suggest-load ${
                        deck === 0 ? "btn-load-a" : "btn-load-b"
                      } ${deck === suggestedDeck ? "suggested" : ""}`}
                      disabled={!meta || loadingDeck !== null}
                      onClick={() => meta && onLoad(deck, meta)}
                      title={
                        deck === suggestedDeck
                          ? `Load onto deck ${deck === 0 ? "A" : "B"} (free deck)`
                          : `Load onto deck ${deck === 0 ? "A" : "B"}`
                      }
                    >
                      {deck === 0 ? "A" : "B"}
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}

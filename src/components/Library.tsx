import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Analysis,
  bpmCompatible,
  camelotCompatible,
  cancelLibraryAnalysis,
  formatTime,
  getAnalysis,
  getEngineSnapshot,
  onAnalysisProgress,
  scanLibrary,
  startLibraryAnalysis,
  TrackMeta,
  TrackStats,
} from "../engine";
import type { DeckData } from "../App";

// Fixed row height enables windowed rendering: only the visible slice of the
// library is in the DOM, so arbitrarily large collections scroll smoothly.
const ROW_H = 30;
const OVERSCAN = 12;

type SortKey =
  | "title"
  | "artist"
  | "album"
  | "genre"
  | "durationSecs"
  | "format"
  | "bpm"
  | "camelot"
  | "playCount";

const COLUMNS: { key: SortKey; label: string; className?: string }[] = [
  { key: "title", label: "Title" },
  { key: "artist", label: "Artist" },
  { key: "album", label: "Album" },
  { key: "genre", label: "Genre", className: "col-genre" },
  { key: "bpm", label: "BPM", className: "col-bpm" },
  { key: "camelot", label: "Key", className: "col-key" },
  { key: "playCount", label: "Plays", className: "col-plays" },
  { key: "durationSecs", label: "Time", className: "col-time" },
  { key: "format", label: "Format", className: "col-fmt" },
];

/** Column count including the load-buttons column, for spacer rows. */
const COL_SPAN = COLUMNS.length + 1;

interface Props {
  onLoad: (deck: number, meta: TrackMeta) => void;
  loadingDeck: number | null;
  analysisMap: Map<string, Analysis>;
  setAnalysisMap: React.Dispatch<React.SetStateAction<Map<string, Analysis>>>;
  decks: [DeckData, DeckData];
  /** Lift the scanned track list so the suggester can search it. */
  onTracksChange: (tracks: TrackMeta[]) => void;
  statsMap: Map<string, TrackStats>;
  onTaste: (path: string, loved: boolean, banned: boolean) => void;
}

export default function Library({
  onLoad,
  loadingDeck,
  analysisMap,
  setAnalysisMap,
  decks,
  onTracksChange,
  statsMap,
  onTaste,
}: Props) {
  const [folders, setFolders] = useState<string[]>([]);
  const [tracks, setTracks] = useState<TrackMeta[]>([]);
  const [scanning, setScanning] = useState(false);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewHeight, setViewHeight] = useState(600);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<1 | -1>(1);
  const [compatOnly, setCompatOnly] = useState(false);
  const [lovedOnly, setLovedOnly] = useState(false);
  const [genreFilter, setGenreFilter] = useState("");
  const [dupeCount, setDupeCount] = useState(0);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const observer = new ResizeObserver(() => setViewHeight(wrap.clientHeight));
    observer.observe(wrap);
    return () => observer.disconnect();
  }, []);

  useEffect(
    () =>
      onAnalysisProgress((p) => {
        setProgress(p.done >= p.total ? null : { done: p.done, total: p.total });
      }),
    [],
  );

  const resetScroll = () => {
    wrapRef.current?.scrollTo({ top: 0 });
    setScrollTop(0);
  };

  const onSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDir((d) => (d === 1 ? -1 : 1));
    } else {
      setSortKey(key);
      setSortDir(1);
    }
    resetScroll();
  };

  /** Scan every configured folder and merge the results. */
  const doScan = async (targets: string[]) => {
    setFolders(targets);
    localStorage.setItem("library-folders", JSON.stringify(targets));
    if (targets.length === 0) {
      setTracks([]);
      onTracksChange([]);
      return;
    }
    setScanning(true);
    setError(null);
    try {
      // A folder can vanish between sessions (moved, deleted, drive
      // unmounted). Scan each independently so one dead folder doesn't take
      // the whole library down, then drop it from the list.
      const settled = await Promise.all(
        targets.map((f) =>
          scanLibrary(f).then(
            (tracks) => ({ folder: f, tracks, ok: true }),
            () => ({ folder: f, tracks: [] as TrackMeta[], ok: false }),
          ),
        ),
      );
      const missing = settled.filter((s) => !s.ok).map((s) => s.folder);
      const live = settled.filter((s) => s.ok);
      if (missing.length > 0) {
        const kept = targets.filter((f) => !missing.includes(f));
        setFolders(kept);
        localStorage.setItem("library-folders", JSON.stringify(kept));
        setError(
          `Removed ${missing.length} folder${missing.length > 1 ? "s" : ""} that no longer exist: ${missing
            .map((m) => m.split("/").slice(-2).join("/"))
            .join(", ")}`,
        );
      }
      const results = live.map((s) => s.tracks);
      // Folders overlap in practice — an old iTunes tree and the Music.app
      // tree hold the same songs at different paths. Dedupe by path first,
      // then by content, so merging two copies of a library doesn't show
      // every song twice. Earlier folders win, so the order you add them in
      // decides which copy is kept.
      const byPath = new Map<string, TrackMeta>();
      for (const list of results) {
        for (const t of list) if (!byPath.has(t.path)) byPath.set(t.path, t);
      }
      const seen = new Set<string>();
      const merged: TrackMeta[] = [];
      let duplicates = 0;
      for (const t of byPath.values()) {
        // Duration to the second separates real alternates (live vs studio)
        // from copies of the same recording.
        const key = `${t.title.trim().toLowerCase()}|${t.artist
          .trim()
          .toLowerCase()}|${Math.round(t.durationSecs)}`;
        if (seen.has(key)) {
          duplicates++;
          continue;
        }
        seen.add(key);
        merged.push(t);
      }
      merged.sort((a, b) =>
        `${a.artist} ${a.title}`
          .toLowerCase()
          .localeCompare(`${b.artist} ${b.title}`.toLowerCase()),
      );
      setTracks(merged);
      onTracksChange(merged);
      setDupeCount(duplicates);
      resetScroll();
      // Pull cached analysis for everything we already know.
      const cached = await getAnalysis(merged.map((t) => t.path));
      setAnalysisMap((prev) => {
        const next = new Map(prev);
        for (const e of cached) next.set(e.path, e.analysis);
        return next;
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  };

  /** Re-scan whatever folders are currently configured. */
  const rescan = () => doScan(folders);

  // Remember folders across launches and rescan automatically so new
  // downloads show up without re-picking.
  useEffect(() => {
    const saved = localStorage.getItem("library-folders");
    if (saved) {
      try {
        const list = JSON.parse(saved);
        if (Array.isArray(list) && list.length) {
          doScan(list);
          return;
        }
      } catch {
        /* fall through to the single-folder key */
      }
    }
    // Migrate the pre-multi-folder setting.
    const legacy = localStorage.getItem("library-folder");
    if (legacy) doScan([legacy]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const addFolder = async () => {
    const selected = await open({
      directory: true,
      multiple: true,
      title: "Add music folders",
    });
    const picked =
      typeof selected === "string"
        ? [selected]
        : Array.isArray(selected)
          ? selected
          : [];
    if (picked.length === 0) return;
    const next = [...folders];
    for (const p of picked) if (!next.includes(p)) next.push(p);
    doScan(next);
  };

  const removeFolder = (target: string) =>
    doScan(folders.filter((f) => f !== target));

  const analyzeLibrary = () => {
    const missing = tracks
      .filter((t) => !analysisMap.has(t.path))
      .map((t) => t.path);
    if (missing.length === 0) return;
    setProgress({ done: 0, total: missing.length });
    startLibraryAnalysis(missing);
  };

  // Reference deck for compatibility: the playing deck, else deck 0 if
  // loaded, else deck 1.
  const refDeck = useMemo(() => {
    const snap = getEngineSnapshot();
    const playing = snap.decks.findIndex((d) => d.playing);
    if (playing >= 0 && decks[playing]?.analysis) return decks[playing];
    return decks[0]?.analysis ? decks[0] : decks[1]?.analysis ? decks[1] : null;
  }, [decks]);

  // Distinct genres present, for the filter dropdown.
  const genres = useMemo(() => {
    const counts = new Map<string, number>();
    for (const t of tracks) {
      const g = t.genre.trim();
      if (g) counts.set(g, (counts.get(g) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) =>
      a[0].toLowerCase().localeCompare(b[0].toLowerCase()),
    );
  }, [tracks]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    let base = !q
      ? tracks
      : tracks.filter((t) =>
          `${t.title} ${t.artist} ${t.album} ${t.genre}`
            .toLowerCase()
            .includes(q),
        );
    if (genreFilter) {
      base =
        genreFilter === "__untagged__"
          ? base.filter((t) => !t.genre.trim())
          : base.filter((t) => t.genre === genreFilter);
    }
    if (compatOnly && refDeck?.analysis) {
      const ref = refDeck.analysis;
      const refBpm = ref.bpm * refDeck.rate;
      base = base.filter((t) => {
        const a = analysisMap.get(t.path);
        if (!a) return false;
        const keyOk = camelotCompatible(a.camelot, ref.camelot);
        // Fluid tracks blend by key alone; steady tracks must also be within
        // tempo reach (half/double allowed).
        const bpmOk = a.fluid || ref.fluid || bpmCompatible(a.bpm, refBpm);
        return keyOk && bpmOk;
      });
    }
    if (lovedOnly) {
      base = base.filter((t) => statsMap.get(t.path)?.loved);
    }
    if (!sortKey) return base;
    const sorted = [...base];
    const valOf = (t: TrackMeta): string | number | null => {
      if (sortKey === "bpm") return analysisMap.get(t.path)?.bpm ?? null;
      if (sortKey === "camelot") return analysisMap.get(t.path)?.camelot ?? null;
      if (sortKey === "durationSecs") return t.durationSecs;
      // Never-played sorts as 0 rather than missing, so "most played first"
      // is one click rather than click-then-reverse.
      if (sortKey === "playCount") return statsMap.get(t.path)?.playCount ?? 0;
      return t[sortKey];
    };
    sorted.sort((a, b) => {
      const av = valOf(a);
      const bv = valOf(b);
      // Missing values always sort to the bottom regardless of direction.
      if ((av === null || av === "") !== (bv === null || bv === ""))
        return av === null || av === "" ? 1 : -1;
      if (av === null || bv === null) return 0;
      if (typeof av === "number" && typeof bv === "number")
        return (av - bv) * sortDir;
      return (
        String(av).toLowerCase().localeCompare(String(bv).toLowerCase()) *
        sortDir
      );
    });
    return sorted;
  }, [
    tracks,
    query,
    genreFilter,
    sortKey,
    sortDir,
    compatOnly,
    lovedOnly,
    statsMap,
    refDeck,
    analysisMap,
  ]);

  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const last = Math.min(
    filtered.length,
    Math.ceil((scrollTop + viewHeight) / ROW_H) + OVERSCAN,
  );
  const visible = filtered.slice(first, last);
  const padTop = first * ROW_H;
  const padBottom = (filtered.length - last) * ROW_H;
  const analyzedCount = useMemo(
    () => tracks.reduce((n, t) => n + (analysisMap.has(t.path) ? 1 : 0), 0),
    [tracks, analysisMap],
  );

  return (
    <section className="library">
      <div className="library-bar">
        <button className="btn btn-folder" onClick={addFolder}>
          {scanning
            ? "Scanning…"
            : folders.length === 0
              ? "Choose Folder"
              : "+ Folder"}
        </button>
        <button
          className="btn"
          disabled={folders.length === 0 || scanning}
          onClick={rescan}
          title="Re-scan all folders for new files"
        >
          ⟳
        </button>
        {progress ? (
          <button className="btn" onClick={() => cancelLibraryAnalysis()}>
            Analyzing {progress.done}/{progress.total} — Stop
          </button>
        ) : (
          <button
            className="btn"
            disabled={tracks.length === 0 || analyzedCount === tracks.length}
            onClick={analyzeLibrary}
            title="Compute BPM and key for all unanalyzed tracks"
          >
            Analyze ({tracks.length - analyzedCount})
          </button>
        )}
        <button
          className={`btn ${compatOnly ? "active btn-compat" : ""}`}
          disabled={!refDeck?.analysis}
          onClick={() => {
            setCompatOnly((v) => !v);
            resetScroll();
          }}
          title="Show only tracks harmonically and tempo-compatible with the current deck"
        >
          ♪ Compatible
        </button>
        <button
          className={`btn ${lovedOnly ? "active btn-loved" : ""}`}
          onClick={() => {
            setLovedOnly((v) => !v);
            resetScroll();
          }}
          title="Show only tracks you've marked as loved"
        >
          ♥ Loved
        </button>
        <select
          className="genre-select"
          value={genreFilter}
          onChange={(e) => {
            setGenreFilter(e.target.value);
            resetScroll();
          }}
          title="Filter by genre"
        >
          <option value="">All genres</option>
          {genres.map(([g, n]) => (
            <option key={g} value={g}>
              {g} ({n})
            </option>
          ))}
          <option value="__untagged__">— untagged —</option>
        </select>
        <input
          className="library-search"
          placeholder="Search title, artist, album, genre…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            resetScroll();
          }}
        />
        <span className="library-status">
          {error
            ? error
            : folders.length > 0
              ? `${filtered.length} of ${tracks.length} tracks${
                  dupeCount > 0 ? ` · ${dupeCount} duplicates hidden` : ""
                }`
              : "Point at your music collection to start"}
        </span>
      </div>

      {folders.length > 0 && (
        <div className="library-folders">
          {folders.map((f) => (
            <span className="folder-chip" key={f} title={f}>
              {f.split("/").filter(Boolean).slice(-2).join("/")}
              <button
                className="folder-chip-x"
                onClick={() => removeFolder(f)}
                title={`Stop including ${f}`}
              >
                ✕
              </button>
            </span>
          ))}
        </div>
      )}
      <div
        className="library-table-wrap"
        ref={wrapRef}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
        <table className="library-table">
          <thead>
            <tr>
              <th className="col-load"></th>
              {COLUMNS.map((c) => (
                <th
                  key={c.key}
                  className={`th-sortable ${c.className ?? ""}`}
                  onClick={() => onSort(c.key)}
                  title="Click to sort"
                >
                  {c.label}
                  {sortKey === c.key && (
                    <span className="sort-arrow">
                      {sortDir === 1 ? " ▲" : " ▼"}
                    </span>
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {padTop > 0 && (
              <tr aria-hidden="true">
                <td
                  colSpan={COL_SPAN}
                  className="row-spacer"
                  style={{ height: padTop }}
                />
              </tr>
            )}
            {visible.map((t) => {
              const a = analysisMap.get(t.path);
              const st = statsMap.get(t.path);
              return (
                <tr key={t.path} onDoubleClick={() => onLoad(0, t)}>
                  <td className="col-load">
                    <button
                      className="btn btn-load btn-load-a"
                      disabled={loadingDeck !== null}
                      onClick={() => onLoad(0, t)}
                    >
                      A
                    </button>
                    <button
                      className="btn btn-load btn-load-b"
                      disabled={loadingDeck !== null}
                      onClick={() => onLoad(1, t)}
                    >
                      B
                    </button>
                  </td>
                  <td className="col-title">{t.title}</td>
                  <td>{t.artist}</td>
                  <td>{t.album}</td>
                  <td className="col-genre">
                    {t.genre ? (
                      <span
                        className="genre-chip"
                        onClick={(e) => {
                          e.stopPropagation();
                          setGenreFilter(t.genre);
                          resetScroll();
                        }}
                        title={`Filter to ${t.genre}`}
                      >
                        {t.genre}
                      </span>
                    ) : (
                      <span className="genre-none">—</span>
                    )}
                  </td>
                  <td className="col-bpm">
                    {a ? (a.fluid ? "~" : a.bpm.toFixed(0)) : ""}
                  </td>
                  <td className="col-key">
                    {a && (
                      <span
                        className={`key-chip key-${a.camelot.endsWith("A") ? "minor" : "major"}`}
                        title={a.keyName + (a.fluid ? " · fluid tempo" : "")}
                      >
                        {a.camelot}
                      </span>
                    )}
                  </td>
                  <td className="col-plays">
                    <span
                      className={`heart ${st?.loved ? "loved" : ""}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onTaste(t.path, !st?.loved, false);
                      }}
                      title={st?.loved ? "Loved — click to unset" : "Mark as loved"}
                    >
                      {st?.loved ? "♥" : "♡"}
                    </span>
                    {st?.playCount ? (
                      <span className="plays-n">{st.playCount}</span>
                    ) : null}
                  </td>
                  <td className="col-time">
                    {t.durationSecs > 0 ? formatTime(t.durationSecs) : "—"}
                  </td>
                  <td className="col-fmt">
                    {t.format}
                    {t.sampleRate ? ` ${(t.sampleRate / 1000).toFixed(1)}k` : ""}
                  </td>
                </tr>
              );
            })}
            {padBottom > 0 && (
              <tr aria-hidden="true">
                <td
                  colSpan={COL_SPAN}
                  className="row-spacer"
                  style={{ height: padBottom }}
                />
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}

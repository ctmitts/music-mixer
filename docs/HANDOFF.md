# Mix Table — Project Handoff

Current state as of **2026-08-07**. Read this first in a new session; it is the
single source of truth for what exists, what's deliberately deferred, and the
traps that will otherwise be rediscovered the hard way.

Companion docs:
- `docs/HOW-IT-WORKS.md` — teaching doc explaining the codebase (Rust + React)
- `docs/library-notes.md` — user-editable knowledge file the recommender reads
- `docs/mixing-theory.md` — mixing theory for the user (a trained classical
  pianist): phrase structure, Camelot-as-circle-of-fifths, EQ-as-voice-leading,
  set arcs, practice program

---

## What this is

A two-deck DJ mixer for an eclectic personal collection (classical and jazz
blended with melodic dance, funk, and a deep reggae section), built
fidelity-first from `~/Downloads/mix-table-plan.md`. All three planned phases
are complete, plus a substantial amount beyond the plan.

**Stack:** Tauri 2 shell, React + TypeScript UI, Rust audio engine.
**Installed at:** `/Applications/Mix Table.app`, with a Desktop alias.
**Library:** `/Users/colin/Music/Music/Media.localized/Music` (~5,750 files).

---

## Architecture in one screen

```
src/                          React UI
  App.tsx                     deck state, load/sync/keyshift handlers
  engine.ts                   the entire IPC bridge, typed + helpers
  components/
    Deck.tsx                  transport (2 rows), cues, loops, FX, key shift
    Mixer.tsx                 4-band EQ w/ link, crossfader, meters, record
    Library.tsx               multi-folder scan, sort/filter, windowed rows
    Suggest.tsx               recommendations, set history, API key field
    Waveform.tsx              canvas overview + zoom (rAF, outside React)
    Fader.tsx                 reusable pointer-driven fader

src-tauri/src/                Rust
  lib.rs                      every #[tauri::command]; app wiring; state
  engine.rs                   decks, lock-free command ring, cpal stream
  dsp.rs                      biquads, 4-band splitter, filter, delay,
                              reverb, vocal filter, limiter, smoothing
  decode.rs                   symphonia → stereo f32 → rubato → peaks
  analysis.rs                 BPM, beat grid, key/Camelot, LUFS, energy
  recommend.rs                local scoring: harmony, tempo, energy, genre
  agent.rs                    Claude Opus 5 calls (raw HTTP; no Rust SDK)
  library.rs                  walkdir scan + lofty tags
  db.rs                       SQLite: analysis cache, cues, settings
```

**Threading rule that governs everything:** control threads may lock; the
audio callback may only use atomics and lock-free ring buffers. Track buffers
are handed back over a "trash ring" so nothing is ever freed on the audio
thread.

---

## What's built

**Phase 1** — multi-folder library with tags, two decks, overview + zoom
waveforms, 4-band kill EQ, constant-power crossfader, soft limiter, output
device selection with automatic re-decode on rate change.

**Phase 2** — BPM + beat grid, musical key → Camelot, fluid-tempo detection
(classical/rubato get key badges instead of fake beat grids), compatible-track
filter, varispeed beat sync, 4 persisted hot cues, loops, pitch faders.

**Phase 3** — key-lock time-stretch (signalsmith), per-deck FX (one-knob
filter sweep, tempo-synced delay, Freeverb reverb), mix-bus WAV recording to
`~/Music/Mix Table Recordings`, EBU R128 auto-gain to −14 LUFS with bypass.

**Beyond the plan**
- **Claude Opus 5 recommender** — two-step: the model picks artists/albums
  from the library's vocabulary that match a described *feel*, then local
  scoring ranks those for key/tempo mixability, then the model picks the final
  few and writes the reasons. Falls back to local-only with no key or on error.
- **Session play history** — recorded Rust-side when a deck starts playing;
  feeds both prompts so "keep the same feel" reads the set's arc. Clickable
  A/B replay per entry.
- **Vocal remove/isolate** — band-limited mid/side. See Deferred below.
- **Genre column** with sorting, dropdown filter with counts, untagged filter.
- **Beat jump** (±4/8 beats) and **auto-loops** (1/2/4/8 beats), grid-snapped.
- **Key shift** ±12 semitones without tempo change; shows resulting Camelot.
- **4-band EQ link** — click a band label to gang bands; dragging one moves
  the linked ones preserving offsets. Off by default.
- **Quantized start** (2026-08-11) — global Q control (OFF/BEAT/BAR/PHRASE,
  default BAR, persisted in localStorage) in the mix view. With the other deck
  running, Play snaps this deck to its own nearest grid line and *arms* it;
  the engine fires it sample-accurately as the master crosses its next
  boundary (`Cmd::PlayQuantized`, pre-roll trick in `fire_pending_starts`).
  Pressing the armed button (⧖, pulsing) cancels. Sync's phase-align step is
  bar-level (mod 4) when Q ≥ BAR, else beat-level.
- **Mix view** (2026-08-11) — `MixView.tsx`: both decks' waveforms stacked on
  a shared *output-time* axis (each lane scaled by its deck's rate), beat /
  bar / phrase ticks from each grid, cue markers with notes, a green full-
  height marker where an armed deck will enter, and a per-deck "bar.beat"
  phrase-count readout for learning to count 32s.
- **Named cues** (2026-08-11) — alt-click a hot cue to attach a note
  ("sax rip", "vocals enter"); stored in the cues table (`label` column),
  shown in tooltips, on both deck waveforms, and in the mix view.

**Caveats shared by quantize/sync/mix-view** — the analyser's bar-1 anchor is
its first detected beat, so bar/phrase alignment is self-consistent per track
but can be offset from the true musical downbeat; and key-lock stretcher
latency is not modeled, so decks with different key-lock states land a few ms
apart.

**Agreed roadmap (2026-08-10)** — phases, in order: ① sync & alignment (done,
above) → ② persistent taste DB (plays/skips/loves/transitions tables) +
Last.fm similarity & scrobbling (user opted in) → ③ set generation: beam
search over a transition-cost graph with target energy curves, seeded-radio
mode → ④ structure detection (intro/build/drop/outro; also upgrades mix-view
suggestions) → ⑤ automix. Design rationale in the 2026-08-10 session; the
Pandora API is dead/ToS-blocked — Last.fm chosen instead.

---

## Build and run

```bash
npm run tauri dev                      # dev, hot-reloads UI
npm run tauri build -- --bundles app   # release .app (note the `--`)
```

### Traps that have already cost time

1. **`incremental = false` in `[profile.dev]` is required.** Incremental
   compilation at opt-level 2 emits internal symbols that fail to link
   ("symbol(s) not found for architecture arm64") on any rebuild after the lib
   changes. Do not "clean this up."
2. **`npm run tauri build --bundles app` silently fails.** npm eats the flag
   without the `--` separator. The build errors but a stale `.app` still
   launches, so it *looks* like it worked. **Always verify the installed
   binary's mtime is newer than the newest source edit** before claiming a
   change shipped.
3. **Backgrounded `npm run tauri dev` orphans the app.** Vite dies with the
   task, leaving the binary running against a dead webview. Verify via release
   build instead.
4. **Tauri embeds the frontend in the binary.** Don't look for JS in
   `Contents/Resources/assets/` — grep `dist/assets/*.js` to confirm a
   frontend change made it into a build.
5. **DMG bundling** intermittently fails on a stale mounted volume;
   `--bundles app` skips it.

### Verification examples (kept in `src-tauri/examples/`)

- `analyze_check.rs` — BPM/key/energy for given files
- `recommend_check.rs` — local ranking + query parsing on real tracks
- `query_check.rs` — genre queries against the cached library DB
- `vocal_check.rs` — L/R correlation and dry/wet RMS of the vocal filter

---

## Deliberate decisions worth not re-litigating

- **Energy is weighted toward mastered LUFS and beat strength**, not spectral
  flux. Flux alone rates a vibrato-heavy choir as percussive — it called the
  Mozart Requiem as energetic as Sandstorm. Current spread is sane
  (Requiem 0.48 → Sandstorm 1.00).
- **Genre tags cannot express a vibe.** 399 Hotel Costes/Buddha-Bar tracks are
  spread across Electronica (127), Electronica/Dance (67), World (49),
  Lounge (47), untagged (45), Electronic (42), Classical (19) — while Sinatra
  is cleanly "Jazz". Tag matching for "dancey jazz" therefore returns exactly
  the wrong music. Hence the artist-vocabulary approach.
- **3 vs 4 EQ bands:** 4 was chosen because 200–800 Hz is where two tracks
  turn to mud. Past 4 it becomes a graphic EQ — a studio tool, not a
  performance one.
- **EQ link is off by default.** Transitions want single-band moves with one
  hand; ganging is for tone-shaping.
- **`BandSplitter4` is built from three independent LR4 crossovers**, not
  cascaded 3-band splitters — the naive cascade double-filters at 800 Hz and
  breaks flat summing, so kill stops being a true kill.
- **`ANALYSIS_VERSION` gates the cache.** Bumping it forces library
  re-analysis (parallelized across cores). Bump it whenever features change.

---

## Deferred / open

- **Vocal stripping** — parked 2026-08-05. The live center-cancel filter works
  as designed but is the wrong tool: "Get Busy" measures 0.991 L/R correlation
  (effectively mono), so there is nothing to cancel; genuinely stereo tracks
  only lose ~4 dB. The real answer is Demucs ML stem separation (~2–3 GB
  install, ~30–60 s per track, offline). The VOX control remains and is
  harmless. See the `vocal-stripping-deferred` memory note.
- **Headphone cue / PFL** — the biggest remaining gap for real DJing. Needs a
  second output device plus a cue bus so the next track can be previewed while
  the room hears the mix. Not started.
- **Quantize toggle** — cues/loops currently snap to the grid unconditionally
  in the beat-jump and auto-loop paths; there is no user-facing on/off.
- **Layout** — reworked 2026-08-07 for a 1280×800 default with the app able to
  scroll if it still doesn't fit. May need further tuning on the actual
  display.

## Library housekeeping already done

- 5,747 files consolidated into the Music.app folder, including 114 tracks
  recovered from two old drives.
- ~55 no-info tracks retagged via beets acoustic fingerprinting; 341 could not
  be identified (list was delivered to the user).
- The old `~/Music/iTunes` tree was verified fully redundant — all 55
  iTunes-only files were its own " 1" collision copies — and moved to
  `~/.Trash/iTunes-library-backup` on 2026-08-05. The iTunes XML/ITL database
  was backed up to `~/Music/Mix Table/itunes-backup/` first.
- 217 DRM `.m4p` purchases can only be replaced by re-downloading from Apple;
  a per-album checklist was delivered to the user.

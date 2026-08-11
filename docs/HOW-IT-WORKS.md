# How Mix Table Works

A guided tour of the codebase for learning Rust and React through a real
program. Read it top to bottom once, then use it as a map while you poke at
the code. Every concept is anchored to a real file in this repo.

---

## 1. The big picture

Mix Table is a **Tauri** app. One process, two worlds:

```
┌─────────────────────────────────────────────────────┐
│  Your app process                                   │
│                                                     │
│  ┌──────────────────┐      ┌─────────────────────┐  │
│  │  WebView (UI)    │ IPC  │  Rust               │  │
│  │  React + TS      │◄────►│  audio engine,      │  │
│  │  src/            │      │  decoding, database │  │
│  └──────────────────┘      │  src-tauri/src/     │  │
│                            └──────────┬──────────┘  │
│                                       │ cpal        │
│                                       ▼             │
│                               audio device          │
└─────────────────────────────────────────────────────┘
```

- The **UI** is a web page (React + TypeScript in `src/`) rendered in the
  system WebView — same tech as a browser tab.
- The **engine** is native Rust (`src-tauri/src/`). It decodes files, mixes
  audio, and talks directly to the sound card. It never blocks on the UI.
- They talk over **IPC** (inter-process-communication style messaging):
  the UI *invokes commands* ("load this track", "set crossfader to 0.3") and
  *listens for events* ("here's the playhead position, 30 times a second").

This is why the bug you found ("reload the page, music keeps playing but
decks look empty") happened: reloading the WebView wipes the UI's memory,
but the Rust engine doesn't live there — it kept playing. The fix
(`restore_decks` in `lib.rs`) makes Rust the *source of truth* the UI can
re-ask on startup. That's a general lesson: **decide deliberately which side
owns each piece of state.**

## 2. Repo map

```
music-mixer/
├─ src/                       React UI (TypeScript)
│  ├─ App.tsx                 top-level state: what's on each deck
│  ├─ engine.ts               the bridge: typed wrappers for every IPC call
│  └─ components/
│     ├─ Deck.tsx             one deck: transport, cues, loops, pitch, FX
│     ├─ Mixer.tsx            faders, EQ, crossfader, meters, record button
│     ├─ Library.tsx          scan, search, sort, analyze, compat filter
│     ├─ Waveform.tsx         canvas drawing (overview + zoomed)
│     └─ Fader.tsx            reusable drag control
└─ src-tauri/src/             Rust
   ├─ lib.rs                  every #[tauri::command]; app wiring
   ├─ engine.rs               real-time mixer: decks, command ring, cpal
   ├─ dsp.rs                  filters, EQ, delay, reverb, limiter
   ├─ decode.rs               file → f32 samples (symphonia, rubato)
   ├─ analysis.rs             BPM, key, loudness (rustfft, ebur128)
   ├─ library.rs              folder scan + tags (walkdir, lofty)
   └─ db.rs                   SQLite cache (analysis, cue points)
```

---

## 3. Rust, taught by this codebase

Rust's pitch: C-level speed with memory safety enforced *at compile time*.
No garbage collector — instead, strict rules about who owns what.

### Ownership and borrowing

Every value has exactly one **owner**. When the owner goes out of scope, the
value is freed. You can lend access with references: `&x` (read-only borrow)
or `&mut x` (exclusive, writable borrow). The compiler proves no two parts
of the program can write the same data at the same time.

In `decode.rs`:

```rust
fn peaks(samples: &[f32], bins: usize) -> Vec<f32>
```

`&[f32]` is a *slice* — a borrowed view of an array. `peaks` can read the
samples but not free or modify them; the caller keeps ownership. It returns
`Vec<f32>` — a heap-allocated growable array the *caller* will now own.

### Arc: shared ownership across threads

Sometimes one owner isn't enough. A decoded track is a giant `Vec<f32>`
(a 5-minute song ≈ 100 MB) that both the control side and the audio thread
need. `Arc<T>` ("atomically reference-counted") lets multiple owners share
one allocation; the data is freed when the *last* Arc drops.

In `engine.rs`:

```rust
pub struct TrackData { pub samples: Vec<f32>, ... }
// shipped around as Arc<TrackData> — cloning the Arc copies a pointer,
// never the 100 MB.
```

### Mutex and atomics: sharing *mutable* data

`Arc` shares read access. To *mutate* shared data you need synchronization:

- `Mutex<T>` — one thread at a time; others block. Used for the control-side
  mirror (`engine.mirror.lock().unwrap()`), where a microsecond of waiting
  is fine.
- **Atomics** (`AtomicUsize`, `AtomicBool`) — single values updated without
  locking. The audio thread publishes the playhead this way (`Shared` in
  `engine.rs`) because the audio thread must **never block** (blocking =
  audible glitch).

The rule of thumb you'll see throughout `engine.rs`: *control threads may
lock; the audio callback may only use atomics and lock-free queues.*

### The lock-free command ring

How does "user clicked play" reach the audio thread without locks? A
**ring buffer** (`rtrb` crate): a fixed-size queue where one thread pushes
and another pops, safely, without locking.

```rust
let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Cmd>::new(1024);
```

The UI's click becomes `Cmd::Play { deck: 0 }` pushed by the control side;
the audio callback pops and applies it at the start of its next block
(`drain_commands`). The same trick runs in reverse for garbage: replaced
tracks are pushed onto a "trash ring" so the 100 MB free happens on a
normal thread, never the audio one.

### Enums and match: making states impossible to misuse

Rust enums carry data. The whole engine protocol is one enum in `engine.rs`:

```rust
pub enum Cmd {
    Load { deck: usize, track: Arc<TrackData>, start_frame: usize },
    Play { deck: usize },
    SetCrossfader(f32),
    ...
}
```

`match cmd { ... }` must handle every variant — add a new command and the
compiler lists every place you forgot to handle it. Compare with a stringly
typed message system where a typo fails at runtime.

`Option<T>` (`Some`/`None`) and `Result<T, E>` (`Ok`/`Err`) are just enums
too. There is no `null` in Rust; "might not exist" is spelled
`Option<(f64, f64)>` — see `loop_region` in `DeckState`. The `?` operator in
`decode.rs` means "if this is an Err, return it to my caller now."

### Traits and generics

A trait is an interface. `cpal` gives us audio buffers as `i16`, `f32`, etc.
Rather than write the mixer four times, `engine.rs` writes it once,
generically:

```rust
fn render<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T])
```

"For any sample type `T` that can be converted from f32" — the compiler
stamps out a specialized version per type. Zero-cost abstraction: the
generated machine code is as if you'd hand-written each one.

### Closures and threads

`std::thread::spawn(move || { ... })` starts an OS thread running a
**closure** (an anonymous function that can capture variables). `move`
transfers ownership of captured variables into the thread — the compiler
then guarantees the original thread can't race on them. See the state
emitter in `lib.rs` (ticks 30×/sec) and the WAV writer in
`start_recording`.

One subtlety worth knowing: cpal's `Stream` is `!Send` (not allowed to move
between threads), so `spawn_stream` in `engine.rs` builds the stream *inside*
a dedicated thread and parks that thread forever just to keep the stream
alive. Types encoding "which thread may touch this" is very Rust.

---

## 4. The audio path, end to end

1. **Decode** (`decode.rs`) — `symphonia` parses the file (MP3, FLAC, ALAC,
   …) into f32 samples at the file's native rate. Everything becomes
   interleaved stereo: `[L0, R0, L1, R1, ...]`.
2. **Resample once** — if the file's rate ≠ the output device's rate,
   `rubato` (windowed-sinc, the high-quality way) converts it at load time.
   After this, playback is just walking an array — that's why seeking is
   instant and gapless.
3. **Waveform peaks** — while we have the samples, we precompute max-abs per
   time bin (1,200 bins for the overview, 100/sec for the zoom view). The UI
   never sees raw audio, just these small arrays.
4. **Deck playback** (`engine.rs`, `DeckState::render_block`) — two modes:
   - *Varispeed* (default): fractional playhead + Catmull-Rom interpolation.
     Rate 1.04 = 4% faster *and* 4% higher pitch, like vinyl.
   - *Key lock*: the `signalsmith-stretch` time-stretcher consumes input at
     `rate` but outputs 1:1 — tempo changes, pitch doesn't.
5. **Per-deck chain** — 3-band EQ (two Linkwitz-Riley crossovers at 250 Hz /
   2.5 kHz — band gain 0.0 is a *true* kill) → filter sweep → delay → reverb
   → channel gain × auto-gain.
6. **Mix bus** — constant-power crossfader (cos/sin curves so the middle
   doesn't dip), master gain, soft limiter (instant attack, 120 ms release),
   then an optional tap into the recording ring.
7. **Out** — cpal hands our callback a buffer some hundreds of frames long,
   ~90 times a second. Everything above happens inside that callback,
   allocation-free.

Parameter changes (fader moves, EQ) are **smoothed** (`Smoothed` in
`dsp.rs`, ~10 ms one-pole ramps) — jumping a gain instantly produces an
audible click called zipper noise.

### The analysis pass (`analysis.rs`)

- **BPM**: slice the audio into 1024-sample FFT frames → measure how much
  each frame's spectrum *grew* vs the last (spectral flux = "onset
  strength") → autocorrelate that envelope: if energy spikes every 0.441 s,
  the track is 136 BPM. Parabolic interpolation refines the peak.
- **Fluid detection**: re-estimate the beat period in 12-second windows,
  constrained near the global value. Metronomic dance music deviates <1%;
  rubato Mozart swings >2.5% and gets flagged `fluid` — the UI then hides
  sync (nonsense for classical) and leans on key instead.
- **Key**: a second, much longer FFT (8192) gives 5 Hz resolution; energy
  lands in 12 pitch-class bins (a *chromagram*), which we correlate against
  the Krumhansl key profiles — 24 correlations, best wins — then map to the
  Camelot wheel (8A = A minor, etc.).
- **Loudness**: `ebur128` measures integrated LUFS; load applies gain toward
  -14 LUFS (clamped ±12 dB) so quiet classical and crushed EDM meet in the
  middle. The AG button bypasses it.

Results are cached in SQLite (`db.rs`), keyed by path + file mtime, so
analysis runs once per track ever.

---

## 5. React, taught by this codebase

React's model: **UI = f(state)**. You never "update the screen" — you update
state, and React re-runs your component functions and reconciles the DOM.

### Components and props

A component is a function returning JSX (HTML-ish syntax):

```tsx
export default function Deck({ index, accent, data, ... }: Props) {
  return <section className="deck">...</section>;
}
```

`App.tsx` renders two `<Deck>`s with different **props** (inputs). Props
flow *down*; events flow *up* via callback props (`onSetCue={...}`). Notice
`Deck` doesn't know how cues are stored — it just calls `onSetCue` and the
parent decides. That separation is most of React's art.

### State: useState and the single source of truth

```tsx
const [decks, setDecks] = useState<[DeckData, DeckData]>([...]);
```

Call `setDecks` → React re-renders with the new value. State is **immutable
by convention**: `patchDeck` builds a *new* array with a *new* object rather
than mutating, because React detects change by comparing references.

`DeckData` in `App.tsx` is the UI's entire knowledge of a deck: metadata,
waveform arrays, cues, loop, rate, FX... Deciding *where* state lives is the
recurring design question — this app keeps it in `App` (the shared parent)
because both `Deck` and `Library` need it.

### Effects: escaping the pure world

`useEffect(fn, deps)` runs `fn` after render, when anything in `deps`
changed; `[]` = run once on mount. Every bridge to the outside world lives
in one:

- subscribe to engine events (`onAnalysisProgress` in `App.tsx`)
- auto-rescan the saved library folder on startup (`Library.tsx`)
- rehydrate decks after reload (`restoreDecks()` — the fix for your bug)

The function you return from an effect is the *cleanup* (unsubscribe),
which React calls when the component unmounts.

### The engine store: useSyncExternalStore

Playhead and meters update 30×/sec from Rust. Instead of setState in every
component, `engine.ts` keeps a module-level `snapshot` updated by the Tauri
event listener, and components subscribe with:

```tsx
const snapshot = useSyncExternalStore(subscribeEngine, getEngineSnapshot);
```

That hook is React's official "my data lives outside React" adapter — it
re-renders the component whenever the store pings its listeners.

### Canvas + requestAnimationFrame: escaping React for speed

Waveforms redraw ~60×/sec. Re-rendering React components at 60 fps for
that would be wasteful, so `Waveform.tsx` opts out: a `<canvas>` element,
a `useRef` to reach it, and a `requestAnimationFrame` loop that draws
directly with the 2D API. The playhead is *extrapolated* between 30 Hz
engine updates (`deckPosition` in `engine.ts`) so motion looks continuous.
Rule of thumb: React for structure, canvas/rAF for high-frequency pixels.

### useMemo and the windowed library list

`useMemo(fn, deps)` caches a computed value until its inputs change — the
library's filter + sort over 5,500 tracks recomputes only when the query,
sort, or data changes, not on every render.

The table itself renders ~40 rows no matter how many tracks you have:
fixed row height × scroll position → which slice is visible → two spacer
rows fake the full scroll height. That's "windowed rendering," and it's why
the list stays instant at any size.

---

## 6. The IPC contract

`engine.ts` is the entire bridge, in one file, typed:

```ts
export const loadTrack = (deck: number, path: string) =>
  invoke<LoadResult>("load_track", { deck, path });
```

matches, in `lib.rs`:

```rust
#[tauri::command]
async fn load_track(..., deck: usize, path: String) -> Result<LoadResult, String>
```

- Rust structs derive `Serialize` with `#[serde(rename_all = "camelCase")]`
  so `duration_secs` in Rust arrives as `durationSecs` in TypeScript.
- Slow commands are `async` and run on a worker (`spawn_blocking`) so a
  4-second decode never freezes the UI or other commands.
- Rust → UI pushes use events: `app.emit("engine-state", ...)` /
  `listen("engine-state", ...)`. Commands pull, events push.

## 7. Where state lives (the reload lesson)

| State | Owner | Survives UI reload? | Survives app restart? |
|---|---|---|---|
| Playback, positions, FX | Rust engine (audio thread) | yes | no |
| Loaded-deck info for UI | Rust `RestoreState` mirror | yes (via `restore_decks`) | no |
| Analysis (BPM/key/LUFS) | SQLite (`mixtable.sqlite3`) | yes | yes |
| Hot cues | SQLite | yes | yes |
| Library folder choice | WebView `localStorage` | yes | yes |
| Search/sort/scroll | React state | no (by design) | no |

## 8. Exploring further

Run it:

```bash
npm run tauri dev
```

Vite hot-reloads UI edits in place; Rust edits trigger a rebuild + app
restart (which stops audio — that's the dev watcher, not a crash).

Good first exercises, in rough order of difficulty:

1. **UI only**: change a color in `App.css`; add a "clear all cues" button
   to `Deck.tsx` (loop over 4 slots calling `onSetCue(slot, null)`).
2. **UI + bridge**: add a keyboard shortcut — an effect in `App.tsx`
   listening for keydown, space = toggle play on deck A.
3. **Rust, gentle**: add `.aiff` artwork support or a new sortable column
   (bit depth is already in `TrackMeta`).
4. **Rust, engine**: add a crossfader-curve setting (the cos/sin law lives
   in one spot in `render`); wire a new `Cmd` variant end to end — that
   round trip (TS wrapper → command → enum → audio thread) is the single
   most instructive change you can make.
5. **DSP**: change the EQ crossover frequencies (constants at the top of
   `engine.rs`) and *listen*. Then read `BandSplitter` in `dsp.rs` and find
   the same Linkwitz-Riley structure on Wikipedia.

Reference docs: The Rust Book (doc.rust-lang.org/book) for chapters on
ownership (4), enums (6), and concurrency (16) — you've now seen every one
of those in production use. For React: react.dev's "Thinking in React"
mirrors exactly how `App.tsx`/`Deck.tsx` divide labor.

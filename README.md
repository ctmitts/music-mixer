# Mix Table

An AI-powered DJ mix table, in progress. Where it's headed: a recommender that
takes natural language to find compatible next tracks, and ML section tags on
every song — where the trumpet starts and ends, where the vocal sits — so you
can play track A and mix in just the trumpet section of track B.

What exists today is the foundation, built fidelity-first: a live two-deck mix
table with a native Rust audio engine (bit-accurate decode → f32 mix graph →
direct device output) and a React UI. This is Phase 1 (MVP) of the build plan.

## What it does

- **Library**: point at a folder; recursive scan with tags (title/artist/album,
  duration, format, sample rate) via lofty. Search across all fields. MP3, FLAC,
  ALAC (.m4a), WAV, AAC, OGG Vorbis, AIFF supported via symphonia.
- **Two decks**: load, play/pause, seek (click the overview waveform), one hot
  cue each (set / jump / clear).
- **Waveforms**: full-track overview + zoomed scrolling view (8 s window)
  centered on the playhead, both rendered from precomputed peak data at 60 fps.
- **Mixer**: constant-power crossfader, per-deck channel fader, 3-band EQ with
  full-kill (Linkwitz-Riley 4th-order crossovers at 250 Hz / 2.5 kHz), latching
  kill buttons, per-deck and master L/R meters, soft limiter on the master bus.
- **Output device selection** with automatic re-decode when the device sample
  rate changes.

## Architecture

```
UI (React/TS) ⇄ Tauri IPC ⇄ Rust engine
                              ├─ decode.rs   symphonia → stereo f32 → rubato resample → peaks
                              ├─ engine.rs   decks, command ring (rtrb), cpal stream, meters
                              ├─ dsp.rs      biquads, LR4 band splitter, smoothing, limiter
                              └─ library.rs  walkdir scan + lofty tags/artwork
```

- The audio callback is lock-free: control threads push commands over an `rtrb`
  ring; position/meters come back via atomics; replaced track buffers are handed
  back over a trash ring so no deallocation happens on the audio thread.
- Sample-rate policy: the engine runs at the output device's rate; tracks are
  resampled once at load time (rubato sinc, group-delay trimmed) so playback is
  a pure index walk — gapless seeking, no realtime resampling cost.
- Engine state (playhead, meters) is emitted to the UI at ~30 Hz; the UI
  extrapolates the playhead between updates for smooth 60 fps waveform motion.

## Run

```bash
npm install
npm run tauri dev
```

Requires Rust (rustup) and Node. First build takes a few minutes.

## Phase 2 (planned)

BPM/beat-grid + key analysis, Camelot wheel, beat sync, fluid-tempo mode for
classical/rubato material, swing detection. See `mix-table-plan.md`.

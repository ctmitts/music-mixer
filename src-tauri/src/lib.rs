pub mod agent;
pub mod analysis;
pub mod db;
pub mod decode;
pub mod dsp;
pub mod engine;
pub mod library;
pub mod recommend;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use analysis::Analysis;
use engine::{Cmd, Engine, LoadedTrack, TrackData, NUM_DECKS};

type EngineState<'a> = State<'a, Arc<Engine>>;
type DbState<'a> = State<'a, Arc<db::Db>>;

/// Cancel handle for the running library-analysis batch: each batch gets its
/// own flag; starting a new batch cancels and replaces the previous one.
struct BatchFlag(std::sync::Mutex<Arc<AtomicBool>>);

/// Control-side record of what's loaded on each deck so a reloaded webview
/// can rehydrate its UI without touching (or interrupting) the engine.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredDeck {
    meta: library::TrackMeta,
    result: LoadResult,
}

struct RestoreState(std::sync::Mutex<[Option<StoredDeck>; NUM_DECKS]>);

/// What's actually been played this session, oldest first. A set has an arc,
/// and the last several tracks describe the feel far better than whatever
/// happens to be on a deck right now — so this is the context the recommender
/// reasons over. Lives in the Rust process, so it survives webview reloads.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayedTrack {
    path: String,
    title: String,
    artist: String,
    genre: String,
    camelot: String,
    bpm: f64,
    fluid: bool,
    energy: f64,
}

struct SessionHistory(std::sync::Mutex<Vec<PlayedTrack>>);

const HISTORY_CAP: usize = 60;

/// Record a track as played. Skips anything already among the last few
/// entries: pausing deck A, playing deck B, then resuming A is one continuous
/// mix, not three plays, and logging it as three both clutters the set list
/// and skews the "what have I been playing" context.
fn record_played(history: &SessionHistory, database: &db::Db, path: &str) {
    let mut log = history.0.lock().unwrap();
    if log.iter().rev().take(3).any(|e| e.path == path) {
        return;
    }
    let meta = library::track_meta(path);
    let a = database.get_analysis(path);
    log.push(PlayedTrack {
        path: path.to_string(),
        title: meta.title,
        artist: meta.artist,
        genre: meta.genre,
        camelot: a.as_ref().map(|x| x.camelot.clone()).unwrap_or_default(),
        bpm: a.as_ref().map(|x| x.bpm).unwrap_or(0.0),
        fluid: a.as_ref().map(|x| x.fluid).unwrap_or(false),
        energy: a.as_ref().map(|x| x.energy).unwrap_or(0.5),
    });
    if log.len() > HISTORY_CAP {
        let excess = log.len() - HISTORY_CAP;
        log.drain(0..excess);
    }
}

#[tauri::command]
fn get_session_history(history: State<'_, SessionHistory>) -> Vec<PlayedTrack> {
    history.0.lock().unwrap().clone()
}

#[tauri::command]
fn clear_session_history(history: State<'_, SessionHistory>) {
    history.0.lock().unwrap().clear();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeckRestore {
    meta: library::TrackMeta,
    result: LoadResult,
    rate: f32,
    key_lock: bool,
    fx: [f32; 3],
    auto_gain: f32,
    vocal: (f32, bool),
    key_shift: f32,
    eqs: [f32; engine::NUM_EQ_BANDS],
    loop_region: Option<(f64, f64)>,
}

/// Rehydrate the UI after a webview reload: what's loaded, plus live mixer
/// state from the control mirror. Playback is untouched.
#[tauri::command]
fn restore_decks(
    engine: EngineState,
    database: DbState,
    restore: State<'_, RestoreState>,
) -> Vec<Option<DeckRestore>> {
    let stored = restore.0.lock().unwrap();
    let mirror = engine.mirror.lock().unwrap();
    stored
        .iter()
        .enumerate()
        .map(|(deck, slot)| {
            slot.as_ref().map(|s| {
                let mut result = s.result.clone();
                // Cues may have changed since load; re-read from the DB.
                result.cues = database.get_cues(&s.meta.path);
                result.cue_labels = database.get_cue_labels(&s.meta.path);
                DeckRestore {
                    meta: s.meta.clone(),
                    result,
                    rate: mirror.rates[deck],
                    key_lock: mirror.key_lock[deck],
                    fx: mirror.fx[deck],
                    auto_gain: mirror.auto_gains[deck],
                    vocal: mirror.vocal[deck],
                    key_shift: mirror.key_shift[deck],
                    eqs: mirror.eqs[deck],
                    loop_region: mirror.loops[deck],
                }
            })
        })
        .collect()
}

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

#[tauri::command]
async fn scan_library(folder: String) -> Result<Vec<library::TrackMeta>, String> {
    if !library::folder_exists(&folder) {
        return Err(format!("folder no longer exists: {folder}"));
    }
    tauri::async_runtime::spawn_blocking(move || library::scan_folder(&folder))
        .await
        .map_err(err_str)
}

#[tauri::command]
async fn get_artwork(path: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || library::artwork(&path))
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// Deck / transport
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LoadResult {
    duration_secs: f64,
    source_rate: u32,
    engine_rate: u32,
    overview: Vec<f32>,
    detail: Vec<f32>,
    detail_bins_per_sec: f32,
    analysis: Option<Analysis>,
    cues: [Option<f64>; 4],
    /// User notes on cue slots ("sax rip", "vocals enter").
    cue_labels: [Option<String>; 4],
    /// Loudness-normalization gain applied at load (dB, clamped ±12).
    auto_gain_db: f64,
}

/// Gain to bring `lufs` to the -14 LUFS reference, clamped to ±12 dB.
fn auto_gain_db_for(lufs: f64) -> f64 {
    (-14.0 - lufs).clamp(-12.0, 12.0)
}

#[tauri::command]
async fn load_track(
    engine: EngineState<'_>,
    database: DbState<'_>,
    restore: State<'_, RestoreState>,
    deck: usize,
    path: String,
) -> Result<LoadResult, String> {
    if deck >= NUM_DECKS {
        return Err("invalid deck".into());
    }
    let engine_rate = engine.sample_rate();
    let path2 = path.clone();
    let db2 = database.inner().clone();
    let (decoded, track_analysis) = tauri::async_runtime::spawn_blocking(move || {
        let decoded = decode::decode_file(&path2, engine_rate)?;
        // Analyze on load if not cached: the samples are already in memory.
        let a = match db2.get_analysis(&path2) {
            Some(a) => Some(a),
            None => {
                let a = analysis::analyze(&decoded.samples, decoded.engine_rate);
                db2.put_analysis(&path2, &a);
                Some(a)
            }
        };
        anyhow::Ok((decoded, a))
    })
    .await
    .map_err(err_str)?
    .map_err(err_str)?;

    let track = Arc::new(TrackData {
        id: engine.next_id(),
        sample_rate: decoded.engine_rate,
        samples: decoded.samples,
    });

    {
        let mut mirror = engine.mirror.lock().unwrap();
        mirror.loaded[deck] = Some(LoadedTrack {
            path: path.clone(),
            track: track.clone(),
        });
    }
    engine.send(Cmd::Load {
        deck,
        track,
        start_frame: 0,
    });

    // Loudness normalization: bring this track toward -14 LUFS so classical
    // dynamics sit sanely against compressed genres. UI can override.
    let auto_gain_db = track_analysis
        .as_ref()
        .map(|a| auto_gain_db_for(a.lufs))
        .unwrap_or(0.0);
    let value = 10f32.powf(auto_gain_db as f32 / 20.0);
    engine.mirror.lock().unwrap().auto_gains[deck] = value;
    engine.send(Cmd::SetAutoGain { deck, value });
    // Default the delay time to a dotted eighth of the track tempo.
    if let Some(a) = &track_analysis {
        if !a.fluid && a.bpm > 0.0 {
            let secs = (45.0 / a.bpm) as f32;
            engine.mirror.lock().unwrap().delay_time[deck] = secs;
            engine.send(Cmd::SetDelayTime { deck, secs });
        }
    }
    engine.collect_trash();

    let result = LoadResult {
        duration_secs: decoded.duration_secs,
        source_rate: decoded.source_rate,
        engine_rate: decoded.engine_rate,
        overview: decoded.overview,
        detail: decoded.detail,
        detail_bins_per_sec: decode::DETAIL_BINS_PER_SEC,
        analysis: track_analysis,
        cues: database.get_cues(&path),
        cue_labels: database.get_cue_labels(&path),
        auto_gain_db,
    };
    restore.0.lock().unwrap()[deck] = Some(StoredDeck {
        meta: library::track_meta(&path),
        result: result.clone(),
    });
    Ok(result)
}

#[tauri::command]
fn play(
    engine: EngineState,
    database: DbState,
    history: State<'_, SessionHistory>,
    deck: usize,
) {
    // Log what actually gets played, not merely loaded — the set list is the
    // context the recommender reasons over.
    if deck < NUM_DECKS {
        let path = engine
            .mirror
            .lock()
            .unwrap()
            .loaded
            .get(deck)
            .and_then(|l| l.as_ref().map(|t| t.path.clone()));
        if let Some(p) = path {
            record_played(&history, &database, &p);
        }
    }
    engine.send(Cmd::Play { deck });
}

/// Arm `deck` to start when `master`'s playhead crosses `master_frame`
/// (engine frames). The engine fires it sample-accurately; see
/// `Cmd::PlayQuantized`.
#[tauri::command]
fn play_quantized(
    engine: EngineState,
    database: DbState,
    history: State<'_, SessionHistory>,
    deck: usize,
    master: usize,
    master_frame: f64,
) {
    if deck >= NUM_DECKS || master >= NUM_DECKS || master == deck {
        return;
    }
    // Same set-list logging as a plain play; armed decks virtually always
    // fire within a bar or two.
    let path = engine
        .mirror
        .lock()
        .unwrap()
        .loaded
        .get(deck)
        .and_then(|l| l.as_ref().map(|t| t.path.clone()));
    if let Some(p) = path {
        record_played(&history, &database, &p);
    }
    engine.send(Cmd::PlayQuantized {
        deck,
        master,
        master_frame,
    });
}

#[tauri::command]
fn pause(engine: EngineState, deck: usize) {
    engine.send(Cmd::Pause { deck });
}

#[tauri::command]
fn seek(engine: EngineState, deck: usize, seconds: f64) {
    let rate = engine.shared.sample_rate.load(Ordering::Relaxed) as f64;
    let frame = (seconds.max(0.0) * rate) as usize;
    engine.send(Cmd::Seek { deck, frame });
}

#[tauri::command]
fn set_rate(engine: EngineState, deck: usize, value: f32) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().rates[deck] = value;
        engine.send(Cmd::SetRate { deck, value });
    }
}

#[tauri::command]
fn set_key_lock(engine: EngineState, deck: usize, enabled: bool) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().key_lock[deck] = enabled;
        let stretcher = if enabled {
            Some(Box::new(signalsmith_stretch::Stretch::preset_default(
                2,
                engine.sample_rate(),
            )))
        } else {
            None
        };
        engine.send(Cmd::SetKeyLock { deck, stretcher });
    }
}

#[tauri::command]
fn set_fx(engine: EngineState, deck: usize, filter: f32, delay_mix: f32, reverb_mix: f32) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().fx[deck] = [filter, delay_mix, reverb_mix];
        engine.send(Cmd::SetFx {
            deck,
            filter,
            delay_mix,
            reverb_mix,
        });
    }
}

#[tauri::command]
fn set_delay_time(engine: EngineState, deck: usize, secs: f32) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().delay_time[deck] = secs;
        engine.send(Cmd::SetDelayTime { deck, secs });
    }
}

#[tauri::command]
fn set_vocal(engine: EngineState, deck: usize, amount: f32, isolate: bool) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().vocal[deck] = (amount, isolate);
        engine.send(Cmd::SetVocal {
            deck,
            amount,
            isolate,
        });
    }
}

#[tauri::command]
fn set_auto_gain(engine: EngineState, deck: usize, value: f32) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().auto_gains[deck] = value;
        engine.send(Cmd::SetAutoGain { deck, value });
    }
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

#[tauri::command]
fn start_recording(engine: EngineState) -> Result<String, String> {
    let sample_rate = engine.sample_rate();
    let dir = dirs_home()
        .join("Music")
        .join("Mix Table Recordings");
    std::fs::create_dir_all(&dir).map_err(err_str)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("mix-{stamp}.wav"));

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).map_err(err_str)?;

    // ~11 s of stereo float at 48 k — generous slack for writer hiccups.
    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(1 << 20);
    engine.send(Cmd::StartRecord(Box::new(producer)));

    std::thread::spawn(move || {
        loop {
            let mut wrote = false;
            while let Ok(s) = consumer.pop() {
                let _ = writer.write_sample(s);
                wrote = true;
            }
            if consumer.is_abandoned() && !wrote {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        if let Err(e) = writer.finalize() {
            eprintln!("failed to finalize recording: {e}");
        }
    });

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn stop_recording(engine: EngineState) {
    engine.send(Cmd::StopRecord);
}

// ---------------------------------------------------------------------------
// Spectral visualizer tap
// ---------------------------------------------------------------------------

/// The visualizer sizes its FFT from the device rate, so it has to ask before
/// it configures anything — and ask again whenever the output device changes.
#[tauri::command]
fn viz_sample_rate(engine: EngineState) -> u32 {
    engine.sample_rate()
}

/// Stream post-limiter mono frames to the webview.
///
/// The audio thread only ever pushes into the ring (see `Cmd::StartViz`); this
/// thread does the draining and the IPC. Running the FFT on the audio thread
/// would risk an overrun, and an overrun is an audible click.
#[tauri::command]
fn start_visualizer(
    engine: EngineState,
    channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) {
    // ~2.7 s of mono float at 48 k. The drain runs every 10 ms, so this is
    // enormous slack; it exists so that a stalled webview can never apply
    // back-pressure to the audio thread.
    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(1 << 17);
    engine.send(Cmd::StartViz(Box::new(producer)));

    std::thread::spawn(move || {
        let mut batch: Vec<f32> = Vec::with_capacity(4096);
        loop {
            batch.clear();
            while let Ok(s) = consumer.pop() {
                batch.push(s);
            }
            if consumer.is_abandoned() && batch.is_empty() {
                break;
            }
            if !batch.is_empty() {
                let mut bytes = Vec::with_capacity(batch.len() * 4);
                for s in &batch {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                // A closed channel means the webview went away, which is a
                // normal shutdown rather than an error worth reporting.
                if channel
                    .send(tauri::ipc::InvokeResponseBody::Raw(bytes))
                    .is_err()
                {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}

#[tauri::command]
fn stop_visualizer(engine: EngineState) {
    engine.send(Cmd::StopViz);
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

#[tauri::command]
fn set_loop(engine: EngineState, deck: usize, start_secs: f64, end_secs: f64) {
    let rate = engine.shared.sample_rate.load(Ordering::Relaxed) as f64;
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().loops[deck] = Some((start_secs, end_secs));
    }
    engine.send(Cmd::SetLoop {
        deck,
        start_frame: (start_secs.max(0.0) * rate) as usize,
        end_frame: (end_secs.max(0.0) * rate) as usize,
    });
}

#[tauri::command]
fn clear_loop(engine: EngineState, deck: usize) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().loops[deck] = None;
    }
    engine.send(Cmd::ClearLoop { deck });
}

// ---------------------------------------------------------------------------
// Analysis & cues
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AnalysisEntry {
    path: String,
    analysis: Analysis,
}

/// Bulk-fetch cached analysis for a scanned library.
#[tauri::command]
async fn get_analysis(
    database: DbState<'_>,
    paths: Vec<String>,
) -> Result<Vec<AnalysisEntry>, String> {
    let db = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .filter_map(|p| {
                db.get_analysis(&p).map(|a| AnalysisEntry {
                    path: p,
                    analysis: a,
                })
            })
            .collect()
    })
    .await
    .map_err(err_str)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AnalysisProgress {
    done: usize,
    total: usize,
    path: String,
    analysis: Option<Analysis>,
}

/// Analyze a set of tracks in the background (skipping cached ones), with
/// progress events. A new call cancels the previous batch.
#[tauri::command]
fn start_library_analysis(
    app: tauri::AppHandle,
    database: DbState<'_>,
    flag: State<'_, BatchFlag>,
    paths: Vec<String>,
) {
    let my_flag = Arc::new(AtomicBool::new(false));
    {
        let mut current = flag.0.lock().unwrap();
        current.store(true, Ordering::Relaxed); // cancel previous batch
        *current = my_flag.clone();
    }
    let shared_flag = my_flag;
    let db = database.inner().clone();
    std::thread::spawn(move || {
        let total = paths.len();
        // Decode + FFT is CPU-bound and embarrassingly parallel across
        // tracks; leave a couple of cores for the audio thread and the UI.
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(2).clamp(1, 8))
            .unwrap_or(2);
        let queue = Arc::new(std::sync::Mutex::new(paths.into_iter()));
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..workers {
            let queue = queue.clone();
            let done = done.clone();
            let db = db.clone();
            let app = app.clone();
            let flag = shared_flag.clone();
            handles.push(std::thread::spawn(move || loop {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Some(path) = queue.lock().unwrap().next() else {
                    break;
                };
                let result = match db.get_analysis(&path) {
                    Some(a) => Some(a),
                    None => match decode::decode_source(&path) {
                        Ok((samples, rate)) => {
                            let a = analysis::analyze(&samples, rate);
                            db.put_analysis(&path, &a);
                            Some(a)
                        }
                        Err(_) => None,
                    },
                };
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = app.emit(
                    "analysis-progress",
                    &AnalysisProgress {
                        done: n,
                        total,
                        path,
                        analysis: result,
                    },
                );
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });
}

#[tauri::command]
fn cancel_library_analysis(flag: State<'_, BatchFlag>) {
    flag.0.lock().unwrap().store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Recommendations
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecommendResult {
    tracks: Vec<recommend::ScoredTrack>,
    /// True when Claude re-ranked and wrote the reasons.
    used_model: bool,
    /// Set when the model layer was attempted but fell back to local scoring.
    notice: Option<String>,
}

/// The DJ's editable notes about their own library. Checked next to the app
/// first (so a bundled copy ships with the release), then in the repo, so the
/// file can be edited in either place.
fn load_library_notes() -> Option<String> {
    let home = dirs_home();
    let candidates = [
        home.join("Music/Mix Table/library-notes.md"),
        home.join("Desktop/music-mixer/docs/library-notes.md"),
    ];
    for p in candidates {
        if let Ok(s) = std::fs::read_to_string(&p) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Resolve the API key: an explicit setting wins, else the environment. A GUI
/// app launched from Finder inherits no shell environment, which is why the
/// stored setting exists at all.
fn resolve_api_key(database: &db::Db) -> Option<String> {
    database
        .get_setting("anthropic_api_key")
        .filter(|k| !k.trim().is_empty())
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
}

#[tauri::command]
fn set_api_key(database: DbState, key: String) {
    database.set_setting("anthropic_api_key", key.trim());
}

#[tauri::command]
fn has_api_key(database: DbState) -> bool {
    resolve_api_key(&database).is_some()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecommendArgs {
    /// Every scanned track path; analysis is read from cache.
    paths: Vec<String>,
    /// Path of the track to mix out of, if any.
    reference_path: Option<String>,
    /// Free-text description of the wanted sound (may be empty).
    description: String,
    /// Paths already loaded on decks.
    exclude: Vec<String>,
    /// How many suggestions to return.
    limit: usize,
    /// Whether to consult Claude when a key is available.
    use_model: bool,
}

#[tauri::command]
async fn recommend_tracks(
    database: DbState<'_>,
    history: State<'_, SessionHistory>,
    args: RecommendArgs,
) -> Result<RecommendResult, String> {
    let db = database.inner().clone();
    let want = args.limit.clamp(1, 20);

    // The last several tracks of the set, rendered for the prompts.
    let history_text = {
        let log = history.0.lock().unwrap();
        let recent: Vec<(String, String, String)> = log
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|e| {
                let facts = if e.camelot.is_empty() {
                    String::new()
                } else if e.fluid {
                    format!("{}, fluid tempo, energy {:.2}", e.camelot, e.energy)
                } else {
                    format!("{}, {:.0} BPM, energy {:.2}", e.camelot, e.bpm, e.energy)
                };
                let facts = if e.genre.is_empty() {
                    facts
                } else if facts.is_empty() {
                    e.genre.clone()
                } else {
                    format!("{facts}, {}", e.genre)
                };
                (e.title.clone(), e.artist.clone(), facts)
            })
            .collect();
        agent::format_history(&recent)
    };

    // Gather analyzed candidates off the UI thread.
    let paths = args.paths.clone();
    let candidates: Vec<recommend::Candidate> = tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .filter_map(|p| {
                let analysis = db.get_analysis(&p)?;
                Some(recommend::Candidate {
                    meta: library::track_meta(&p),
                    analysis,
                })
            })
            .collect()
    })
    .await
    .map_err(err_str)?;

    if candidates.is_empty() {
        return Err("No analyzed tracks yet — run Analyze on your library first.".into());
    }

    let reference = args
        .reference_path
        .as_ref()
        .and_then(|p| database.get_analysis(p));
    let query = if args.description.trim().is_empty() {
        None
    } else {
        Some(recommend::parse_query(&args.description))
    };

    // Shortlist locally, then optionally let Claude choose from it.
    let (shortlist, info) = recommend::rank(
        &candidates,
        reference.as_ref(),
        query.as_ref(),
        &args.exclude,
        (want * 8).clamp(24, 60),
    );

    // Be explicit when a requested style isn't in the library, instead of
    // silently returning whatever else scored well.
    let style_notice = if info.genre_requested_but_empty {
        Some(
            "Nothing in your library is tagged with that style — showing the \
             closest matches by key, tempo, and energy instead."
                .to_string(),
        )
    } else {
        None
    };

    let api_key = resolve_api_key(&database);
    if !args.use_model || api_key.is_none() {
        let mut tracks = shortlist;
        tracks.truncate(want);
        return Ok(RecommendResult {
            tracks,
            used_model: false,
            notice: style_notice,
        });
    }

    // Build a ScoredTrack for the reference so the model can see what's playing.
    #[allow(unused_assignments)]
    let reference_track = args.reference_path.as_ref().and_then(|p| {
        reference.as_ref().map(|a| {
            let meta = library::track_meta(p);
            recommend::ScoredTrack {
                path: p.clone(),
                title: meta.title,
                artist: meta.artist,
                genre: meta.genre,
                bpm: a.bpm,
                camelot: a.camelot.clone(),
                key_name: a.key_name.clone(),
                fluid: a.fluid,
                energy: a.energy,
                duration_secs: meta.duration_secs,
                score: 1.0,
                reason: String::new(),
            }
        })
    });

    let key = api_key.unwrap();

    // Vibe search: a described *feel* ("dancey jazz, Parisian chic") cannot be
    // resolved from genre tags — the same aesthetic is scattered across
    // Electronica / World / Lounge / untagged, while a literal tag match pulls
    // in music that merely shares the word. So when the DJ describes a feel,
    // first ask the model which artists and albums in *this* library sound
    // that way, then rank those for mixability.
    //
    // Also runs with an empty description when there's a set history, since
    // "what fits what I've been playing" is itself a feel to match.
    let mut shortlist = shortlist;
    let mut style_notice = style_notice;
    if !args.description.trim().is_empty() || !history_text.is_empty() {
        let (artists, albums) = recommend::vocabulary(&candidates);
        let notes = load_library_notes();
        match agent::select_by_vibe(
            &key,
            &args.description,
            reference_track.as_ref(),
            &artists,
            &albums,
            notes.as_deref(),
            &history_text,
        )
        .await
        {
            Ok(sel) if !sel.artists.is_empty() || !sel.albums.is_empty() => {
                let pool = recommend::filter_by_names(&candidates, &sel.artists, &sel.albums);
                if !pool.is_empty() {
                    // Rank the vibe-matched pool for mixability. Drop the text
                    // query here: the feel is already satisfied by the pool,
                    // so what remains is choosing the best-mixing tracks.
                    let (ranked, _) = recommend::rank(
                        &pool,
                        reference.as_ref(),
                        None,
                        &args.exclude,
                        (want * 8).clamp(24, 60),
                    );
                    if !ranked.is_empty() {
                        shortlist = ranked;
                        if !sel.style_note.trim().is_empty() {
                            style_notice = Some(format!(
                                "{} — {} tracks in your library",
                                sel.style_note.trim(),
                                pool.len()
                            ));
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                style_notice = Some(format!("Style match unavailable ({e}); ranked locally."));
            }
        }
    }

    match agent::refine(
        &key,
        reference_track.as_ref(),
        &args.description,
        &shortlist,
        want,
        &history_text,
    )
    .await
    {
        Ok(picks) => Ok(RecommendResult {
            tracks: picks,
            used_model: true,
            notice: style_notice,
        }),
        Err(e) => {
            let mut tracks = shortlist;
            tracks.truncate(want);
            Ok(RecommendResult {
                tracks,
                used_model: false,
                notice: Some(format!("Showing local matches — Claude call failed: {e}")),
            })
        }
    }
}

#[tauri::command]
fn set_cue_label(database: DbState, path: String, slot: usize, label: Option<String>) {
    if slot < 4 {
        database.set_cue_label(&path, slot, label.as_deref());
    }
}

#[tauri::command]
fn set_cue_point(database: DbState, path: String, slot: usize, seconds: Option<f64>) {
    if slot < 4 {
        database.set_cue(&path, slot, seconds);
    }
}

// ---------------------------------------------------------------------------
// Mixer
// ---------------------------------------------------------------------------

#[tauri::command]
fn set_gain(engine: EngineState, deck: usize, value: f32) {
    if deck < NUM_DECKS {
        engine.mirror.lock().unwrap().gains[deck] = value;
        engine.send(Cmd::SetGain { deck, value });
    }
}

#[tauri::command]
fn set_eq(engine: EngineState, deck: usize, band: usize, value: f32) {
    if deck < NUM_DECKS && band < engine::NUM_EQ_BANDS {
        engine.mirror.lock().unwrap().eqs[deck][band] = value;
        engine.send(Cmd::SetEq { deck, band, value });
    }
}

/// Transpose without changing tempo. Key lock supplies the time-stretcher
/// that does the work, so engaging a shift turns it on automatically.
#[tauri::command]
fn set_key_shift(engine: EngineState, deck: usize, semitones: f32) {
    if deck >= NUM_DECKS {
        return;
    }
    let semitones = semitones.clamp(-12.0, 12.0);
    let needs_lock = semitones != 0.0;
    let already_locked = {
        let mut m = engine.mirror.lock().unwrap();
        m.key_shift[deck] = semitones;
        let was = m.key_lock[deck];
        if needs_lock && !was {
            m.key_lock[deck] = true;
        }
        was
    };
    if needs_lock && !already_locked {
        engine.send(Cmd::SetKeyLock {
            deck,
            stretcher: Some(Box::new(signalsmith_stretch::Stretch::preset_default(
                2,
                engine.sample_rate(),
            ))),
        });
    }
    engine.send(Cmd::SetKeyShift { deck, semitones });
}

#[tauri::command]
fn set_crossfader(engine: EngineState, value: f32) {
    engine.mirror.lock().unwrap().crossfader = value;
    engine.send(Cmd::SetCrossfader(value));
}

#[tauri::command]
fn set_master_gain(engine: EngineState, value: f32) {
    engine.mirror.lock().unwrap().master = value;
    engine.send(Cmd::SetMaster(value));
}

// ---------------------------------------------------------------------------
// Output device
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceList {
    devices: Vec<String>,
    default_device: Option<String>,
    current: Option<String>,
    sample_rate: u32,
}

#[tauri::command]
fn list_output_devices(engine: EngineState) -> Result<DeviceList, String> {
    let (devices, default_device) = engine::list_output_devices().map_err(err_str)?;
    let conn = engine.conn.lock().unwrap();
    Ok(DeviceList {
        devices,
        default_device,
        current: conn.device_name.clone(),
        sample_rate: conn.sample_rate,
    })
}

/// Switch output device: build the new stream first (so failure leaves the
/// old one running), replay mixer state, and re-decode loaded tracks if the
/// device rate changed. Decks come back paused at their previous position.
#[tauri::command]
async fn set_output_device(
    engine: EngineState<'_>,
    device: Option<String>,
) -> Result<u32, String> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || switch_device(&engine, device))
        .await
        .map_err(err_str)?
}

fn switch_device(engine: &Arc<Engine>, device: Option<String>) -> Result<u32, String> {
    let old_positions: Vec<usize> = engine
        .shared
        .decks
        .iter()
        .map(|d| d.position.load(Ordering::Relaxed))
        .collect();
    let old_rate = engine.sample_rate();

    let new_conn = engine::spawn_stream(device, engine.shared.clone()).map_err(err_str)?;
    let new_rate = new_conn.sample_rate;

    {
        let mut conn = engine.conn.lock().unwrap();
        conn.shutdown();
        *conn = new_conn;
    }
    engine.replay_state();

    // Reload deck contents onto the new stream.
    for deck in 0..NUM_DECKS {
        let (path, track) = {
            let mirror = engine.mirror.lock().unwrap();
            match &mirror.loaded[deck] {
                Some(l) => (l.path.clone(), l.track.clone()),
                None => continue,
            }
        };
        let start_frame =
            (old_positions[deck] as f64 * new_rate as f64 / old_rate as f64) as usize;
        if track.sample_rate == new_rate {
            engine.send(Cmd::Load {
                deck,
                track,
                start_frame,
            });
        } else {
            // Rate changed: re-decode at the new rate.
            match decode::decode_file(&path, new_rate) {
                Ok(decoded) => {
                    let new_track = Arc::new(TrackData {
                        id: engine.next_id(),
                        sample_rate: new_rate,
                        samples: decoded.samples,
                    });
                    engine.mirror.lock().unwrap().loaded[deck] = Some(LoadedTrack {
                        path,
                        track: new_track.clone(),
                    });
                    engine.send(Cmd::Load {
                        deck,
                        track: new_track,
                        start_frame,
                    });
                }
                Err(e) => eprintln!("re-decode after device switch failed: {e}"),
            }
        }
    }
    engine.collect_trash();
    Ok(new_rate)
}

// ---------------------------------------------------------------------------
// State event loop
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeckSnapshot {
    position_secs: f64,
    playing: bool,
    peak: f32,
    track_id: u32,
    /// A quantized start is waiting on the other deck's playhead.
    armed: bool,
    /// Master-deck time the pending start fires at (secs; valid while armed).
    armed_master_secs: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EngineSnapshot {
    decks: Vec<DeckSnapshot>,
    master_peak: [f32; 2],
    sample_rate: u32,
}

fn spawn_state_emitter(app: tauri::AppHandle, engine: Arc<Engine>) {
    std::thread::spawn(move || loop {
        let rate = engine.shared.sample_rate.load(Ordering::Relaxed).max(1) as f64;
        let decks = engine
            .shared
            .decks
            .iter()
            .map(|d| DeckSnapshot {
                position_secs: d.position.load(Ordering::Relaxed) as f64 / rate,
                playing: d.playing.load(Ordering::Relaxed),
                peak: engine::load_f32(&d.peak),
                track_id: d.track_id.load(Ordering::Relaxed),
                armed: d.armed.load(Ordering::Relaxed),
                armed_master_secs: d.armed_master_frame.load(Ordering::Relaxed) as f64
                    / rate,
            })
            .collect();
        let snapshot = EngineSnapshot {
            decks,
            master_peak: [
                engine::load_f32(&engine.shared.master_peak_l),
                engine::load_f32(&engine.shared.master_peak_r),
            ],
            sample_rate: rate as u32,
        };
        let _ = app.emit("engine-state", &snapshot);
        engine.collect_trash();
        std::thread::sleep(std::time::Duration::from_millis(33));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let engine = Arc::new(Engine::start().expect("failed to start audio engine"));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(engine.clone())
        .manage(BatchFlag(std::sync::Mutex::new(Arc::new(AtomicBool::new(
            false,
        )))))
        .manage(RestoreState(std::sync::Mutex::new([None, None])))
        .manage(SessionHistory(std::sync::Mutex::new(Vec::new())))
        .setup(move |app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("no app data dir");
            let database =
                Arc::new(db::Db::open(&data_dir).expect("failed to open database"));
            app.manage(database);
            spawn_state_emitter(app.handle().clone(), engine.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_library,
            get_artwork,
            load_track,
            restore_decks,
            play,
            pause,
            seek,
            set_gain,
            set_eq,
            set_crossfader,
            set_master_gain,
            set_rate,
            set_key_lock,
            set_fx,
            set_delay_time,
            set_auto_gain,
            set_vocal,
            set_key_shift,
            start_recording,
            stop_recording,
            viz_sample_rate,
            start_visualizer,
            stop_visualizer,
            set_loop,
            clear_loop,
            get_analysis,
            start_library_analysis,
            cancel_library_analysis,
            recommend_tracks,
            set_api_key,
            has_api_key,
            get_session_history,
            clear_session_history,
            play_quantized,
            set_cue_label,
            set_cue_point,
            list_output_devices,
            set_output_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

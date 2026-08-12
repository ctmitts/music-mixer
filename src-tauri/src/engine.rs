//! Real-time mix engine.
//!
//! Control threads talk to the audio callback exclusively through a lock-free
//! command ring (`rtrb`); the callback reports position and meters through
//! atomics in [`Shared`]. Track buffers are `Arc`s handed over in commands;
//! replaced tracks are pushed onto a trash ring so their memory is never
//! freed on the audio thread.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use signalsmith_stretch::Stretch;

use crate::dsp::{BandSplitter4, Delay, FilterSweep, Limiter, Reverb, Smoothed, VocalFilter};

pub const NUM_DECKS: usize = 2;
pub const NUM_EQ_BANDS: usize = 4;
/// Crossovers for low / low-mid / high-mid / high. The 200–800 Hz band is
/// the one that matters most when blending sustained low-register material
/// against a bassline.
const EQ_F1_HZ: f32 = 200.0;
const EQ_F2_HZ: f32 = 800.0;
const EQ_F3_HZ: f32 = 3000.0;

/// A fully decoded track: interleaved stereo f32 at `sample_rate`.
pub struct TrackData {
    pub id: u64,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl TrackData {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
}

pub enum Cmd {
    Load {
        deck: usize,
        track: Arc<TrackData>,
        start_frame: usize,
    },
    Play {
        deck: usize,
    },
    /// Arm `deck` to start when `master`'s playhead reaches `master_frame`
    /// (track frames, which are engine frames since tracks are resampled on
    /// load). The callback starts the deck at the exact crossing; if the
    /// command arrives after the crossing, the deck starts immediately with
    /// its position advanced by the overshoot so alignment still holds.
    PlayQuantized {
        deck: usize,
        master: usize,
        master_frame: f64,
    },
    Pause {
        deck: usize,
    },
    Seek {
        deck: usize,
        frame: usize,
    },
    /// Playback rate (1.0 = normal). Varispeed: pitch follows tempo.
    SetRate {
        deck: usize,
        value: f32,
    },
    SetLoop {
        deck: usize,
        start_frame: usize,
        end_frame: usize,
    },
    ClearLoop {
        deck: usize,
    },
    /// Key lock: time-stretch instead of varispeed. The stretcher is built
    /// control-side so the audio thread never allocates.
    SetKeyLock {
        deck: usize,
        stretcher: Option<Box<Stretch>>,
    },
    SetFx {
        deck: usize,
        filter: f32,
        delay_mix: f32,
        reverb_mix: f32,
    },
    SetDelayTime {
        deck: usize,
        secs: f32,
    },
    SetAutoGain {
        deck: usize,
        value: f32,
    },
    SetVocal {
        deck: usize,
        amount: f32,
        isolate: bool,
    },
    /// Transpose in semitones, independent of tempo. Requires key lock.
    SetKeyShift {
        deck: usize,
        semitones: f32,
    },
    StartRecord(Box<rtrb::Producer<f32>>),
    StopRecord,
    StartViz(Box<rtrb::Producer<f32>>),
    StopViz,
    SetGain {
        deck: usize,
        value: f32,
    },
    SetEq {
        deck: usize,
        band: usize,
        value: f32,
    },
    SetCrossfader(f32),
    SetMaster(f32),
}

#[derive(Default)]
pub struct DeckShared {
    pub position: AtomicUsize,
    pub playing: AtomicBool,
    pub peak: AtomicU32,
    pub track_id: AtomicU32,
    /// True while a quantized start is pending on this deck.
    pub armed: AtomicBool,
    /// Master-deck frame the pending start fires at (valid while `armed`).
    pub armed_master_frame: AtomicUsize,
}

#[derive(Default)]
pub struct Shared {
    pub decks: [DeckShared; NUM_DECKS],
    pub master_peak_l: AtomicU32,
    pub master_peak_r: AtomicU32,
    pub sample_rate: AtomicU32,
}

#[inline]
fn store_f32(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

pub fn load_f32(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// Audio-thread state
// ---------------------------------------------------------------------------

struct DeckState {
    track: Option<Arc<TrackData>>,
    position: f64,
    playing: bool,
    /// Pending quantized start: (master deck, master frame to fire at).
    pending_start: Option<(usize, f64)>,
    gain: Smoothed,
    auto_gain: Smoothed,
    eq: [Smoothed; NUM_EQ_BANDS],
    rate: Smoothed,
    loop_region: Option<(f64, f64)>,
    split_l: BandSplitter4,
    split_r: BandSplitter4,
    vocal: VocalFilter,
    filter: FilterSweep,
    delay: Delay,
    reverb: Reverb,
    stretcher: Option<Box<Stretch>>,
    stretch_in: Vec<f32>,
    frac_in: f64,
    peak_accum: f32,
}

impl DeckState {
    fn new(sample_rate: f32) -> Self {
        Self {
            track: None,
            position: 0.0,
            playing: false,
            pending_start: None,
            gain: Smoothed::new(1.0, sample_rate),
            auto_gain: Smoothed::new(1.0, sample_rate),
            eq: [Smoothed::new(1.0, sample_rate); NUM_EQ_BANDS],
            rate: Smoothed::new(1.0, sample_rate),
            loop_region: None,
            split_l: BandSplitter4::new(sample_rate, EQ_F1_HZ, EQ_F2_HZ, EQ_F3_HZ),
            split_r: BandSplitter4::new(sample_rate, EQ_F1_HZ, EQ_F2_HZ, EQ_F3_HZ),
            vocal: VocalFilter::new(sample_rate),
            filter: FilterSweep::new(sample_rate),
            delay: Delay::new(sample_rate),
            reverb: Reverb::new(sample_rate),
            stretcher: None,
            stretch_in: vec![0.0; 65536],
            frac_in: 0.0,
            peak_accum: 0.0,
        }
    }

    /// Read one input frame at integer position `p` with loop/end handling
    /// applied by the caller.
    #[inline]
    fn raw_frame(track: &TrackData, p: usize) -> (f32, f32) {
        let frames = track.frames();
        if p >= frames {
            return (0.0, 0.0);
        }
        (track.samples[p * 2], track.samples[p * 2 + 1])
    }

    /// Fill `out` (interleaved stereo, `n` frames) with raw (pre-EQ/FX)
    /// track audio, advancing the playhead.
    fn fill_raw(&mut self, out: &mut [f32], n: usize, engine_rate: u32) {
        out[..n * 2].fill(0.0);
        let Some(track) = self.track.clone() else {
            self.rate_advance_only(n);
            return;
        };
        if !self.playing || track.sample_rate != engine_rate {
            self.rate_advance_only(n);
            return;
        }
        let total = track.frames() as f64;

        if self.stretcher.is_some() {
            // Key-lock: consume input at `rate`, output 1:1.
            let rate = {
                // advance smoothing across the block in one step batch
                let mut r = 0.0;
                for _ in 0..n {
                    r = self.rate.step();
                }
                r as f64
            };
            let want = n as f64 * rate + self.frac_in;
            let in_frames = (want.floor() as usize).min(self.stretch_in.len() / 2);
            self.frac_in = want - in_frames as f64;
            let mut p = self.position;
            for i in 0..in_frames {
                let (l, r) = Self::raw_frame(&track, p as usize);
                self.stretch_in[i * 2] = l;
                self.stretch_in[i * 2 + 1] = r;
                p += 1.0;
                if let Some((start, end)) = self.loop_region {
                    if p >= end && end > start {
                        p = start;
                    }
                }
                if p >= total {
                    p = total - 1.0;
                    self.playing = false;
                }
            }
            self.position = p;
            let stretcher = self.stretcher.as_mut().unwrap();
            stretcher.process(&self.stretch_in[..in_frames * 2], &mut out[..n * 2]);
        } else {
            // Varispeed: fractional playhead, Catmull-Rom interpolation.
            for i in 0..n {
                let rate = self.rate.step() as f64;
                if !self.playing {
                    break;
                }
                out[i * 2] = Self::sample_at(&track, self.position, 0);
                out[i * 2 + 1] = Self::sample_at(&track, self.position, 1);
                self.position += rate;
                if let Some((start, end)) = self.loop_region {
                    if self.position >= end && end > start {
                        self.position = start + (self.position - end);
                    }
                }
                if self.position >= total - 1.0 {
                    self.position = total - 1.0;
                    self.playing = false;
                }
            }
        }
    }

    /// Keep the rate smoother moving while silent so a queued change doesn't
    /// jump when playback resumes.
    fn rate_advance_only(&mut self, n: usize) {
        for _ in 0..n {
            self.rate.step();
        }
    }

    /// Full deck processing for one block: raw audio → 3-band EQ → filter
    /// sweep → delay → reverb → channel gain × auto-gain.
    fn render_block(&mut self, out: &mut [f32], n: usize, engine_rate: u32) {
        self.fill_raw(out, n, engine_rate);
        self.filter.update_block();
        let delay_frames = self.delay.block_time();
        for i in 0..n {
            let g = self.gain.step() * self.auto_gain.step();
            let eq_gains = [
                self.eq[0].step(),
                self.eq[1].step(),
                self.eq[2].step(),
                self.eq[3].step(),
            ];
            // Vocal processing runs first, on the original stereo image —
            // it depends on the L/R relationship, so it must happen before
            // anything that could disturb it.
            let (mut l, mut r) = self.vocal.process(out[i * 2], out[i * 2 + 1]);
            let (b0, b1, b2, b3) = self.split_l.split(l);
            l = b0 * eq_gains[0] + b1 * eq_gains[1] + b2 * eq_gains[2] + b3 * eq_gains[3];
            let (b0, b1, b2, b3) = self.split_r.split(r);
            r = b0 * eq_gains[0] + b1 * eq_gains[1] + b2 * eq_gains[2] + b3 * eq_gains[3];
            let (fl, fr) = self.filter.process(l, r);
            let (dl, dr) = self.delay.process(fl, fr, delay_frames);
            let (rl, rr) = self.reverb.process(dl, dr);
            let (ol, or_) = (rl * g, rr * g);
            out[i * 2] = ol;
            out[i * 2 + 1] = or_;
            let peak = ol.abs().max(or_.abs());
            if peak > self.peak_accum {
                self.peak_accum = peak;
            }
        }
    }

    /// Catmull-Rom interpolated read of one channel at fractional frame `pos`.
    #[inline]
    fn sample_at(track: &TrackData, pos: f64, chan: usize) -> f32 {
        let frames = track.frames();
        let i = pos.floor() as isize;
        let frac = (pos - pos.floor()) as f32;
        let get = |idx: isize| -> f32 {
            let clamped = idx.clamp(0, frames as isize - 1) as usize;
            track.samples[clamped * 2 + chan]
        };
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        let a = frac;
        // Catmull-Rom spline.
        p1 + 0.5
            * a
            * (p2 - p0 + a * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + a * (3.0 * (p1 - p2) + p3 - p0)))
    }

}

struct AudioState {
    decks: [DeckState; NUM_DECKS],
    crossfader: Smoothed,
    master_gain: Smoothed,
    limiter: Limiter,
    engine_rate: u32,
    channels: usize,
    cmd_rx: rtrb::Consumer<Cmd>,
    trash_tx: rtrb::Producer<Arc<TrackData>>,
    shared: Arc<Shared>,
    scratch: Vec<f32>,
    deck_bufs: [Vec<f32>; NUM_DECKS],
    rec_tx: Option<Box<rtrb::Producer<f32>>>,
    /// Post-limiter tap feeding the spectral visualizer. Same best-effort
    /// contract as `rec_tx`: never block the audio thread.
    viz_tx: Option<Box<rtrb::Producer<f32>>>,
    master_peak_l: f32,
    master_peak_r: f32,
}

impl AudioState {
    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.cmd_rx.pop() {
            match cmd {
                Cmd::Load {
                    deck,
                    track,
                    start_frame,
                } => {
                    let d = &mut self.decks[deck];
                    self.shared.decks[deck]
                        .track_id
                        .store(track.id as u32, Ordering::Relaxed);
                    let start = start_frame.min(track.frames().saturating_sub(1));
                    if let Some(old) = d.track.replace(track) {
                        // Never free multi-hundred-MB buffers here; hand them
                        // to the control side. If the trash ring is somehow
                        // full, leak-drop inline as a last resort.
                        let _ = self.trash_tx.push(old);
                    }
                    d.position = start as f64;
                    d.playing = false;
                    d.pending_start = None;
                    d.loop_region = None;
                }
                Cmd::Play { deck } => {
                    let d = &mut self.decks[deck];
                    // An explicit play overrides a pending quantized start.
                    d.pending_start = None;
                    if let Some(t) = &d.track {
                        if d.position >= t.frames() as f64 - 1.0 {
                            d.position = 0.0;
                        }
                        d.playing = true;
                    }
                }
                Cmd::PlayQuantized {
                    deck,
                    master,
                    master_frame,
                } => {
                    let ok = master < NUM_DECKS
                        && master != deck
                        && self.decks[deck].track.is_some();
                    self.decks[deck].pending_start =
                        ok.then_some((master, master_frame));
                }
                Cmd::Pause { deck } => {
                    let d = &mut self.decks[deck];
                    d.playing = false;
                    // Pausing an armed deck cancels the pending start.
                    d.pending_start = None;
                }
                Cmd::Seek { deck, frame } => {
                    let d = &mut self.decks[deck];
                    if let Some(t) = &d.track {
                        d.position = frame.min(t.frames().saturating_sub(1)) as f64;
                    }
                }
                Cmd::SetRate { deck, value } => {
                    self.decks[deck].rate.set_target(value.clamp(0.5, 2.0))
                }
                Cmd::SetLoop {
                    deck,
                    start_frame,
                    end_frame,
                } => {
                    if end_frame > start_frame {
                        self.decks[deck].loop_region =
                            Some((start_frame as f64, end_frame as f64));
                    }
                }
                Cmd::ClearLoop { deck } => self.decks[deck].loop_region = None,
                Cmd::SetGain { deck, value } => self.decks[deck].gain.set_target(value),
                Cmd::SetEq { deck, band, value } => {
                    if band < NUM_EQ_BANDS {
                        self.decks[deck].eq[band].set_target(value)
                    }
                }
                Cmd::SetKeyShift { deck, semitones } => {
                    // Only meaningful with the stretcher engaged; the control
                    // side turns key lock on before sending this.
                    if let Some(s) = &mut self.decks[deck].stretcher {
                        s.set_transpose_factor_semitones(semitones, None);
                    }
                }
                Cmd::SetCrossfader(v) => self.crossfader.set_target(v.clamp(0.0, 1.0)),
                Cmd::SetMaster(v) => self.master_gain.set_target(v),
                Cmd::SetKeyLock { deck, stretcher } => {
                    // Ship any replaced stretcher out via... it's small (a few
                    // hundred KB of FFT state); dropping inline is acceptable
                    // as this only happens on explicit user toggles.
                    self.decks[deck].stretcher = stretcher;
                    self.decks[deck].frac_in = 0.0;
                }
                Cmd::SetFx {
                    deck,
                    filter,
                    delay_mix,
                    reverb_mix,
                } => {
                    let d = &mut self.decks[deck];
                    d.filter.set_knob(filter);
                    d.delay.set_mix(delay_mix);
                    d.reverb.set_mix(reverb_mix);
                }
                Cmd::SetDelayTime { deck, secs } => {
                    self.decks[deck].delay.set_time_secs(secs)
                }
                Cmd::SetAutoGain { deck, value } => {
                    self.decks[deck].auto_gain.set_target(value.clamp(0.05, 8.0))
                }
                Cmd::SetVocal {
                    deck,
                    amount,
                    isolate,
                } => self.decks[deck].vocal.set(amount, isolate),
                Cmd::StartRecord(producer) => self.rec_tx = Some(producer),
                Cmd::StopRecord => {
                    // Dropping the producer signals the writer thread to
                    // finalize the file.
                    self.rec_tx = None;
                }
                Cmd::StartViz(producer) => self.viz_tx = Some(producer),
                Cmd::StopViz => self.viz_tx = None,
            }
        }
    }

    /// Fire any pending quantized starts whose master crossing lands in (or
    /// before) this block. Runs before deck rendering, so a start scheduled
    /// `k` frames into the block pre-rolls the deck's position by `k` frames'
    /// worth: the deck plays from slightly before its cue point and passes it
    /// exactly as the master crosses the target — worst case one buffer
    /// (~3–10 ms) of pre-roll, which is inaudible, in exchange for
    /// sample-accurate alignment from then on.
    ///
    /// Known limit: key-lock adds stretcher latency that this doesn't model,
    /// so two decks with different key-lock states land a few ms apart. The
    /// same is true of the manual Sync path.
    fn fire_pending_starts(&mut self, frames: usize) {
        for i in 0..NUM_DECKS {
            let Some((m, target)) = self.decks[i].pending_start else {
                continue;
            };
            let (m_pos, m_rate, m_running) = {
                let md = &self.decks[m];
                (
                    md.position,
                    (md.rate.current() as f64).max(0.01),
                    md.playing && md.track.is_some(),
                )
            };
            if !m_running {
                // Stay armed until the master actually runs.
                continue;
            }
            let d_rate = self.decks[i].rate.current() as f64;
            let d = &mut self.decks[i];
            if m_pos >= target {
                // Command arrived after the crossing: start now, advanced by
                // the overshoot so the grids still line up.
                d.position += (m_pos - target) * d_rate / m_rate;
                d.playing = true;
                d.pending_start = None;
            } else {
                let k = (target - m_pos) / m_rate; // output frames to crossing
                if k < frames as f64 {
                    d.position = (d.position - k * d_rate).max(0.0);
                    d.playing = true;
                    d.pending_start = None;
                }
            }
        }
    }

    fn render<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T]) {
        self.drain_commands();
        let channels = self.channels;
        let frames = out.len() / channels;
        self.fire_pending_starts(frames);
        if self.scratch.len() < frames * 2 {
            // Only happens if the device reports a larger buffer than the
            // initial allocation; a one-time growth, not steady-state churn.
            self.scratch.resize(frames * 2, 0.0);
        }
        for buf in &mut self.deck_bufs {
            if buf.len() < frames * 2 {
                buf.resize(frames * 2, 0.0);
            }
        }
        for (i, deck) in self.decks.iter_mut().enumerate() {
            deck.render_block(&mut self.deck_bufs[i], frames, self.engine_rate);
        }
        for f in 0..frames {
            let xf = self.crossfader.step();
            // Constant-power crossfade.
            let xf_gain = [
                (xf * std::f32::consts::FRAC_PI_2).cos(),
                (xf * std::f32::consts::FRAC_PI_2).sin(),
            ];
            let mut mix_l = 0.0f32;
            let mut mix_r = 0.0f32;
            for i in 0..NUM_DECKS {
                mix_l += self.deck_bufs[i][f * 2] * xf_gain[i];
                mix_r += self.deck_bufs[i][f * 2 + 1] * xf_gain[i];
            }
            let mg = self.master_gain.step();
            let (l, r) = self.limiter.process(mix_l * mg, mix_r * mg);
            self.scratch[f * 2] = l;
            self.scratch[f * 2 + 1] = r;
            if let Some(rec) = &mut self.rec_tx {
                // Best-effort: drop samples if the writer stalls rather than
                // ever blocking the audio thread.
                let _ = rec.push(l);
                let _ = rec.push(r);
            }
            if let Some(viz) = &mut self.viz_tx {
                // Mono: the spectral analysis is mono anyway, so downmixing
                // here halves the IPC traffic. Dropping a sample makes the
                // visualizer skip a frame, which is invisible; blocking the
                // audio thread to avoid that would be audible.
                let _ = viz.push((l + r) * 0.5);
            }
            let pl = l.abs();
            let pr = r.abs();
            if pl > self.master_peak_l {
                self.master_peak_l = pl;
            }
            if pr > self.master_peak_r {
                self.master_peak_r = pr;
            }
        }
        for f in 0..frames {
            out[f * channels] = T::from_sample(self.scratch[f * 2]);
            if channels > 1 {
                out[f * channels + 1] = T::from_sample(self.scratch[f * 2 + 1]);
            }
            for c in 2..channels {
                out[f * channels + c] = T::from_sample(0.0f32);
            }
        }
        self.publish();
    }

    fn publish(&mut self) {
        for (i, deck) in self.decks.iter_mut().enumerate() {
            let sd = &self.shared.decks[i];
            sd.position.store(deck.position as usize, Ordering::Relaxed);
            sd.playing.store(deck.playing, Ordering::Relaxed);
            sd.armed
                .store(deck.pending_start.is_some(), Ordering::Relaxed);
            sd.armed_master_frame.store(
                deck.pending_start.map(|(_, f)| f as usize).unwrap_or(0),
                Ordering::Relaxed,
            );
            store_f32(&sd.peak, deck.peak_accum);
            deck.peak_accum = 0.0;
        }
        store_f32(&self.shared.master_peak_l, self.master_peak_l);
        store_f32(&self.shared.master_peak_r, self.master_peak_r);
        self.master_peak_l = 0.0;
        self.master_peak_r = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Stream lifecycle
// ---------------------------------------------------------------------------

/// Control-side handle to a live output stream. Dropping `kill_tx`'s channel
/// (via `shutdown`) ends the stream thread.
pub struct StreamConn {
    pub cmd_tx: rtrb::Producer<Cmd>,
    pub trash_rx: rtrb::Consumer<Arc<TrackData>>,
    kill_tx: mpsc::Sender<()>,
    pub sample_rate: u32,
    pub device_name: Option<String>,
}

impl StreamConn {
    pub fn shutdown(&self) {
        let _ = self.kill_tx.send(());
    }
}

fn find_device(host: &cpal::Host, name: &Option<String>) -> Result<cpal::Device> {
    match name {
        Some(wanted) => host
            .output_devices()?
            .find(|d| d.name().map(|n| &n == wanted).unwrap_or(false))
            .ok_or_else(|| anyhow!("output device '{wanted}' not found")),
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default output device")),
    }
}

fn build_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: AudioState,
) -> Result<cpal::Stream> {
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| state.render(data),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?)
}

/// Spawn a thread that owns the cpal stream (cpal streams are !Send, so the
/// stream must live and die on one thread). Returns once the stream is live.
pub fn spawn_stream(device_name: Option<String>, shared: Arc<Shared>) -> Result<StreamConn> {
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Cmd>::new(1024);
    let (trash_tx, trash_rx) = rtrb::RingBuffer::<Arc<TrackData>>::new(64);
    let (kill_tx, kill_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<u32>>();

    let name = device_name.clone();
    thread::Builder::new()
        .name("mix-table-stream".into())
        .spawn(move || {
            let build = move || -> Result<(cpal::Stream, u32)> {
                let host = cpal::default_host();
                let device = find_device(&host, &name)?;
                let supported = device.default_output_config()?;
                let sample_rate = supported.sample_rate().0;
                let channels = supported.channels() as usize;
                let config: cpal::StreamConfig = supported.config();
                shared.sample_rate.store(sample_rate, Ordering::Relaxed);
                let sr = sample_rate as f32;
                let state = AudioState {
                    decks: [DeckState::new(sr), DeckState::new(sr)],
                    crossfader: Smoothed::new(0.5, sr),
                    master_gain: Smoothed::new(1.0, sr),
                    limiter: Limiter::new(sr),
                    engine_rate: sample_rate,
                    channels,
                    cmd_rx,
                    trash_tx,
                    shared: shared.clone(),
                    scratch: vec![0.0; 16384],
                    deck_bufs: [vec![0.0; 16384], vec![0.0; 16384]],
                    rec_tx: None,
                    viz_tx: None,
                    master_peak_l: 0.0,
                    master_peak_r: 0.0,
                };
                let stream = match supported.sample_format() {
                    cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, state)?,
                    cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, state)?,
                    cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, state)?,
                    cpal::SampleFormat::I32 => build_stream::<i32>(&device, &config, state)?,
                    other => bail!("unsupported sample format: {other}"),
                };
                stream.play()?;
                Ok((stream, sample_rate))
            };
            match build() {
                Ok((stream, sample_rate)) => {
                    let _ = ready_tx.send(Ok(sample_rate));
                    // Park until told to die; keeps `stream` alive.
                    let _ = kill_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })?;

    let sample_rate = ready_rx
        .recv()
        .map_err(|_| anyhow!("stream thread died during startup"))??;

    Ok(StreamConn {
        cmd_tx,
        trash_rx,
        kill_tx,
        sample_rate,
        device_name,
    })
}

// ---------------------------------------------------------------------------
// Control-side engine handle
// ---------------------------------------------------------------------------

pub struct LoadedTrack {
    pub path: String,
    pub track: Arc<TrackData>,
}

/// Mirror of all mixer parameters so state can be replayed onto a fresh
/// stream after an output-device switch.
pub struct Mirror {
    pub gains: [f32; NUM_DECKS],
    pub eqs: [[f32; NUM_EQ_BANDS]; NUM_DECKS],
    pub rates: [f32; NUM_DECKS],
    pub key_shift: [f32; NUM_DECKS],
    pub key_lock: [bool; NUM_DECKS],
    /// filter, delay mix, reverb mix per deck.
    pub fx: [[f32; 3]; NUM_DECKS],
    pub delay_time: [f32; NUM_DECKS],
    pub auto_gains: [f32; NUM_DECKS],
    /// Vocal processing: (amount, isolate) per deck.
    pub vocal: [(f32, bool); NUM_DECKS],
    /// Active loop (start, end) in seconds, mirrored for UI restore.
    pub loops: [Option<(f64, f64)>; NUM_DECKS],
    pub crossfader: f32,
    pub master: f32,
    pub loaded: [Option<LoadedTrack>; NUM_DECKS],
}

impl Default for Mirror {
    fn default() -> Self {
        Self {
            gains: [1.0; NUM_DECKS],
            eqs: [[1.0; NUM_EQ_BANDS]; NUM_DECKS],
            rates: [1.0; NUM_DECKS],
            key_shift: [0.0; NUM_DECKS],
            key_lock: [false; NUM_DECKS],
            fx: [[0.0; 3]; NUM_DECKS],
            delay_time: [0.35; NUM_DECKS],
            auto_gains: [1.0; NUM_DECKS],
            vocal: [(0.0, false); NUM_DECKS],
            loops: [None; NUM_DECKS],
            crossfader: 0.5,
            master: 1.0,
            loaded: [None, None],
        }
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    pub conn: Mutex<StreamConn>,
    pub mirror: Mutex<Mirror>,
    next_track_id: AtomicU32,
}

impl Engine {
    pub fn start() -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let conn = spawn_stream(None, shared.clone())?;
        Ok(Self {
            shared,
            conn: Mutex::new(conn),
            mirror: Mutex::new(Mirror::default()),
            next_track_id: AtomicU32::new(1),
        })
    }

    pub fn next_id(&self) -> u64 {
        self.next_track_id.fetch_add(1, Ordering::Relaxed) as u64
    }

    pub fn sample_rate(&self) -> u32 {
        self.conn.lock().unwrap().sample_rate
    }

    /// Push a command onto the audio-thread ring, retrying briefly if full.
    pub fn send(&self, cmd: Cmd) {
        let mut conn = self.conn.lock().unwrap();
        let mut cmd = cmd;
        for _ in 0..200 {
            match conn.cmd_tx.push(cmd) {
                Ok(()) => return,
                Err(rtrb::PushError::Full(returned)) => {
                    cmd = returned;
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        }
        eprintln!("engine command ring full; command dropped");
    }

    /// Drop track buffers the audio thread has discarded.
    pub fn collect_trash(&self) {
        let mut conn = self.conn.lock().unwrap();
        while conn.trash_rx.pop().is_ok() {}
    }

    /// Replay mirrored mixer state onto a (new) stream.
    pub fn replay_state(&self) {
        let mirror = self.mirror.lock().unwrap();
        for deck in 0..NUM_DECKS {
            self.send(Cmd::SetGain {
                deck,
                value: mirror.gains[deck],
            });
            for band in 0..NUM_EQ_BANDS {
                self.send(Cmd::SetEq {
                    deck,
                    band,
                    value: mirror.eqs[deck][band],
                });
            }
            self.send(Cmd::SetRate {
                deck,
                value: mirror.rates[deck],
            });
            self.send(Cmd::SetFx {
                deck,
                filter: mirror.fx[deck][0],
                delay_mix: mirror.fx[deck][1],
                reverb_mix: mirror.fx[deck][2],
            });
            self.send(Cmd::SetDelayTime {
                deck,
                secs: mirror.delay_time[deck],
            });
            self.send(Cmd::SetAutoGain {
                deck,
                value: mirror.auto_gains[deck],
            });
            self.send(Cmd::SetVocal {
                deck,
                amount: mirror.vocal[deck].0,
                isolate: mirror.vocal[deck].1,
            });
            self.send(Cmd::SetKeyLock {
                deck,
                stretcher: if mirror.key_lock[deck] {
                    Some(Box::new(Stretch::preset_default(2, self.sample_rate())))
                } else {
                    None
                },
            });
            // Transpose must follow the stretcher it applies to.
            if mirror.key_lock[deck] && mirror.key_shift[deck] != 0.0 {
                self.send(Cmd::SetKeyShift {
                    deck,
                    semitones: mirror.key_shift[deck],
                });
            }
        }
        self.send(Cmd::SetCrossfader(mirror.crossfader));
        self.send(Cmd::SetMaster(mirror.master));
    }
}

/// Name of the current system default output.
///
/// AirPlay destinations are not enumerable CoreAudio devices — macOS routes
/// them at the system level, so an Apple TV never appears in
/// [`list_output_devices`]. The only way to reach one is to follow whatever
/// the system default currently is, which means noticing when the user
/// switches it in Control Center. Cheap enough to poll.
pub fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

pub fn list_output_devices() -> Result<(Vec<String>, Option<String>)> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let names = host
        .output_devices()?
        .filter_map(|d| d.name().ok())
        .collect();
    Ok((names, default_name))
}

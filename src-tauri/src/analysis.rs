//! Offline track analysis: BPM + beat grid, musical key (Krumhansl profile
//! correlation → Camelot code), and fluid-tempo detection for classical /
//! rubato material. Runs off the audio thread on decoded samples.

use rustfft::{num_complex::Complex32, FftPlanner};
use serde::{Deserialize, Serialize};

const FFT_SIZE: usize = 1024;
const HOP: usize = 512;
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;

/// Bump when the feature set changes so cached rows are recomputed.
pub const ANALYSIS_VERSION: i64 = 2;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub bpm: f64,
    /// Seconds into the track of the first beat.
    pub beat_offset: f64,
    /// Relative std-dev of windowed tempo estimates (0 = rock solid).
    pub tempo_drift: f64,
    /// True when the beat grid is unreliable (classical, rubato jazz).
    pub fluid: bool,
    pub key_name: String,
    pub camelot: String,
    /// EBU R128 integrated loudness (LUFS).
    #[serde(default = "default_lufs")]
    pub lufs: f64,
    /// Perceptual drive, 0 (ambient) → 1 (peak-time). Onset density,
    /// loudness, and rhythmic definition combined.
    #[serde(default)]
    pub energy: f64,
    /// Spectral centroid mapped to 0 (dark) → 1 (bright).
    #[serde(default)]
    pub brightness: f64,
    /// How strongly the track states its pulse, 0 → 1.
    #[serde(default)]
    pub beat_strength: f64,
}

fn default_lufs() -> f64 {
    -14.0
}

fn measure_lufs(samples: &[f32], sample_rate: u32) -> Option<f64> {
    let mut meter = ebur128::EbuR128::new(2, sample_rate, ebur128::Mode::I).ok()?;
    meter.add_frames_f32(samples).ok()?;
    meter.loudness_global().ok().filter(|l| l.is_finite())
}

/// Mix interleaved stereo down to mono.
fn mono(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks_exact(2)
        .map(|f| (f[0] + f[1]) * 0.5)
        .collect()
}

/// Magnitude spectrogram frames (Hann window) at the given size/hop.
fn spectrogram(mono: &[f32], size: usize, hop: usize) -> Vec<Vec<f32>> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let window: Vec<f32> = (0..size)
        .map(|i| {
            let x = std::f32::consts::PI * 2.0 * i as f32 / size as f32;
            0.5 * (1.0 - x.cos())
        })
        .collect();
    let mut frames = Vec::new();
    let mut buf = vec![Complex32::new(0.0, 0.0); size];
    let mut pos = 0;
    while pos + size <= mono.len() {
        for i in 0..size {
            buf[i] = Complex32::new(mono[pos + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        frames.push(buf[..size / 2].iter().map(|c| c.norm()).collect());
        pos += hop;
    }
    frames
}

/// Spectral-flux onset envelope with local-mean removal.
fn onset_envelope(frames: &[Vec<f32>]) -> Vec<f32> {
    if frames.len() < 2 {
        return vec![];
    }
    let mut env: Vec<f32> = Vec::with_capacity(frames.len() - 1);
    for w in frames.windows(2) {
        let flux: f32 = w[1]
            .iter()
            .zip(w[0].iter())
            .map(|(cur, prev)| (cur - prev).max(0.0))
            .sum();
        env.push(flux);
    }
    // Remove slow-moving mean so sustained texture (strings) doesn't read as
    // onsets.
    let half = 8usize;
    let mut out = vec![0.0f32; env.len()];
    for i in 0..env.len() {
        let a = i.saturating_sub(half);
        let b = (i + half + 1).min(env.len());
        let mean: f32 = env[a..b].iter().sum::<f32>() / (b - a) as f32;
        out[i] = (env[i] - mean).max(0.0);
    }
    out
}

/// Tempo from the onset envelope by autocorrelation, with a mild preference
/// for the 90–150 BPM octave.
fn estimate_bpm(env: &[f32], fps: f64) -> Option<f64> {
    if env.len() < 200 {
        return None;
    }
    let min_lag = (fps * 60.0 / MAX_BPM) as usize;
    let max_lag = ((fps * 60.0 / MIN_BPM) as usize).min(env.len() / 2);
    if min_lag >= max_lag {
        return None;
    }
    let energy: f32 = env.iter().map(|x| x * x).sum();
    if energy <= f32::EPSILON {
        return None;
    }
    let mut best = (0usize, f32::MIN);
    for lag in min_lag..max_lag {
        let mut acc = 0.0f32;
        for i in 0..env.len() - lag {
            acc += env[i] * env[i + lag];
        }
        let bpm = fps * 60.0 / lag as f64;
        // Log-gaussian weighting centered near 120 BPM.
        let w = (-((bpm / 120.0).ln().powi(2)) / (2.0 * 0.55f64.powi(2))).exp() as f32;
        let score = acc / energy * (0.4 + 0.6 * w);
        if score > best.1 {
            best = (lag, score);
        }
    }
    if best.0 == 0 {
        return None;
    }
    // Parabolic refinement around the peak lag for sub-frame precision.
    let lag = best.0;
    let ac = |l: usize| -> f64 {
        let mut acc = 0.0f64;
        for i in 0..env.len().saturating_sub(l) {
            acc += env[i] as f64 * env[i + l] as f64;
        }
        acc
    };
    let (ym, y0, yp) = (ac(lag - 1), ac(lag), ac(lag + 1));
    let denom = ym - 2.0 * y0 + yp;
    let shift = if denom.abs() > f64::EPSILON {
        (0.5 * (ym - yp) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(fps * 60.0 / (lag as f64 + shift))
}

/// Local tempo deviation: in each window, find the best lag within ±10% of
/// the global lag and measure its relative deviation. Metronomic material
/// stays near zero; rubato swings to the bounds.
fn local_drift(env: &[f32], fps: f64, bpm: f64) -> f64 {
    let global_lag = fps * 60.0 / bpm;
    let win = (12.0 * fps) as usize;
    if win == 0 || env.len() < win * 2 {
        return 0.0;
    }
    let lo = ((global_lag * 0.90) as usize).max(2);
    let hi = (global_lag * 1.10) as usize;
    let mut deviations: Vec<f64> = Vec::new();
    let mut start = 0;
    while start + win <= env.len() {
        let w = &env[start..start + win];
        let energy: f64 = w.iter().map(|x| (*x as f64) * (*x as f64)).sum();
        if energy > 1e-9 {
            let mut best = (global_lag, f64::MIN);
            for lag in lo..=hi.min(win / 2) {
                let mut acc = 0.0f64;
                for i in 0..win - lag {
                    acc += w[i] as f64 * w[i + lag] as f64;
                }
                if acc > best.1 {
                    best = (lag as f64, acc);
                }
            }
            deviations.push((best.0 - global_lag).abs() / global_lag);
        }
        start += win / 2;
    }
    if deviations.len() < 3 {
        return 0.0;
    }
    deviations.iter().sum::<f64>() / deviations.len() as f64
}

/// Mean spectral centroid in Hz, mapped to 0..1 over a log scale from 200 Hz
/// (dark) to 4 kHz (bright).
fn brightness(frames: &[Vec<f32>], sample_rate: f64) -> f64 {
    let bin_hz = sample_rate / FFT_SIZE as f64;
    // Restrict to the band where musical energy actually lives. Including the
    // full spectrum lets the sheer number of near-silent HF bins drag every
    // track's centroid upward, which flattens the measure to uselessness.
    let max_bin = ((8000.0 / bin_hz) as usize).min(FFT_SIZE / 2);
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in frames {
        let mut weighted = 0.0f64;
        let mut total = 0.0f64;
        for (bin, mag) in frame.iter().take(max_bin).enumerate().skip(1) {
            let m = *mag as f64;
            weighted += m * bin as f64 * bin_hz;
            total += m;
        }
        if total > 1e-6 {
            sum += weighted / total;
            count += 1;
        }
    }
    if count == 0 {
        return 0.5;
    }
    let centroid = sum / count as f64;
    // 250 Hz (dark/orchestral) → 3 kHz (bright/percussive) covers the range
    // real recordings occupy.
    let norm = (centroid.max(1.0).ln() - 250f64.ln()) / (3000f64.ln() - 250f64.ln());
    norm.clamp(0.0, 1.0)
}

/// Attack density: how much of the spectrum is *newly appearing* each frame,
/// as a fraction of the spectrum that's present. Percussive material restates
/// its spectrum on every hit; sustained strings or pads carry the same energy
/// forward, so their flux is small relative to their magnitude.
///
/// Computed from the raw spectrogram rather than the onset envelope — that
/// envelope is mean-removed and half-wave rectified, so it's sparse-positive
/// and any variance measure on it reads high for every genre.
fn attack_density(frames: &[Vec<f32>]) -> f64 {
    if frames.len() < 2 {
        return 0.0;
    }
    let mut flux_total = 0.0f64;
    let mut mag_total = 0.0f64;
    for w in frames.windows(2) {
        for (cur, prev) in w[1].iter().zip(w[0].iter()) {
            flux_total += (cur - prev).max(0.0) as f64;
            mag_total += *cur as f64;
        }
    }
    if mag_total <= 1e-9 {
        return 0.0;
    }
    // Ratio lands around 0.05 for sustained orchestral, 0.25+ for dense drums.
    ((flux_total / mag_total - 0.04) / 0.18).clamp(0.0, 1.0)
}

/// Normalized autocorrelation peak at the detected period: how strongly the
/// track states its pulse.
fn beat_strength(env: &[f32], fps: f64, bpm: f64) -> f64 {
    if bpm <= 0.0 || env.len() < 100 {
        return 0.0;
    }
    let lag = (fps * 60.0 / bpm).round() as usize;
    if lag == 0 || lag >= env.len() {
        return 0.0;
    }
    let energy: f64 = env.iter().map(|x| (*x as f64) * (*x as f64)).sum();
    if energy <= 1e-9 {
        return 0.0;
    }
    let mut acc = 0.0f64;
    for i in 0..env.len() - lag {
        acc += env[i] as f64 * env[i + lag] as f64;
    }
    (acc / energy * 2.5).clamp(0.0, 1.0)
}

/// Beat phase: offset (in envelope frames) maximizing comb energy at `period`.
fn beat_phase(env: &[f32], period: f64) -> f64 {
    let p = period.max(1.0);
    let steps = p.floor() as usize;
    let mut best = (0usize, f32::MIN);
    for off in 0..steps {
        let mut acc = 0.0f32;
        let mut k = off as f64;
        while (k as usize) < env.len() {
            acc += env[k as usize];
            k += p;
        }
        if acc > best.1 {
            best = (off, acc);
        }
    }
    best.0 as f64
}

const KRUMHANSL_MAJOR: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const KRUMHANSL_MINOR: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
const NOTE_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];
// Camelot wheel positions indexed by pitch class.
const CAMELOT_MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
const CAMELOT_MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];

fn correlate(profile: &[f64; 12], chroma: &[f64; 12]) -> f64 {
    let mp: f64 = profile.iter().sum::<f64>() / 12.0;
    let mc: f64 = chroma.iter().sum::<f64>() / 12.0;
    let mut num = 0.0;
    let mut dp = 0.0;
    let mut dc = 0.0;
    for i in 0..12 {
        let a = profile[i] - mp;
        let b = chroma[i] - mc;
        num += a * b;
        dp += a * a;
        dc += b * b;
    }
    if dp <= 0.0 || dc <= 0.0 {
        0.0
    } else {
        num / (dp * dc).sqrt()
    }
}

const KEY_FFT: usize = 8192;

fn estimate_key(mono: &[f32], sample_rate: f64) -> (String, String) {
    // High-resolution spectrogram: 5.4 Hz bins at 44.1k, enough to separate
    // semitones from ~110 Hz upward.
    let frames = spectrogram(mono, KEY_FFT, KEY_FFT / 2);
    let mut chroma = [0.0f64; 12];
    let bin_hz = sample_rate / KEY_FFT as f64;
    for frame in &frames {
        for (bin, mag) in frame.iter().enumerate() {
            let freq = bin as f64 * bin_hz;
            if !(110.0..=1760.0).contains(&freq) {
                continue;
            }
            let semis = 12.0 * (freq / 440.0).log2() + 69.0; // MIDI note (60 = C4)
            // Reject energy that falls between semitones (broadband noise).
            let nearest = semis.round();
            if (semis - nearest).abs() > 0.35 {
                continue;
            }
            let pc = ((nearest as i64) % 12 + 12) % 12; // pc 0 = C
            chroma[pc as usize] += (*mag as f64).sqrt();
        }
    }
    let mut best = (0usize, true, f64::MIN);
    for root in 0..12 {
        let mut rotated = [0.0f64; 12];
        for i in 0..12 {
            rotated[i] = chroma[(root + i) % 12];
        }
        let maj = correlate(&KRUMHANSL_MAJOR, &rotated);
        let min = correlate(&KRUMHANSL_MINOR, &rotated);
        if maj > best.2 {
            best = (root, true, maj);
        }
        if min > best.2 {
            best = (root, false, min);
        }
    }
    let (root, major, _) = best;
    let name = if major {
        format!("{} major", NOTE_NAMES[root])
    } else {
        format!("{} minor", NOTE_NAMES[root])
    };
    let camelot = if major {
        format!("{}B", CAMELOT_MAJOR[root])
    } else {
        format!("{}A", CAMELOT_MINOR[root])
    };
    (name, camelot)
}

/// Analyze interleaved stereo samples at `sample_rate`.
pub fn analyze(samples: &[f32], sample_rate: u32) -> Analysis {
    let sr = sample_rate as f64;
    let mono = mono(samples);
    let frames = spectrogram(&mono, FFT_SIZE, HOP);
    let env = onset_envelope(&frames);
    let fps = sr / HOP as f64;

    let bpm = estimate_bpm(&env, fps).unwrap_or(0.0);
    let drift = if bpm > 0.0 {
        local_drift(&env, fps, bpm)
    } else {
        1.0
    };
    let fluid = bpm <= 0.0 || drift > 0.025;

    let beat_offset = if bpm > 0.0 {
        beat_phase(&env, fps * 60.0 / bpm) / fps
    } else {
        0.0
    };

    let (key_name, camelot) = estimate_key(&mono, sr);
    let lufs = measure_lufs(samples, sample_rate).unwrap_or(-14.0);

    let bright = brightness(&frames, sr);
    let density = attack_density(&frames);
    let pulse = beat_strength(&env, fps, bpm);
    // Mastered loudness is the single most reliable energy signal in practice
    // — the range from a classical recording (~-22 LUFS) to a club master
    // (~-7) tracks perceived drive closely, and it's measured to a standard
    // rather than estimated. Beat strength is the next most reliable.
    //
    // Attack density is weighted lowest deliberately: dense orchestral music
    // with choir and vibrato produces genuinely high spectral flux, so on its
    // own it would call the Mozart Requiem as percussive as a house track.
    let loud = ((lufs + 24.0) / 18.0).clamp(0.0, 1.0);
    let energy = (0.25 * density + 0.45 * loud + 0.30 * pulse).clamp(0.0, 1.0);

    Analysis {
        bpm: (bpm * 10.0).round() / 10.0,
        beat_offset,
        tempo_drift: drift,
        fluid,
        key_name,
        camelot,
        lufs,
        energy,
        brightness: bright,
        beat_strength: pulse,
    }
}

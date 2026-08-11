//! DSP primitives for the mix engine. Everything here runs on the audio
//! thread: no allocation, no locking.

/// RBJ biquad (Direct Form I).
#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Butterworth low-pass (Q = 1/sqrt(2)); two in series form an LR4 section.
    pub fn lowpass(sample_rate: f32, fc: f32) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * fc as f64 / sample_rate as f64;
        let (sin, cos) = (w0.sin(), w0.cos());
        // Q = 1/sqrt(2) => alpha = sin / (2Q) = sin / sqrt(2)
        let alpha = sin * std::f64::consts::FRAC_1_SQRT_2;
        let b1 = 1.0 - cos;
        let b0 = b1 / 2.0;
        Self::from_coeffs(b0, b1, b0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Butterworth high-pass (Q = 1/sqrt(2)).
    pub fn highpass(sample_rate: f32, fc: f32) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * fc as f64 / sample_rate as f64;
        let (sin, cos) = (w0.sin(), w0.cos());
        let alpha = sin * std::f64::consts::FRAC_1_SQRT_2;
        let b1 = -(1.0 + cos);
        let b0 = -b1 / 2.0;
        Self::from_coeffs(b0, b1, b0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Carry filter memory across a coefficient change (avoids clicks when a
    /// sweep refreshes coefficients each block).
    pub fn copy_state(&mut self, from: &Biquad) {
        self.x1 = from.x1;
        self.x2 = from.x2;
        self.y1 = from.y1;
        self.y2 = from.y2;
    }
}

/// Three-way band splitter for one channel: LR4 crossovers at `f_lo` and
/// `f_hi`. Bands sum back approximately flat, and a band gain of exactly 0.0
/// gives a full kill.
#[derive(Clone, Copy)]
pub struct BandSplitter {
    lp1a: Biquad,
    lp1b: Biquad,
    hp1a: Biquad,
    hp1b: Biquad,
    lp2a: Biquad,
    lp2b: Biquad,
    hp2a: Biquad,
    hp2b: Biquad,
}

impl BandSplitter {
    pub fn new(sample_rate: f32, f_lo: f32, f_hi: f32) -> Self {
        Self {
            lp1a: Biquad::lowpass(sample_rate, f_lo),
            lp1b: Biquad::lowpass(sample_rate, f_lo),
            hp1a: Biquad::highpass(sample_rate, f_lo),
            hp1b: Biquad::highpass(sample_rate, f_lo),
            lp2a: Biquad::lowpass(sample_rate, f_hi),
            lp2b: Biquad::lowpass(sample_rate, f_hi),
            hp2a: Biquad::highpass(sample_rate, f_hi),
            hp2b: Biquad::highpass(sample_rate, f_hi),
        }
    }

    #[inline]
    pub fn split(&mut self, x: f32) -> (f32, f32, f32) {
        let lo = self.lp1b.process(self.lp1a.process(x));
        let rest = self.hp1b.process(self.hp1a.process(x));
        let mid = self.lp2b.process(self.lp2a.process(rest));
        let hi = self.hp2b.process(self.hp2a.process(rest));
        (lo, mid, hi)
    }
}

/// Four-way splitter for one channel: low / low-mid / high-mid / high.
///
/// The extra split versus [`BandSplitter`] exists for one job — low-mid
/// (~200–800 Hz) is where two tracks fight when a sustained low register
/// overlaps a bassline, and cutting it on one side opens room without
/// thinning either. Built from cascaded LR4 crossovers so the bands still sum
/// approximately flat and a band gain of 0.0 is a true kill.
/// A single Linkwitz-Riley 4th-order two-way crossover: one input, a low and
/// a high output that sum approximately flat.
#[derive(Clone, Copy)]
pub struct Crossover2 {
    lp_a: Biquad,
    lp_b: Biquad,
    hp_a: Biquad,
    hp_b: Biquad,
}

impl Crossover2 {
    pub fn new(sample_rate: f32, fc: f32) -> Self {
        Self {
            lp_a: Biquad::lowpass(sample_rate, fc),
            lp_b: Biquad::lowpass(sample_rate, fc),
            hp_a: Biquad::highpass(sample_rate, fc),
            hp_b: Biquad::highpass(sample_rate, fc),
        }
    }

    #[inline]
    pub fn split(&mut self, x: f32) -> (f32, f32) {
        let low = self.lp_b.process(self.lp_a.process(x));
        let high = self.hp_b.process(self.hp_a.process(x));
        (low, high)
    }
}

#[derive(Clone, Copy)]
pub struct BandSplitter4 {
    x1: Crossover2,
    x2: Crossover2,
    x3: Crossover2,
}

impl BandSplitter4 {
    pub fn new(sample_rate: f32, f1: f32, f2: f32, f3: f32) -> Self {
        Self {
            x1: Crossover2::new(sample_rate, f1),
            x2: Crossover2::new(sample_rate, f2),
            x3: Crossover2::new(sample_rate, f3),
        }
    }

    /// Each crossover splits only what the previous one passed upward, so no
    /// band is filtered twice at the same corner and the four still sum flat.
    #[inline]
    pub fn split(&mut self, x: f32) -> (f32, f32, f32, f32) {
        let (low, rest) = self.x1.split(x);
        let (low_mid, rest) = self.x2.split(rest);
        let (high_mid, high) = self.x3.split(rest);
        (low, low_mid, high_mid, high)
    }
}

/// One-pole parameter smoother (~10 ms) to avoid zipper noise on fader and
/// EQ moves.
#[derive(Clone, Copy)]
pub struct Smoothed {
    current: f32,
    target: f32,
    k: f32,
}

impl Smoothed {
    pub fn new(value: f32, sample_rate: f32) -> Self {
        let tau = 0.010_f32;
        Self {
            current: value,
            target: value,
            k: 1.0 - (-1.0 / (tau * sample_rate)).exp(),
        }
    }

    pub fn set_target(&mut self, v: f32) {
        self.target = v;
    }

    /// Current smoothed value without advancing. Used by control logic that
    /// needs to reason about the rate a deck is actually running at.
    pub fn current(&self) -> f32 {
        self.current
    }

    #[inline]
    pub fn step(&mut self) -> f32 {
        self.current += (self.target - self.current) * self.k;
        self.current
    }
}

/// One-knob DJ filter: knob in [-1, 1]; negative sweeps a low-pass down,
/// positive sweeps a high-pass up, near zero is bypass. 12 dB/oct.
pub struct FilterSweep {
    knob: Smoothed,
    biquad_l: Biquad,
    biquad_r: Biquad,
    sample_rate: f32,
    active: bool,
}

impl FilterSweep {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            knob: Smoothed::new(0.0, sample_rate / 64.0), // block-rate smoothing
            biquad_l: Biquad::lowpass(sample_rate, 20000.0),
            biquad_r: Biquad::lowpass(sample_rate, 20000.0),
            sample_rate,
            active: false,
        }
    }

    pub fn set_knob(&mut self, v: f32) {
        self.knob.set_target(v.clamp(-1.0, 1.0));
    }

    /// Call once per block to advance smoothing and refresh coefficients.
    pub fn update_block(&mut self) {
        let k = self.knob.step();
        if k.abs() < 0.02 {
            self.active = false;
            return;
        }
        // Refresh coefficients but keep filter state for continuity.
        let (l_state, r_state) = (self.biquad_l, self.biquad_r);
        if k < 0.0 {
            let fc = 20000.0 * (110.0f32 / 20000.0).powf(-k);
            self.biquad_l = Biquad::lowpass(self.sample_rate, fc);
            self.biquad_r = Biquad::lowpass(self.sample_rate, fc);
        } else {
            let fc = 30.0 * (8000.0f32 / 30.0).powf(k);
            self.biquad_l = Biquad::highpass(self.sample_rate, fc);
            self.biquad_r = Biquad::highpass(self.sample_rate, fc);
        }
        self.biquad_l.copy_state(&l_state);
        self.biquad_r.copy_state(&r_state);
        self.active = true;
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.active {
            return (l, r);
        }
        (self.biquad_l.process(l), self.biquad_r.process(r))
    }
}

/// Stereo feedback delay, post-EQ pre-fader. Time changes crossfade via the
/// smoothed read offset.
pub struct Delay {
    buf: Vec<f32>, // interleaved stereo
    write: usize,
    time: Smoothed,
    mix: Smoothed,
    feedback: f32,
    sample_rate: f32,
}

impl Delay {
    pub fn new(sample_rate: f32) -> Self {
        let cap = (sample_rate as usize * 2).next_power_of_two();
        Self {
            buf: vec![0.0; cap * 2],
            write: 0,
            time: Smoothed::new(0.35 * sample_rate, sample_rate / 64.0),
            mix: Smoothed::new(0.0, sample_rate),
            feedback: 0.45,
            sample_rate,
        }
    }

    pub fn set_time_secs(&mut self, secs: f32) {
        let frames = (secs.clamp(0.03, 1.8) * self.sample_rate).round();
        self.time.set_target(frames);
    }

    pub fn set_mix(&mut self, v: f32) {
        self.mix.set_target(v.clamp(0.0, 1.0));
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, delay_frames: f32) -> (f32, f32) {
        let mix = self.mix.step();
        let frames = self.buf.len() / 2;
        let d = (delay_frames as usize).clamp(1, frames - 1);
        let read = (self.write + frames - d) % frames;
        let wl = self.buf[read * 2];
        let wr = self.buf[read * 2 + 1];
        self.buf[self.write * 2] = l + wl * self.feedback;
        self.buf[self.write * 2 + 1] = r + wr * self.feedback;
        self.write = (self.write + 1) % frames;
        (l + wl * mix, r + wr * mix)
    }

    /// Advance block-rate time smoothing; returns current delay in frames.
    pub fn block_time(&mut self) -> f32 {
        self.time.step()
    }
}

const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    filter_store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            filter_store: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32, damp: f32, room: f32) -> f32 {
        let out = self.buf[self.idx];
        self.filter_store = out * (1.0 - damp) + self.filter_store * damp;
        self.buf[self.idx] = input + self.filter_store * room;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let bufout = self.buf[self.idx];
        let out = -input + bufout;
        self.buf[self.idx] = input + bufout * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Freeverb-style stereo reverb with a wet-mix knob.
pub struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpasses_l: Vec<Allpass>,
    allpasses_r: Vec<Allpass>,
    mix: Smoothed,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let scale = sample_rate / 44100.0;
        let st = |n: usize| ((n as f32) * scale) as usize;
        const SPREAD: usize = 23;
        Self {
            combs_l: COMB_TUNINGS.iter().map(|&n| Comb::new(st(n))).collect(),
            combs_r: COMB_TUNINGS
                .iter()
                .map(|&n| Comb::new(st(n + SPREAD)))
                .collect(),
            allpasses_l: ALLPASS_TUNINGS.iter().map(|&n| Allpass::new(st(n))).collect(),
            allpasses_r: ALLPASS_TUNINGS
                .iter()
                .map(|&n| Allpass::new(st(n + SPREAD)))
                .collect(),
            mix: Smoothed::new(0.0, sample_rate),
        }
    }

    pub fn set_mix(&mut self, v: f32) {
        self.mix.set_target(v.clamp(0.0, 1.0));
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mix = self.mix.step();
        if mix < 0.001 {
            return (l, r);
        }
        const DAMP: f32 = 0.4;
        const ROOM: f32 = 0.84;
        let input = (l + r) * 0.015;
        let mut wl = 0.0f32;
        let mut wr = 0.0f32;
        for c in &mut self.combs_l {
            wl += c.process(input, DAMP, ROOM);
        }
        for c in &mut self.combs_r {
            wr += c.process(input, DAMP, ROOM);
        }
        for a in &mut self.allpasses_l {
            wl = a.process(wl);
        }
        for a in &mut self.allpasses_r {
            wr = a.process(wr);
        }
        (l + wl * mix, r + wr * mix)
    }
}

/// Vocal remove / isolate via band-limited mid-side processing.
///
/// Lead vocals are almost always panned dead center, so they live in the mid
/// signal M = (L+R)/2 while the instruments spread into the side signal
/// S = (L-R)/2. Cancelling M removes the vocal — but bass, kick and snare are
/// usually centered too, so a naive full-band cancel guts the low end. This
/// only cancels M between `f_lo` and `f_hi`, keeping centered lows and the
/// air band intact.
///
/// This is a stereo-image trick, not source separation: it cannot recover a
/// vocal mixed off-center or drenched in stereo reverb, and it thins any
/// instrument sharing the vocal band. True stem separation needs an offline
/// ML model — this is the real-time approximation hardware mixers use.
pub struct VocalFilter {
    split: BandSplitter,
    amount: Smoothed,
    isolate: bool,
}

impl VocalFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            // Roughly the fundamental-to-presence range of a sung voice.
            split: BandSplitter::new(sample_rate, 180.0, 8000.0),
            amount: Smoothed::new(0.0, sample_rate),
            isolate: false,
        }
    }

    pub fn set(&mut self, amount: f32, isolate: bool) {
        self.amount.set_target(amount.clamp(0.0, 1.0));
        self.isolate = isolate;
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let amt = self.amount.step();
        let mid = (l + r) * 0.5;
        let side = (l - r) * 0.5;
        // The splitter must run every sample regardless of `amount`, or its
        // filter state goes stale and the effect clicks when dialed back in.
        let (m_lo, m_mid, m_hi) = self.split.split(mid);
        if amt < 0.001 {
            return (l, r);
        }
        let (wet_l, wet_r) = if self.isolate {
            // Keep only the centered vocal band; drop the sides entirely.
            (m_mid, m_mid)
        } else {
            // Keep the sides plus the centered content outside the vocal band.
            let keep = m_lo + m_hi;
            (keep + side, keep - side)
        };
        (l + (wet_l - l) * amt, r + (wet_r - r) * amt)
    }
}

/// Soft peak limiter on the master bus: instant attack, exponential release.
pub struct Limiter {
    envelope: f32,
    release: f32,
    threshold: f32,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            envelope: 0.0,
            release: (-1.0 / (0.120 * sample_rate)).exp(),
            threshold: 0.97,
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        self.envelope = if peak > self.envelope {
            peak
        } else {
            peak + (self.envelope - peak) * self.release
        };
        let gain = if self.envelope > self.threshold {
            self.threshold / self.envelope
        } else {
            1.0
        };
        ((l * gain).clamp(-1.0, 1.0), (r * gain).clamp(-1.0, 1.0))
    }
}

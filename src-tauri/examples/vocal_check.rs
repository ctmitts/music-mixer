//! Measures what the vocal filter actually does to a real track.
//! Usage: cargo run --example vocal_check -- <file>

use mix_table_lib::{decode, dsp::VocalFilter};

/// RMS of a mono-summed slice, in dB.
fn rms_db(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum / samples.len() as f64).sqrt();
    if rms <= 1e-9 {
        -120.0
    } else {
        20.0 * rms.log10()
    }
}

/// How correlated L and R are: 1.0 = mono (no side content at all),
/// 0.0 = fully decorrelated. Center-cancelling a mono file yields silence.
fn correlation(inter: &[f32]) -> f64 {
    let mut num = 0.0f64;
    let mut dl = 0.0f64;
    let mut dr = 0.0f64;
    for f in inter.chunks_exact(2) {
        num += f[0] as f64 * f[1] as f64;
        dl += (f[0] as f64).powi(2);
        dr += (f[1] as f64).powi(2);
    }
    if dl <= 0.0 || dr <= 0.0 {
        return 1.0;
    }
    num / (dl * dr).sqrt()
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: vocal_check <file>");
        return;
    };
    let (samples, rate) = match decode::decode_source(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("decode failed: {e}");
            return;
        }
    };

    // Take 30 s from a third of the way in — past any intro.
    let start = (samples.len() / 3) & !1;
    let len = (30 * rate as usize * 2).min(samples.len() - start);
    let slice = &samples[start..start + len];

    println!("file: {}", path.rsplit('/').next().unwrap_or(&path));
    println!("stereo correlation: {:.3}  (1.0 = mono, nothing to cancel)", correlation(slice));

    for (label, isolate) in [("REMOVE (VOX-)", false), ("ISOLATE (VOX+)", true)] {
        let mut vf = VocalFilter::new(rate as f32);
        vf.set(1.0, isolate);
        let mut out = Vec::with_capacity(slice.len());
        for f in slice.chunks_exact(2) {
            let (l, r) = vf.process(f[0], f[1]);
            out.push(l);
            out.push(r);
        }
        // Skip the smoothing ramp-in.
        let skip = (rate as usize * 2).min(out.len());
        let dry_mono: Vec<f32> = slice[skip..].chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
        let wet_mono: Vec<f32> = out[skip..].chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
        let diff: Vec<f32> = dry_mono
            .iter()
            .zip(wet_mono.iter())
            .map(|(d, w)| d - w)
            .collect();
        println!(
            "  {label:<15} dry {:>6.1} dB → wet {:>6.1} dB   (removed content: {:>6.1} dB)",
            rms_db(&dry_mono),
            rms_db(&wet_mono),
            rms_db(&diff),
        );
    }
}

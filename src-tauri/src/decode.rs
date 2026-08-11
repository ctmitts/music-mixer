//! Offline track decoding: symphonia decode → stereo f32 → (optional)
//! rubato resample to the engine rate → waveform peak precompute.

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Result};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const OVERVIEW_BINS: usize = 1200;
pub const DETAIL_BINS_PER_SEC: f32 = 100.0;

pub struct Decoded {
    /// Interleaved stereo at `engine_rate`.
    pub samples: Vec<f32>,
    pub engine_rate: u32,
    pub source_rate: u32,
    pub duration_secs: f64,
    /// Max-abs peaks, fixed bin count over the whole track.
    pub overview: Vec<f32>,
    /// Max-abs peaks at DETAIL_BINS_PER_SEC for the zoomed view.
    pub detail: Vec<f32>,
}

/// Decode a file to interleaved stereo f32 at its source rate (no resample,
/// no peak computation) — used by the offline analysis worker.
pub fn decode_source(path: &str) -> Result<(Vec<f32>, u32)> {
    decode_to_stereo(Path::new(path))
}

/// Decode a file to interleaved stereo f32 at its source rate.
fn decode_to_stereo(path: &Path) -> Result<(Vec<f32>, u32)> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no decodable audio track"))?;
    let track_id = track.id;

    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);

    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                sample_rate = spec.rate;
                let channels = spec.channels.count();
                if sample_buf
                    .as_ref()
                    .map(|b| b.capacity() < decoded.capacity() * channels)
                    .unwrap_or(true)
                {
                    sample_buf = Some(SampleBuffer::<f32>::new(
                        decoded.capacity() as u64,
                        spec,
                    ));
                }
                let buf = sample_buf.as_mut().unwrap();
                buf.copy_interleaved_ref(decoded);
                let data = buf.samples();
                match channels {
                    0 => {}
                    1 => {
                        for &s in data {
                            interleaved.push(s);
                            interleaved.push(s);
                        }
                    }
                    n => {
                        for frame in data.chunks_exact(n) {
                            interleaved.push(frame[0]);
                            interleaved.push(frame[1]);
                        }
                    }
                }
            }
            // Corrupt packet: skip and keep going (common with slightly
            // damaged MP3s).
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => return Err(e.into()),
        }
    }

    if sample_rate == 0 || interleaved.is_empty() {
        return Err(anyhow!("no audio decoded"));
    }
    Ok((interleaved, sample_rate))
}

/// High-quality sinc resample of interleaved stereo.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>> {
    const CHUNK: usize = 4096;
    let frames = input.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for frame in input.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = to_rate as f64 / from_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.1, params, CHUNK, 2)?;
    let delay = resampler.output_delay();

    let expected = (frames as f64 * ratio).round() as usize;
    let mut out_l: Vec<f32> = Vec::with_capacity(expected + CHUNK);
    let mut out_r: Vec<f32> = Vec::with_capacity(expected + CHUNK);

    let mut pos = 0;
    while pos + CHUNK <= frames {
        let chunk = [&left[pos..pos + CHUNK], &right[pos..pos + CHUNK]];
        let out = resampler.process(&chunk, None)?;
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
        pos += CHUNK;
    }
    if pos < frames {
        let chunk = [&left[pos..], &right[pos..]];
        let out = resampler.process_partial(Some(&chunk), None)?;
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
    }
    // Flush the resampler's tail so the end of the track isn't cut off.
    loop {
        let out = resampler.process_partial::<&[f32]>(None, None)?;
        if out[0].is_empty() {
            break;
        }
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
        if out_l.len() >= delay + expected {
            break;
        }
    }

    // Trim the sinc filter's group delay and clamp to the expected length so
    // waveforms and positions stay sample-aligned with the source.
    let start = delay.min(out_l.len());
    let end = (start + expected).min(out_l.len());
    let mut interleaved = Vec::with_capacity((end - start) * 2);
    for i in start..end {
        interleaved.push(out_l[i]);
        interleaved.push(out_r[i]);
    }
    Ok(interleaved)
}

fn peaks(samples: &[f32], bins: usize) -> Vec<f32> {
    let frames = samples.len() / 2;
    if frames == 0 || bins == 0 {
        return vec![0.0; bins];
    }
    let mut out = vec![0.0f32; bins];
    let frames_per_bin = (frames as f64 / bins as f64).max(1.0);
    for (bin, slot) in out.iter_mut().enumerate() {
        let start = (bin as f64 * frames_per_bin) as usize;
        let end = (((bin + 1) as f64 * frames_per_bin) as usize).min(frames);
        let mut max = 0.0f32;
        for f in start..end {
            let a = samples[f * 2].abs().max(samples[f * 2 + 1].abs());
            if a > max {
                max = a;
            }
        }
        *slot = max;
    }
    out
}

pub fn decode_file(path: &str, engine_rate: u32) -> Result<Decoded> {
    let path_ref = Path::new(path);
    let (raw, source_rate) = decode_to_stereo(path_ref)?;
    let samples = if source_rate != engine_rate {
        resample(&raw, source_rate, engine_rate)?
    } else {
        raw
    };
    let frames = samples.len() / 2;
    let duration_secs = frames as f64 / engine_rate as f64;
    let overview = peaks(&samples, OVERVIEW_BINS);
    let detail_bins = ((duration_secs as f32) * DETAIL_BINS_PER_SEC).ceil() as usize;
    let detail = peaks(&samples, detail_bins.max(1));
    Ok(Decoded {
        samples,
        engine_rate,
        source_rate,
        duration_secs,
        overview,
        detail,
    })
}

//! Golden-path decode tests over real files generated with macOS
//! `say`/`afconvert` (set MIX_TABLE_TEST_AUDIO to the folder containing them).

use mix_table_lib::decode::decode_file;
use mix_table_lib::library::scan_folder;

fn audio_dir() -> Option<String> {
    std::env::var("MIX_TABLE_TEST_AUDIO").ok()
}

fn assert_decodes(path: &str, engine_rate: u32) {
    let decoded = decode_file(path, engine_rate)
        .unwrap_or_else(|e| panic!("decode failed for {path}: {e}"));
    assert_eq!(decoded.engine_rate, engine_rate, "{path}");
    assert!(decoded.duration_secs > 0.5, "{path}: too short");
    assert!(decoded.samples.len() % 2 == 0, "{path}: not stereo-interleaved");
    let frames = decoded.samples.len() / 2;
    let expected = decoded.duration_secs * engine_rate as f64;
    assert!(
        (frames as f64 - expected).abs() < engine_rate as f64 * 0.1,
        "{path}: frame count {frames} inconsistent with duration"
    );
    let peak = decoded.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "{path}: decoded audio is silent (peak {peak})");
    assert!(peak <= 1.5, "{path}: implausible peak {peak}");
    assert!(!decoded.overview.is_empty() && !decoded.detail.is_empty());
    let ov_peak = decoded.overview.iter().cloned().fold(0.0f32, f32::max);
    assert!(ov_peak > 0.05, "{path}: overview peaks empty");
}

#[test]
fn decodes_all_generated_formats_at_native_and_resampled_rates() {
    let Some(dir) = audio_dir() else {
        eprintln!("MIX_TABLE_TEST_AUDIO not set; skipping");
        return;
    };
    for file in ["test.aiff", "test.flac", "test.m4a", "test44.wav", "test96.wav"] {
        let path = format!("{dir}/{file}");
        // Native-ish rate and a forced-resample rate.
        assert_decodes(&path, 44100);
        assert_decodes(&path, 48000);
    }
}

#[test]
fn library_scan_finds_generated_files() {
    let Some(dir) = audio_dir() else {
        eprintln!("MIX_TABLE_TEST_AUDIO not set; skipping");
        return;
    };
    let tracks = scan_folder(&dir);
    assert!(
        tracks.len() >= 5,
        "expected at least 5 tracks, found {}",
        tracks.len()
    );
    for t in &tracks {
        assert!(!t.title.is_empty(), "{}: empty title", t.path);
    }
}

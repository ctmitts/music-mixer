//! Analysis sanity check: decode files and print BPM / key / fluid flag.
//! Usage: cargo run --release --example analyze_check -- <file> [<file>...]

fn main() {
    for path in std::env::args().skip(1) {
        let started = std::time::Instant::now();
        match mix_table_lib::decode::decode_source(&path) {
            Ok((samples, rate)) => {
                let a = mix_table_lib::analysis::analyze(&samples, rate);
                println!(
                    "{}\n  bpm={} offset={:.2}s drift={:.1}% fluid={} key={} ({})  [{:.1}s]",
                    path.rsplit('/').next().unwrap_or(&path),
                    a.bpm,
                    a.beat_offset,
                    a.tempo_drift * 100.0,
                    a.fluid,
                    a.key_name,
                    a.camelot,
                    started.elapsed().as_secs_f64(),
                );
            }
            Err(e) => println!("{path}: decode failed: {e}"),
        }
    }
}

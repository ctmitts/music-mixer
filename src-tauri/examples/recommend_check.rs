//! Sanity check for the local recommender: analyze a handful of real tracks,
//! then rank the rest against the first one.
//! Usage: cargo run --example recommend_check -- <reference> <candidate>...

use mix_table_lib::{analysis, decode, library, recommend};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.len() < 2 {
        eprintln!("usage: recommend_check <reference> <candidate>...");
        return;
    }

    let mut candidates = Vec::new();
    for p in &paths {
        match decode::decode_source(p) {
            Ok((samples, rate)) => {
                let a = analysis::analyze(&samples, rate);
                println!(
                    "analyzed {:<38} {:>6} {:>4}  energy {:.2} bright {:.2} pulse {:.2}{}",
                    p.rsplit('/').next().unwrap_or(p).chars().take(38).collect::<String>(),
                    format!("{:.0}bpm", a.bpm),
                    a.camelot,
                    a.energy,
                    a.brightness,
                    a.beat_strength,
                    if a.fluid { "  FLUID" } else { "" },
                );
                candidates.push(recommend::Candidate {
                    meta: library::track_meta(p),
                    analysis: a,
                    taste: Default::default(),
                });
            }
            Err(e) => eprintln!("decode failed {p}: {e}"),
        }
    }

    let reference = candidates[0].analysis.clone();
    println!("\n--- ranked against \"{}\" ---", candidates[0].meta.title);
    let (picks, _) = recommend::rank(&candidates, Some(&reference), None, &[paths[0].clone()], 8);
    for t in &picks {
        println!("  {:.3}  {:<34}  {}", t.score, t.title.chars().take(34).collect::<String>(), t.reason);
    }

    for q in [
        "something dark and driving around 120",
        "chill jazzy downtempo",
    ] {
        let parsed = recommend::parse_query(q);
        println!("\n--- query: \"{q}\" ---");
        println!("  parsed: bpm={:?} energy={:?} bright={:?} keywords={:?}",
                 parsed.bpm_target, parsed.energy_target, parsed.brightness_target, parsed.keywords);
        let (picks, _) = recommend::rank(&candidates, None, Some(&parsed), &[], 3);
        for t in &picks {
            println!("  {:.3}  {}", t.score, t.title.chars().take(46).collect::<String>());
        }
    }
}

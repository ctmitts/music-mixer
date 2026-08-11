//! Reproduces the "every query returns the same dance tracks" bug against the
//! real library, using the cached analysis DB.
//! Usage: cargo run --example query_check -- "<reference track path>"

use mix_table_lib::{db, library, recommend};

fn main() {
    let reference_path = std::env::args().nth(1);

    let dir = std::env::var("HOME").unwrap() + "/Library/Application Support/com.colin.mixtable";
    let database = match db::Db::open(std::path::Path::new(&dir)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open db failed: {e}");
            return;
        }
    };

    let root = "/Users/colin/Music/Music/Media.localized/Music";
    let tracks = library::scan_folder(root);
    println!("scanned {} tracks", tracks.len());

    let candidates: Vec<recommend::Candidate> = tracks
        .into_iter()
        .filter_map(|meta| {
            let analysis = database.get_analysis(&meta.path)?;
            Some(recommend::Candidate { meta, analysis })
        })
        .collect();
    println!("with cached analysis: {}\n", candidates.len());
    if candidates.is_empty() {
        println!("Nothing analyzed yet — run Analyze in the app first.");
        return;
    }

    let reference = reference_path.as_ref().and_then(|p| database.get_analysis(p));
    if let Some(r) = &reference {
        println!("reference: {} {} energy {:.2}\n", r.camelot, r.bpm, r.energy);
    }

    for q in [
        "reggae",
        "dancey jazz that's parisian chic",
        "dark driving house around 128",
        "something classical and slow",
    ] {
        let parsed = recommend::parse_query(q);
        let (picks, info) = recommend::rank(&candidates, reference.as_ref(), Some(&parsed), &[], 5);
        println!("── \"{q}\"");
        println!(
            "   genre terms {:?} → {} library matches{}",
            parsed.genre_terms,
            info.genre_matches,
            if info.genre_requested_but_empty { " (NONE — fell back)" } else { "" }
        );
        for t in &picks {
            println!(
                "   {:.2}  {:<34} {:<22} [{}]",
                t.score,
                t.title.chars().take(34).collect::<String>(),
                t.artist.chars().take(22).collect::<String>(),
                t.genre
            );
        }
        println!();
    }
}

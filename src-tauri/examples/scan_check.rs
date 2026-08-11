//! Debug harness: scan a folder exactly like the app does and grep titles.
//! Usage: cargo run --example scan_check -- <folder> <search>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = args.get(1).expect("usage: scan_check <folder> <search>");
    let needle = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
    let tracks = mix_table_lib::library::scan_folder(folder);
    println!("total tracks scanned: {}", tracks.len());
    for t in &tracks {
        let hay = format!("{} {} {}", t.title, t.artist, t.album).to_lowercase();
        if hay.contains(&needle) {
            println!(
                "MATCH: title={:?} artist={:?} album={:?} path={:?}",
                t.title, t.artist, t.album, t.path
            );
        }
    }
}

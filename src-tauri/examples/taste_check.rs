//! Verifies the taste schema against the real database: migrations apply on
//! an existing DB, a play row opens/accumulates/closes, and the aggregate
//! query returns what the UI expects.
//! Usage: cargo run --example taste_check

use mix_table_lib::db;

fn main() {
    let dir = std::env::var("HOME").unwrap() + "/Library/Application Support/com.colin.mixtable";
    let database = match db::Db::open(std::path::Path::new(&dir)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open db failed: {e}");
            return;
        }
    };
    println!("opened {dir} — migrations applied cleanly");

    let stats = database.all_stats();
    println!("existing rows with plays or taste: {}", stats.len());
    let mut ranked: Vec<_> = stats.iter().filter(|s| s.1 > 0).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    for s in ranked.iter().take(5) {
        println!(
            "   {:>3} plays  {:>7.0}s  loved={} banned={}  {}",
            s.1,
            s.2,
            s.4,
            s.5,
            s.0.rsplit('/').next().unwrap_or(&s.0)
        );
    }

    // Round-trip a synthetic play against a path that isn't a real file.
    let probe = "/__taste_check_probe__.mp3";
    let id = database.open_play(probe, 300.0);
    println!("\nopened play row id={id}");
    database.add_play_time(id, 200.0);
    let found = database
        .all_stats()
        .into_iter()
        .find(|s| s.0 == probe)
        .expect("probe row missing from all_stats");
    println!("   after 200s: count={} secs={:.0}", found.1, found.2);
    assert_eq!(found.1, 1);
    assert!((found.2 - 200.0).abs() < 0.01);

    // 200s of a 300s track passes the half-track rule.
    let pending = database.pending_scrobbles(50);
    let mine = pending.iter().find(|p| p.1 == probe);
    println!("   qualifies for scrobble: {}", mine.is_some());
    assert!(mine.is_some(), "should qualify at 2/3 played");
    database.mark_scrobbled(mine.unwrap().0);
    assert!(
        !database.pending_scrobbles(50).iter().any(|p| p.1 == probe),
        "marked row still pending"
    );
    println!("   marked scrobbled, no longer pending");

    database.set_taste(probe, true, false);
    let t = database.all_stats().into_iter().find(|s| s.0 == probe).unwrap();
    println!("   taste loved={} banned={}", t.4, t.5);
    assert!(t.4 && !t.5);

    database.log_transition(probe, "/__taste_check_probe_b__.mp3");
    println!("   logged a transition");

    println!("\nAll assertions passed. Cleaning up probe rows.");
    let conn = database.0.lock().unwrap();
    let _ = conn.execute("DELETE FROM plays WHERE path LIKE '/__taste_check%'", []);
    let _ = conn.execute("DELETE FROM taste WHERE path LIKE '/__taste_check%'", []);
    let _ = conn.execute(
        "DELETE FROM transitions WHERE from_path LIKE '/__taste_check%'",
        [],
    );
}

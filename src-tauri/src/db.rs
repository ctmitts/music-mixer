//! SQLite cache for track analysis and hot-cue points.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::analysis::Analysis;

pub struct Db(pub Mutex<Connection>);

pub fn file_mtime(path: &str) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let conn = Connection::open(dir.join("mixtable.sqlite3"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS analysis (
                path TEXT PRIMARY KEY,
                mtime INTEGER NOT NULL,
                bpm REAL, beat_offset REAL, tempo_drift REAL,
                fluid INTEGER, key_name TEXT, camelot TEXT,
                analyzed_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS cues (
                path TEXT NOT NULL,
                slot INTEGER NOT NULL,
                seconds REAL NOT NULL,
                PRIMARY KEY (path, slot)
            );",
        )?;
        // Migrations for databases created by earlier versions. Each is
        // best-effort: an error means the column already exists.
        for stmt in [
            "ALTER TABLE analysis ADD COLUMN lufs REAL",
            "ALTER TABLE analysis ADD COLUMN energy REAL",
            "ALTER TABLE analysis ADD COLUMN brightness REAL",
            "ALTER TABLE analysis ADD COLUMN beat_strength REAL",
            "ALTER TABLE analysis ADD COLUMN version INTEGER DEFAULT 1",
            "ALTER TABLE cues ADD COLUMN label TEXT",
        ] {
            let _ = conn.execute(stmt, []);
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- One row per play. `secs_played` accumulates only while the deck
            -- actually runs, so pausing mid-track doesn't inflate it and the
            -- completion fraction stays an honest like/skip signal.
            CREATE TABLE IF NOT EXISTS plays (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                secs_played REAL NOT NULL DEFAULT 0,
                track_secs REAL NOT NULL DEFAULT 0,
                scrobbled INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS plays_path ON plays(path);

            -- Explicit taste. Absent row = no opinion.
            CREATE TABLE IF NOT EXISTS taste (
                path TEXT PRIMARY KEY,
                loved INTEGER NOT NULL DEFAULT 0,
                banned INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER
            );

            -- Every real A->B mix: the labelled examples a personal
            -- transition model is eventually trained on.
            CREATE TABLE IF NOT EXISTS transitions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_path TEXT NOT NULL,
                to_path TEXT NOT NULL,
                at INTEGER NOT NULL,
                rating INTEGER
            );
            CREATE INDEX IF NOT EXISTS transitions_pair
                ON transitions(from_path, to_path);

            -- Cached Last.fm artist similarity, so the network is hit once
            -- per artist rather than once per recommendation.
            CREATE TABLE IF NOT EXISTS similar_artists (
                artist TEXT PRIMARY KEY,
                json TEXT NOT NULL,
                fetched_at INTEGER NOT NULL
            );",
        )?;
        Ok(Db(Mutex::new(conn)))
    }

    /// Cached analysis for `path`, but only if it was produced by the current
    /// analyzer version and the file hasn't changed since.
    pub fn get_analysis(&self, path: &str) -> Option<Analysis> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT bpm, beat_offset, tempo_drift, fluid, key_name, camelot, lufs,
                    energy, brightness, beat_strength
             FROM analysis
             WHERE path = ?1 AND mtime = ?2 AND COALESCE(version, 1) = ?3",
            params![path, file_mtime(path), crate::analysis::ANALYSIS_VERSION],
            |r| {
                Ok(Analysis {
                    bpm: r.get(0)?,
                    beat_offset: r.get(1)?,
                    tempo_drift: r.get(2)?,
                    fluid: r.get::<_, i64>(3)? != 0,
                    key_name: r.get(4)?,
                    camelot: r.get(5)?,
                    lufs: r.get::<_, Option<f64>>(6)?.unwrap_or(-14.0),
                    energy: r.get::<_, Option<f64>>(7)?.unwrap_or(0.5),
                    brightness: r.get::<_, Option<f64>>(8)?.unwrap_or(0.5),
                    beat_strength: r.get::<_, Option<f64>>(9)?.unwrap_or(0.0),
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        );
    }

    pub fn put_analysis(&self, path: &str, a: &Analysis) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO analysis
             (path, mtime, bpm, beat_offset, tempo_drift, fluid, key_name, camelot,
              lufs, energy, brightness, beat_strength, version, analyzed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     strftime('%s','now'))",
            params![
                path,
                file_mtime(path),
                a.bpm,
                a.beat_offset,
                a.tempo_drift,
                a.fluid as i64,
                a.key_name,
                a.camelot,
                a.lufs,
                a.energy,
                a.brightness,
                a.beat_strength,
                crate::analysis::ANALYSIS_VERSION
            ],
        );
    }

    /// 4 cue slots per track; None = unset.
    pub fn get_cues(&self, path: &str) -> [Option<f64>; 4] {
        let conn = self.0.lock().unwrap();
        let mut out = [None; 4];
        if let Ok(mut stmt) = conn.prepare("SELECT slot, seconds FROM cues WHERE path = ?1") {
            let rows = stmt.query_map(params![path], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    if (0..4).contains(&row.0) {
                        out[row.0 as usize] = Some(row.1);
                    }
                }
            }
        }
        out
    }

    /// User notes on cue slots ("sax rip", "vocals enter"). None = unnamed.
    pub fn get_cue_labels(&self, path: &str) -> [Option<String>; 4] {
        let conn = self.0.lock().unwrap();
        let mut out: [Option<String>; 4] = Default::default();
        if let Ok(mut stmt) = conn.prepare("SELECT slot, label FROM cues WHERE path = ?1") {
            let rows = stmt.query_map(params![path], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    if (0..4).contains(&row.0) {
                        out[row.0 as usize] = row.1.filter(|s| !s.trim().is_empty());
                    }
                }
            }
        }
        out
    }

    pub fn set_cue(&self, path: &str, slot: usize, seconds: Option<f64>) {
        let conn = self.0.lock().unwrap();
        match seconds {
            Some(s) => {
                // Upsert rather than INSERT OR REPLACE so re-placing a cue
                // keeps its label.
                let _ = conn.execute(
                    "INSERT INTO cues (path, slot, seconds) VALUES (?1, ?2, ?3)
                     ON CONFLICT(path, slot) DO UPDATE SET seconds = excluded.seconds",
                    params![path, slot as i64, s],
                );
            }
            None => {
                let _ = conn.execute(
                    "DELETE FROM cues WHERE path = ?1 AND slot = ?2",
                    params![path, slot as i64],
                );
            }
        }
    }

    /// Name (or clear the name of) an existing cue slot. No-op if the slot
    /// has no cue point.
    pub fn set_cue_label(&self, path: &str, slot: usize, label: Option<&str>) {
        let conn = self.0.lock().unwrap();
        let label = label.map(str::trim).filter(|s| !s.is_empty());
        let _ = conn.execute(
            "UPDATE cues SET label = ?3 WHERE path = ?1 AND slot = ?2",
            params![path, slot as i64, label],
        );
    }

    // -- taste: plays, loves, transitions ---------------------------------

    /// Open a play row and return its id, for `close_play` to finish.
    pub fn open_play(&self, path: &str, track_secs: f64) -> i64 {
        let conn = self.0.lock().unwrap();
        match conn.execute(
            "INSERT INTO plays (path, started_at, secs_played, track_secs)
             VALUES (?1, strftime('%s','now'), 0, ?2)",
            params![path, track_secs],
        ) {
            Ok(_) => conn.last_insert_rowid(),
            Err(_) => 0,
        }
    }

    /// Add listened time to an open play row.
    pub fn add_play_time(&self, id: i64, secs: f64) {
        if id == 0 || secs <= 0.0 {
            return;
        }
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "UPDATE plays SET secs_played = secs_played + ?2 WHERE id = ?1",
            params![id, secs],
        );
    }

    pub fn log_transition(&self, from: &str, to: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO transitions (from_path, to_path, at)
             VALUES (?1, ?2, strftime('%s','now'))",
            params![from, to],
        );
    }

    /// Per-track listening stats for the whole library, keyed by path.
    /// `(play_count, secs_played_total, last_played, loved, banned)`.
    pub fn all_stats(&self) -> Vec<(String, i64, f64, i64, bool, bool)> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        // Plays and taste are independent facts about a path, so this is a
        // full outer join expressed as two passes: loved-but-never-played
        // tracks must still appear.
        if let Ok(mut stmt) = conn.prepare(
            "SELECT p.path,
                    COUNT(*),
                    COALESCE(SUM(p.secs_played), 0),
                    MAX(p.started_at)
             FROM plays p GROUP BY p.path",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            }) {
                out.extend(rows.flatten().map(|(p, c, s, l)| (p, c, s, l, false, false)));
            }
        }
        let mut index: std::collections::HashMap<String, usize> = out
            .iter()
            .enumerate()
            .map(|(i, r)| (r.0.clone(), i))
            .collect();
        if let Ok(mut stmt) =
            conn.prepare("SELECT path, loved, banned FROM taste WHERE loved = 1 OR banned = 1")
        {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)? != 0,
                ))
            }) {
                for (path, loved, banned) in rows.flatten() {
                    match index.get(&path) {
                        Some(&i) => {
                            out[i].4 = loved;
                            out[i].5 = banned;
                        }
                        None => {
                            index.insert(path.clone(), out.len());
                            out.push((path, 0, 0.0, 0, loved, banned));
                        }
                    }
                }
            }
        }
        out
    }

    pub fn set_taste(&self, path: &str, loved: bool, banned: bool) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO taste (path, loved, banned, updated_at)
             VALUES (?1, ?2, ?3, strftime('%s','now'))
             ON CONFLICT(path) DO UPDATE SET
                loved = excluded.loved,
                banned = excluded.banned,
                updated_at = excluded.updated_at",
            params![path, loved as i64, banned as i64],
        );
    }

    // -- Last.fm -----------------------------------------------------------

    /// Cached similar-artist JSON, if it was fetched within `max_age_secs`.
    pub fn get_similar_artists(&self, artist: &str, max_age_secs: i64) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT json FROM similar_artists
             WHERE artist = ?1 AND strftime('%s','now') - fetched_at < ?2",
            params![artist.to_lowercase(), max_age_secs],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn put_similar_artists(&self, artist: &str, json: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO similar_artists (artist, json, fetched_at)
             VALUES (?1, ?2, strftime('%s','now'))",
            params![artist.to_lowercase(), json],
        );
    }

    /// Plays that qualify for scrobbling and haven't been sent yet:
    /// Last.fm's rule is half the track or 4 minutes, whichever comes first,
    /// and tracks must be longer than 30 s.
    pub fn pending_scrobbles(&self, limit: usize) -> Vec<(i64, String, i64)> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, path, started_at FROM plays
             WHERE scrobbled = 0 AND track_secs > 30
               AND (secs_played >= track_secs / 2.0 OR secs_played >= 240)
             ORDER BY started_at LIMIT ?1",
        ) {
            if let Ok(rows) = stmt.query_map(params![limit as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            }) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    pub fn mark_scrobbled(&self, id: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("UPDATE plays SET scrobbled = 1 WHERE id = ?1", params![id]);
    }
}

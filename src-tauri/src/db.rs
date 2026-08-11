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
}

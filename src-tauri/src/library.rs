//! Library folder scanning and tag reading (lofty).

use std::path::Path;

use base64::Engine as _;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::Accessor;
use serde::Serialize;
use walkdir::WalkDir;

/// Formats symphonia can decode with the "all" feature set.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "mp4", "aac", "ogg", "oga", "aif", "aiff", "aifc", "caf", "mka",
    "mkv", "webm",
];

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrackMeta {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub bit_depth: Option<u8>,
    pub format: String,
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Public single-file metadata read (used for deck-state restore).
pub fn track_meta(path: &str) -> TrackMeta {
    read_meta(Path::new(path))
}

fn read_meta(path: &Path) -> TrackMeta {
    let format = path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();

    let fallback = TrackMeta {
        path: path.to_string_lossy().into_owned(),
        title: file_stem(path),
        artist: String::new(),
        album: String::new(),
        genre: String::new(),
        duration_secs: 0.0,
        sample_rate: 0,
        bit_depth: None,
        format: format.clone(),
    };

    let tagged = match Probe::open(path).and_then(|p| p.read()) {
        Ok(t) => t,
        Err(_) => return fallback,
    };

    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    TrackMeta {
        path: fallback.path,
        title: tag
            .and_then(|t| t.title().map(|s| s.into_owned()))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback.title),
        artist: tag
            .and_then(|t| t.artist().map(|s| s.into_owned()))
            .unwrap_or_default(),
        album: tag
            .and_then(|t| t.album().map(|s| s.into_owned()))
            .unwrap_or_default(),
        genre: tag
            .and_then(|t| t.genre().map(|s| s.into_owned()))
            .unwrap_or_default(),
        duration_secs: props.duration().as_secs_f64(),
        sample_rate: props.sample_rate().unwrap_or(0),
        bit_depth: props.bit_depth(),
        format,
    }
}

/// True when the folder still exists and can be scanned. A configured folder
/// can vanish between launches (moved, deleted, or an unmounted drive), and
/// silently keeping it would leave the library pointing at dead paths.
pub fn folder_exists(folder: &str) -> bool {
    Path::new(folder).is_dir()
}

pub fn scan_folder(folder: &str) -> Vec<TrackMeta> {
    let mut tracks: Vec<TrackMeta> = WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|e| read_meta(e.path()))
        .collect();
    tracks.sort_by(|a, b| {
        (a.artist.to_lowercase(), a.title.to_lowercase())
            .cmp(&(b.artist.to_lowercase(), b.title.to_lowercase()))
    });
    tracks
}

/// First embedded picture as a data URL, if any.
pub fn artwork(path: &str) -> Option<String> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().first()?;
    let mime = pic
        .mime_type()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());
    let data = base64::engine::general_purpose::STANDARD.encode(pic.data());
    Some(format!("data:{mime};base64,{data}"))
}

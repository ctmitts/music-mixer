//! Track recommendation: score library candidates against what's playing,
//! optionally steered by a natural-language description of the wanted sound.
//!
//! This is the offline half of the feature — it always works, with no network
//! and no API key. The optional Claude layer in `agent.rs` re-ranks the top
//! slice of these scores and writes the human-readable reasons.

use serde::{Deserialize, Serialize};

use crate::analysis::Analysis;
use crate::library::TrackMeta;

/// A library track plus its analysis, as the scorer sees it.
#[derive(Clone)]
pub struct Candidate {
    pub meta: TrackMeta,
    pub analysis: Analysis,
    pub taste: Taste,
}

/// What the DJ's own listening says about a track. Defaults to "no opinion",
/// which is what every track looks like before any history exists.
#[derive(Clone, Copy, Default)]
pub struct Taste {
    pub play_count: i64,
    pub loved: bool,
    pub banned: bool,
    /// True when a similar-artist source (Last.fm) vouches for this artist.
    pub similar_artist: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScoredTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub bpm: f64,
    pub camelot: String,
    pub key_name: String,
    pub fluid: bool,
    pub energy: f64,
    pub duration_secs: f64,
    pub score: f64,
    /// Locally-generated explanation, replaced by Claude's when available.
    pub reason: String,
}

/// Constraints and preferences parsed out of a free-text description.
#[derive(Default, Debug, Clone, Deserialize)]
pub struct Query {
    pub text: String,
    /// Explicit tempo target if the text named one ("around 120").
    pub bpm_target: Option<f64>,
    /// Desired energy 0..1, if the text implied one.
    pub energy_target: Option<f64>,
    /// Desired brightness 0..1 ("dark" / "bright").
    pub brightness_target: Option<f64>,
    /// Genre/keyword tokens to match against tags and titles.
    pub keywords: Vec<String>,
    /// Expanded genre-family terms (e.g. "reggae" also matches dancehall,
    /// ska, dub). When any library track matches one of these, the request is
    /// treated as a hard style constraint rather than a preference.
    pub genre_terms: Vec<String>,
    /// True when the text asks for minor-key / moody material.
    pub prefer_minor: Option<bool>,
}

fn parse_camelot(s: &str) -> Option<(i32, char)> {
    let s = s.trim();
    let (num, letter) = s.split_at(s.len().checked_sub(1)?);
    let letter = letter.chars().next()?;
    if letter != 'A' && letter != 'B' {
        return None;
    }
    Some((num.parse().ok()?, letter))
}

/// Harmonic compatibility on the Camelot wheel, 0..1.
pub fn key_score(a: &str, b: &str) -> f64 {
    let (Some((na, la)), Some((nb, lb))) = (parse_camelot(a), parse_camelot(b)) else {
        return 0.4; // unknown key: neutral, don't punish
    };
    let ring = |x: i32, y: i32| {
        let d = (x - y).abs();
        d.min(12 - d)
    };
    let dist = ring(na, nb);
    match (la == lb, dist) {
        (true, 0) => 1.0,   // same key
        (true, 1) => 0.88,  // neighbour on the wheel
        (false, 0) => 0.85, // relative major/minor
        (true, 2) => 0.55,  // two steps — a reach, but usable
        (false, 1) => 0.5,
        (true, 7) | (false, 7) => 0.45, // dominant-ish
        _ => 0.12,
    }
}

/// Tempo reach, 0..1, allowing half- and double-time relationships.
pub fn tempo_score(candidate_bpm: f64, target_bpm: f64) -> f64 {
    if candidate_bpm <= 0.0 || target_bpm <= 0.0 {
        return 0.4;
    }
    [target_bpm, target_bpm * 2.0, target_bpm / 2.0]
        .iter()
        .map(|t| {
            let rel = (candidate_bpm - t).abs() / t;
            // Full marks within 3%, decaying to zero at 10%.
            (1.0 - ((rel - 0.03).max(0.0) / 0.07)).clamp(0.0, 1.0)
        })
        .fold(0.0f64, f64::max)
}

/// Extract tempo/energy/mood hints from a free-text description.
pub fn parse_query(text: &str) -> Query {
    let lower = text.to_lowercase();
    let mut q = Query {
        text: text.to_string(),
        ..Default::default()
    };

    // Explicit tempo: "120 bpm", "around 128", "~95"
    let mut chars = lower.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_ascii_digit() {
            let start = i;
            let mut end = i + 1;
            while let Some((j, d)) = chars.peek() {
                if d.is_ascii_digit() {
                    end = j + 1;
                    chars.next();
                } else {
                    break;
                }
            }
            if let Ok(v) = lower[start..end].parse::<f64>() {
                if (60.0..=200.0).contains(&v) {
                    q.bpm_target = Some(v);
                }
            }
        }
    }

    // Tempo words when no number was given.
    if q.bpm_target.is_none() {
        for (words, bpm) in [
            (["downtempo", "slow", "sluggish", "crawling"].as_slice(), 90.0),
            (["midtempo", "mid-tempo", "walking"].as_slice(), 105.0),
            (["uptempo", "fast", "driving", "banging"].as_slice(), 126.0),
            (["frantic", "breakneck", "double-time"].as_slice(), 160.0),
        ] {
            if words.iter().any(|w| lower.contains(w)) {
                q.bpm_target = Some(bpm);
                break;
            }
        }
    }

    // Energy vocabulary.
    let energy_words: [(&[&str], f64); 4] = [
        (&["ambient", "sparse", "quiet", "gentle", "meditative"], 0.15),
        (&["chill", "mellow", "laid back", "laid-back", "smooth", "warm"], 0.35),
        (&["groovy", "steady", "rolling", "funky"], 0.6),
        (&["peak", "banging", "intense", "hard", "driving", "energetic", "relentless"], 0.9),
    ];
    for (words, e) in energy_words {
        if words.iter().any(|w| lower.contains(w)) {
            q.energy_target = Some(e);
        }
    }

    // Brightness vocabulary.
    if ["dark", "murky", "brooding", "heavy", "sombre", "somber"]
        .iter()
        .any(|w| lower.contains(w))
    {
        q.brightness_target = Some(0.25);
    } else if ["bright", "airy", "shimmering", "sparkling", "crisp", "sunny"]
        .iter()
        .any(|w| lower.contains(w))
    {
        q.brightness_target = Some(0.8);
    }

    if ["minor", "moody", "melancholy", "sad", "wistful", "haunting"]
        .iter()
        .any(|w| lower.contains(w))
    {
        q.prefer_minor = Some(true);
    } else if ["major", "happy", "uplifting", "joyful", "triumphant"]
        .iter()
        .any(|w| lower.contains(w))
    {
        q.prefer_minor = Some(false);
    }

    // Everything else that looks like a content word becomes a keyword to
    // match against genre/title/artist text.
    const STOP: &[&str] = &[
        "something", "a", "an", "the", "with", "that", "like", "this", "and", "or",
        "for", "to", "of", "in", "on", "some", "more", "want", "need", "find", "me",
        "track", "tracks", "song", "songs", "bpm", "around", "about", "but", "not",
        "very", "really", "kind", "sort", "bit", "little", "next", "play", "put",
    ];
    q.keywords = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| w.len() > 2 && !STOP.contains(w) && !w.chars().all(|c| c.is_ascii_digit()))
        .map(|w| w.to_string())
        .collect();
    q.keywords.dedup();
    q.genre_terms = expand_genre_terms(&lower);
    q
}

/// Genre families. Naming any member should reach the whole family — a DJ
/// asking for "reggae" wants the dancehall and dub tracks too, and tags in a
/// real library are never consistent enough to match literally.
const GENRE_FAMILIES: &[(&str, &[&str])] = &[
    ("reggae", &["reggae", "dancehall", "ska", "dub", "roots", "rocksteady", "ragga", "riddim"]),
    ("dancehall", &["dancehall", "reggae", "ragga", "riddim"]),
    ("ska", &["ska", "reggae", "rocksteady"]),
    ("dub", &["dub", "reggae", "roots"]),
    ("jazz", &["jazz", "bossa", "swing", "bebop", "big band", "fusion"]),
    ("bossa", &["bossa", "jazz", "latin", "samba"]),
    ("funk", &["funk", "soul", "disco", "r&b", "rnb", "motown"]),
    ("soul", &["soul", "funk", "r&b", "rnb", "motown"]),
    ("disco", &["disco", "funk", "dance"]),
    ("classical", &["classical", "baroque", "orchestral", "symphony", "concerto", "opera", "chamber"]),
    ("house", &["house", "electronica", "dance", "electronic", "techno", "deep house"]),
    ("techno", &["techno", "electronic", "electronica", "dance"]),
    ("dance", &["dance", "electronica", "electronic", "house", "techno"]),
    ("electronic", &["electronic", "electronica", "dance", "house", "techno", "idm"]),
    ("hiphop", &["hip hop", "hip-hop", "rap", "hiphop"]),
    ("rap", &["rap", "hip hop", "hip-hop"]),
    ("rock", &["rock", "alternative", "punk", "indie", "grunge"]),
    ("punk", &["punk", "alternative", "rock"]),
    ("indie", &["indie", "alternative", "rock"]),
    ("pop", &["pop"]),
    ("blues", &["blues", "r&b", "soul"]),
    ("country", &["country", "folk", "americana", "bluegrass"]),
    ("folk", &["folk", "country", "americana", "singer-songwriter"]),
    ("latin", &["latin", "salsa", "reggaeton", "bossa", "samba", "cumbia"]),
    ("reggaeton", &["reggaeton", "latin", "dancehall"]),
    ("world", &["world", "afrobeat", "african", "global"]),
    ("soundtrack", &["soundtrack", "score", "film"]),
    ("ambient", &["ambient", "downtempo", "chillout", "trip hop", "trip-hop"]),
    ("downtempo", &["downtempo", "trip hop", "trip-hop", "chillout", "ambient"]),
];

/// Phrases whose meaning isn't the sum of their words. "Dancey jazz" is the
/// nu-jazz / lounge aesthetic (Hotel Costes, Buddha-Bar) — expanding it as
/// jazz + dance returns big-band vocals and house, which is exactly wrong.
/// Compilation-series names are included because that aesthetic is scattered
/// across Electronica / World / Lounge / untagged in real libraries, so the
/// album name is a better signal than the genre tag.
const LOUNGE_TERMS: &[&str] = &[
    "lounge", "chillout", "chill out", "downtempo", "nu jazz", "nu-jazz",
    "acid jazz", "trip hop", "trip-hop", "bossa", "buddha-bar", "buddha bar",
    "hotel costes", "costes", "café del mar", "cafe del mar", "nouvelle vague",
    "thievery", "st germain", "st. germain", "gotan", "zero 7", "bonobo",
    "jazzanova", "de-phazz", "dephazz", "koop", "parov stelar",
];

const LOUNGE_PHRASES: &[&str] = &[
    "dancey jazz", "dance jazz", "jazzy house", "jazzy electronic",
    "nu jazz", "nu-jazz", "acid jazz", "lounge", "chillout", "chill out",
    "downtempo", "trip hop", "trip-hop", "cocktail", "chic", "buddha",
    "costes", "del mar", "smoky bar", "cocktail bar",
];

/// Expand any genre words in the text into their family's search terms.
fn expand_genre_terms(lower: &str) -> Vec<String> {
    // A lounge phrase overrides the plain word families it contains, so
    // "dancey jazz" doesn't also drag in bebop and techno.
    if LOUNGE_PHRASES.iter().any(|p| lower.contains(p)) {
        return LOUNGE_TERMS.iter().map(|s| s.to_string()).collect();
    }
    let mut out: Vec<String> = Vec::new();
    for (name, family) in GENRE_FAMILIES {
        // Match "reggae" inside "reggae/dancehall" or "some reggae please".
        if lower.contains(name) {
            for term in *family {
                if !out.iter().any(|t| t == term) {
                    out.push(term.to_string());
                }
            }
        }
    }
    out
}

/// Does this track belong to the requested genre family? Checks the genre tag
/// first, then falls back to album/title text (compilations often carry the
/// style in the album name when the tag is blank).
fn matches_genre(c: &Candidate, terms: &[String]) -> bool {
    if terms.is_empty() {
        return false;
    }
    let genre = c.meta.genre.to_lowercase();
    if terms.iter().any(|t| genre.contains(t.as_str())) {
        return true;
    }
    let text = format!("{} {}", c.meta.album, c.meta.title).to_lowercase();
    terms.iter().any(|t| text.contains(t.as_str()))
}

fn keyword_score(c: &Candidate, keywords: &[String]) -> f64 {
    if keywords.is_empty() {
        return 0.0;
    }
    let hay = format!(
        "{} {} {} {}",
        c.meta.genre, c.meta.artist, c.meta.title, c.meta.album
    )
    .to_lowercase();
    let hits = keywords.iter().filter(|k| hay.contains(k.as_str())).count();
    (hits as f64 / keywords.len() as f64).min(1.0)
}

/// Score one candidate. `reference` is the currently-playing track, if any;
/// `query` is the user's description, if any. At least one should be present.
fn score_candidate(c: &Candidate, reference: Option<&Analysis>, query: Option<&Query>) -> f64 {
    let mut score = 0.0;
    let mut weight = 0.0;

    // When the DJ has typed a request, it is an explicit statement of intent
    // and must outrank the playing track's mixability. Scaling the reference
    // terms down is what stops every query returning the same list.
    let ref_scale = if query.is_some() { 0.45 } else { 1.0 };

    if let Some(r) = reference {
        // Harmony carries the most weight — it's what makes a blend sound
        // intentional, and it's the only axis that works for fluid-tempo
        // classical and rubato jazz.
        score += ref_scale * 0.40 * key_score(&c.analysis.camelot, &r.camelot);
        weight += ref_scale * 0.40;

        // Tempo only matters when both tracks have a trustworthy grid.
        if !c.analysis.fluid && !r.fluid {
            score += ref_scale * 0.30 * tempo_score(c.analysis.bpm, r.bpm);
            weight += ref_scale * 0.30;
        } else if c.analysis.fluid != r.fluid {
            // Exactly one side is free-tempo: a deliberate, hands-on
            // transition rather than a default suggestion. Score it on the
            // same axis so it can't win by simply skipping the tempo test.
            score += ref_scale * 0.30 * 0.45;
            weight += ref_scale * 0.30;
        }

        // Energy continuity: prefer matching or a touch above, so sets build.
        let want = (r.energy + 0.05).min(1.0);
        score += ref_scale * 0.15 * (1.0 - (c.analysis.energy - want).abs()).max(0.0);
        weight += ref_scale * 0.15;
    }

    if let Some(q) = query {
        if let Some(bpm) = q.bpm_target {
            score += 0.25 * tempo_score(c.analysis.bpm, bpm);
            weight += 0.25;
        }
        if let Some(e) = q.energy_target {
            score += 0.25 * (1.0 - (c.analysis.energy - e).abs()).max(0.0);
            weight += 0.25;
        }
        if let Some(b) = q.brightness_target {
            score += 0.15 * (1.0 - (c.analysis.brightness - b).abs()).max(0.0);
            weight += 0.15;
        }
        if let Some(minor) = q.prefer_minor {
            let is_minor = c.analysis.camelot.ends_with('A');
            score += 0.10 * if is_minor == minor { 1.0 } else { 0.0 };
            weight += 0.10;
        }
        // Style match dominates the query side: asking for a genre is a
        // stronger signal than any tempo or energy adjective in the sentence.
        if !q.genre_terms.is_empty() {
            score += 0.60 * if matches_genre(c, &q.genre_terms) { 1.0 } else { 0.0 };
            weight += 0.60;
        }
        let kw = keyword_score(c, &q.keywords);
        if !q.keywords.is_empty() {
            score += 0.30 * kw;
            weight += 0.30;
        }
    }

    if weight <= 0.0 {
        return 0.0;
    }
    let base = score / weight;

    // Taste is a *modifier*, not a scoring axis. Mixability has to stay the
    // dominant term — a loved track in the wrong key is still the wrong
    // record — so these are bounded multipliers rather than added weight.
    let mut mult = 1.0;
    if c.taste.loved {
        mult *= 1.25;
    }
    if c.taste.similar_artist {
        mult *= 1.12;
    }
    // Familiarity, saturating: the difference between never played and played
    // twice is meaningful; between 20 and 40 plays it isn't.
    mult *= 1.0 + 0.10 * (c.taste.play_count as f64 / (c.taste.play_count as f64 + 4.0));
    (base * mult).min(1.0)
}

fn describe(c: &Candidate, reference: Option<&Analysis>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if c.taste.loved {
        parts.push("loved".into());
    }
    if c.taste.similar_artist {
        parts.push("similar artist".into());
    }
    if let Some(r) = reference {
        let k = key_score(&c.analysis.camelot, &r.camelot);
        if k >= 0.99 {
            parts.push(format!("same key ({})", c.analysis.camelot));
        } else if k >= 0.84 {
            parts.push(format!("{} sits next to {} on the wheel", c.analysis.camelot, r.camelot));
        }
        if !c.analysis.fluid && !r.fluid && tempo_score(c.analysis.bpm, r.bpm) > 0.6 {
            parts.push(format!("{:.0} vs {:.0} BPM", c.analysis.bpm, r.bpm));
        }
    }
    if c.analysis.fluid {
        parts.push("fluid tempo — blend on key with a long fade".into());
    }
    if parts.is_empty() {
        parts.push(format!(
            "{} · {}",
            c.analysis.camelot,
            if c.analysis.fluid {
                "free tempo".to_string()
            } else {
                format!("{:.0} BPM", c.analysis.bpm)
            }
        ));
    }
    parts.join(", ")
}

/// Outcome of the genre constraint, so the UI can be honest about it.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RankInfo {
    /// How many library tracks matched the requested style.
    pub genre_matches: usize,
    /// True when a style was requested but nothing in the library matched.
    pub genre_requested_but_empty: bool,
}

/// Distinct artist and album names in the candidate pool — the vocabulary the
/// model chooses from when matching a described feel.
pub fn vocabulary(candidates: &[Candidate]) -> (Vec<String>, Vec<String>) {
    let mut artists: Vec<String> = Vec::new();
    let mut albums: Vec<String> = Vec::new();
    let mut seen_a: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_b: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in candidates {
        let a = c.meta.artist.trim();
        if !a.is_empty() && seen_a.insert(a.to_lowercase()) {
            artists.push(a.to_string());
        }
        let b = c.meta.album.trim();
        if !b.is_empty() && seen_b.insert(b.to_lowercase()) {
            albums.push(b.to_string());
        }
    }
    artists.sort();
    albums.sort();
    (artists, albums)
}

/// Restrict the pool to tracks by the named artists or on the named albums.
/// Matching is case-insensitive and tolerates the model returning a slightly
/// shortened name (e.g. "Hotel Costes" for "Hotel Costes Vol. 11").
pub fn filter_by_names(
    candidates: &[Candidate],
    artists: &[String],
    albums: &[String],
) -> Vec<Candidate> {
    let norm = |s: &str| s.trim().to_lowercase();
    let want_artists: Vec<String> = artists.iter().map(|a| norm(a)).collect();
    let want_albums: Vec<String> = albums.iter().map(|a| norm(a)).collect();
    candidates
        .iter()
        .filter(|c| {
            let artist = norm(&c.meta.artist);
            let album = norm(&c.meta.album);
            want_artists
                .iter()
                .any(|w| !w.is_empty() && (artist == *w || artist.contains(w.as_str())))
                || want_albums
                    .iter()
                    .any(|w| !w.is_empty() && (album == *w || album.contains(w.as_str())))
        })
        .cloned()
        .collect()
}

/// Rank the library. `exclude` holds paths already on a deck.
pub fn rank(
    candidates: &[Candidate],
    reference: Option<&Analysis>,
    query: Option<&Query>,
    exclude: &[String],
    limit: usize,
) -> (Vec<ScoredTrack>, RankInfo) {
    let mut info = RankInfo::default();

    // Banned tracks are removed outright — a ban is an instruction, not a
    // preference to be outweighed by a good key match.
    let pool: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| !c.taste.banned && !exclude.iter().any(|e| e == &c.meta.path))
        .collect();

    // A named genre is a hard constraint, not a preference. Without this, a
    // request for reggae still returns whatever happens to be harmonically
    // near the playing track — which is the whole library, mostly dance.
    let genre_terms = query.map(|q| q.genre_terms.clone()).unwrap_or_default();
    let pool: Vec<&Candidate> = if genre_terms.is_empty() {
        pool
    } else {
        let matching: Vec<&Candidate> = pool
            .iter()
            .copied()
            .filter(|c| matches_genre(c, &genre_terms))
            .collect();
        info.genre_matches = matching.len();
        if matching.is_empty() {
            // Nothing tagged that way — fall back to the full pool rather
            // than returning nothing, and let the caller say so.
            info.genre_requested_but_empty = true;
            pool
        } else {
            matching
        }
    };

    let mut scored: Vec<(f64, &Candidate)> = pool
        .into_iter()
        .map(|c| (score_candidate(c, reference, query), c))
        .filter(|(s, _)| *s > 0.05)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Artist diversity: after the first pick from an artist, later ones are
    // pushed down so a single album can't fill the whole list.
    let mut seen_artists: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    // Libraries carry genuine duplicate files (re-rips, " 1" copies); showing
    // the same song twice wastes a suggestion slot.
    let mut seen_songs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<ScoredTrack> = Vec::with_capacity(limit);
    for (score, c) in scored {
        let song_key = format!(
            "{}|{}",
            c.meta.title.to_lowercase().trim(),
            c.meta.artist.to_lowercase().trim()
        );
        if !seen_songs.insert(song_key) {
            continue;
        }
        let artist_key = c.meta.artist.to_lowercase();
        let n = seen_artists.entry(artist_key).or_insert(0);
        if *n >= 2 {
            continue;
        }
        *n += 1;
        out.push(ScoredTrack {
            path: c.meta.path.clone(),
            title: c.meta.title.clone(),
            artist: c.meta.artist.clone(),
            genre: c.meta.genre.clone(),
            bpm: c.analysis.bpm,
            camelot: c.analysis.camelot.clone(),
            key_name: c.analysis.key_name.clone(),
            fluid: c.analysis.fluid,
            energy: c.analysis.energy,
            duration_secs: c.meta.duration_secs,
            score,
            reason: describe(c, reference),
        });
        if out.len() >= limit {
            break;
        }
    }
    (out, info)
}

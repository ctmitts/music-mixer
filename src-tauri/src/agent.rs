//! Optional Claude layer over the local recommender.
//!
//! The local scorer in `recommend.rs` narrows the library to a shortlist using
//! harmony, tempo, and energy. This module hands that shortlist plus the
//! user's description to Claude, which picks the best few and explains why in
//! DJ terms — judgement the numeric score can't make ("jazzy but danceable",
//! "something that bridges into the funk section").
//!
//! Rust has no official Anthropic SDK, so this speaks the Messages API over
//! raw HTTP. Everything degrades gracefully: no key, no network, or an API
//! error all fall back to the local ranking.

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::recommend::ScoredTrack;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MODEL: &str = "claude-opus-5";

#[derive(Deserialize)]
struct Pick {
    index: usize,
    reason: String,
}

#[derive(Deserialize)]
struct Picks {
    picks: Vec<Pick>,
}

#[derive(Deserialize, Default)]
pub struct VibeSelection {
    /// Artists from the library that fit the requested feel.
    #[serde(default)]
    pub artists: Vec<String>,
    /// Albums from the library that fit (compilation series usually land here).
    #[serde(default)]
    pub albums: Vec<String>,
    /// One line naming the aesthetic, shown to the DJ.
    #[serde(default)]
    pub style_note: String,
}

fn vibe_system_prompt() -> String {
    "You match a DJ's description of a *feel* to artists and albums in their \
own library.\n\n\
This is the step that genre tags cannot do. Tags in a real library are \
inconsistent — the same lounge/nu-jazz aesthetic may be filed under \
Electronica, World, Lounge, Downtempo, or nothing at all, while a \
big-band vocal record is cleanly tagged Jazz. Use what you know about how \
these artists and records actually *sound*, not what they'd be filed under.\n\n\
Worked example: \"dancey jazz, Parisian chic\" means the nu-jazz / lounge / \
downtempo aesthetic — Hotel Costes, Buddha-Bar, Café del Mar, Thievery \
Corporation, St Germain, Gotan Project, Zero 7, Nouvelle Vague. It does NOT \
mean Frank Sinatra or Ella Fitzgerald, even though those are the tracks \
tagged \"Jazz\".\n\n\
Select generously — 10 to 40 artists or albums — so there is room to pick \
good mixes afterwards. Choose only from the supplied lists, copying names \
exactly. If the library genuinely has little matching the feel, return fewer \
rather than padding with poor fits."
        .to_string()
}

fn vibe_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "artists": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Artist names copied exactly from the supplied list."
            },
            "albums": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Album names copied exactly from the supplied list."
            },
            "styleNote": {
                "type": "string",
                "description": "One short line naming the aesthetic being targeted."
            }
        },
        "required": ["artists", "albums", "styleNote"],
        "additionalProperties": false
    })
}

async fn call_messages(api_key: &str, body: Value) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let resp = client
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let payload: Value = resp.json().await?;
    if !status.is_success() {
        let msg = payload
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("Claude API error ({}): {msg}", status.as_u16());
    }
    // A safety classifier can decline the request; content is then empty or
    // partial, so check before reading it.
    if payload.get("stop_reason").and_then(|s| s.as_str()) == Some("refusal") {
        bail!("request was declined by the model's safety classifier");
    }
    payload
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("no text block in response"))
}

/// Step one of the vibe search: given the library's artist and album
/// vocabulary, ask which entries match the requested feel. This is what makes
/// "look-alike music" work — the model knows Hotel Costes and Thievery
/// Corporation belong together regardless of how they happen to be tagged.
pub async fn select_by_vibe(
    api_key: &str,
    description: &str,
    reference: Option<&ScoredTrack>,
    artists: &[String],
    albums: &[String],
    notes: Option<&str>,
    history: &str,
    taste: &str,
) -> Result<VibeSelection> {
    let mut prompt = String::new();
    // The DJ's own notes outrank the model's general knowledge: they describe
    // this specific collection and any corrections the DJ has made.
    if let Some(n) = notes {
        if !n.trim().is_empty() {
            prompt.push_str(
                "THE DJ'S OWN NOTES ON THIS LIBRARY (authoritative — prefer \
                 these over your general knowledge, and honour any exclusions):\n",
            );
            prompt.push_str(n.trim());
            prompt.push_str("\n\n");
        }
    }
    if !taste.is_empty() {
        prompt.push_str(taste);
    }
    if !history.is_empty() {
        prompt.push_str(history);
    }
    if let Some(r) = reference {
        prompt.push_str(&format!(
            "NOW PLAYING: \"{}\" by {}\n\n",
            r.title,
            if r.artist.is_empty() { "unknown artist" } else { &r.artist }
        ));
    }
    prompt.push_str(&format!("THE FEEL THEY WANT: {}\n\n", description.trim()));
    if !history.is_empty() {
        prompt.push_str(
            "A request like \"keep the same feel\" refers to the set above — \
             read the aesthetic off what has actually been played, not just \
             the current track.\n\n",
        );
    }
    prompt.push_str(&format!(
        "ARTISTS IN THEIR LIBRARY ({}):\n{}\n\n",
        artists.len(),
        artists.join(" | ")
    ));
    prompt.push_str(&format!(
        "ALBUMS IN THEIR LIBRARY ({}):\n{}\n\n",
        albums.len(),
        albums.join(" | ")
    ));
    prompt.push_str(
        "Which of these artists and albums match that feel? Copy names exactly.",
    );

    let body = json!({
        "model": MODEL,
        "max_tokens": 8000,
        "system": vibe_system_prompt(),
        "messages": [{ "role": "user", "content": prompt }],
        "output_config": {
            "effort": "medium",
            "format": { "type": "json_schema", "schema": vibe_schema() }
        },
        "fallbacks": "default"
    });

    let text = call_messages(api_key, body).await?;
    Ok(serde_json::from_str(&text)?)
}

fn system_prompt() -> String {
    "You are a DJ's crate-digging assistant for a two-deck mixer. You are given \
the track currently playing (if any), a description of the sound the DJ wants, \
and a shortlist of candidates from their own library. Every candidate has \
already passed a harmonic and tempo compatibility filter.\n\n\
Pick the best matches and explain each in one sentence of practical DJ terms — \
why it works against the current track and the request. Reference concrete \
detail (key relationship, tempo relationship, energy, genre, texture) rather \
than generic praise. Never invent tracks: choose only from the numbered \
candidates by index.\n\n\
Two house rules from this DJ's setup:\n\
- Tracks marked FLUID have unstable tempo (classical, rubato jazz). They \
cannot be beatmatched; recommend them as key-compatible texture blended with a \
long crossfade, and say so.\n\
- Energy runs 0 (ambient) to 1 (peak-time). A good next track usually holds or \
slightly lifts the current energy unless the DJ asked otherwise."
        .to_string()
}

/// Render the set so far, oldest first. Gives the model the *arc* — a request
/// like "keep the same feel" is meaningless without it.
/// Describe what the DJ actually reaches for, from real play counts and
/// explicit loves. This is evidence rather than self-report — it's the part
/// of "their taste" that no description in a text box would capture.
pub fn format_taste(loved: &[String], most_played: &[(String, i64)]) -> String {
    if loved.is_empty() && most_played.is_empty() {
        return String::new();
    }
    let mut s = String::from("WHAT THIS DJ ACTUALLY PLAYS (from their own history):\n");
    if !loved.is_empty() {
        s.push_str("Marked as loved: ");
        s.push_str(&loved.join(", "));
        s.push('\n');
    }
    if !most_played.is_empty() {
        s.push_str("Most-played artists: ");
        let parts: Vec<String> = most_played
            .iter()
            .map(|(a, n)| format!("{a} ({n})"))
            .collect();
        s.push_str(&parts.join(", "));
        s.push('\n');
    }
    s.push_str(
        "Lean toward this when the request is open-ended, but never at the \
         cost of an explicit instruction.\n\n",
    );
    s
}

pub fn format_history(history: &[(String, String, String)]) -> String {
    if history.is_empty() {
        return String::new();
    }
    let mut s = String::from("PLAYED SO FAR THIS SET (oldest first):\n");
    for (title, artist, facts) in history {
        s.push_str(&format!(
            "  - \"{}\"{}{}\n",
            title,
            if artist.is_empty() {
                String::new()
            } else {
                format!(" by {artist}")
            },
            if facts.is_empty() {
                String::new()
            } else {
                format!(" [{facts}]")
            },
        ));
    }
    s.push('\n');
    s
}

fn user_prompt(
    reference: Option<&ScoredTrack>,
    description: &str,
    candidates: &[ScoredTrack],
    want: usize,
    history: &str,
) -> String {
    let mut s = String::new();
    if !history.is_empty() {
        s.push_str(history);
    }
    match reference {
        Some(r) => s.push_str(&format!(
            "NOW PLAYING: \"{}\" by {} — key {} ({}), {}, energy {:.2}{}\n\n",
            r.title,
            if r.artist.is_empty() { "unknown artist" } else { &r.artist },
            r.camelot,
            r.key_name,
            if r.fluid {
                "FLUID tempo".to_string()
            } else {
                format!("{:.0} BPM", r.bpm)
            },
            r.energy,
            if r.genre.is_empty() {
                String::new()
            } else {
                format!(", genre {}", r.genre)
            },
        )),
        None => s.push_str("NOW PLAYING: nothing — this is an opening track.\n\n"),
    }

    if description.trim().is_empty() {
        s.push_str("REQUEST: suggest the strongest next tracks for this mix.\n\n");
    } else {
        s.push_str(&format!("REQUEST: {}\n\n", description.trim()));
    }
    if !history.is_empty() {
        s.push_str(
            "Weigh the direction of the set so far, not just the current \
             track — where it has been going matters as much as where it is.\n\n",
        );
    }

    s.push_str("CANDIDATES:\n");
    for (i, c) in candidates.iter().enumerate() {
        s.push_str(&format!(
            "{}. \"{}\" — {} | {} | {} | energy {:.2}{}\n",
            i,
            c.title,
            if c.artist.is_empty() { "unknown artist" } else { &c.artist },
            c.camelot,
            if c.fluid {
                "FLUID tempo".to_string()
            } else {
                format!("{:.0} BPM", c.bpm)
            },
            c.energy,
            if c.genre.is_empty() {
                String::new()
            } else {
                format!(" | {}", c.genre)
            },
        ));
    }
    s.push_str(&format!(
        "\nChoose the {want} best candidates, ordered best first.",
    ));
    s
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "picks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": {
                            "type": "integer",
                            "description": "Index of the chosen candidate."
                        },
                        "reason": {
                            "type": "string",
                            "description": "One sentence on why this track works next."
                        }
                    },
                    "required": ["index", "reason"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["picks"],
        "additionalProperties": false
    })
}

/// Ask Claude to choose from `candidates`. Returns the picks in Claude's
/// order, with its reasons substituted in.
pub async fn refine(
    api_key: &str,
    reference: Option<&ScoredTrack>,
    description: &str,
    candidates: &[ScoredTrack],
    want: usize,
    history: &str,
) -> Result<Vec<ScoredTrack>> {
    if candidates.is_empty() {
        return Ok(vec![]);
    }
    let body = json!({
        "model": MODEL,
        // Thinking is on by default on Claude Opus 5 and shares this budget
        // with the response, so leave generous headroom.
        "max_tokens": 8000,
        "system": system_prompt(),
        "messages": [{
            "role": "user",
            "content": user_prompt(reference, description, candidates, want, history)
        }],
        "output_config": {
            "effort": "medium",
            "format": { "type": "json_schema", "schema": output_schema() }
        },
        // Route a safety decline to Anthropic's recommended fallback model
        // rather than failing the request.
        "fallbacks": "default"
    });

    let text = call_messages(api_key, body).await?;
    let parsed: Picks = serde_json::from_str(&text)?;
    let mut out = Vec::new();
    for p in parsed.picks {
        if let Some(track) = candidates.get(p.index) {
            let mut t = track.clone();
            t.reason = p.reason;
            out.push(t);
        }
    }
    if out.is_empty() {
        bail!("model returned no usable picks");
    }
    Ok(out)
}

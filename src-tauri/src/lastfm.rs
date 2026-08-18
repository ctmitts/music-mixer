//! Optional Last.fm layer: artist similarity as a recommendation signal, and
//! scrobbling so the play history accumulates somewhere portable.
//!
//! Why Last.fm and not Pandora: Pandora's partner API was discontinued and
//! what remains is a reverse-engineered, ToS-violating endpoint that only
//! serves DRM'd audio — useless here. Last.fm has a documented free API, and
//! its similarity data is *collaborative* ("people who played X played Y"),
//! which complements the local audio analysis instead of duplicating it.
//!
//! Everything degrades gracefully: no key, no session, or a network error
//! leaves the local recommender exactly as it was.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::db::Db;

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
/// Similar-artist lists are stable on the scale of months; a week keeps the
/// cache fresh without ever making a recommendation wait on the network twice.
const SIMILAR_TTL_SECS: i64 = 7 * 24 * 3600;

pub const KEY_SETTING: &str = "lastfm_api_key";
pub const SECRET_SETTING: &str = "lastfm_api_secret";
pub const SESSION_SETTING: &str = "lastfm_session_key";
pub const USER_SETTING: &str = "lastfm_user";

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?)
}

/// Last.fm signs authenticated calls with md5(sorted params ++ secret).
fn sign(params: &[(&str, &str)], secret: &str) -> String {
    let mut sorted: Vec<&(&str, &str)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    format!("{:x}", md5::compute(s))
}

async fn get_json(params: &[(&str, &str)]) -> Result<Value> {
    let resp = client()?.get(API_URL).query(params).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    if let Some(msg) = body.get("message").and_then(|m| m.as_str()) {
        bail!("Last.fm: {msg}");
    }
    if !status.is_success() {
        bail!("Last.fm HTTP {status}");
    }
    Ok(body)
}

async fn post_json(params: &[(&str, &str)]) -> Result<Value> {
    let resp = client()?.post(API_URL).form(params).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    if let Some(msg) = body.get("message").and_then(|m| m.as_str()) {
        bail!("Last.fm: {msg}");
    }
    if !status.is_success() {
        bail!("Last.fm HTTP {status}");
    }
    Ok(body)
}

// -- auth -------------------------------------------------------------------

/// Step 1 of the desktop auth flow: get a request token and the URL the user
/// must visit to approve it.
pub async fn begin_auth(api_key: &str) -> Result<(String, String)> {
    let body = get_json(&[
        ("method", "auth.getToken"),
        ("api_key", api_key),
        ("format", "json"),
    ])
    .await?;
    let token = body
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow!("no token in response"))?
        .to_string();
    let url = format!("https://www.last.fm/api/auth/?api_key={api_key}&token={token}");
    Ok((token, url))
}

/// Step 2, after the user approves in the browser: trade the token for a
/// session key, which does not expire.
pub async fn finish_auth(api_key: &str, secret: &str, token: &str) -> Result<(String, String)> {
    let sig = sign(
        &[
            ("method", "auth.getSession"),
            ("api_key", api_key),
            ("token", token),
        ],
        secret,
    );
    let body = get_json(&[
        ("method", "auth.getSession"),
        ("api_key", api_key),
        ("token", token),
        ("api_sig", &sig),
        ("format", "json"),
    ])
    .await?;
    let session = body
        .get("session")
        .ok_or_else(|| anyhow!("no session in response"))?;
    let key = session
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or_else(|| anyhow!("no session key"))?
        .to_string();
    let user = session
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((key, user))
}

// -- similarity -------------------------------------------------------------

/// Similar artists by name, most similar first, from cache when possible.
/// Returns an empty list rather than an error when Last.fm isn't configured —
/// this is an optional signal, never a hard dependency.
pub async fn similar_artists(db: &Db, api_key: &str, artist: &str) -> Vec<String> {
    let artist = artist.trim();
    if artist.is_empty() || api_key.is_empty() {
        return Vec::new();
    }
    if let Some(json) = db.get_similar_artists(artist, SIMILAR_TTL_SECS) {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(&json) {
            return v;
        }
    }
    let body = match get_json(&[
        ("method", "artist.getSimilar"),
        ("artist", artist),
        ("api_key", api_key),
        ("autocorrect", "1"),
        ("limit", "40"),
        ("format", "json"),
    ])
    .await
    {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let names: Vec<String> = body
        .get("similarartists")
        .and_then(|s| s.get("artist"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    // Cache negative results too: a misspelled or obscure artist shouldn't be
    // retried on every single recommendation.
    if let Ok(json) = serde_json::to_string(&names) {
        db.put_similar_artists(artist, &json);
    }
    names
}

// -- scrobbling -------------------------------------------------------------

async fn scrobble_one(
    api_key: &str,
    secret: &str,
    session: &str,
    artist: &str,
    track: &str,
    timestamp: i64,
) -> Result<()> {
    let ts = timestamp.to_string();
    let mut params = vec![
        ("method", "track.scrobble"),
        ("artist", artist),
        ("track", track),
        ("timestamp", ts.as_str()),
        ("api_key", api_key),
        ("sk", session),
    ];
    let sig = sign(&params, secret);
    params.push(("api_sig", &sig));
    params.push(("format", "json"));
    post_json(&params).await?;
    Ok(())
}

/// Send any qualifying plays that haven't been scrobbled yet. Called
/// periodically; a failure just leaves rows pending for the next pass.
pub async fn flush_scrobbles(db: &Db) {
    let (Some(api_key), Some(secret), Some(session)) = (
        db.get_setting(KEY_SETTING),
        db.get_setting(SECRET_SETTING),
        db.get_setting(SESSION_SETTING),
    ) else {
        return;
    };
    if api_key.is_empty() || secret.is_empty() || session.is_empty() {
        return;
    }
    for (id, path, started_at) in db.pending_scrobbles(20) {
        let meta = crate::library::track_meta(&path);
        if meta.artist.trim().is_empty() || meta.title.trim().is_empty() {
            // Nothing useful to send; mark it done so it stops being retried.
            db.mark_scrobbled(id);
            continue;
        }
        match scrobble_one(
            &api_key,
            &secret,
            &session,
            &meta.artist,
            &meta.title,
            started_at,
        )
        .await
        {
            Ok(()) => db.mark_scrobbled(id),
            // Network or auth trouble: stop this pass and retry later rather
            // than hammering the API for every pending row.
            Err(_) => return,
        }
    }
}

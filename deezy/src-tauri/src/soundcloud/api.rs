use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use url::Url;

use super::{track_key, validate_id};
use crate::audio::read_capped;

const API: &str = "https://api-v2.soundcloud.com";
const HOME_PAGE: &str = "https://soundcloud.com/";
const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SCRIPT_BYTES: usize = 8 * 1024 * 1024;
const SEARCH_LIMIT: &str = "20";
/// `/tracks?ids=` accepts at most 50 IDs per request.
const TRACK_BATCH: usize = 50;
const MAX_USER_TRACKS: usize = 500;
const MAX_USER_SETS: usize = 120;
const MAX_PAGES: usize = 50;

static SCRIPT_SRC: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"src="(https://a-v2\.sndcdn\.com/assets/[^"]+\.js)""#)
        .expect("script pattern is valid")
});
static CLIENT_ID_PATTERN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"client_id\s*[:=]\s*"([0-9A-Za-z]{32})""#).expect("client_id pattern is valid")
});

/// The anonymous web `client_id`, found on first use and replaced when
/// SoundCloud rejects it. Held while a new one is fetched, so concurrent
/// requests share one lookup.
static CLIENT_ID: tokio::sync::Mutex<Option<String>> = tokio::sync::Mutex::const_new(None);

/// HTTPS-only client for the API, the web page, artwork and audio, with the
/// same timeouts as the Spotify client. Audio is fetched segment by segment
/// (or with per-chunk reads), so the total timeout is not applied to it.
pub(crate) fn http() -> Result<reqwest::Client, String> {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = HTTP.get() {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("Wavekeep/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;
    Ok(HTTP.get_or_init(|| client).clone())
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundcloudTrack {
    /// `soundcloud:<id>` so the download queue can route it without a flag.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album: String,
    /// Seconds, matching the Deezer `Track` shape.
    pub duration: u64,
    pub cover_small: String,
    pub cover_medium: String,
    pub preview: Option<String>,
    pub release_date: Option<String>,
    pub isrc: Option<String>,
    pub provider: String,
    /// 500×500 artwork for the embedded cover.
    #[serde(skip)]
    pub cover_large: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundcloudAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub cover_small: String,
    pub cover_medium: String,
    pub nb_tracks: u64,
    pub release_date: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundcloudArtist {
    pub id: String,
    pub name: String,
    pub picture_small: String,
    pub picture_medium: String,
    pub nb_album: u64,
    pub nb_fan: u64,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundcloudPlaylist {
    pub id: String,
    pub title: String,
    pub creator: String,
    pub cover_small: String,
    pub cover_medium: String,
    pub nb_tracks: u64,
    pub provider: String,
}

/// What a soundcloud.com link points at.
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

enum Failure {
    /// 401: the `client_id` is no longer accepted.
    Unauthorized,
    Other(String),
}

/// Returns the cached `client_id`, or fetches a new one when there is none or
/// when `stale` (the ID a request was just refused with) is still cached.
async fn client_id(stale: Option<&str>) -> Result<String, String> {
    if let Some(id) = crate::settings::app_env("SOUNDCLOUD_CLIENT_ID") {
        let id = id.trim().to_string();
        if id.len() == 32 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Ok(id);
        }
        crate::logging::warn("soundcloud", "WAVEKEEP_SOUNDCLOUD_CLIENT_ID is not 32 letters and digits; ignoring it");
    }
    let mut cached = CLIENT_ID.lock().await;
    if let Some(id) = cached.as_deref() {
        if stale != Some(id) {
            return Ok(id.to_string());
        }
    }
    let id = discover_client_id().await.map_err(|e| {
        crate::logging::warn("soundcloud", format!("Client ID lookup failed: {}", e));
        e
    })?;
    crate::logging::info("soundcloud", "Client ID fetched from soundcloud.com");
    *cached = Some(id.clone());
    Ok(id)
}

/// Reads the `client_id` the SoundCloud web player uses from its scripts. It
/// sits in one of the last bundles, so those are tried first.
async fn discover_client_id() -> Result<String, String> {
    let http = http()?;
    let page = get_text(&http, HOME_PAGE, MAX_PAGE_BYTES, "SoundCloud page").await?;
    let scripts: Vec<&str> = SCRIPT_SRC
        .captures_iter(&page)
        .filter_map(|capture| capture.get(1).map(|m| m.as_str()))
        .collect();
    for script in scripts.iter().rev() {
        let Ok(body) = get_text(&http, script, MAX_SCRIPT_BYTES, "SoundCloud script").await else {
            continue;
        };
        if let Some(id) = find_client_id(&body) {
            return Ok(id);
        }
    }
    Err("Could not connect to SoundCloud: no client ID found on soundcloud.com".to_string())
}

fn find_client_id(script: &str) -> Option<String> {
    CLIENT_ID_PATTERN
        .captures(script)
        .and_then(|capture| capture.get(1))
        .map(|m| m.as_str().to_string())
}

async fn get_text(http: &reqwest::Client, url: &str, max_bytes: usize, label: &str) -> Result<String, String> {
    let response = http
        .get(url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("{} request failed: {}", label, e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("{} request failed ({})", label, response.status()));
    }
    let body = read_capped(response, max_bytes, label).await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// GET on `api-v2` with the `client_id` added. A refused `client_id` is
/// replaced once and the request repeated.
pub(crate) async fn api_url(url: Url) -> Result<Value, String> {
    let http = http()?;
    let mut stale: Option<String> = None;
    for _ in 0..2 {
        let id = client_id(stale.as_deref()).await?;
        let mut request_url = url.clone();
        request_url.query_pairs_mut().append_pair("client_id", &id);
        match get_json(&http, request_url).await {
            Ok(value) => return Ok(value),
            Err(Failure::Unauthorized) => {
                crate::logging::warn("soundcloud", "Client ID refused; fetching a new one");
                stale = Some(id);
            }
            Err(Failure::Other(e)) => return Err(e),
        }
    }
    Err("SoundCloud refused the request. Try again later.".to_string())
}

pub(crate) async fn api_get(path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let mut url = Url::parse(&format!("{}{}", API, path)).map_err(|_| "Invalid SoundCloud request".to_string())?;
    if !params.is_empty() {
        url.query_pairs_mut().extend_pairs(params);
    }
    api_url(url).await
}

async fn get_json(http: &reqwest::Client, url: Url) -> Result<Value, Failure> {
    // Errors never include the URL: it carries the client_id and search text.
    let response = http
        .get(url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| Failure::Other(format!("SoundCloud request failed: {}", e.without_url())))?;
    let status = response.status();
    match status.as_u16() {
        401 => return Err(Failure::Unauthorized),
        // A refused ID is a 401; a 403 is about the item, so no new ID is fetched for it.
        403 => return Err(Failure::Other("SoundCloud refused access to this item. It may be private or restricted.".to_string())),
        404 => return Err(Failure::Other("SoundCloud could not find this item. It may be private or removed.".to_string())),
        429 => {
            crate::logging::warn("soundcloud", "Rate limited (429)");
            return Err(Failure::Other("SoundCloud rate limit reached. Try again in a few minutes.".to_string()));
        }
        _ if !status.is_success() => {
            return Err(Failure::Other(format!("SoundCloud request failed ({})", status)));
        }
        _ => {}
    }
    let body = read_capped(response, MAX_JSON_BYTES, "SoundCloud response")
        .await
        .map_err(Failure::Other)?;
    serde_json::from_slice(&body).map_err(|e| Failure::Other(format!("SoundCloud sent an invalid response: {}", e)))
}

/// Follows `next_href` links (only on `api-v2`) until `max_items` are read.
async fn collect_pages(path: &str, params: &[(&str, &str)], max_items: usize) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut page = api_get(path, params).await?;
    for _ in 0..MAX_PAGES {
        if let Some(collection) = page["collection"].as_array() {
            items.extend(collection.iter().cloned());
        }
        if items.len() >= max_items {
            items.truncate(max_items);
            break;
        }
        let Some(next) = page["next_href"].as_str().and_then(next_page_url) else {
            break;
        };
        page = api_url(next).await?;
    }
    Ok(items)
}

/// A `next_href` on `api-v2`, without any `client_id` it carried (ours is
/// added per request). Anything else is not followed.
fn next_page_url(href: &str) -> Option<Url> {
    let url = Url::parse(href).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("api-v2.soundcloud.com") {
        crate::logging::warn("soundcloud", "Refused a pagination link outside api-v2.soundcloud.com");
        return None;
    }
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "client_id")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let mut clean = url;
    clean.set_query(None);
    if !pairs.is_empty() {
        clean.query_pairs_mut().extend_pairs(pairs);
    }
    Some(clean)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// SoundCloud artwork URLs end in `-large.jpg` (100×100); other sizes use the
/// same URL with another suffix. Only `https` images are passed on.
fn artwork(url: Option<&str>, size: &str) -> String {
    match url {
        Some(url) if url.starts_with("https://") => url.replacen("-large.", &format!("-{}.", size), 1),
        _ => String::new(),
    }
}

/// `YYYY-MM-DD` from an ISO timestamp.
fn date(value: &Value) -> Option<String> {
    let date: String = text(value)?.chars().take(10).collect();
    (date.len() == 10 && date.as_bytes()[4] == b'-').then_some(date)
}

fn id_string(value: &Value) -> String {
    value.as_u64().map(|id| id.to_string()).unwrap_or_default()
}

/// Maps a full track object; stubs without a title (unfilled set entries)
/// give `None`.
pub(crate) fn track_from(value: &Value) -> Option<SoundcloudTrack> {
    let id = value["id"].as_u64()?;
    let title = text(&value["title"])?.to_string();
    let user = &value["user"];
    let publisher = &value["publisher_metadata"];
    let artist = text(&publisher["artist"])
        .or_else(|| text(&user["username"]))
        .unwrap_or("Unknown")
        .to_string();
    let art = text(&value["artwork_url"]).or_else(|| text(&user["avatar_url"]));
    let millis = value["full_duration"]
        .as_u64()
        .filter(|ms| *ms > 0)
        .or_else(|| value["duration"].as_u64())
        .unwrap_or(0);
    Some(SoundcloudTrack {
        id: track_key(id),
        title,
        artist,
        artist_id: id_string(&user["id"]),
        album: text(&publisher["album_title"]).unwrap_or_default().to_string(),
        duration: millis / 1000,
        cover_small: artwork(art, "large"),
        cover_medium: artwork(art, "t300x300"),
        preview: None,
        release_date: date(&value["release_date"])
            .or_else(|| date(&value["display_date"]))
            .or_else(|| date(&value["created_at"])),
        isrc: text(&publisher["isrc"]).map(str::to_string),
        provider: "soundcloud".to_string(),
        cover_large: artwork(art, "t500x500"),
    })
}

fn set_artwork(value: &Value) -> Option<&str> {
    text(&value["artwork_url"])
        .or_else(|| value["tracks"].as_array()?.iter().find_map(|t| text(&t["artwork_url"])))
        .or_else(|| text(&value["user"]["avatar_url"]))
}

fn album_from(value: &Value) -> Option<SoundcloudAlbum> {
    let id = value["id"].as_u64()?;
    let art = set_artwork(value);
    Some(SoundcloudAlbum {
        id: id.to_string(),
        title: text(&value["title"]).unwrap_or("Untitled").to_string(),
        artist: text(&value["user"]["username"]).unwrap_or_default().to_string(),
        artist_id: id_string(&value["user"]["id"]),
        cover_small: artwork(art, "large"),
        cover_medium: artwork(art, "t300x300"),
        nb_tracks: value["track_count"].as_u64().unwrap_or(0),
        release_date: date(&value["release_date"]).or_else(|| date(&value["published_at"])),
        provider: "soundcloud".to_string(),
    })
}

fn playlist_from(value: &Value) -> Option<SoundcloudPlaylist> {
    let album = album_from(value)?;
    Some(SoundcloudPlaylist {
        id: album.id,
        title: album.title,
        creator: album.artist,
        cover_small: album.cover_small,
        cover_medium: album.cover_medium,
        nb_tracks: album.nb_tracks,
        provider: album.provider,
    })
}

fn artist_from(value: &Value) -> Option<SoundcloudArtist> {
    let id = value["id"].as_u64()?;
    let picture = text(&value["avatar_url"]);
    Some(SoundcloudArtist {
        id: id.to_string(),
        name: text(&value["username"]).unwrap_or("Unknown").to_string(),
        picture_small: artwork(picture, "large"),
        picture_medium: artwork(picture, "t300x300"),
        nb_album: value["playlist_count"].as_u64().unwrap_or(0),
        nb_fan: value["followers_count"].as_u64().unwrap_or(0),
        provider: "soundcloud".to_string(),
    })
}

fn collection(value: &Value) -> impl Iterator<Item = &Value> {
    value["collection"].as_array().into_iter().flatten()
}

pub async fn search_tracks(query: &str) -> Result<Vec<SoundcloudTrack>, String> {
    let page = api_get("/search/tracks", &[("q", query), ("limit", SEARCH_LIMIT)]).await?;
    Ok(collection(&page).filter_map(track_from).collect())
}

pub async fn search_albums(query: &str) -> Result<Vec<SoundcloudAlbum>, String> {
    let page = api_get("/search/albums", &[("q", query), ("limit", SEARCH_LIMIT)]).await?;
    Ok(collection(&page).filter_map(album_from).collect())
}

pub async fn search_artists(query: &str) -> Result<Vec<SoundcloudArtist>, String> {
    let page = api_get("/search/users", &[("q", query), ("limit", SEARCH_LIMIT)]).await?;
    Ok(collection(&page).filter_map(artist_from).collect())
}

pub async fn search_playlists(query: &str) -> Result<Vec<SoundcloudPlaylist>, String> {
    let page = api_get("/search/playlists_without_albums", &[("q", query), ("limit", SEARCH_LIMIT)]).await?;
    Ok(collection(&page).filter_map(playlist_from).collect())
}

/// The full track object (with `media` and `track_authorization`) and its
/// catalog form.
pub async fn track(id: &str) -> Result<(SoundcloudTrack, Value), String> {
    let id = validate_id(id, "track")?;
    let value = api_get(&format!("/tracks/{}", id), &[]).await?;
    let track = track_from(&value).ok_or("SoundCloud sent an incomplete track")?;
    Ok((track, value))
}

/// Tracks of an album or playlist, in order. A set lists only its first few
/// tracks in full; the rest are fetched by ID, and tracks SoundCloud no longer
/// returns (removed or private) are left out.
pub async fn set_tracks(id: &str) -> Result<Vec<SoundcloudTrack>, String> {
    let id = validate_id(id, "playlist")?;
    let set = api_get(&format!("/playlists/{}", id), &[]).await?;
    let entries = set["tracks"].as_array().cloned().unwrap_or_default();
    let mut full: HashMap<u64, SoundcloudTrack> = HashMap::new();
    let mut missing = Vec::new();
    for entry in &entries {
        match (entry["id"].as_u64(), track_from(entry)) {
            (Some(track_id), Some(track)) => {
                full.insert(track_id, track);
            }
            (Some(track_id), None) => missing.push(track_id),
            _ => {}
        }
    }
    for batch in missing.chunks(TRACK_BATCH) {
        let ids = batch.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let found = api_get("/tracks", &[("ids", ids.as_str())]).await?;
        for value in found.as_array().into_iter().flatten() {
            if let (Some(track_id), Some(track)) = (value["id"].as_u64(), track_from(value)) {
                full.insert(track_id, track);
            }
        }
    }
    let tracks: Vec<SoundcloudTrack> = entries
        .iter()
        .filter_map(|entry| full.remove(&entry["id"].as_u64()?))
        .collect();
    if tracks.is_empty() && !entries.is_empty() {
        return Err("SoundCloud returned none of this set's tracks".to_string());
    }
    Ok(tracks)
}

/// A user's own uploads, newest first.
pub async fn user_tracks(id: &str) -> Result<Vec<SoundcloudTrack>, String> {
    let id = validate_id(id, "user")?;
    let items = collect_pages(&format!("/users/{}/tracks", id), &[("limit", "50")], MAX_USER_TRACKS).await?;
    Ok(items.iter().filter_map(track_from).collect())
}

/// A user's albums followed by their other playlists.
pub async fn user_sets(id: &str) -> Result<Vec<SoundcloudAlbum>, String> {
    let id = validate_id(id, "user")?;
    let mut sets = collect_pages(&format!("/users/{}/albums", id), &[("limit", "50")], MAX_USER_SETS).await?;
    sets.extend(
        collect_pages(&format!("/users/{}/playlists_without_albums", id), &[("limit", "50")], MAX_USER_SETS).await?,
    );
    Ok(sets.iter().filter_map(album_from).collect())
}

/// What a soundcloud.com URL points at: a track, album, playlist or artist.
pub async fn resolve(url: &str) -> Result<Resolved, String> {
    let value = api_get("/resolve", &[("url", url)]).await?;
    let id = value["id"].as_u64().ok_or("SoundCloud could not resolve this link")?;
    let kind = match value["kind"].as_str() {
        Some("track") => "track",
        Some("playlist") if value["is_album"].as_bool() == Some(true) => "album",
        Some("playlist") => "playlist",
        Some("user") => "artist",
        _ => return Err("This SoundCloud link is not a track, set or profile".to_string()),
    };
    Ok(Resolved {
        kind: kind.to_string(),
        id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_client_id_in_a_script() {
        let script = r#"a=1,{client_id:"BzENOQHynNEEqrW7hjR3mefOi7JYXB3s",env:"production"}"#;
        assert_eq!(find_client_id(script).as_deref(), Some("BzENOQHynNEEqrW7hjR3mefOi7JYXB3s"));
        assert_eq!(find_client_id(r#"client_id="short""#), None);
    }

    #[test]
    fn maps_tracks_with_publisher_artist_and_sized_artwork() {
        let track = track_from(&json!({
            "id": 242136202,
            "title": "Never Be Like You feat. Kai",
            "duration": 30000,
            "full_duration": 235019,
            "artwork_url": "https://i1.sndcdn.com/artworks-000143264206-mtz96c-large.jpg",
            "release_date": "2016-01-12T00:00:00Z",
            "display_date": "2016-01-15T19:58:21Z",
            "user": { "id": 2976616, "username": "Flume", "avatar_url": "https://i1.sndcdn.com/avatars-x-large.jpg" },
            "publisher_metadata": { "artist": "Seekae", "isrc": "AUFF01400535", "album_title": "" }
        }))
        .unwrap();
        assert_eq!(track.id, "soundcloud:242136202");
        assert_eq!(track.artist, "Seekae");
        assert_eq!(track.artist_id, "2976616");
        assert_eq!(track.album, "");
        // full_duration, not the 30 s snippet length.
        assert_eq!(track.duration, 235);
        assert_eq!(track.cover_medium, "https://i1.sndcdn.com/artworks-000143264206-mtz96c-t300x300.jpg");
        assert_eq!(track.cover_large, "https://i1.sndcdn.com/artworks-000143264206-mtz96c-t500x500.jpg");
        assert_eq!(track.release_date.as_deref(), Some("2016-01-12"));
        assert_eq!(track.isrc.as_deref(), Some("AUFF01400535"));
    }

    #[test]
    fn falls_back_to_the_uploader_and_avatar() {
        let track = track_from(&json!({
            "id": 1, "title": "x", "duration": 61000, "artwork_url": null,
            "display_date": "2020-05-01T10:00:00Z",
            "user": { "id": 2, "username": "Uploader", "avatar_url": "https://i1.sndcdn.com/avatars-y-large.jpg" },
            "publisher_metadata": { "artist": "  " }
        }))
        .unwrap();
        assert_eq!(track.artist, "Uploader");
        assert_eq!(track.duration, 61);
        assert_eq!(track.cover_small, "https://i1.sndcdn.com/avatars-y-large.jpg");
        assert_eq!(track.release_date.as_deref(), Some("2020-05-01"));
    }

    #[test]
    fn set_stubs_are_not_tracks() {
        assert!(track_from(&json!({ "id": 5, "kind": "track" })).is_none());
    }

    #[test]
    fn refuses_non_https_artwork() {
        assert_eq!(artwork(Some("http://i1.sndcdn.com/a-large.jpg"), "t300x300"), "");
        assert_eq!(artwork(None, "t300x300"), "");
    }

    #[test]
    fn follows_only_api_v2_pages_and_drops_their_client_id() {
        let next = next_page_url("https://api-v2.soundcloud.com/users/1/tracks?offset=50&limit=50&client_id=old").unwrap();
        assert_eq!(next.as_str(), "https://api-v2.soundcloud.com/users/1/tracks?offset=50&limit=50");
        assert!(next_page_url("https://evil.example/users/1/tracks?offset=50").is_none());
        assert!(next_page_url("http://api-v2.soundcloud.com/users/1/tracks").is_none());
    }
}

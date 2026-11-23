use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use url::Url;

use super::process;
use super::{bare_track_id, channel_tab_url, channel_url, parse_link, playlist_url, video_id, video_url, Link};

const SEARCH_LIMIT: usize = 20;
const ARTIST_SEARCH_LIMIT: usize = 30;
const MAX_ARTISTS: usize = 10;
const MAX_ITEMS: usize = 1000;
const MAX_CHANNEL_SETS: usize = 120;
pub const PLAYLIST_ALBUM_PREFIX: &str = "playlist:";

#[derive(Debug, Clone, Serialize)]
pub struct YoutubeTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album: String,
    pub duration: u64,
    pub cover_small: String,
    pub cover_medium: String,
    pub preview: Option<String>,
    pub release_date: Option<String>,
    pub isrc: Option<String>,
    pub provider: String,
    #[serde(skip)]
    pub cover_large: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct YoutubePlaylist {
    pub id: String,
    pub title: String,
    pub creator: String,
    pub cover_small: String,
    pub cover_medium: String,
    pub nb_tracks: u64,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct YoutubeArtist {
    pub id: String,
    pub name: String,
    pub picture_small: String,
    pub picture_medium: String,
    pub nb_album: u64,
    pub nb_fan: u64,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct YoutubeAlbum {
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
pub struct Resolved {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

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

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().map(str::trim).filter(|text| !text.is_empty())
}

fn first_text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| text(value, key))
}

fn validate_query(query: &str) -> Result<&str, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 300 {
        return Err("YouTube search must contain between 1 and 300 characters".to_string());
    }
    Ok(query)
}

fn date(value: &Value) -> Option<String> {
    let date = first_text(value, &["upload_date", "release_date"])?;
    if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..8]))
    } else {
        None
    }
}

fn valid_image(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    (host == "i.ytimg.com"
        || host.ends_with(".ytimg.com")
        || host.ends_with(".googleusercontent.com")
        || host.ends_with(".ggpht.com"))
        .then_some(url.to_string())
}

fn thumbnail(value: &Value, target: u32) -> String {
    let Some(images) = value.get("thumbnails").and_then(Value::as_array) else {
        return String::new();
    };
    let mut candidates: Vec<(u32, String)> = images
        .iter()
        .filter_map(|image| {
            let url = valid_image(text(image, "url")?)?;
            let width = image.get("width").and_then(Value::as_u64).unwrap_or(480) as u32;
            Some((width.abs_diff(target), url))
        })
        .collect();
    candidates.sort_by_key(|(distance, _)| *distance);
    candidates.into_iter().next().map(|(_, url)| url).unwrap_or_default()
}

fn video_art(id: &str) -> (String, String, String) {
    (
        format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", id),
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id),
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id),
    )
}

fn artist_id(value: &Value, parent: &Value) -> String {
    first_text(value, &["channel_id", "uploader_id", "channel_url", "uploader_url"])
        .or_else(|| first_text(parent, &["channel_id", "uploader_id", "channel_url", "uploader_url"]))
        .unwrap_or_default()
        .to_string()
}

fn artist_name(value: &Value, parent: &Value) -> String {
    first_text(value, &["channel", "uploader"])
        .or_else(|| first_text(parent, &["channel", "uploader"]))
        .unwrap_or("Unknown artist")
        .to_string()
}

pub(crate) fn track_from_value(value: &Value, parent: &Value) -> Option<YoutubeTrack> {
    let id = first_text(value, &["id", "display_id"]).and_then(|id| video_id(id).ok())?;
    let title = text(value, "title")?.to_string();
    let (cover_small, cover_medium, cover_large) = video_art(id);
    Some(YoutubeTrack {
        id: super::track_key(id).ok()?,
        title,
        artist: artist_name(value, parent),
        artist_id: artist_id(value, parent),
        album: text(value, "album").unwrap_or("YouTube").to_string(),
        duration: value.get("duration").and_then(Value::as_u64).unwrap_or(0),
        cover_small,
        cover_medium,
        preview: None,
        release_date: date(value),
        isrc: None,
        provider: "youtube".to_string(),
        cover_large,
    })
}

fn entries(value: &Value) -> &[Value] {
    value.get("entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// The playlist ID of a flat playlist-search or channel-playlists entry.
/// Those lists also hold channel cards (`/channel/UC...`), which are skipped.
fn playlist_entry_id(entry: &Value) -> Option<String> {
    match parse_link(text(entry, "url")?)? {
        Link::Playlist(id) => Some(id),
        _ => None,
    }
}

fn tracks(value: &Value) -> Vec<YoutubeTrack> {
    entries(value).iter().filter_map(|entry| track_from_value(entry, value)).collect()
}

async fn search_value(query: &str, limit: usize) -> Result<Value, String> {
    let query = validate_query(query)?;
    process::ytdl_json(&format!("ytsearch{}:{}", limit, query), true, limit, false, None)
        .await?
        .ok_or_else(|| "YouTube search was cancelled".to_string())
}

pub async fn search_tracks(query: &str) -> Result<Vec<YoutubeTrack>, String> {
    Ok(tracks(&search_value(query, SEARCH_LIMIT).await?))
}

pub async fn search_playlists(query: &str) -> Result<Vec<YoutubePlaylist>, String> {
    let query = validate_query(query)?;
    let mut url = Url::parse("https://www.youtube.com/results").map_err(|_| "Invalid YouTube search URL".to_string())?;
    url.query_pairs_mut().append_pair("search_query", query).append_pair("sp", "EgIQAw==");
    let value = process::ytdl_json(url.as_str(), true, SEARCH_LIMIT, false, None)
        .await?
        .ok_or_else(|| "YouTube search was cancelled".to_string())?;
    Ok(entries(&value)
        .iter()
        .filter_map(|entry| {
            let id = playlist_entry_id(entry)?;
            let cover = thumbnail(entry, 480);
            Some(YoutubePlaylist {
                id: id.to_string(),
                title: text(entry, "title")?.to_string(),
                creator: artist_name(entry, &value),
                cover_small: thumbnail(entry, 180),
                cover_medium: cover,
                nb_tracks: entry.get("playlist_count").and_then(Value::as_u64).unwrap_or(0),
                provider: "youtube".to_string(),
            })
        })
        .collect())
}

pub async fn search_artists(query: &str) -> Result<Vec<YoutubeArtist>, String> {
    // yt-dlp's channel-search filter is not supported by every extractor
    // version. Video search exposes the same channels without another endpoint.
    let value = search_value(query, ARTIST_SEARCH_LIMIT).await?;
    // Keeps the search's relevance order.
    let mut artists: Vec<YoutubeArtist> = Vec::new();
    for entry in entries(&value) {
        let id = artist_id(entry, &value);
        let name = artist_name(entry, &value);
        if id.is_empty() || name == "Unknown artist" || artists.iter().any(|artist| artist.id == id) {
            continue;
        }
        artists.push(YoutubeArtist {
            id,
            name,
            picture_small: String::new(),
            picture_medium: String::new(),
            nb_album: 0,
            nb_fan: entry.get("channel_follower_count").and_then(Value::as_u64).unwrap_or(0),
            provider: "youtube".to_string(),
        });
        if artists.len() >= MAX_ARTISTS {
            break;
        }
    }
    Ok(artists)
}

pub async fn track(id: &str) -> Result<YoutubeTrack, String> {
    let url = video_url(bare_track_id(id))?;
    let value = process::ytdl_json(&url, false, 1, true, None)
        .await?
        .ok_or_else(|| "YouTube metadata lookup was cancelled".to_string())?;
    track_from_value(&value, &Value::Null).ok_or_else(|| "YouTube did not return this video".to_string())
}

pub(crate) async fn download_metadata(id: &str, cancel_flag: &AtomicBool) -> Result<Option<Value>, String> {
    let url = video_url(bare_track_id(id))?;
    process::ytdl_json(&url, false, 1, true, Some(cancel_flag)).await
}

pub async fn playlist_tracks(id: &str) -> Result<Vec<YoutubeTrack>, String> {
    let value = process::ytdl_json(&playlist_url(id)?, true, MAX_ITEMS, false, None)
        .await?
        .ok_or_else(|| "YouTube playlist lookup was cancelled".to_string())?;
    Ok(tracks(&value))
}

pub async fn album_tracks(id: &str) -> Result<Vec<YoutubeTrack>, String> {
    if let Some(channel) = id.strip_prefix(super::USER_TRACKS_PREFIX) {
        let value = process::ytdl_json(&channel_tab_url(channel, "videos")?, true, MAX_ITEMS, false, None)
            .await?
            .ok_or_else(|| "YouTube channel lookup was cancelled".to_string())?;
        return Ok(tracks(&value));
    }
    if let Some(playlist) = id.strip_prefix(PLAYLIST_ALBUM_PREFIX) {
        return playlist_tracks(playlist).await;
    }
    playlist_tracks(id).await
}

pub async fn artist_albums(id: &str) -> Result<Vec<YoutubeAlbum>, String> {
    let value = process::ytdl_json(&channel_tab_url(id, "playlists")?, true, MAX_CHANNEL_SETS, false, None)
        .await?
        .ok_or_else(|| "YouTube channel lookup was cancelled".to_string())?;
    Ok(entries(&value)
        .iter()
        .filter_map(|entry| {
            let id = playlist_entry_id(entry)?;
            Some(YoutubeAlbum {
                id: format!("{}{}", PLAYLIST_ALBUM_PREFIX, id),
                title: text(entry, "title")?.to_string(),
                artist: artist_name(entry, &value),
                artist_id: artist_id(entry, &value),
                cover_small: thumbnail(entry, 180),
                cover_medium: thumbnail(entry, 480),
                nb_tracks: entry.get("playlist_count").and_then(Value::as_u64).unwrap_or(0),
                release_date: None,
                provider: "youtube".to_string(),
            })
        })
        .collect())
}

pub fn resolve(url: &str) -> Result<Resolved, String> {
    match parse_link(url).ok_or("Invalid YouTube URL")? {
        Link::Track(id) => Ok(Resolved { kind: "track".to_string(), id }),
        Link::Playlist(id) => Ok(Resolved { kind: "playlist".to_string(), id }),
        Link::Artist(id) => {
            // Also validates legacy `/c/` and `/user/` channel targets.
            channel_url(&id)?;
            Ok(Resolved { kind: "artist".to_string(), id })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_video_metadata_with_prefixed_ids_and_jpeg_artwork() {
        let value = json!({
            "id": "jNQXAC9IVRw", "title": "Me at the zoo", "channel": "jawed",
            "channel_id": "UC4QobU6STFB0P71PMvOGN5A", "duration": 19,
            "upload_date": "20050424"
        });
        let track = track_from_value(&value, &Value::Null).unwrap();
        assert_eq!(track.id, "youtube:jNQXAC9IVRw");
        assert_eq!(track.artist, "jawed");
        assert_eq!(track.release_date.as_deref(), Some("2005-04-24"));
        assert!(track.cover_large.ends_with(".jpg"));
        assert_eq!(track.provider, "youtube");
    }

    #[test]
    fn accepts_only_expected_image_hosts() {
        assert!(valid_image("https://i.ytimg.com/vi/x/hqdefault.jpg").is_some());
        assert!(valid_image("https://yt3.googleusercontent.com/avatar").is_some());
        assert!(valid_image("https://i.ytimg.com.evil.example/x.jpg").is_none());
        assert!(valid_image("http://i.ytimg.com/vi/x/hqdefault.jpg").is_none());
    }

    #[test]
    fn resolves_links_without_network_access() {
        assert_eq!(resolve("https://youtu.be/jNQXAC9IVRw").unwrap().kind, "track");
        assert_eq!(resolve("https://www.youtube.com/@jawed").unwrap().kind, "artist");
        assert!(resolve("https://evil.example/watch?v=jNQXAC9IVRw").is_err());
    }
}

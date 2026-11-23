//! Account-free YouTube source backed by yt-dlp for discovery and metadata.
//! Audio bytes still pass through Wavekeep's shared limiter and temp-file pipeline.

pub mod api;
pub mod download;
mod process;

use url::Url;

pub const KEY_PREFIX: &str = "youtube:";
pub const USER_TRACKS_PREFIX: &str = "uploads:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Track(String),
    Playlist(String),
    Artist(String),
}

pub fn is_youtube_track(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

pub fn bare_track_id(key: &str) -> &str {
    key.strip_prefix(KEY_PREFIX).unwrap_or(key)
}

pub fn track_key(id: &str) -> Result<String, String> {
    Ok(format!("{}{}", KEY_PREFIX, video_id(id)?))
}

pub fn video_id(id: &str) -> Result<&str, String> {
    if id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        Ok(id)
    } else {
        Err("Invalid YouTube video ID".to_string())
    }
}

pub fn playlist_id(id: &str) -> Result<&str, String> {
    if !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(id)
    } else {
        Err("Invalid YouTube playlist ID".to_string())
    }
}

pub fn video_url(id: &str) -> Result<String, String> {
    Ok(format!("https://www.youtube.com/watch?v={}", video_id(id)?))
}

pub fn playlist_url(id: &str) -> Result<String, String> {
    Ok(format!("https://www.youtube.com/playlist?list={}", playlist_id(id)?))
}

fn valid_handle(id: &str) -> bool {
    id.len() > 1
        && id.len() <= 65
        && id.starts_with('@')
        && id[1..].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn valid_channel_id(id: &str) -> bool {
    id.len() >= 2
        && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn allowed_host(host: &str) -> bool {
    matches!(host, "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com")
}

fn channel_parts(url: &Url) -> Option<(String, String)> {
    if !allowed_host(url.host_str()?.to_ascii_lowercase().as_str()) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|segment| !segment.is_empty()).collect();
    let first = *segments.first()?;
    if first.starts_with('@') && valid_handle(first) {
        return Some((first.to_string(), format!("https://www.youtube.com/{}", first)));
    }
    match first {
        "channel" => {
            let id = *segments.get(1)?;
            if !valid_channel_id(id) {
                return None;
            }
            Some((id.to_string(), format!("https://www.youtube.com/channel/{}", id)))
        }
        "c" | "user" => {
            let name = *segments.get(1)?;
            if name.len() > 128 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
                return None;
            }
            let canonical = format!("https://www.youtube.com/{}/{}", first, name);
            Some((canonical.clone(), canonical))
        }
        _ => None,
    }
}

pub fn channel_url(id: &str) -> Result<String, String> {
    if valid_handle(id) {
        return Ok(format!("https://www.youtube.com/{}", id));
    }
    if valid_channel_id(id) {
        return Ok(format!("https://www.youtube.com/channel/{}", id));
    }
    let parsed = Url::parse(id).map_err(|_| "Invalid YouTube channel".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Invalid YouTube channel".to_string());
    }
    channel_parts(&parsed)
        .map(|(_, url)| url)
        .ok_or_else(|| "Invalid YouTube channel".to_string())
}

pub fn channel_tab_url(id: &str, tab: &str) -> Result<String, String> {
    Ok(format!("{}/{}", channel_url(id)?, tab))
}

/// Parses a YouTube video, playlist or channel URL without trusting lookalike hosts.
pub fn parse_link(input: &str) -> Option<Link> {
    let url = Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segments: Vec<&str> = url.path_segments()?.filter(|segment| !segment.is_empty()).collect();

    if matches!(host.as_str(), "youtu.be" | "www.youtu.be") {
        let id = video_id(segments.first()?).ok()?;
        return Some(Link::Track(id.to_string()));
    }
    if !allowed_host(&host) {
        return None;
    }
    if let Some(id) = url.query_pairs().find_map(|(key, value)| (key == "v").then_some(value)) {
        return video_id(&id).ok().map(|id| Link::Track(id.to_string()));
    }
    match segments.first().copied() {
        Some("shorts" | "embed" | "live") => {
            let id = video_id(segments.get(1).copied()?).ok()?;
            Some(Link::Track(id.to_string()))
        }
        Some("playlist") => {
            let id = url.query_pairs().find_map(|(key, value)| (key == "list").then_some(value))?;
            playlist_id(&id).ok().map(|id| Link::Playlist(id.to_string()))
        }
        Some("watch") => {
            let id = url.query_pairs().find_map(|(key, value)| (key == "list").then_some(value))?;
            playlist_id(&id).ok().map(|id| Link::Playlist(id.to_string()))
        }
        _ => channel_parts(&url).map(|(id, _)| Link::Artist(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_provider_ids_and_keys() {
        assert_eq!(track_key("jNQXAC9IVRw").unwrap(), "youtube:jNQXAC9IVRw");
        assert_eq!(bare_track_id("youtube:jNQXAC9IVRw"), "jNQXAC9IVRw");
        for bad in ["", "short", "jNQXAC9IVR!", "jNQXAC9IVRw/extra"] {
            assert!(video_id(bad).is_err(), "{}", bad);
        }
        assert!(playlist_id("PLuhl9TnQPDCnWIhy_KSbtFwXVQnNvgfSh").is_ok());
        assert!(playlist_id("list?next=1").is_err());
    }

    #[test]
    fn parses_video_links_with_video_precedence() {
        for url in [
            "https://www.youtube.com/watch?v=jNQXAC9IVRw&list=PL123",
            "https://music.youtube.com/watch?v=jNQXAC9IVRw",
            "https://youtu.be/jNQXAC9IVRw?t=10",
            "https://www.youtube.com/shorts/jNQXAC9IVRw",
            "https://www.youtube.com/embed/jNQXAC9IVRw",
            "https://www.youtube.com/live/jNQXAC9IVRw",
        ] {
            assert_eq!(parse_link(url), Some(Link::Track("jNQXAC9IVRw".to_string())), "{}", url);
        }
    }

    #[test]
    fn parses_playlist_and_channel_links() {
        assert_eq!(
            parse_link("https://www.youtube.com/playlist?list=PLuhl9TnQPDCnWIhy_KSbtFwXVQnNvgfSh"),
            Some(Link::Playlist("PLuhl9TnQPDCnWIhy_KSbtFwXVQnNvgfSh".to_string()))
        );
        assert_eq!(parse_link("https://www.youtube.com/@jawed/videos"), Some(Link::Artist("@jawed".to_string())));
        assert_eq!(
            parse_link("https://www.youtube.com/channel/UC4QobU6STFB0P71PMvOGN5A"),
            Some(Link::Artist("UC4QobU6STFB0P71PMvOGN5A".to_string()))
        );
        assert_eq!(
            channel_url("https://www.youtube.com/user/jawed").unwrap(),
            "https://www.youtube.com/user/jawed"
        );
    }

    #[test]
    fn rejects_lookalike_hosts_and_malformed_links() {
        for bad in [
            "https://youtube.com.evil.example/watch?v=jNQXAC9IVRw",
            "https://evil.example/watch?v=jNQXAC9IVRw",
            "file:///etc/passwd",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/",
        ] {
            assert_eq!(parse_link(bad), None, "{}", bad);
        }
    }
}

//! SoundCloud download source.
//!
//! Uses SoundCloud's public `api-v2` with the anonymous `client_id` that
//! soundcloud.com itself embeds in its JavaScript, so no account is needed.
//! Unencrypted streams (HLS AAC/MP3 and progressive MP3) always work. The
//! Widevine-encrypted streams (`ctr-encrypted-hls`, common for major-label
//! releases) need a `.wvd` device the user picks in Settings; `widevine.rs`
//! runs the licence exchange and decrypts the CENC samples. FairPlay
//! (`cbc-encrypted-hls`) and preview-only streams are still refused.

pub mod api;
pub mod download;

pub use api::{SoundcloudAlbum, SoundcloudArtist, SoundcloudPlaylist, SoundcloudTrack};

/// Track keys handed to the download queue are prefixed so a SoundCloud ID
/// can never collide with a numeric Deezer ID.
pub const KEY_PREFIX: &str = "soundcloud:";

pub fn is_soundcloud_track(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

pub fn bare_track_id(key: &str) -> &str {
    key.strip_prefix(KEY_PREFIX).unwrap_or(key)
}

pub fn track_key(id: u64) -> String {
    format!("{}{}", KEY_PREFIX, id)
}

/// Checks an ID from the renderer before it is formatted into a URL:
/// SoundCloud track, playlist and user IDs are 1–20 ASCII digits.
pub fn validate_id<'a>(id: &'a str, kind: &str) -> Result<&'a str, String> {
    if !id.is_empty() && id.len() <= 20 && id.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(format!("Invalid SoundCloud {} ID", kind))
    }
}

/// Prefix of the artist "album" that stands for all of a user's uploads.
pub const USER_TRACKS_PREFIX: &str = "user:";

/// `https://soundcloud.com/<path>` for a link to soundcloud.com (also `www.`
/// and `m.`), without its query or fragment (`?in=`, `?si=`), so only the
/// path reaches `/resolve`. `None` for anything else.
pub fn canonical_url(input: &str) -> Option<String> {
    let url = url::Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if !matches!(host.as_str(), "soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com") {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    Some(format!("https://soundcloud.com/{}", segments.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalises_soundcloud_links() {
        assert_eq!(
            canonical_url("https://m.soundcloud.com/flume/never-be-like-you-feat-kai?in=flume/sets/skin&si=abc").as_deref(),
            Some("https://soundcloud.com/flume/never-be-like-you-feat-kai")
        );
        assert_eq!(canonical_url(" http://www.SoundCloud.com/flume/ ").as_deref(), Some("https://soundcloud.com/flume"));
        assert_eq!(canonical_url("https://soundcloud.com/"), None);
        assert_eq!(canonical_url("https://soundcloud.com.evil.example/flume"), None);
        assert_eq!(canonical_url("https://api-v2.soundcloud.com/tracks/1"), None);
        assert_eq!(canonical_url("ftp://soundcloud.com/flume"), None);
    }

    #[test]
    fn accepts_only_numeric_ids() {
        assert_eq!(validate_id("242136202", "track"), Ok("242136202"));
        for rejected in ["", "12a", "../me", "1?x=2", "123456789012345678901", "-1"] {
            assert!(validate_id(rejected, "track").is_err(), "{rejected:?} should be rejected");
        }
    }

    #[test]
    fn track_keys_round_trip() {
        let key = track_key(242136202);
        assert_eq!(key, "soundcloud:242136202");
        assert!(is_soundcloud_track(&key));
        assert_eq!(bare_track_id(&key), "242136202");
        assert!(!is_soundcloud_track("spotify:4uLU6hMCjMI75M1A2tKUQC"));
        assert!(!is_soundcloud_track("3135556"));
    }
}

//! Spotify download engine.
//!
//! Search starts on Spotify's Web API, with a Pathfinder fallback for track,
//! album, and artist rate limits. Catalog metadata and audio use the librespot
//! AP session — the same path the official desktop client uses. Web API and AP
//! requests share one OAuth access token.

pub mod api;
pub mod ap;
pub mod auth;
pub mod download;
pub mod pathfinder;

pub use api::{SpotifyAlbum, SpotifyArtist, SpotifyPlaylist, SpotifyTrack};
pub use auth::SpotifyClient;

/// Track keys handed to the download queue are prefixed so a Spotify ID can
/// never collide with a numeric Deezer ID (or with a Deezer download in flight).
pub const KEY_PREFIX: &str = "spotify:";

pub fn is_spotify_track(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

pub fn bare_track_id(key: &str) -> &str {
    key.strip_prefix(KEY_PREFIX).unwrap_or(key)
}

pub fn track_key(id: &str) -> String {
    format!("{}{}", KEY_PREFIX, id)
}

/// Length of a base62 Spotify ID (track, album, artist and playlist alike).
const ID_LENGTH: usize = 22;

/// Largest Web API or Pathfinder response read into memory.
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Whether `id` is a bare 22-character base62 Spotify ID. librespot's own
/// parser accepts nothing else, so this only rejects input that could never
/// have worked, such as `../` or `?` spliced into a request path.
pub fn is_valid_id(id: &str) -> bool {
    id.len() == ID_LENGTH && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Checks an ID from the renderer (or an API response) before it is formatted
/// into a URL or URI.
pub fn validate_id<'a>(id: &'a str, kind: &str) -> Result<&'a str, String> {
    if is_valid_id(id) {
        Ok(id)
    } else {
        Err(format!("Invalid Spotify {} ID", kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_bare_base62_ids() {
        assert!(is_valid_id("4uLU6hMCjMI75M1A2tKUQC"));
        assert!(is_valid_id("37i9dQZF1DXcBWIGoYBM5M"));
        assert_eq!(validate_id("4aawyAB9vmqN3uQ7FjRGTy", "album"), Ok("4aawyAB9vmqN3uQ7FjRGTy"));

        for rejected in [
            "",
            "4uLU6hMCjMI75M1A2tKUQ",   // 21 characters
            "4uLU6hMCjMI75M1A2tKUQCX", // 23 characters
            "x/../../me#aaaaaaaaaaaa",
            "4uLU6hMCjMI75M1A2tKU?C",
            "spotify:4uLU6hMCjMI75M1",
            "4uLU6hMCjMI75M1A2tKUQ\u{e9}",
        ] {
            assert!(!is_valid_id(rejected), "{rejected:?} should be rejected");
        }
        assert_eq!(
            validate_id("../me", "artist"),
            Err("Invalid Spotify artist ID".to_string())
        );
    }

    #[test]
    fn track_keys_round_trip_to_valid_ids() {
        let key = track_key("4uLU6hMCjMI75M1A2tKUQC");
        assert!(is_spotify_track(&key));
        assert!(is_valid_id(bare_track_id(&key)));
        assert!(is_valid_id(bare_track_id("4uLU6hMCjMI75M1A2tKUQC")));
    }
}

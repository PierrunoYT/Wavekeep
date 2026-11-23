//! Helpers every download engine shares: capped body reads, cover art, the
//! `download-progress` event and the audio magic-byte check.

use std::time::Duration;

use futures::StreamExt;
use tauri::Emitter;

use crate::deezer::models::DownloadProgress;

const MAX_COVER_BYTES: usize = 20 * 1024 * 1024;
const COVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Reads a response body, failing once it grows past `max_bytes` instead of
/// buffering whatever the server sends. The status is left to the caller,
/// because Spotify's error bodies (401, 429) are still needed.
pub(crate) async fn read_capped(
    response: reqwest::Response,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!("{} is too large", label));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{} read failed: {}", label, e.without_url()))?;
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            return Err(format!("{} is too large", label));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Whether `bytes` begins like audio of the container `ext` names.
pub(crate) fn looks_like(ext: &str, bytes: &[u8]) -> bool {
    match ext {
        ".ogg" => bytes.starts_with(b"OggS"),
        ".flac" => bytes.starts_with(b"fLaC"),
        ".mp3" => {
            bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        }
        ".m4a" => bytes.len() >= 8 && &bytes[4..8] == b"ftyp",
        _ => false,
    }
}

/// Downloads cover art, or `None` when it is missing, too large or too slow.
pub(crate) async fn fetch_cover(http: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }

    // Bounded, so a stalled server cannot hold the download at "tagging".
    let response = http
        .get(url)
        .timeout(COVER_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_COVER_BYTES as u64)
    {
        return None;
    }

    // Read in chunks and stop at the cap: a body without a Content-Length
    // could otherwise be any size.
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if chunk.len() > MAX_COVER_BYTES.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    (!bytes.is_empty()).then_some(bytes)
}

pub(crate) fn emit_progress(
    app: &tauri::AppHandle,
    track_id: &str,
    title: &str,
    percent: f64,
    status: &str,
) {
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            track_id: track_id.to_string(),
            title: title.to_string(),
            percent,
            status: status.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &'static str) -> reqwest::Response {
        reqwest::Response::from(tauri::http::Response::builder().status(429).body(body).unwrap())
    }

    #[tokio::test]
    async fn reads_bodies_up_to_the_cap_whatever_the_status() {
        let body = read_capped(response("{\"error\":\"slow down\"}"), 64, "Test response")
            .await
            .unwrap();
        assert_eq!(body, b"{\"error\":\"slow down\"}");

        let exact = read_capped(response("12345678"), 8, "Test response").await.unwrap();
        assert_eq!(exact.len(), 8);
    }

    #[tokio::test]
    async fn rejects_bodies_over_the_cap() {
        let error = read_capped(response("123456789"), 8, "Test response")
            .await
            .unwrap_err();
        assert_eq!(error, "Test response is too large");
    }

    #[tokio::test]
    async fn rejects_streamed_bodies_without_a_length_once_they_pass_the_cap() {
        let chunks = futures::stream::iter(
            ["abcd", "efgh", "ijkl"].map(|chunk| Ok::<_, std::io::Error>(chunk.as_bytes().to_vec())),
        );
        let streamed = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(200)
                .body(reqwest::Body::wrap_stream(chunks))
                .unwrap(),
        );
        assert_eq!(streamed.content_length(), None);
        let error = read_capped(streamed, 10, "Test response").await.unwrap_err();
        assert_eq!(error, "Test response is too large");
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;
use tauri::Emitter;
use tokio::io::AsyncWriteExt;
use url::Url;

use super::api;
use super::bare_track_id;
use crate::deezer::bandwidth::BandwidthLimiter;
use crate::deezer::models::DownloadResult;
use crate::paths::{
    build_download_path, cleanup_temp_file_async, create_temp_download_file,
    finalize_download_file, find_existing_download, TrackPathMeta,
};
use crate::settings::FolderStructure;
use crate::spotify::download::{emit_progress, fetch_cover, looks_like};
use crate::spotify::tags::{self, TrackTags};

/// Shown as the requested quality: SoundCloud has no quality setting, the
/// best unencrypted stream is always taken.
const REQUESTED_QUALITY: &str = "SOUNDCLOUD";
/// Formats that count as "already downloaded" at the destination.
const AUDIO_EXTENSIONS: [&str; 2] = [".m4a", ".mp3"];
const MAX_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 1024 * 1024 * 1024;
/// Per segment (or per progressive file) request, so one stalled response
/// cannot hold a download forever. The client's 30 s read timeout still
/// applies between chunks.
const PART_TIMEOUT: Duration = Duration::from_secs(600);
const API_PREFIX: &str = "https://api-v2.soundcloud.com/";

const DRM_MESSAGE: &str =
    "SoundCloud only offers this track as a DRM-protected stream, which Wavekeep does not download";
const PREVIEW_MESSAGE: &str =
    "SoundCloud only offers a 30-second preview of this track (SoundCloud Go+ or region-restricted)";

#[derive(Debug, Clone, PartialEq)]
struct Choice {
    url: String,
    progressive: bool,
    ext: &'static str,
    label: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn download_track(
    track_key: &str,
    output_dir: &str,
    folder_structure: &FolderStructure,
    custom_folder_template: &str,
    skip_existing: bool,
    bandwidth: &BandwidthLimiter,
    speed_limit_kbps: u32,
    app: &tauri::AppHandle,
    cancel_flag: Arc<AtomicBool>,
) -> Result<DownloadResult, String> {
    crate::logging::info("soundcloud", format!("download start: {}", track_key));
    let (track, raw) = api::track(bare_track_id(track_key)).await.map_err(|e| {
        crate::logging::warn("soundcloud", format!("track metadata failed: {}", e));
        e
    })?;
    crate::logging::debug(
        "soundcloud",
        format!("track metadata ok: {}", crate::logging::detail(format!("{} - {}", track.artist, track.title))),
    );
    emit_progress(app, track_key, &track.title, 0.0, "resolving");

    let choice = select_transcoding(&raw).map_err(|e| {
        crate::logging::warn("soundcloud", format!("no usable stream: {}", e));
        e
    })?;
    crate::logging::debug(
        "soundcloud",
        format!("selected {} ({}, {})", choice.label, choice.ext, if choice.progressive { "progressive" } else { "hls" }),
    );

    let path_meta = TrackPathMeta {
        artist: track.artist.clone(),
        album: if track.album.is_empty() { "Unknown".to_string() } else { track.album.clone() },
        title: track.title.clone(),
        track_number: None,
        disc_number: None,
        release_date: track.release_date.clone(),
    };

    if skip_existing {
        if let Some(existing) = find_existing_download(
            output_dir,
            folder_structure,
            custom_folder_template,
            &path_meta,
            &AUDIO_EXTENSIONS,
        )
        .await?
        {
            emit_progress(app, track_key, &track.title, 100.0, "complete");
            return Ok(result(existing.to_string_lossy().to_string(), &choice.label, "skipped"));
        }
    }

    let authorization = raw["track_authorization"].as_str().unwrap_or_default();
    let stream_url = stream_url(&choice.url, authorization).await?;
    let http = api::http()?;
    let parts = if choice.progressive {
        vec![stream_url]
    } else {
        let playlist = fetch_text(&http, &stream_url).await?;
        parse_hls(&stream_url, &playlist)?
    };
    crate::logging::debug("soundcloud", format!("{} part(s) to fetch", parts.len()));

    let download_path = build_download_path(output_dir, folder_structure, custom_folder_template, &path_meta, choice.ext)?;
    let download_dir = download_path
        .parent()
        .ok_or("Cannot determine download directory")?
        .to_path_buf();
    tokio::fs::create_dir_all(&download_dir).await.map_err(|e| e.to_string())?;
    let base_stem = download_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    emit_progress(app, track_key, &track.title, 5.0, "downloading");

    // The guard removes the temp file on any early return or panic until the
    // file is published.
    let (mut temp_guard, mut file) = create_temp_download_file(&download_path, track_key).await?;
    let temp_path = temp_guard.path().to_path_buf();

    let mut head: Vec<u8> = Vec::with_capacity(8);
    let mut written = 0u64;
    let count = parts.len();
    for (index, part) in parts.iter().enumerate() {
        let mut progress = |part_done: u64, part_len: Option<u64>| {
            // Segments advance progress one by one; a single progressive file
            // advances it by bytes.
            let fraction = match part_len {
                Some(len) if count == 1 && len > 0 => (part_done as f64 / len as f64).min(1.0),
                _ => index as f64 / count as f64,
            };
            emit_progress(app, track_key, &track.title, 5.0 + fraction * 85.0, "downloading");
        };
        let fetched = fetch_part(
            &http,
            part,
            &mut file,
            &mut head,
            &mut written,
            bandwidth,
            speed_limit_kbps,
            &cancel_flag,
            &mut progress,
        )
        .await;
        match fetched {
            Ok(true) => {}
            Ok(false) => {
                drop(file);
                cleanup_temp_file_async(&temp_path).await;
                return Ok(result(String::new(), &choice.label, "canceled"));
            }
            Err(e) => {
                crate::logging::warn("soundcloud", format!("part {}/{} failed after {} bytes: {}", index + 1, count, written, e));
                drop(file);
                cleanup_temp_file_async(&temp_path).await;
                return Err(e);
            }
        }
    }

    if let Err(e) = file.flush().await {
        drop(file);
        cleanup_temp_file_async(&temp_path).await;
        return Err(e.to_string());
    }
    drop(file);

    if !looks_like(choice.ext, &head) {
        crate::logging::warn("soundcloud", format!("stream is not {} audio ({} bytes)", choice.ext, written));
        cleanup_temp_file_async(&temp_path).await;
        return Err(format!("SoundCloud did not send valid {} audio", choice.ext.trim_start_matches('.')));
    }

    crate::logging::info("soundcloud", format!("stream complete: {} bytes, tagging", written));
    emit_progress(app, track_key, &track.title, 92.0, "tagging");

    let cover = fetch_cover(&http, &track.cover_large).await;
    let tag_path = temp_path.clone();
    let tag_ext = choice.ext.to_string();
    let tag_track = TrackTags {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        track_number: 0,
        disc_number: 0,
        release_date: track.release_date.clone(),
        isrc: track.isrc.clone(),
    };
    // A panicking tagger is a tag failure like any other: the audio is intact.
    let tag_result = tokio::task::spawn_blocking(move || tags::write_tags(&tag_path, &tag_track, cover, &tag_ext))
        .await
        .unwrap_or_else(|e| Err(format!("Tagging task failed: {}", e)));
    if let Err(e) = tag_result {
        crate::logging::warn("soundcloud", format!("Tags not written: {}", e));
        let _ = app.emit(
            "tag-writing-error",
            serde_json::json!({ "track_id": track_key, "title": track.title, "error": e }),
        );
    }

    if cancel_flag.load(Ordering::Relaxed) {
        cleanup_temp_file_async(&temp_path).await;
        return Ok(result(String::new(), &choice.label, "canceled"));
    }

    let finalize_temp = temp_path.clone();
    let preferred_path = download_path.clone();
    let finalize_dir = download_dir.clone();
    let finalize_ext = choice.ext.to_string();
    let finalize_cancel = cancel_flag.clone();
    let finalized = tokio::task::spawn_blocking(move || {
        finalize_download_file(&finalize_temp, &preferred_path, &finalize_dir, &base_stem, &finalize_ext, &finalize_cancel)
    })
    .await
    .unwrap_or_else(|e| Err(format!("File finalization task failed: {}", e)));
    let download_path = match finalized {
        Ok(path) => path,
        Err(e) => {
            cleanup_temp_file_async(&temp_path).await;
            return Err(e);
        }
    };
    temp_guard.disarm();

    crate::logging::info(
        "soundcloud",
        format!("download complete: {}", crate::logging::detail(download_path.display())),
    );
    emit_progress(app, track_key, &track.title, 100.0, "complete");
    Ok(result(download_path.to_string_lossy().to_string(), &choice.label, "complete"))
}

fn result(file_path: String, actual_quality: &str, status: &str) -> DownloadResult {
    DownloadResult {
        file_path,
        requested_quality: REQUESTED_QUALITY.to_string(),
        actual_quality: actual_quality.to_string(),
        status: status.to_string(),
    }
}

/// Bitrate in kbps from a preset such as `aac_160k`; MP3 presets
/// (`mp3_0_1`, `mp3_standard`) are always 128.
fn bitrate(preset: &str, codec: &str) -> u32 {
    if codec == "MP3" {
        return 128;
    }
    preset
        .rsplit('_')
        .next()
        .and_then(|last| last.strip_suffix('k'))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(128)
}

/// Picks the highest-bitrate stream SoundCloud serves unencrypted and in full:
/// AAC or MP3 over HLS, or progressive MP3. Encrypted (DRM) and snipped
/// (30-second preview) streams are never selected, and neither is the
/// adaptive `audio/mpegurl` master playlist.
fn select_transcoding(track: &Value) -> Result<Choice, String> {
    if track["policy"].as_str() == Some("BLOCK") {
        return Err("This SoundCloud track is not available in your country".to_string());
    }
    let mut best: Option<(u32, Choice)> = None;
    let (mut saw_encrypted, mut saw_snipped) = (false, false);
    for transcoding in track["media"]["transcodings"].as_array().into_iter().flatten() {
        let protocol = transcoding["format"]["protocol"].as_str().unwrap_or_default();
        let mime = transcoding["format"]["mime_type"].as_str().unwrap_or_default();
        if protocol.contains("encrypted") {
            saw_encrypted = true;
            continue;
        }
        if transcoding["snipped"].as_bool() == Some(true) {
            saw_snipped = true;
            continue;
        }
        let progressive = match protocol {
            "progressive" => true,
            "hls" => false,
            _ => continue,
        };
        let (ext, codec) = if mime.starts_with("audio/mpeg") {
            (".mp3", "MP3")
        } else if mime.starts_with("audio/mp4") {
            (".m4a", "AAC")
        } else {
            continue;
        };
        // The resolve request carries the client_id, so it may only go to api-v2.
        let Some(url) = transcoding["url"].as_str().filter(|url| url.starts_with(API_PREFIX)) else {
            continue;
        };
        let kbps = bitrate(transcoding["preset"].as_str().unwrap_or_default(), codec);
        // Higher bitrate wins; at equal bitrate a single progressive file wins.
        let score = kbps * 2 + u32::from(progressive);
        if best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
            best = Some((
                score,
                Choice {
                    url: url.to_string(),
                    progressive,
                    ext,
                    label: format!("SOUNDCLOUD_{}_{}", codec, kbps),
                },
            ));
        }
    }
    match best {
        Some((_, choice)) => Ok(choice),
        None if saw_encrypted => Err(DRM_MESSAGE.to_string()),
        None if saw_snipped => Err(PREVIEW_MESSAGE.to_string()),
        None => Err("SoundCloud has no downloadable stream for this track".to_string()),
    }
}

/// Exchanges a transcoding URL for the signed stream (or playlist) URL.
async fn stream_url(transcoding_url: &str, authorization: &str) -> Result<Url, String> {
    let mut url = Url::parse(transcoding_url).map_err(|_| "Invalid SoundCloud stream URL".to_string())?;
    if !authorization.is_empty() {
        url.query_pairs_mut().append_pair("track_authorization", authorization);
    }
    let answer = api::api_url(url).await?;
    answer["url"]
        .as_str()
        .and_then(|url| Url::parse(url).ok())
        .filter(|url| url.scheme() == "https")
        .ok_or_else(|| "SoundCloud did not return a stream URL".to_string())
}

async fn fetch_text(http: &reqwest::Client, url: &Url) -> Result<String, String> {
    let response = http
        .get(url.clone())
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("SoundCloud playlist request failed: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("SoundCloud playlist request failed ({})", response.status()));
    }
    let body = crate::spotify::read_capped(response, MAX_PLAYLIST_BYTES, "SoundCloud playlist").await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// The init section (`#EXT-X-MAP`) followed by every media segment of an HLS
/// media playlist. Encrypted playlists (any `#EXT-X-KEY` other than
/// `METHOD=NONE`) are refused, and so are master playlists and non-https URLs.
fn parse_hls(base: &Url, playlist: &str) -> Result<Vec<Url>, String> {
    if !playlist.trim_start().starts_with("#EXTM3U") {
        return Err("SoundCloud sent an invalid HLS playlist".to_string());
    }
    let resolve = |reference: &str| -> Result<Url, String> {
        base.join(reference)
            .ok()
            .filter(|url| url.scheme() == "https")
            .ok_or_else(|| "SoundCloud sent an invalid HLS segment URL".to_string())
    };
    let mut parts = Vec::new();
    for line in playlist.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(attributes) = line.strip_prefix("#EXT-X-KEY:") {
            if !attributes.split(',').any(|attribute| attribute.trim() == "METHOD=NONE") {
                return Err(DRM_MESSAGE.to_string());
            }
        } else if line.starts_with("#EXT-X-STREAM-INF") {
            return Err("SoundCloud sent a master playlist instead of a media playlist".to_string());
        } else if let Some(attributes) = line.strip_prefix("#EXT-X-MAP:") {
            if !parts.is_empty() {
                return Err("SoundCloud sent an HLS playlist with several init sections".to_string());
            }
            let uri = attributes
                .split("URI=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .ok_or("SoundCloud sent an invalid HLS init section")?;
            parts.push(resolve(uri)?);
        } else if !line.starts_with('#') {
            parts.push(resolve(line)?);
        }
    }
    if parts.is_empty() {
        return Err("SoundCloud sent an empty HLS playlist".to_string());
    }
    Ok(parts)
}

/// Appends one URL's body to `file`, paced by the shared bandwidth limiter.
/// Retries once when the request fails before any byte of it was written.
/// Returns `Ok(false)` when the download was cancelled.
#[allow(clippy::too_many_arguments)]
async fn fetch_part(
    http: &reqwest::Client,
    url: &Url,
    file: &mut tokio::fs::File,
    head: &mut Vec<u8>,
    written: &mut u64,
    bandwidth: &BandwidthLimiter,
    speed_limit_kbps: u32,
    cancel_flag: &AtomicBool,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<bool, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut part_written = 0u64;
        let outcome: Result<bool, String> = async {
            let response = http
                .get(url.clone())
                .timeout(PART_TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("SoundCloud audio request failed: {}", e.without_url()))?;
            if !response.status().is_success() {
                return Err(format!("SoundCloud audio request failed ({})", response.status()));
            }
            let length = response.content_length();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let chunk = chunk.map_err(|e| format!("SoundCloud audio read failed: {}", e.without_url()))?;
                if *written + chunk.len() as u64 > MAX_AUDIO_BYTES {
                    return Err("SoundCloud audio is too large".to_string());
                }
                bandwidth.acquire(chunk.len(), speed_limit_kbps, cancel_flag).await;
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                let missing = 8usize.saturating_sub(head.len());
                head.extend_from_slice(&chunk[..missing.min(chunk.len())]);
                *written += chunk.len() as u64;
                part_written += chunk.len() as u64;
                progress(part_written, length);
            }
            Ok(true)
        }
        .await;
        match outcome {
            Err(e) if part_written == 0 && attempt < 2 && !cancel_flag.load(Ordering::Relaxed) => {
                crate::logging::debug("soundcloud", format!("retrying part: {}", e));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcoding(preset: &str, protocol: &str, mime: &str, snipped: bool) -> Value {
        json!({
            "url": format!("https://api-v2.soundcloud.com/media/soundcloud:tracks:1/{}/stream/{}", preset, protocol),
            "preset": preset,
            "snipped": snipped,
            "format": { "protocol": protocol, "mime_type": mime }
        })
    }

    fn track(transcodings: Vec<Value>) -> Value {
        json!({ "policy": "MONETIZE", "media": { "transcodings": transcodings } })
    }

    const AAC: &str = "audio/mp4; codecs=\"mp4a.40.2\"";

    #[test]
    fn prefers_the_highest_unencrypted_bitrate() {
        // The shape SoundCloud sends today: AAC 160 and 96 over HLS plus the
        // adaptive master playlist.
        let choice = select_transcoding(&track(vec![
            transcoding("aac_96k", "hls", AAC, false),
            transcoding("abr_sq", "hls", "audio/mpegurl", false),
            transcoding("aac_160k", "hls", AAC, false),
        ]))
        .unwrap();
        assert_eq!(choice.label, "SOUNDCLOUD_AAC_160");
        assert_eq!(choice.ext, ".m4a");
        assert!(!choice.progressive);
        assert!(choice.url.contains("aac_160k"));
    }

    #[test]
    fn prefers_progressive_mp3_over_hls_mp3_and_aac_160_over_mp3() {
        let mp3 = select_transcoding(&track(vec![
            transcoding("mp3_1_0", "hls", "audio/mpeg", false),
            transcoding("mp3_0_1", "progressive", "audio/mpeg", false),
        ]))
        .unwrap();
        assert!(mp3.progressive);
        assert_eq!((mp3.ext, mp3.label.as_str()), (".mp3", "SOUNDCLOUD_MP3_128"));

        let aac = select_transcoding(&track(vec![
            transcoding("mp3_0_1", "progressive", "audio/mpeg", false),
            transcoding("aac_160k", "hls", AAC, false),
        ]))
        .unwrap();
        assert_eq!(aac.label, "SOUNDCLOUD_AAC_160");
    }

    #[test]
    fn never_selects_encrypted_or_snipped_streams() {
        let drm = select_transcoding(&track(vec![
            transcoding("aac_256k", "ctr-encrypted-hls", AAC, false),
            transcoding("aac_160k", "cbc-encrypted-hls", AAC, false),
        ]))
        .unwrap_err();
        assert_eq!(drm, DRM_MESSAGE);

        let preview = select_transcoding(&track(vec![transcoding("mp3_0_1", "progressive", "audio/mpeg", true)])).unwrap_err();
        assert_eq!(preview, PREVIEW_MESSAGE);

        // A full unencrypted stream still wins over a better encrypted one.
        let choice = select_transcoding(&track(vec![
            transcoding("aac_256k", "ctr-encrypted-hls", AAC, false),
            transcoding("aac_96k", "hls", AAC, false),
        ]))
        .unwrap();
        assert_eq!(choice.label, "SOUNDCLOUD_AAC_96");
    }

    #[test]
    fn only_resolves_transcodings_on_api_v2() {
        let mut elsewhere = transcoding("aac_160k", "hls", AAC, false);
        elsewhere["url"] = json!("https://evil.example/media/stream/hls");
        assert!(select_transcoding(&track(vec![elsewhere])).is_err());
    }

    #[test]
    fn parses_fmp4_playlists_in_order_with_the_init_section_first() {
        let base = Url::parse("https://playback.media-streaming.soundcloud.cloud/x/aac_160k/id/playlist.m3u8?sig=1").unwrap();
        let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n\
            #EXT-X-MAP:URI=\"https://playback.media-streaming.soundcloud.cloud/x/aac_160k/id/init.mp4?sig=2\"\n\
            #EXTINF:10.0,\nhttps://playback.media-streaming.soundcloud.cloud/x/aac_160k/id/data000.m4s?sig=3\n\
            #EXTINF:10.0,\ndata001.m4s?sig=4\n#EXT-X-ENDLIST\n";
        let parts = parse_hls(&base, playlist).unwrap();
        let paths: Vec<&str> = parts.iter().map(|url| url.path()).collect();
        assert_eq!(paths, ["/x/aac_160k/id/init.mp4", "/x/aac_160k/id/data000.m4s", "/x/aac_160k/id/data001.m4s"]);
        assert_eq!(parts[2].query(), Some("sig=4"));
    }

    #[test]
    fn refuses_encrypted_master_and_insecure_playlists() {
        let base = Url::parse("https://cf-hls-media.sndcdn.com/playlist.m3u8").unwrap();
        let encrypted = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"\n#EXTINF:10,\nseg0.m4s\n";
        assert_eq!(parse_hls(&base, encrypted).unwrap_err(), DRM_MESSAGE);
        let unencrypted = "#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:10,\nseg0.mp3\n";
        assert_eq!(parse_hls(&base, unencrypted).unwrap().len(), 1);
        assert!(parse_hls(&base, "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nlow.m3u8\n").is_err());
        assert!(parse_hls(&base, "#EXTM3U\n#EXTINF:10,\nhttp://cf-hls-media.sndcdn.com/seg0.mp3\n").is_err());
        assert!(parse_hls(&base, "<html>").is_err());
        assert!(parse_hls(&base, "#EXTM3U\n#EXT-X-ENDLIST\n").is_err());
    }

    #[test]
    fn reads_bitrates_from_presets() {
        assert_eq!(bitrate("aac_160k", "AAC"), 160);
        assert_eq!(bitrate("aac_96k", "AAC"), 96);
        assert_eq!(bitrate("mp3_0_1", "MP3"), 128);
        assert_eq!(bitrate("aac_hq", "AAC"), 128);
    }
}

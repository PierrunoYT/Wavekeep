use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use super::api;
use super::process;
use super::bare_track_id;
use crate::audio::{emit_progress, fetch_cover, looks_like};
use crate::deezer::bandwidth::BandwidthLimiter;
use crate::deezer::models::DownloadResult;
use crate::paths::{
    build_download_path, cleanup_temp_file_async, create_temp_download_file, finalize_download_file,
    find_existing_download, TrackPathMeta,
};
use crate::settings::FolderStructure;
use crate::tags::{self, TrackTags};

const REQUESTED_QUALITY: &str = "YOUTUBE";
const AUDIO_EXTENSIONS: [&str; 3] = [".ogg", ".m4a", ".mp3"];
const MAX_AUDIO_BYTES: u64 = 1024 * 1024 * 1024;
const PART_TIMEOUT: Duration = Duration::from_secs(600);
/// YouTube throttles one request for a whole file to roughly playback speed,
/// but serves `&range=` slices of up to 10 MiB at full speed (the chunk size
/// yt-dlp itself uses).
const CHUNK_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug)]
struct Selection {
    url: Url,
    headers: HeaderMap,
    source_ext: String,
    output_ext: &'static str,
    label: String,
    /// Exact byte size when yt-dlp reports one (`filesize`).
    size: Option<u64>,
}

fn media_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "YouTube returned an invalid audio URL".to_string())?;
    let valid_host = url
        .host_str()
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == "googlevideo.com" || host.ends_with(".googlevideo.com") || host.ends_with(".youtube.com")
        })
        .unwrap_or(false);
    if url.scheme() != "https" || !valid_host {
        return Err("YouTube returned an untrusted audio URL".to_string());
    }
    Ok(url)
}

fn select_download(value: &Value) -> Result<Selection, String> {
    let format = value
        .get("requested_downloads")
        .and_then(Value::as_array)
        .and_then(|downloads| downloads.first())
        .unwrap_or(value);
    if text(format, "protocol") != Some("https") {
        return Err("YouTube's selected audio format does not support direct HTTPS download".to_string());
    }
    let url = media_url(text(format, "url").ok_or("YouTube did not return an audio URL")?)?;
    let source_ext = text(format, "ext").or_else(|| text(format, "audio_ext")).unwrap_or("").to_string();
    let codec = text(format, "acodec").unwrap_or("").to_ascii_lowercase();
    let (output_ext, codec_label) = if source_ext == "webm" && codec.contains("opus") {
        (".ogg", "OPUS")
    } else if matches!(source_ext.as_str(), "m4a" | "mp4") && (codec.contains("mp4a") || codec.contains("aac")) {
        (".m4a", "AAC")
    } else if source_ext == "mp3" {
        (".mp3", "MP3")
    } else {
        return Err("YouTube did not offer an Opus, AAC or MP3 audio format Wavekeep can save".to_string());
    };
    let abr = format.get("abr").and_then(Value::as_f64).unwrap_or(0.0).round().max(0.0) as u64;
    let label = if abr == 0 {
        format!("YOUTUBE_{}", codec_label)
    } else {
        format!("YOUTUBE_{}_{}", codec_label, abr)
    };

    let mut headers = HeaderMap::new();
    if let Some(values) = format.get("http_headers").and_then(Value::as_object) {
        for (key, value) in values {
            let lower = key.to_ascii_lowercase();
            if !matches!(lower.as_str(), "user-agent" | "accept" | "accept-language" | "sec-fetch-mode" | "referer" | "origin") {
                continue;
            }
            let Some(value) = value.as_str() else {
                continue;
            };
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(lower.as_bytes()), HeaderValue::from_str(value)) {
                headers.insert(name, value);
            }
        }
    }
    let size = format.get("filesize").and_then(Value::as_u64).filter(|size| *size > 0);
    if size.is_some_and(|size| size > MAX_AUDIO_BYTES) {
        return Err("YouTube audio is too large".to_string());
    }
    Ok(Selection { url, headers, source_ext: format!(".{}", source_ext), output_ext, label, size })
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().map(str::trim).filter(|text| !text.is_empty())
}

fn result(file_path: String, quality: &str, status: &str) -> DownloadResult {
    DownloadResult {
        file_path,
        requested_quality: REQUESTED_QUALITY.to_string(),
        actual_quality: quality.to_string(),
        status: status.to_string(),
    }
}

fn source_looks_valid(ext: &str, head: &[u8]) -> bool {
    match ext {
        ".webm" => head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        ".m4a" | ".mp4" => looks_like(".m4a", head),
        ".mp3" => looks_like(".mp3", head),
        _ => false,
    }
}

async fn read_head(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut head = vec![0u8; 8];
    let read = file.read(&mut head).await.map_err(|e| e.to_string())?;
    head.truncate(read);
    Ok(head)
}

/// Downloads the audio in `CHUNK_BYTES` slices. A failed slice is resumed
/// from the last byte written, once per slice. Returns `None` on cancellation.
#[allow(clippy::too_many_arguments)]
async fn fetch_audio(
    http: &reqwest::Client,
    selection: &Selection,
    destination: &Path,
    bandwidth: &BandwidthLimiter,
    speed_limit_kbps: u32,
    cancel_flag: &AtomicBool,
    progress: &(dyn Fn(f64) + Sync),
) -> Result<Option<u64>, String> {
    let mut file = tokio::fs::File::create(destination).await.map_err(|e| e.to_string())?;
    let mut downloaded = 0u64;
    let mut last_percent = 4u64;
    let mut retried = false;
    loop {
        let start = downloaded;
        let end = match selection.size {
            Some(total) => start.saturating_add(CHUNK_BYTES).min(total) - 1,
            None => start + CHUNK_BYTES - 1,
        };
        let requested = end - start + 1;
        let mut url = selection.url.clone();
        url.query_pairs_mut().append_pair("range", &format!("{}-{}", start, end));
        let outcome: Result<(), String> = async {
            let response = http
                .get(url)
                .headers(selection.headers.clone())
                .timeout(PART_TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("YouTube audio request failed: {}", e.without_url()))?;
            if !response.status().is_success() {
                return Err(format!("YouTube audio request failed ({})", response.status()));
            }
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let chunk = chunk.map_err(|e| format!("YouTube audio read failed: {}", e.without_url()))?;
                if downloaded.saturating_add(chunk.len() as u64) > MAX_AUDIO_BYTES {
                    return Err("YouTube audio is too large".to_string());
                }
                bandwidth.acquire(chunk.len(), speed_limit_kbps, cancel_flag).await;
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(());
                }
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                downloaded += chunk.len() as u64;
                if let Some(total) = selection.size {
                    let percent = (5 + ((downloaded as f64 / total as f64) * 70.0) as u64).min(75);
                    if percent > last_percent {
                        last_percent = percent;
                        progress(last_percent as f64);
                    }
                }
            }
            Ok(())
        }
        .await;
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match outcome {
            Ok(()) => {
                retried = false;
                let received = downloaded - start;
                let finished = match selection.size {
                    Some(total) => downloaded >= total,
                    None => received < requested,
                };
                // `received > requested`: the server ignored the range and
                // sent the whole file.
                if finished || received == 0 || received > requested {
                    break;
                }
            }
            Err(e) if !retried => {
                retried = true;
                crate::logging::debug("youtube", format!("Resuming audio at byte {}: {}", downloaded, e));
            }
            Err(e) => return Err(e),
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    if downloaded == 0 {
        return Err("YouTube returned an empty audio stream".to_string());
    }
    if selection.size.is_some_and(|total| total != downloaded) {
        return Err("YouTube audio download was incomplete".to_string());
    }
    Ok(Some(downloaded))
}

async fn remux(selection: &Selection, source: &Path, output: &Path, cancel_flag: &AtomicBool) -> Result<bool, String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        source.to_string_lossy().into_owned(),
        "-vn".to_string(),
        "-c:a".to_string(),
        "copy".to_string(),
    ];
    if selection.output_ext == ".m4a" {
        args.push("-movflags".to_string());
        args.push("+faststart".to_string());
    }
    args.push(output.to_string_lossy().into_owned());
    let output = process::run_tool("ffmpeg", &args, Some(cancel_flag)).await?;
    let Some(output) = output else {
        return Ok(false);
    };
    output.success("ffmpeg")?;
    Ok(true)
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
    crate::logging::info("youtube", format!("download start: {}", track_key));
    emit_progress(app, track_key, "YouTube", 0.0, "resolving");
    let Some(metadata) = api::download_metadata(bare_track_id(track_key), &cancel_flag).await? else {
        return Ok(result(String::new(), REQUESTED_QUALITY, "canceled"));
    };
    let track = api::track_from_value(&metadata, &Value::Null).ok_or("YouTube did not return track metadata")?;
    let selection = select_download(&metadata)?;
    crate::logging::debug(
        "youtube",
        format!("track metadata ok: {}", crate::logging::detail(format!("{} - {}", track.artist, track.title))),
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
            return Ok(result(existing.to_string_lossy().to_string(), &selection.label, "skipped"));
        }
    }

    let download_path = build_download_path(output_dir, folder_structure, custom_folder_template, &path_meta, selection.output_ext)?;
    let download_dir = download_path.parent().ok_or("Cannot determine download directory")?.to_path_buf();
    tokio::fs::create_dir_all(&download_dir).await.map_err(|e| e.to_string())?;
    let base_stem = download_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string();
    let work = tempfile::Builder::new()
        .prefix(".wavekeep-youtube-")
        .tempdir_in(&download_dir)
        .map_err(|e| format!("Could not create YouTube work folder: {}", e))?;
    let source = work.path().join(format!("source{}", selection.source_ext));
    let converted = work.path().join(format!("audio{}", selection.output_ext));
    let (mut temp_guard, file) = create_temp_download_file(&download_path, track_key).await?;
    let temp_path = temp_guard.path().to_path_buf();
    drop(file);

    emit_progress(app, track_key, &track.title, 5.0, "downloading");
    let http = api::http()?;
    let progress = |percent: f64| emit_progress(app, track_key, &track.title, percent, "downloading");
    let Some(downloaded) = fetch_audio(
        &http,
        &selection,
        &source,
        bandwidth,
        speed_limit_kbps,
        &cancel_flag,
        &progress,
    )
    .await?
    else {
        return Ok(result(String::new(), &selection.label, "canceled"));
    };
    if !source_looks_valid(&selection.source_ext, &read_head(&source).await?) {
        return Err("YouTube did not send the expected audio container".to_string());
    }
    crate::logging::debug("youtube", format!("Received {} audio bytes", downloaded));

    emit_progress(app, track_key, &track.title, 80.0, "downloading");
    if !remux(&selection, &source, &converted, &cancel_flag).await? {
        return Ok(result(String::new(), &selection.label, "canceled"));
    }
    if !looks_like(selection.output_ext, &read_head(&converted).await?) {
        return Err("ffmpeg did not produce the expected audio container".to_string());
    }
    tokio::fs::remove_file(&temp_path).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&converted, &temp_path).await.map_err(|e| e.to_string())?;
    drop(work);

    emit_progress(app, track_key, &track.title, 92.0, "tagging");
    let cover = fetch_cover(&http, &track.cover_large).await;
    let tag_path = temp_path.clone();
    let tag_ext = selection.output_ext.to_string();
    let tag_track = TrackTags {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        track_number: 0,
        disc_number: 0,
        release_date: track.release_date.clone(),
        isrc: None,
    };
    let tag_result = tokio::task::spawn_blocking(move || tags::write_tags(&tag_path, &tag_track, cover, &tag_ext))
        .await
        .unwrap_or_else(|e| Err(format!("Tagging task failed: {}", e)));
    if let Err(e) = tag_result {
        crate::logging::warn("youtube", format!("Tags not written: {}", e));
        let _ = app.emit(
            "tag-writing-error",
            serde_json::json!({ "track_id": track_key, "title": track.title, "error": e }),
        );
    }
    if cancel_flag.load(Ordering::Relaxed) {
        cleanup_temp_file_async(&temp_path).await;
        return Ok(result(String::new(), &selection.label, "canceled"));
    }

    let finalize_temp = temp_path.clone();
    let preferred_path = download_path.clone();
    let finalize_dir = download_dir.clone();
    let finalize_ext = selection.output_ext.to_string();
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
    crate::logging::info("youtube", format!("download complete: {}", crate::logging::detail(download_path.display())));
    emit_progress(app, track_key, &track.title, 100.0, "complete");
    Ok(result(download_path.to_string_lossy().to_string(), &selection.label, "complete"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn format(protocol: &str, url: &str) -> Value {
        json!({
            "requested_downloads": [{
                "protocol": protocol, "url": url, "ext": "webm", "acodec": "opus", "abr": 106.064,
                "http_headers": { "User-Agent": "test", "Cookie": "do-not-copy" }
            }]
        })
    }

    #[test]
    fn selects_direct_opus_audio_without_credentials() {
        let selection = select_download(&format("https", "https://rr1.googlevideo.com/audio?sig=secret")).unwrap();
        assert_eq!(selection.output_ext, ".ogg");
        assert_eq!(selection.source_ext, ".webm");
        assert_eq!(selection.label, "YOUTUBE_OPUS_106");
        assert!(selection.headers.contains_key("user-agent"));
        assert!(!selection.headers.contains_key("cookie"));
    }

    #[test]
    fn refuses_hls_and_untrusted_media_urls() {
        assert!(select_download(&format("m3u8_native", "https://manifest.googlevideo.com/list.m3u8")).is_err());
        assert!(select_download(&format("https", "https://evil.example/audio.webm")).is_err());
        assert!(select_download(&format("https", "http://rr1.googlevideo.com/audio.webm")).is_err());
    }

    #[test]
    fn recognizes_source_containers() {
        assert!(source_looks_valid(".webm", &[0x1a, 0x45, 0xdf, 0xa3, 0]));
        assert!(!source_looks_valid(".webm", b"<html>"));
        assert!(source_looks_valid(".m4a", &[0, 0, 0, 8, b'f', b't', b'y', b'p']));
    }
}

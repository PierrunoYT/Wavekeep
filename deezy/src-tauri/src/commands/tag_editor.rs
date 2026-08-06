use super::*;

const MAX_COVER_ART_BYTES: u64 = 20 * 1024 * 1024;

// ── Tag Editor ────────────────────────────────────────────────────────────────

/// Open a file picker limited to MP3 and FLAC files.
#[tauri::command]
pub async fn pick_audio_file(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    app.dialog()
        .file()
        .set_title("Select Audio File")
        .add_filter("Audio Files", &["mp3", "flac"])
        .pick_file(move |file_path| {
            let _ = tx.send(file_path.map(|p| p.to_string()));
        });

    match rx.await {
        Ok(path) => Ok(path),
        Err(_) => Ok(None),
    }
}

/// Open a file picker limited to image files (for cover art replacement).
#[tauri::command]
pub async fn pick_cover_image(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    app.dialog()
        .file()
        .set_title("Select Cover Image")
        .add_filter("Image Files", &["jpg", "jpeg", "png"])
        .pick_file(move |file_path| {
            let _ = tx.send(file_path.map(|p| p.to_string()));
        });

    match rx.await {
        Ok(path) => Ok(path),
        Err(_) => Ok(None),
    }
}

/// Read an image file from disk and return it as a base64 data URL,
/// for previewing newly-picked cover art in the UI before saving.
#[tauri::command]
#[allow(non_snake_case)]
pub async fn read_image_as_data_url(filePath: String) -> Result<String, String> {
    run_blocking(move || read_image_as_data_url_blocking(filePath)).await
}

fn read_image_as_data_url_blocking(file_path: String) -> Result<String, String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;

    let bytes = read_cover_image(&file_path)?;
    let mime = detect_image_mime(&bytes)?;
    Ok(format!("data:{};base64,{}", mime, B64.encode(&bytes)))
}

/// Read metadata tags from an MP3 or FLAC file.
#[tauri::command]
#[allow(non_snake_case)]
pub async fn read_file_tags(filePath: String) -> Result<FileTagData, String> {
    run_blocking(move || read_file_tags_blocking(filePath)).await
}

#[allow(non_snake_case)]
fn read_file_tags_blocking(filePath: String) -> Result<FileTagData, String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;

    let path = std::path::Path::new(&filePath);

    if !path.exists() {
        return Err("File not found".to_string());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "mp3" => {
            use id3::TagLike;
            let tag = id3::Tag::read_from_path(path)
                .unwrap_or_else(|_| id3::Tag::new());

            let title        = tag.title().map(|s| s.to_string());
            let artist       = tag.artist().map(|s| s.to_string());
            let album        = tag.album().map(|s| s.to_string());
            let album_artist = tag.album_artist().map(|s| s.to_string());
            let year         = tag.year();
            let track_num    = tag.track();
            let total_tracks = tag.total_tracks();
            let disc         = tag.disc();
            let total_discs  = tag.total_discs();
            let genre        = tag.genre().map(|s| s.to_string());

            let label = tag.frames()
                .find(|f| f.id() == "TPUB")
                .and_then(|f| {
                    if let id3::Content::Text(t) = f.content() { Some(t.clone()) } else { None }
                });

            let comment = tag.comments().next().map(|c| c.text.clone());

            let (cover_data, cover_mime) = tag.pictures().next()
                .filter(|p| p.data.len() as u64 <= MAX_COVER_ART_BYTES)
                .map(|p| (
                    Some(B64.encode(&p.data)),
                    Some(p.mime_type.clone()),
                ))
                .unwrap_or((None, None));

            Ok(FileTagData {
                file_path: filePath,
                format: "mp3".to_string(),
                title, artist, album, album_artist,
                year, track: track_num, total_tracks, disc, total_discs,
                genre, label, comment, cover_data, cover_mime,
            })
        }
        "flac" => {
            let tag = metaflac::Tag::read_from_path(path)
                .map_err(|e| format!("FLAC read error: {}", e))?;

            let get = |key: &str| -> Option<String> {
                tag.get_vorbis(key)
                    .and_then(|mut v| v.next())
                    .map(|s| s.to_string())
            };

            let title        = get("TITLE");
            let artist       = get("ARTIST");
            let album        = get("ALBUM");
            let album_artist = get("ALBUMARTIST");
            let year         = get("DATE").and_then(|d| d[..4.min(d.len())].parse::<i32>().ok());
            let track_num    = get("TRACKNUMBER").and_then(|v| v.parse::<u32>().ok());
            let total_tracks = get("TOTALTRACKS").or_else(|| get("TRACKTOTAL"))
                                   .and_then(|v| v.parse::<u32>().ok());
            let disc         = get("DISCNUMBER").and_then(|v| v.parse::<u32>().ok());
            let total_discs  = get("TOTALDISCS").or_else(|| get("DISCTOTAL"))
                                   .and_then(|v| v.parse::<u32>().ok());
            let genre        = get("GENRE");
            let label        = get("LABEL");
            let comment      = get("COMMENT");

            let (cover_data, cover_mime) = tag.pictures().next()
                .filter(|p| p.data.len() as u64 <= MAX_COVER_ART_BYTES)
                .map(|p| (
                    Some(B64.encode(&p.data)),
                    Some(p.mime_type.clone()),
                ))
                .unwrap_or((None, None));

            Ok(FileTagData {
                file_path: filePath,
                format: "flac".to_string(),
                title, artist, album, album_artist,
                year, track: track_num, total_tracks, disc, total_discs,
                genre, label, comment, cover_data, cover_mime,
            })
        }
        _ => Err(format!("Unsupported file format: {}", ext)),
    }
}

/// Write metadata tags to an MP3 or FLAC file.
#[tauri::command]
#[allow(non_snake_case)]
pub async fn write_file_tags(filePath: String, tags: WriteTagData) -> Result<(), String> {
    run_blocking(move || write_file_tags_blocking(filePath, tags)).await
}

#[allow(non_snake_case)]
fn write_file_tags_blocking(filePath: String, tags: WriteTagData) -> Result<(), String> {
    let path = std::path::Path::new(&filePath);

    if !path.exists() {
        return Err("File not found".to_string());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Resolve new cover bytes once (used for both MP3 and FLAC branches)
    let new_cover: Option<Vec<u8>> = if let Some(ref cover_path) = tags.new_cover_path {
        let bytes = read_cover_image(cover_path)?;
        detect_image_mime(&bytes)?;
        Some(bytes)
    } else {
        None
    };

    match ext.as_str() {
        "mp3" => {
            use id3::TagLike;

            let mut tag = id3::Tag::read_from_path(path)
                .unwrap_or_else(|_| id3::Tag::new());

            if let Some(v) = &tags.title        { tag.set_title(v); }
            if let Some(v) = &tags.artist       { tag.set_artist(v); }
            if let Some(v) = &tags.album        { tag.set_album(v); }
            if let Some(v) = &tags.album_artist { tag.set_album_artist(v); }
            if let Some(v) = tags.year          { tag.set_year(v); }
            if let Some(v) = tags.track         { tag.set_track(v); }
            if let Some(v) = tags.total_tracks  { tag.set_total_tracks(v); }
            if let Some(v) = tags.disc          { tag.set_disc(v); }
            if let Some(v) = tags.total_discs   { tag.set_total_discs(v); }
            if let Some(v) = &tags.genre        { tag.set_genre(v); }

            if let Some(v) = &tags.label {
                tag.remove("TPUB");
                tag.add_frame(id3::Frame::with_content(
                    "TPUB",
                    id3::Content::Text(v.clone()),
                ));
            }

            if let Some(v) = &tags.comment {
                tag.remove("COMM");
                tag.add_frame(id3::Frame::with_content(
                    "COMM",
                    id3::Content::Comment(id3::frame::Comment {
                        lang: "eng".to_string(),
                        description: String::new(),
                        text: v.clone(),
                    }),
                ));
            }

            if tags.remove_cover {
                tag.remove_picture_by_type(id3::frame::PictureType::CoverFront);
            } else if let Some(cover_bytes) = new_cover {
                let mime = detect_image_mime(&cover_bytes)?;
                tag.remove_picture_by_type(id3::frame::PictureType::CoverFront);
                tag.add_frame(id3::Frame::with_content(
                    "APIC",
                    id3::Content::Picture(id3::frame::Picture {
                        mime_type: mime,
                        picture_type: id3::frame::PictureType::CoverFront,
                        description: String::new(),
                        data: cover_bytes,
                    }),
                ));
            }

            tag.write_to_path(path, id3::Version::Id3v24)
                .map_err(|e| format!("Failed to write MP3 tags: {}", e))?;
        }
        "flac" => {
            let mut tag = metaflac::Tag::read_from_path(path)
                .map_err(|e| format!("FLAC read error: {}", e))?;

            if let Some(v) = &tags.title        { tag.set_vorbis("TITLE",       vec![v.as_str()]); }
            if let Some(v) = &tags.artist       { tag.set_vorbis("ARTIST",      vec![v.as_str()]); }
            if let Some(v) = &tags.album        { tag.set_vorbis("ALBUM",       vec![v.as_str()]); }
            if let Some(v) = &tags.album_artist { tag.set_vorbis("ALBUMARTIST", vec![v.as_str()]); }
            if let Some(v) = tags.year          { tag.set_vorbis("DATE",        vec![v.to_string()]); }
            if let Some(v) = tags.track         { tag.set_vorbis("TRACKNUMBER", vec![v.to_string()]); }
            if let Some(v) = tags.total_tracks  { tag.set_vorbis("TOTALTRACKS", vec![v.to_string()]); }
            if let Some(v) = tags.disc          { tag.set_vorbis("DISCNUMBER",  vec![v.to_string()]); }
            if let Some(v) = tags.total_discs   { tag.set_vorbis("TOTALDISCS",  vec![v.to_string()]); }
            if let Some(v) = &tags.genre        { tag.set_vorbis("GENRE",       vec![v.as_str()]); }
            if let Some(v) = &tags.label        { tag.set_vorbis("LABEL",       vec![v.as_str()]); }
            if let Some(v) = &tags.comment      { tag.set_vorbis("COMMENT",     vec![v.as_str()]); }

            if tags.remove_cover {
                tag.remove_picture_type(metaflac::block::PictureType::CoverFront);
            } else if let Some(cover_bytes) = new_cover {
                let mime = detect_image_mime(&cover_bytes)?;
                tag.remove_picture_type(metaflac::block::PictureType::CoverFront);
                tag.add_picture(&mime, metaflac::block::PictureType::CoverFront, cover_bytes);
            }

            tag.write_to_path(path)
                .map_err(|e| format!("Failed to write FLAC tags: {}", e))?;
        }
        _ => return Err(format!("Unsupported file format: {}", ext)),
    }

    Ok(())
}

/// Detect MIME type from image magic bytes.
fn detect_image_mime(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Ok("image/jpeg".to_string())
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Ok("image/png".to_string())
    } else {
        Err("Cover art must be a valid JPEG or PNG image".to_string())
    }
}

fn read_cover_image(path: &str) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to inspect cover image: {}", e))?;
    if !metadata.is_file() {
        return Err("Cover image path is not a file".to_string());
    }
    if metadata.len() > MAX_COVER_ART_BYTES {
        return Err(format!(
            "Cover image is too large (maximum {} MiB)",
            MAX_COVER_ART_BYTES / 1024 / 1024
        ));
    }

    std::fs::read(path).map_err(|e| format!("Failed to read cover image: {}", e))
}

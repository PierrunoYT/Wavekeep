use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagType};

use super::api::SpotifyTrack;

/// The tag fields the Spotify and SoundCloud engines write.
#[derive(Debug, Clone)]
pub struct TrackTags {
    pub title: String,
    pub artist: String,
    /// Left out of the file when empty.
    pub album: String,
    /// 0 when unknown.
    pub track_number: u32,
    pub disc_number: u32,
    pub release_date: Option<String>,
    pub isrc: Option<String>,
}

impl From<&SpotifyTrack> for TrackTags {
    fn from(track: &SpotifyTrack) -> Self {
        Self {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            release_date: track.release_date.clone(),
            isrc: track.isrc.clone(),
        }
    }
}

/// Writes Vorbis comments (OGG/FLAC), ID3v2 (MP3) or MP4 ilst tags plus the
/// album cover. Uses one tag model for every container Spotify and SoundCloud
/// can deliver.
pub fn write_tags(
    path: &Path,
    track: &TrackTags,
    cover: Option<Vec<u8>>,
    ext: &str,
) -> Result<(), String> {
    // The file is still the `.wavekeep.part` temp file, so the format cannot come
    // from its extension; sniff it from the content instead.
    let mut tagged = Probe::open(path)
        .map_err(|e| format!("Tag read error: {}", e))?
        .guess_file_type()
        .map_err(|e| format!("Tag read error: {}", e))?
        .read()
        .map_err(|e| format!("Tag read error: {}", e))?;

    // The temp file's own extension is `.part`, so the real container comes
    // from the selected format.
    let tag_type = match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "mp3" => TagType::Id3v2,
        "m4a" | "mp4" => TagType::Mp4Ilst,
        _ => TagType::VorbisComments,
    };

    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }

    let tag = tagged
        .primary_tag_mut()
        .ok_or("Audio file does not accept tags")?;

    tag.set_title(track.title.clone());
    tag.set_artist(track.artist.clone());
    if !track.album.is_empty() {
        tag.set_album(track.album.clone());
    }
    if track.track_number > 0 {
        tag.set_track(track.track_number);
    }
    if track.disc_number > 0 {
        tag.set_disk(track.disc_number);
    }
    if let Some(date) = track.release_date.as_deref() {
        let year: String = date.chars().take(4).collect();
        if year.len() == 4 {
            tag.insert_text(ItemKey::Year, year);
        }
    }
    if let Some(isrc) = track.isrc.as_deref() {
        tag.insert_text(ItemKey::Isrc, isrc.to_string());
    }

    if let Some(data) = cover {
        if !data.is_empty() {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(
                Picture::unchecked(data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Jpeg)
                    .build(),
            );
        }
    }

    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("Tag write error: {}", e))
}

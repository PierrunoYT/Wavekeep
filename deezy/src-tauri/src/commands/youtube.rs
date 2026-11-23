//! YouTube catalog IPC. No account or session is required; yt-dlp performs
//! discovery and metadata extraction using the user's installed executable.

use crate::youtube::api::{self, Resolved, YoutubeAlbum, YoutubeArtist, YoutubePlaylist, YoutubeTrack};

#[tauri::command]
pub async fn search_youtube_tracks(query: String) -> Result<Vec<YoutubeTrack>, String> {
    api::search_tracks(&query).await
}

#[tauri::command]
pub async fn search_youtube_artists(query: String) -> Result<Vec<YoutubeArtist>, String> {
    api::search_artists(&query).await
}

#[tauri::command]
pub async fn search_youtube_playlists(query: String) -> Result<Vec<YoutubePlaylist>, String> {
    api::search_playlists(&query).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_youtube_track(trackId: String) -> Result<YoutubeTrack, String> {
    api::track(&trackId).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_youtube_album_tracks(albumId: String) -> Result<Vec<YoutubeTrack>, String> {
    api::album_tracks(&albumId).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_youtube_playlist_tracks(playlistId: String) -> Result<Vec<YoutubeTrack>, String> {
    api::playlist_tracks(&playlistId).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_youtube_artist_albums(artistId: String) -> Result<Vec<YoutubeAlbum>, String> {
    api::artist_albums(&artistId).await
}

/// What a pasted YouTube link points at (`track`, `playlist` or `artist`).
#[tauri::command]
pub fn resolve_youtube_url(url: String) -> Result<Resolved, String> {
    api::resolve(&url)
}

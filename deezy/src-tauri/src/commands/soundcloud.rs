//! SoundCloud catalog IPC. No login is involved: every command works with the
//! anonymous web client ID (see `soundcloud/api.rs`).

use crate::soundcloud::{
    self, api, SoundcloudAlbum, SoundcloudArtist, SoundcloudPlaylist, SoundcloudTrack,
};

#[tauri::command]
pub async fn search_soundcloud_tracks(query: String) -> Result<Vec<SoundcloudTrack>, String> {
    api::search_tracks(&query).await
}

#[tauri::command]
pub async fn search_soundcloud_albums(query: String) -> Result<Vec<SoundcloudAlbum>, String> {
    api::search_albums(&query).await
}

#[tauri::command]
pub async fn search_soundcloud_artists(query: String) -> Result<Vec<SoundcloudArtist>, String> {
    api::search_artists(&query).await
}

#[tauri::command]
pub async fn search_soundcloud_playlists(query: String) -> Result<Vec<SoundcloudPlaylist>, String> {
    api::search_playlists(&query).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_soundcloud_track(trackId: String) -> Result<SoundcloudTrack, String> {
    // Accepts both a bare ID and a `soundcloud:<id>` track key.
    Ok(api::track(soundcloud::bare_track_id(&trackId)).await?.0)
}

/// Tracks of an album, or every upload of a user for `user:<id>` (the "All
/// tracks" entry the artist page adds in front of the albums).
#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_soundcloud_album_tracks(albumId: String) -> Result<Vec<SoundcloudTrack>, String> {
    match albumId.strip_prefix(soundcloud::USER_TRACKS_PREFIX) {
        Some(user_id) => api::user_tracks(user_id).await,
        None => api::set_tracks(&albumId).await,
    }
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_soundcloud_playlist_tracks(playlistId: String) -> Result<Vec<SoundcloudTrack>, String> {
    api::set_tracks(&playlistId).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn get_soundcloud_artist_albums(artistId: String) -> Result<Vec<SoundcloudAlbum>, String> {
    api::user_sets(&artistId).await
}

/// What a pasted soundcloud.com link points at (`track`, `album`, `playlist`
/// or `artist`) and its ID.
#[tauri::command]
pub async fn resolve_soundcloud_url(url: String) -> Result<api::Resolved, String> {
    let url = soundcloud::canonical_url(&url).ok_or("Invalid SoundCloud URL")?;
    api::resolve(&url).await
}

/// Open a picker for a Widevine device (`.wvd`) and approve it. The file is
/// parsed before it is returned, so a wrong file is refused straight away
/// rather than at download time.
#[tauri::command]
pub async fn pick_cdm_file(
    state: tauri::State<'_, crate::AppState>,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select Widevine Device")
        .add_filter("Widevine Device", &["wvd"])
        .pick_file(move |file_path| {
            let _ = tx.send(file_path.map(|p| p.to_string()));
        });

    let path = rx.await.unwrap_or(None);
    if let Some(ref path) = path {
        crate::widevine::Device::load(std::path::Path::new(path)).await?;
        super::grant_cdm_file(&state, path).await?;
    }
    Ok(path)
}

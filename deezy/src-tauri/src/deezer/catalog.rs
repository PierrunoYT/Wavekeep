use super::*;

impl DeezerClient {
    pub async fn search_tracks(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchResult>, String> {
        let url = format!("{}/search/track", LEGACY_API_URL);

        let res = self
            .http
            .get(&url)
            .query(&[
                ("q", query),
                ("limit", &limit.to_string()),
                ("index", "0"),
            ])
            .send()
            .await
            .map_err(|e| format!("Search failed: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse results: {}", e))?;

        if let Some(error) = data.get("error") {
            if let Some(obj) = error.as_object() {
                if !obj.is_empty() {
                    let msg = obj
                        .values()
                        .next()
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("API error: {}", msg));
                }
            }
        }

        let tracks = data["data"]
            .as_array()
            .ok_or("No results found")?
            .iter()
            .filter_map(|t| {
                Some(SearchResult {
                    id: t["id"].as_u64()?,
                    title: t["title"].as_str()?.to_string(),
                    artist: t["artist"]["name"].as_str()?.to_string(),
                    artist_id: t["artist"]["id"].as_u64().unwrap_or(0),
                    album: t["album"]["title"].as_str().unwrap_or("Unknown").to_string(),
                    duration: t["duration"].as_u64().unwrap_or(0),
                    cover_small: t["album"]["cover_small"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    cover_medium: t["album"]["cover_medium"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    preview: t["preview"].as_str().map(|s| s.to_string()),
                })
            })
            .collect();

        Ok(tracks)
    }

    pub async fn search_albums(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<AlbumResult>, String> {
        let url = format!("{}/search/album", LEGACY_API_URL);

        let res = self
            .http
            .get(&url)
            .query(&[
                ("q", query),
                ("limit", &limit.to_string()),
                ("index", "0"),
            ])
            .send()
            .await
            .map_err(|e| format!("Search failed: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse results: {}", e))?;

        if let Some(error) = data.get("error") {
            if let Some(obj) = error.as_object() {
                if !obj.is_empty() {
                    let msg = obj
                        .values()
                        .next()
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("API error: {}", msg));
                }
            }
        }

        let albums = data["data"]
            .as_array()
            .ok_or("No results found")?
            .iter()
            .filter_map(|a| {
                Some(AlbumResult {
                    id: a["id"].as_u64()?,
                    title: a["title"].as_str()?.to_string(),
                    artist: a["artist"]["name"].as_str()?.to_string(),
                    artist_id: a["artist"]["id"].as_u64().unwrap_or(0),
                    cover_small: a["cover_small"].as_str().unwrap_or("").to_string(),
                    cover_medium: a["cover_medium"].as_str().unwrap_or("").to_string(),
                    nb_tracks: a["nb_tracks"].as_u64().unwrap_or(0),
                })
            })
            .collect();

        Ok(albums)
    }

    pub async fn get_album_tracks(
        &self,
        album_id: &str,
    ) -> Result<Vec<SearchResult>, String> {
        let tracks_url = format!("{}/album/{}/tracks", LEGACY_API_URL, album_id);

        // Fetch tracks and album metadata concurrently.
        let (tracks_res, album_data) = tokio::try_join!(
            async {
                let response = self.http
                    .get(&tracks_url)
                    .query(&[("limit", "500")])
                    .send()
                    .await
                    .map_err(|e| format!("Failed to get album tracks: {}", e))?;
                response_json::<Value>(response)
                    .await
                    .map_err(|e| format!("Failed to parse album tracks: {}", e))
            },
            self.get_album(album_id),
        )?;

        let data = tracks_res;
        let album_title = album_data["title"].as_str().unwrap_or("Unknown").to_string();
        let cover_small = album_data["cover_small"].as_str().unwrap_or("").to_string();
        let cover_medium = album_data["cover_medium"].as_str().unwrap_or("").to_string();

        let tracks = data["data"]
            .as_array()
            .ok_or("No tracks found in album")?
            .iter()
            .filter_map(|t| {
                Some(SearchResult {
                    id: t["id"].as_u64()?,
                    title: t["title"].as_str()?.to_string(),
                    artist: t["artist"]["name"].as_str()?.to_string(),
                    artist_id: t["artist"]["id"].as_u64().unwrap_or(0),
                    album: album_title.clone(),
                    duration: t["duration"].as_u64().unwrap_or(0),
                    cover_small: cover_small.clone(),
                    cover_medium: cover_medium.clone(),
                    preview: t["preview"].as_str().map(|s| s.to_string()),
                })
            })
            .collect();

        Ok(tracks)
    }

    pub async fn search_artists(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<ArtistResult>, String> {
        let url = format!("{}/search/artist", LEGACY_API_URL);

        let res = self
            .http
            .get(&url)
            .query(&[
                ("q", query),
                ("limit", &limit.to_string()),
                ("index", "0"),
            ])
            .send()
            .await
            .map_err(|e| format!("Search failed: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse results: {}", e))?;

        if let Some(error) = data.get("error") {
            if let Some(obj) = error.as_object() {
                if !obj.is_empty() {
                    let msg = obj
                        .values()
                        .next()
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("API error: {}", msg));
                }
            }
        }

        let artists = data["data"]
            .as_array()
            .ok_or("No results found")?
            .iter()
            .filter_map(|a| {
                Some(ArtistResult {
                    id: a["id"].as_u64()?,
                    name: a["name"].as_str()?.to_string(),
                    picture_small: a["picture_small"].as_str().unwrap_or("").to_string(),
                    picture_medium: a["picture_medium"].as_str().unwrap_or("").to_string(),
                    nb_album: a["nb_album"].as_u64().unwrap_or(0),
                    nb_fan: a["nb_fan"].as_u64().unwrap_or(0),
                })
            })
            .collect();

        Ok(artists)
    }

    pub async fn get_artist_albums(
        &self,
        artist_id: &str,
    ) -> Result<Vec<AlbumResult>, String> {
        let url = format!("{}/artist/{}/albums", LEGACY_API_URL, artist_id);

        let res = self
            .http
            .get(&url)
            .query(&[("limit", "100")])
            .send()
            .await
            .map_err(|e| format!("Failed to get artist albums: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse artist albums: {}", e))?;

        let albums = data["data"]
            .as_array()
            .ok_or("No albums found for artist")?
            .iter()
            .filter_map(|a| {
                Some(AlbumResult {
                    id: a["id"].as_u64()?,
                    title: a["title"].as_str()?.to_string(),
                    artist: a["artist"]["name"].as_str().unwrap_or("").to_string(),
                    artist_id: a["artist"]["id"].as_u64().unwrap_or(0),
                    cover_small: a["cover_small"].as_str().unwrap_or("").to_string(),
                    cover_medium: a["cover_medium"].as_str().unwrap_or("").to_string(),
                    nb_tracks: a["nb_tracks"].as_u64().unwrap_or(0),
                })
            })
            .collect();

        Ok(albums)
    }

    pub async fn search_playlists(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<PlaylistResult>, String> {
        let url = format!("{}/search/playlist", LEGACY_API_URL);

        let res = self
            .http
            .get(&url)
            .query(&[
                ("q", query),
                ("limit", &limit.to_string()),
                ("index", "0"),
            ])
            .send()
            .await
            .map_err(|e| format!("Search failed: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse results: {}", e))?;

        if let Some(error) = data.get("error") {
            if let Some(obj) = error.as_object() {
                if !obj.is_empty() {
                    let msg = obj
                        .values()
                        .next()
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("API error: {}", msg));
                }
            }
        }

        let playlists = data["data"]
            .as_array()
            .ok_or("No results found")?
            .iter()
            .filter_map(|p| {
                Some(PlaylistResult {
                    id: p["id"].as_u64()?,
                    title: p["title"].as_str()?.to_string(),
                    creator: p["user"]["name"].as_str().unwrap_or("").to_string(),
                    cover_small: p["picture_small"].as_str().unwrap_or("").to_string(),
                    cover_medium: p["picture_medium"].as_str().unwrap_or("").to_string(),
                    nb_tracks: p["nb_tracks"].as_u64().unwrap_or(0),
                })
            })
            .collect();

        Ok(playlists)
    }

    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<SearchResult>, String> {
        let url = format!("{}/playlist/{}", LEGACY_API_URL, playlist_id);

        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to get playlist tracks: {}", e))?;

        let data: Value = response_json(res)
            .await
            .map_err(|e| format!("Failed to parse playlist tracks: {}", e))?;

        let cover_small = data["picture_small"].as_str().unwrap_or("").to_string();
        let cover_medium = data["picture_medium"].as_str().unwrap_or("").to_string();

        let tracks = data["tracks"]["data"]
            .as_array()
            .ok_or("No tracks found in playlist")?
            .iter()
            .filter_map(|t| {
                Some(SearchResult {
                    id: t["id"].as_u64()?,
                    title: t["title"].as_str()?.to_string(),
                    artist: t["artist"]["name"].as_str()?.to_string(),
                    artist_id: t["artist"]["id"].as_u64().unwrap_or(0),
                    album: t["album"]["title"].as_str().unwrap_or("Unknown").to_string(),
                    duration: t["duration"].as_u64().unwrap_or(0),
                    cover_small: t["album"]["cover_small"]
                        .as_str()
                        .unwrap_or(&cover_small)
                        .to_string(),
                    cover_medium: t["album"]["cover_medium"]
                        .as_str()
                        .unwrap_or(&cover_medium)
                        .to_string(),
                    preview: t["preview"].as_str().map(|s| s.to_string()),
                })
            })
            .collect();

        Ok(tracks)
    }

    pub async fn get_track_by_id(&self, track_id: &str) -> Result<SearchResult, String> {
        let url = format!("{}/track/{}", LEGACY_API_URL, track_id);

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to get track: {}", e))?;
        let data: Value = response_json(response)
            .await
            .map_err(|e| format!("Failed to parse track: {}", e))?;

        if let Some(error) = data.get("error") {
            if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                return Err(format!("API error: {}", message));
            }
        }

        let id = data["id"]
            .as_u64()
            .or_else(|| track_id.parse::<u64>().ok())
            .ok_or("Track not found")?;

        let title = data["title"].as_str().unwrap_or("").to_string();
        if title.is_empty() {
            return Err("Track not found".to_string());
        }

        Ok(SearchResult {
            id,
            title,
            artist: data["artist"]["name"].as_str().unwrap_or("Unknown").to_string(),
            artist_id: data["artist"]["id"].as_u64().unwrap_or(0),
            album: data["album"]["title"].as_str().unwrap_or("Unknown").to_string(),
            duration: data["duration"].as_u64().unwrap_or(0),
            cover_small: data["album"]["cover_small"].as_str().unwrap_or("").to_string(),
            cover_medium: data["album"]["cover_medium"].as_str().unwrap_or("").to_string(),
            preview: data["preview"].as_str().map(|s| s.to_string()),
        })
    }
}

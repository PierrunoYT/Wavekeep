export type Provider = 'deezer' | 'spotify' | 'soundcloud';
export type CatalogId = number | string;

export interface AlbumResult {
  id: CatalogId;
  title: string;
  artist: string;
  artist_id: CatalogId;
  cover_small: string;
  cover_medium: string;
  nb_tracks: number;
  release_date?: string;
  provider?: Provider;
}

export interface ArtistResult {
  id: CatalogId;
  name: string;
  picture_small: string;
  picture_medium: string;
  nb_album: number;
  nb_fan: number;
  provider?: Provider;
}

export interface PlaylistResult {
  id: CatalogId;
  title: string;
  creator: string;
  cover_small: string;
  cover_medium: string;
  nb_tracks: number;
  provider?: Provider;
}

export interface SelectedArtist {
  id: CatalogId;
  name: string;
  picture: string;
}

export interface SelectedPlaylist {
  id: CatalogId;
  title: string;
  cover: string;
  creator: string;
  provider?: Provider;
}

export type CatalogSearchType = 'tracks' | 'albums' | 'artists' | 'playlists';
/** Tabs that list the logged-in user's own library instead of search results. */
export type LibraryType = 'loved' | 'myPlaylists';
export type SearchType = CatalogSearchType | LibraryType;

export function isLibraryType(type: SearchType): type is LibraryType {
  return type === 'loved' || type === 'myPlaylists';
}

/** Catalog lookups that every provider offers, each under its own command. */
export type CatalogCommand = 'track' | 'albumTracks' | 'playlistTracks' | 'artistAlbums';

const CATALOG_COMMANDS: Record<Provider, Record<CatalogCommand, string>> = {
  deezer: {
    track: 'get_track_by_id',
    albumTracks: 'get_album_tracks',
    playlistTracks: 'get_playlist_tracks',
    artistAlbums: 'get_artist_albums'
  },
  spotify: {
    track: 'get_spotify_track',
    albumTracks: 'get_spotify_album_tracks',
    playlistTracks: 'get_spotify_playlist_tracks',
    artistAlbums: 'get_spotify_artist_albums'
  },
  soundcloud: {
    track: 'get_soundcloud_track',
    albumTracks: 'get_soundcloud_album_tracks',
    playlistTracks: 'get_soundcloud_playlist_tracks',
    artistAlbums: 'get_soundcloud_artist_albums'
  }
};

/** The command for `kind` on `provider`; results without a provider are Deezer's. */
export function catalogCommand(provider: Provider | undefined, kind: CatalogCommand): string {
  return CATALOG_COMMANDS[provider ?? 'deezer'][kind];
}

/** Search command for a catalog tab, e.g. `search_tracks` or `search_soundcloud_tracks`. */
export function searchCommand(provider: Provider, type: CatalogSearchType): string {
  return provider === 'deezer' ? `search_${type}` : `search_${provider}_${type}`;
}

/** Album ID that lists every upload of a SoundCloud user (`get_soundcloud_album_tracks`). */
export function soundcloudUserTracksId(userId: CatalogId): string {
  return `user:${userId}`;
}

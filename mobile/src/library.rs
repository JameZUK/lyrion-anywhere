//! Library browsing (no player needed).

use std::sync::Arc;

use serde_json::Value;

use crate::commands::{self, AlbumSort, ArtistRole, Command, LibraryFilter};
use crate::error::ApiError;
use crate::model::{
    self, Album, AlbumPage, ArtistPage, FavouritePage, FolderPage, Genre, GenrePage, PlaylistPage,
    SearchResults, Track, VirtualLibrary, YearPage,
};
use crate::session::Session;

#[derive(uniffi::Object)]
pub struct MusicLibrary {
    session: Arc<Session>,
}

impl MusicLibrary {
    async fn query(&self, cmd: Command) -> Result<Value, ApiError> {
        self.session.core().rpc("", cmd).await
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl MusicLibrary {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(MusicLibrary { session })
    }

    /// A page of albums (`count` at most 500), optionally filtered by a search text and
    /// narrowed by `filter` (genre, year, artist, compilations, role, virtual library).
    pub async fn albums(
        &self,
        start: u32,
        count: u32,
        sort: AlbumSort,
        search: Option<String>,
        filter: LibraryFilter,
    ) -> Result<AlbumPage, ApiError> {
        let cmd = commands::albums(start, count, sort, search.as_deref(), &filter)?;
        Ok(model::albums_from(&self.query(cmd).await?))
    }

    /// One album, or `None` if the library doesn't have it.
    pub async fn album(&self, album_id: i64) -> Result<Option<Album>, ApiError> {
        let v = self.query(commands::album_by_id(album_id)?).await?;
        Ok(model::albums_from(&v).items.into_iter().next())
    }

    /// An album's tracks, in order.
    pub async fn album_tracks(&self, album_id: i64) -> Result<Vec<Track>, ApiError> {
        let v = self.query(commands::album_tracks(album_id)?).await?;
        Ok(model::tracks_from(&v, "titles_loop"))
    }

    /// A page of artists, narrowed by genre, role and virtual library.
    pub async fn artists(
        &self,
        start: u32,
        count: u32,
        search: Option<String>,
        filter: LibraryFilter,
    ) -> Result<ArtistPage, ApiError> {
        let cmd = commands::artists(start, count, search.as_deref(), &filter)?;
        Ok(model::artists_from(&self.query(cmd).await?))
    }

    /// Whether the library has any artists in this role (to offer it as a view).
    pub async fn has_role(
        &self,
        role: ArtistRole,
        library_id: Option<String>,
    ) -> Result<bool, ApiError> {
        let v = self
            .query(commands::has_role(role, library_id.as_deref())?)
            .await?;
        Ok(model::artists_from(&v).total > 0)
    }

    /// An artist's albums, oldest first; with a role, only those where they have it.
    pub async fn artist_albums(
        &self,
        artist_id: i64,
        role: Option<ArtistRole>,
    ) -> Result<Vec<Album>, ApiError> {
        let v = self
            .query(commands::artist_albums(artist_id, role)?)
            .await?;
        Ok(model::albums_from(&v).items)
    }

    pub async fn genres(
        &self,
        start: u32,
        count: u32,
        library_id: Option<String>,
        search: Option<String>,
    ) -> Result<GenrePage, ApiError> {
        let cmd = commands::genres(start, count, library_id.as_deref(), search.as_deref())?;
        Ok(model::genres_from(&self.query(cmd).await?))
    }

    /// One genre, or `None` if the library doesn't have it.
    pub async fn genre(&self, genre_id: i64) -> Result<Option<Genre>, ApiError> {
        let v = self.query(commands::genre_by_id(genre_id)?).await?;
        Ok(model::genres_from(&v).items.into_iter().next())
    }

    /// Years that have albums, newest first as LMS lists them.
    pub async fn years(
        &self,
        start: u32,
        count: u32,
        library_id: Option<String>,
    ) -> Result<YearPage, ApiError> {
        let v = self
            .query(commands::years(start, count, library_id.as_deref())?)
            .await?;
        Ok(model::years_from(&v))
    }

    /// One folder of the music folder: the top level without an id.
    pub async fn music_folder(
        &self,
        start: u32,
        count: u32,
        folder_id: Option<i64>,
    ) -> Result<FolderPage, ApiError> {
        let v = self
            .query(commands::music_folder(start, count, folder_id)?)
            .await?;
        Ok(model::folder_from(&v))
    }

    /// LMS's virtual libraries (empty when it has none).
    pub async fn libraries(&self) -> Result<Vec<VirtualLibrary>, ApiError> {
        Ok(model::libraries_from(
            &self.query(commands::libraries()).await?,
        ))
    }

    /// Favourites, or a folder of them.
    pub async fn favourites(
        &self,
        start: u32,
        count: u32,
        item_id: Option<String>,
    ) -> Result<FavouritePage, ApiError> {
        let v = self
            .query(commands::favourites(start, count, item_id.as_deref())?)
            .await?;
        Ok(model::favourites_from(&v))
    }

    /// Saved playlists.
    pub async fn playlists(&self, start: u32, count: u32) -> Result<PlaylistPage, ApiError> {
        Ok(model::playlists_from(
            &self.query(commands::playlists(start, count)).await?,
        ))
    }

    /// A saved playlist's tracks (the first 500).
    pub async fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>, ApiError> {
        let v = self.query(commands::playlist_tracks(playlist_id)?).await?;
        Ok(model::tracks_from(&v, "playlisttracks_loop"))
    }

    /// Artists, albums and tracks matching `term` (up to `count` of each).
    pub async fn search(&self, term: String, count: u32) -> Result<SearchResults, ApiError> {
        let v = self.query(commands::search(&term, count)?).await?;
        Ok(model::search_from(&v))
    }
}

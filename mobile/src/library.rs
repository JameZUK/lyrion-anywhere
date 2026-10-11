//! Library browsing (no player needed).

use std::sync::Arc;

use crate::commands::{self, AlbumSort};
use crate::error::ApiError;
use crate::model::{self, Album, AlbumPage, ArtistPage, GenrePage, SearchResults, Track};
use crate::session::Session;

#[derive(uniffi::Object)]
pub struct Library {
    session: Arc<Session>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Library {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Library { session })
    }

    /// A page of albums (`count` at most 500), optionally filtered by a search text.
    pub async fn albums(
        &self,
        start: u32,
        count: u32,
        sort: AlbumSort,
        search: Option<String>,
    ) -> Result<AlbumPage, ApiError> {
        let cmd = commands::albums(start, count, sort, search.as_deref());
        Ok(model::albums_from(&self.session.core().rpc("", cmd).await?))
    }

    /// One album, or `None` if the library doesn't have it.
    pub async fn album(&self, album_id: i64) -> Result<Option<Album>, ApiError> {
        let v = self
            .session
            .core()
            .rpc("", commands::album_by_id(album_id)?)
            .await?;
        Ok(model::albums_from(&v).items.into_iter().next())
    }

    /// An album's tracks, in order.
    pub async fn album_tracks(&self, album_id: i64) -> Result<Vec<Track>, ApiError> {
        let v = self
            .session
            .core()
            .rpc("", commands::album_tracks(album_id)?)
            .await?;
        Ok(model::tracks_from(&v, "titles_loop"))
    }

    pub async fn artists(
        &self,
        start: u32,
        count: u32,
        search: Option<String>,
    ) -> Result<ArtistPage, ApiError> {
        let cmd = commands::artists(start, count, search.as_deref());
        Ok(model::artists_from(
            &self.session.core().rpc("", cmd).await?,
        ))
    }

    /// An artist's albums, oldest first.
    pub async fn artist_albums(&self, artist_id: i64) -> Result<Vec<Album>, ApiError> {
        let v = self
            .session
            .core()
            .rpc("", commands::artist_albums(artist_id)?)
            .await?;
        Ok(model::albums_from(&v).items)
    }

    pub async fn genres(&self, start: u32, count: u32) -> Result<GenrePage, ApiError> {
        let v = self
            .session
            .core()
            .rpc("", commands::genres(start, count))
            .await?;
        Ok(model::genres_from(&v))
    }

    /// Artists, albums and tracks matching `term` (up to `count` of each).
    pub async fn search(&self, term: String, count: u32) -> Result<SearchResults, ApiError> {
        let v = self
            .session
            .core()
            .rpc("", commands::search(&term, count)?)
            .await?;
        Ok(model::search_from(&v))
    }
}

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

/// Favourite folders opened at most (one level deep).
const MAX_FAVOURITE_FOLDERS: usize = 20;
/// Most ids taken from favourites.
const MAX_FAVOURITE_IDS: usize = 2000;

/// Library ids found among the favourites.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct FavouriteLibraryItems {
    pub track_ids: Vec<i64>,
    pub album_ids: Vec<i64>,
}

/// A positive library id, as LMS writes them (a number or a string of digits).
fn library_id(v: &Value) -> Option<i64> {
    let n = match v {
        Value::Number(n) => n.as_i64()?,
        Value::String(s)
            if (1..=12).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse().ok()?
        }
        _ => return None,
    };
    (1..=999_999_999_999).contains(&n).then_some(n)
}

/// Add the library ids in one page of `favorites items … menu:1` to `found`, and return
/// the folders to look into. An item's ids may sit on the item itself, in its `params`, or
/// in the parameters of its actions (`play`, `add`, `more`, …).
pub(crate) fn collect_favourite_ids(
    page: &Value,
    found: &mut FavouriteLibraryItems,
) -> Vec<String> {
    let items = page
        .get("item_loop")
        .or_else(|| page.get("loop_loop"))
        .and_then(Value::as_array);
    let mut folders = Vec::new();
    for item in items.into_iter().flatten().take(500) {
        let mut places: Vec<&Value> = vec![item];
        if let Some(p) = item.get("params") {
            places.push(p);
        }
        if let Some(actions) = item.get("actions").and_then(Value::as_object) {
            for a in actions.values().take(16) {
                if let Some(p) = a.get("params") {
                    places.push(p);
                }
            }
        }
        for p in places {
            if let Some(t) = p.get("track_id").and_then(library_id)
                && !found.track_ids.contains(&t)
                && found.track_ids.len() < MAX_FAVOURITE_IDS
            {
                found.track_ids.push(t);
            }
            if let Some(a) = p.get("album_id").and_then(library_id)
                && !found.album_ids.contains(&a)
                && found.album_ids.len() < MAX_FAVOURITE_IDS
            {
                found.album_ids.push(a);
            }
        }
        let has_items = item.get("hasitems").and_then(Value::as_i64) == Some(1)
            || item.get("hasitems").and_then(Value::as_str) == Some("1");
        let audio = item.get("isaudio").and_then(Value::as_i64) == Some(1);
        if has_items
            && !audio
            && let Some(id) = item.get("id").and_then(Value::as_str)
            && commands::item_id(id).is_ok()
        {
            folders.push(id.to_string());
        }
    }
    folders
}

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

    /// Library tracks and albums among the user's LMS favourites (one folder level deep),
    /// for "Keep favourites offline". Only favourites LMS identifies by library id are found:
    /// streams, radio and favourites saved by file location are not library downloads.
    pub async fn favourite_library_items(&self) -> Result<FavouriteLibraryItems, ApiError> {
        let mut found = FavouriteLibraryItems::default();
        let top = self.query(commands::favourite_menu(None)?).await?;
        let folders = collect_favourite_ids(&top, &mut found);
        for f in folders.into_iter().take(MAX_FAVOURITE_FOLDERS) {
            // A folder that fails to open is skipped; the rest still count.
            if let Ok(v) = self.query(commands::favourite_menu(Some(&f))?).await {
                collect_favourite_ids(&v, &mut found);
            }
        }
        Ok(found)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn favourite_ids_from_items_params_and_actions() {
        let page = json!({"item_loop": [
            {"text": "Song", "isaudio": 1, "track_id": "12"},
            {"text": "Record", "hasitems": 1, "id": "f0.1",
             "actions": {"more": {"cmd": ["albuminfo", "items"], "params": {"album_id": 34}}}},
            {"text": "Live", "isaudio": 1, "params": {"track_id": 56, "item_id": "f0.2"}},
            {"text": "Radio", "isaudio": 1, "id": "f0.3"},
            {"text": "Folder", "hasitems": "1", "id": "f0.4"},
            {"text": "Hostile", "track_id": "-5", "album_id": "1e3",
             "actions": {"play": {"params": {"track_id": "12; rm -rf /"}}}},
            {"text": "Bad folder", "hasitems": 1, "id": "../../x"},
            {"text": "Dup", "track_id": 12}
        ]});
        let mut found = FavouriteLibraryItems::default();
        let folders = collect_favourite_ids(&page, &mut found);
        assert_eq!(found.track_ids, vec![12, 56]);
        assert_eq!(found.album_ids, vec![34]);
        assert_eq!(folders, vec!["f0.1", "f0.4"]);
        // The plain CLI form, and nonsense.
        let mut found = FavouriteLibraryItems::default();
        collect_favourite_ids(&json!({"loop_loop": [{"track_id": 7}]}), &mut found);
        assert_eq!(found.track_ids, vec![7]);
        for junk in [
            json!(null),
            json!([]),
            json!({"item_loop": "x"}),
            json!({"item_loop": [1, "a", null]}),
        ] {
            let mut f = FavouriteLibraryItems::default();
            assert!(collect_favourite_ids(&junk, &mut f).is_empty());
            assert_eq!(f, FavouriteLibraryItems::default());
        }
    }

    #[test]
    fn favourite_menus_are_well_formed() {
        assert_eq!(
            commands::favourite_menu(Some("f0.1")).unwrap(),
            vec![
                json!("favorites"),
                json!("items"),
                json!(0),
                json!(200),
                json!("item_id:f0.1"),
                json!("menu:1")
            ]
        );
        assert!(commands::favourite_menu(Some("x y")).is_err());
    }
}

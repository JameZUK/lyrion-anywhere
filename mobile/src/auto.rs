//! The browse tree for cars: Android Auto now, CarPlay later.
//!
//! The car asks for children by an opaque media id and plays items by id. Ids come back
//! from outside the app (the car's media centre), so they are untrusted: [`parse_node`]
//! accepts only the grammar [`node_id`] writes, with numeric ids in the gateway's range,
//! favourite ids in its item-id alphabet and download references in their own short form.
//! Nothing in an id, or anywhere in the tree, is a URL, a path or a server address: the
//! app looks items up by these ids and plays them on this phone's player (SPEC D20), or
//! from its downloads when offline.
//!
//! This module is the tree's shape (which nodes exist, in what order, what can be browsed
//! or played) and the id grammar, as pure functions; the platform side fetches the
//! contents and maps them to its media items.

/// Longest media id accepted.
const MAX_ID: usize = 200;
/// Highest playlist position an id may name.
const MAX_INDEX: u32 = 9_999;
/// Largest page the car may ask for, and the furthest it may page.
pub const MAX_PAGE_SIZE: u32 = 100;
pub const MAX_START: u32 = 100_000;

/// A node of the car's browse tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BrowseNode {
    /// The root: the tabs.
    Root,
    /// Tab: what's playing, new music, favourites and playlists.
    Home,
    /// Tab: the library by category.
    Library,
    /// Tab: what is downloaded on this phone (the whole tree when offline).
    Downloads,
    /// Carry on with this phone's queue.
    Resume,
    /// Recently added albums.
    NewMusic,
    Albums,
    Artists,
    Playlists,
    Favourites,
    Album {
        id: i64,
    },
    Artist {
        id: i64,
    },
    Playlist {
        id: i64,
    },
    /// A folder of LMS favourites.
    FavouriteFolder {
        item_id: String,
    },
    /// A favourite that plays (a station, a track, an app item).
    Favourite {
        item_id: String,
    },
    /// Track `index` of an album: plays the album from there.
    AlbumTrack {
        album_id: i64,
        index: u32,
    },
    /// Track `index` of a playlist: plays the playlist from there.
    PlaylistTrack {
        playlist_id: i64,
        index: u32,
    },
    /// A downloaded album (by its download reference).
    DownloadedAlbum {
        album_ref: String,
    },
    /// Track `index` of a downloaded album: plays the album from there.
    DownloadedTrack {
        album_ref: String,
        index: u32,
    },
}

fn id_ok(id: i64) -> bool {
    (1..=999_999_999_999).contains(&id)
}

/// LMS favourite ids, in the gateway's item-id alphabet.
fn item_id_ok(s: &str) -> bool {
    (1..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// A download's album reference: `a<album id>` or `t<16 hex digits of a track key>`.
fn album_ref_ok(s: &str) -> bool {
    match s.as_bytes() {
        [b'a', rest @ ..] => {
            (1..=12).contains(&rest.len()) && rest.iter().all(u8::is_ascii_digit) && rest[0] != b'0'
        }
        [b't', rest @ ..] => {
            rest.len() == 16
                && rest
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        }
        _ => false,
    }
}

fn num(s: &str) -> Option<i64> {
    if s.is_empty() || s.len() > 12 || !s.bytes().all(|b| b.is_ascii_digit()) || s.starts_with('0')
    {
        return None;
    }
    s.parse().ok().filter(|v| id_ok(*v))
}

fn index(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse().ok().filter(|v| *v <= MAX_INDEX)
}

/// The media id for a node (`None` if the node holds a value outside the grammar, which
/// the app never creates).
#[uniffi::export]
pub fn node_id(node: BrowseNode) -> Option<String> {
    let s = match &node {
        BrowseNode::Root => "root".into(),
        BrowseNode::Home => "home".into(),
        BrowseNode::Library => "library".into(),
        BrowseNode::Downloads => "downloads".into(),
        BrowseNode::Resume => "resume".into(),
        BrowseNode::NewMusic => "new".into(),
        BrowseNode::Albums => "albums".into(),
        BrowseNode::Artists => "artists".into(),
        BrowseNode::Playlists => "playlists".into(),
        BrowseNode::Favourites => "favourites".into(),
        BrowseNode::Album { id } => format!("al/{id}"),
        BrowseNode::Artist { id } => format!("ar/{id}"),
        BrowseNode::Playlist { id } => format!("pl/{id}"),
        BrowseNode::FavouriteFolder { item_id } => format!("ff/{item_id}"),
        BrowseNode::Favourite { item_id } => format!("fv/{item_id}"),
        BrowseNode::AlbumTrack { album_id, index } => format!("at/{album_id}/{index}"),
        BrowseNode::PlaylistTrack { playlist_id, index } => format!("pt/{playlist_id}/{index}"),
        BrowseNode::DownloadedAlbum { album_ref } => format!("da/{album_ref}"),
        BrowseNode::DownloadedTrack { album_ref, index } => format!("dt/{album_ref}/{index}"),
    };
    // Only what parses back to the same node is ever handed out.
    (parse_node(s.clone()).as_ref() == Some(&node)).then_some(s)
}

/// The node a media id names, or `None` for anything outside the grammar.
#[uniffi::export]
pub fn parse_node(id: String) -> Option<BrowseNode> {
    if id.is_empty() || id.len() > MAX_ID || !id.is_ascii() {
        return None;
    }
    let parts: Vec<&str> = id.split('/').collect();
    Some(match parts.as_slice() {
        ["root"] => BrowseNode::Root,
        ["home"] => BrowseNode::Home,
        ["library"] => BrowseNode::Library,
        ["downloads"] => BrowseNode::Downloads,
        ["resume"] => BrowseNode::Resume,
        ["new"] => BrowseNode::NewMusic,
        ["albums"] => BrowseNode::Albums,
        ["artists"] => BrowseNode::Artists,
        ["playlists"] => BrowseNode::Playlists,
        ["favourites"] => BrowseNode::Favourites,
        ["al", n] => BrowseNode::Album { id: num(n)? },
        ["ar", n] => BrowseNode::Artist { id: num(n)? },
        ["pl", n] => BrowseNode::Playlist { id: num(n)? },
        ["ff", i] if item_id_ok(i) => BrowseNode::FavouriteFolder {
            item_id: (*i).into(),
        },
        ["fv", i] if item_id_ok(i) => BrowseNode::Favourite {
            item_id: (*i).into(),
        },
        ["at", n, i] => BrowseNode::AlbumTrack {
            album_id: num(n)?,
            index: index(i)?,
        },
        ["pt", n, i] => BrowseNode::PlaylistTrack {
            playlist_id: num(n)?,
            index: index(i)?,
        },
        ["da", r] if album_ref_ok(r) => BrowseNode::DownloadedAlbum {
            album_ref: (*r).into(),
        },
        ["dt", r, i] if album_ref_ok(r) => BrowseNode::DownloadedTrack {
            album_ref: (*r).into(),
            index: index(i)?,
        },
        _ => return None,
    })
}

/// Can the car open this node (list its children)?
#[uniffi::export]
pub fn node_browsable(node: BrowseNode) -> bool {
    !matches!(
        node,
        BrowseNode::Resume
            | BrowseNode::Favourite { .. }
            | BrowseNode::AlbumTrack { .. }
            | BrowseNode::PlaylistTrack { .. }
            | BrowseNode::DownloadedTrack { .. }
    )
}

/// Can the car play this node? (Albums, playlists and downloaded albums play whole.)
#[uniffi::export]
pub fn node_playable(node: BrowseNode) -> bool {
    matches!(
        node,
        BrowseNode::Resume
            | BrowseNode::Album { .. }
            | BrowseNode::Playlist { .. }
            | BrowseNode::Favourite { .. }
            | BrowseNode::AlbumTrack { .. }
            | BrowseNode::PlaylistTrack { .. }
            | BrowseNode::DownloadedAlbum { .. }
            | BrowseNode::DownloadedTrack { .. }
    )
}

/// Does playing it need the server (rather than the downloads)?
#[uniffi::export]
pub fn node_needs_server(node: BrowseNode) -> bool {
    !matches!(
        node,
        BrowseNode::Root
            | BrowseNode::Downloads
            | BrowseNode::DownloadedAlbum { .. }
            | BrowseNode::DownloadedTrack { .. }
    )
}

/// The tabs. Online: Home and Library, and Downloads when there are any. Offline: only
/// Downloads (the server's tree can't be reached).
#[uniffi::export]
pub fn browse_root(online: bool, has_downloads: bool) -> Vec<BrowseNode> {
    if !online {
        return vec![BrowseNode::Downloads];
    }
    let mut v = vec![BrowseNode::Home, BrowseNode::Library];
    if has_downloads {
        v.push(BrowseNode::Downloads);
    }
    v
}

/// The Home tab's entries: carry on (when the phone has a queue), then new music,
/// favourites and playlists.
#[uniffi::export]
pub fn browse_home(has_queue: bool) -> Vec<BrowseNode> {
    let mut v = Vec::new();
    if has_queue {
        v.push(BrowseNode::Resume);
    }
    v.extend([
        BrowseNode::NewMusic,
        BrowseNode::Favourites,
        BrowseNode::Playlists,
    ]);
    v
}

/// The Library tab's categories.
#[uniffi::export]
pub fn browse_library() -> Vec<BrowseNode> {
    vec![
        BrowseNode::NewMusic,
        BrowseNode::Albums,
        BrowseNode::Artists,
        BrowseNode::Playlists,
        BrowseNode::Favourites,
    ]
}

/// The (start, count) of a page the car asked for, bounded (`None`: past the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PageWindow {
    pub start: u32,
    pub count: u32,
}

#[uniffi::export]
pub fn page_window(page: i32, page_size: i32) -> Option<PageWindow> {
    let size = u32::try_from(page_size)
        .ok()
        .filter(|s| *s > 0)
        .unwrap_or(MAX_PAGE_SIZE);
    let size = size.min(MAX_PAGE_SIZE);
    let page = u32::try_from(page).ok()?;
    let start = page.checked_mul(size).filter(|s| *s < MAX_START)?;
    Some(PageWindow { start, count: size })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_kinds() -> Vec<BrowseNode> {
        vec![
            BrowseNode::Root,
            BrowseNode::Home,
            BrowseNode::Library,
            BrowseNode::Downloads,
            BrowseNode::Resume,
            BrowseNode::NewMusic,
            BrowseNode::Albums,
            BrowseNode::Artists,
            BrowseNode::Playlists,
            BrowseNode::Favourites,
            BrowseNode::Album { id: 7 },
            BrowseNode::Artist {
                id: 999_999_999_999,
            },
            BrowseNode::Playlist { id: 1 },
            BrowseNode::FavouriteFolder {
                item_id: "b99447ab.1".into(),
            },
            BrowseNode::Favourite {
                item_id: "b99447ab.1.2".into(),
            },
            BrowseNode::AlbumTrack {
                album_id: 7,
                index: 0,
            },
            BrowseNode::PlaylistTrack {
                playlist_id: 3,
                index: 9_999,
            },
            BrowseNode::DownloadedAlbum {
                album_ref: "a42".into(),
            },
            BrowseNode::DownloadedTrack {
                album_ref: "t0123456789abcdef".into(),
                index: 12,
            },
        ]
    }

    #[test]
    fn ids_round_trip() {
        for n in all_kinds() {
            let id = node_id(n.clone()).unwrap_or_else(|| panic!("{n:?}"));
            assert!(id.len() <= MAX_ID && id.is_ascii());
            assert!(!id.contains("://") && !id.starts_with('/'), "{id}");
            assert_eq!(parse_node(id), Some(n));
        }
    }

    #[test]
    fn nodes_outside_the_grammar_get_no_id() {
        for n in [
            BrowseNode::Album { id: 0 },
            BrowseNode::Album { id: -4 },
            BrowseNode::Artist {
                id: 1_000_000_000_000,
            },
            BrowseNode::Favourite {
                item_id: "a/b".into(),
            },
            BrowseNode::FavouriteFolder {
                item_id: String::new(),
            },
            BrowseNode::AlbumTrack {
                album_id: 1,
                index: 10_000,
            },
            BrowseNode::DownloadedAlbum {
                album_ref: "/srv/music".into(),
            },
            BrowseNode::DownloadedTrack {
                album_ref: "tXYZ".into(),
                index: 0,
            },
        ] {
            assert_eq!(node_id(n.clone()), None, "{n:?}");
        }
    }

    #[test]
    fn hostile_ids_are_refused() {
        for bad in [
            "",
            "/",
            "root/",
            "ROOT",
            "al/",
            "al/0",
            "al/-1",
            "al/+1",
            "al/01",
            "al/1.5",
            "al/1e3",
            "al/1000000000000",
            "al/7/8",
            "ar/x",
            "pl/ 1",
            "ff/",
            "ff/a b",
            "ff/../../etc",
            "fv/http:x",
            "fv/a%2Fb",
            "at/1",
            "at/1/",
            "at/1/10000",
            "at/1/-1",
            "at/1/01",
            "pt/x/1",
            "da/",
            "da/a",
            "da/a0",
            "da/../x",
            "da/t0123",
            "da/t0123456789ABCDEF",
            "dt/a1",
            "dt/a1/x",
            "file:///srv/music/a.flac",
            "https://music.example.com/x",
            "/srv/music",
            "\\\\server\\share",
            "al/1\n",
            "resume/1",
        ] {
            assert_eq!(parse_node(bad.into()), None, "{bad:?}");
        }
        assert_eq!(parse_node(format!("ff/{}", "a".repeat(129))), None);
        assert_eq!(parse_node("x".repeat(201)), None);
        assert_eq!(parse_node("al/é".into()), None);
    }

    #[test]
    fn arbitrary_ids_never_panic_and_only_valid_ones_parse() {
        let alphabet = b"al/rtfvdp0123456789.-_ xA%:\\\n";
        let mut seed = 0x0000_ddb1_a5e5_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..50_000 {
            let len = (next() % 24) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize] as char)
                .collect();
            if let Some(n) = parse_node(s.clone()) {
                // Whatever parses is exactly what the app would have written.
                assert_eq!(node_id(n).as_deref(), Some(s.as_str()));
            }
        }
    }

    #[test]
    fn browsable_and_playable() {
        for n in all_kinds() {
            assert!(
                node_browsable(n.clone()) || node_playable(n.clone()),
                "{n:?} is neither"
            );
        }
        assert!(
            node_browsable(BrowseNode::Album { id: 1 })
                && node_playable(BrowseNode::Album { id: 1 })
        );
        assert!(!node_browsable(BrowseNode::Resume) && node_playable(BrowseNode::Resume));
        assert!(!node_playable(BrowseNode::Artist { id: 1 }));
        assert!(!node_playable(BrowseNode::Root) && !node_playable(BrowseNode::Downloads));
        assert!(!node_needs_server(BrowseNode::DownloadedTrack {
            album_ref: "a1".into(),
            index: 0
        }));
        assert!(node_needs_server(BrowseNode::Album { id: 1 }));
    }

    #[test]
    fn the_tabs_follow_the_connection_and_the_downloads() {
        assert_eq!(
            browse_root(true, true),
            [BrowseNode::Home, BrowseNode::Library, BrowseNode::Downloads]
        );
        assert_eq!(
            browse_root(true, false),
            [BrowseNode::Home, BrowseNode::Library]
        );
        assert_eq!(browse_root(false, false), [BrowseNode::Downloads]);
        assert_eq!(browse_root(false, true), [BrowseNode::Downloads]);
        assert!(
            browse_root(true, true).len() <= 4,
            "Android Auto shows at most four tabs"
        );
        assert_eq!(browse_home(true)[0], BrowseNode::Resume);
        assert!(!browse_home(false).contains(&BrowseNode::Resume));
        assert!(browse_home(false).contains(&BrowseNode::NewMusic));
        assert_eq!(browse_library()[0], BrowseNode::NewMusic);
        for n in browse_root(true, true)
            .into_iter()
            .chain(browse_home(true))
            .chain(browse_library())
        {
            assert!(node_id(n).is_some());
        }
    }

    #[test]
    fn pages_are_bounded() {
        assert_eq!(
            page_window(0, 20),
            Some(PageWindow {
                start: 0,
                count: 20
            })
        );
        assert_eq!(
            page_window(3, 20),
            Some(PageWindow {
                start: 60,
                count: 20
            })
        );
        assert_eq!(
            page_window(0, 10_000),
            Some(PageWindow {
                start: 0,
                count: 100
            })
        );
        assert_eq!(
            page_window(0, 0),
            Some(PageWindow {
                start: 0,
                count: 100
            })
        );
        assert_eq!(
            page_window(0, -5),
            Some(PageWindow {
                start: 0,
                count: 100
            })
        );
        assert_eq!(page_window(-1, 20), None);
        assert_eq!(page_window(i32::MAX, 100), None);
        assert_eq!(page_window(1000, 100), None);
    }
}

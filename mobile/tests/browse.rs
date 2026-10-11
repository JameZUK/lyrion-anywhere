//! Browsing (genres, years, folders, favourites, playlists, virtual libraries) and LMS menus
//! (apps, radio, plugins, "More…") against the real gateway: every command the app builds
//! must pass its allow-list, menu actions must be ones LMS offered, and anything else must be
//! refused before it reaches LMS.

mod common;

use anywhere_mobile::{
    AlbumSort, ApiError, ArtistRole, FolderKind, InfoKind, LibraryFilter, MenuAction, MenuItemKind,
    MenuOutcome, MenuParam, Menus, MusicLibrary, Placement, Players, Selection, Suitability,
    app_menu, info_menu,
};
use common::*;
use serde_json::{Value, json};

fn calls_since(h: &Harness, before: usize) -> Vec<Value> {
    h.lms.rpc_calls()[before..].to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_browse_command_passes_the_allow_list() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let lib = MusicLibrary::new(s.clone());
    let players = Players::new(s.clone());

    let everything = LibraryFilter {
        genre_id: Some(4),
        year: Some(1999),
        artist_id: Some(3),
        compilation: Some(true),
        role: Some(ArtistRole::AlbumArtist),
        library_id: Some("ab12".into()),
    };
    for sort in [
        AlbumSort::Title,
        AlbumSort::Artist,
        AlbumSort::Year,
        AlbumSort::Newest,
        AlbumSort::Random,
    ] {
        lib.albums(0, 120, sort, Some("live".into()), everything.clone())
            .await
            .unwrap();
    }
    for role in [
        ArtistRole::AlbumArtist,
        ArtistRole::Artist,
        ArtistRole::Composer,
        ArtistRole::Conductor,
        ArtistRole::Band,
        ArtistRole::TrackArtist,
    ] {
        let f = LibraryFilter {
            role: Some(role),
            genre_id: Some(4),
            library_id: Some("ab12".into()),
            ..Default::default()
        };
        lib.artists(0, 200, None, f).await.unwrap();
        assert!(lib.has_role(role, Some("ab12".into())).await.unwrap());
        lib.artist_albums(3, Some(role)).await.unwrap();
    }
    let compilations = LibraryFilter {
        compilation: Some(true),
        ..Default::default()
    };
    lib.albums(0, 120, AlbumSort::Title, None, compilations)
        .await
        .unwrap();

    let genres = lib
        .genres(0, 100, Some("ab12".into()), Some("ja".into()))
        .await
        .unwrap();
    assert_eq!(genres.items[0].name, "Jazz");
    assert_eq!(lib.genre(4).await.unwrap().unwrap().name, "Jazz");
    assert_eq!(
        lib.years(0, 100, None).await.unwrap().items,
        vec![2001, 1999]
    );
    lib.years(0, 100, Some("ab12".into())).await.unwrap();

    let top = lib.music_folder(0, 200, None).await.unwrap();
    assert_eq!(top.items[0].kind, FolderKind::Folder);
    assert_eq!(top.items[1].duration.as_deref(), Some("1:01"));
    lib.music_folder(0, 200, Some(12)).await.unwrap();

    let libs = lib.libraries().await.unwrap();
    assert_eq!(libs[0].id, "ab12");
    assert_eq!(libs[0].name, "Classical");

    let fav = lib.favourites(0, 500, None).await.unwrap();
    assert_eq!(fav.items.len(), 2);
    assert!(fav.items[1].is_folder);
    lib.favourites(0, 500, Some("f0.1".into())).await.unwrap();

    players.sync_groups().await.unwrap();
    let pl = lib.playlists(0, 100).await.unwrap();
    assert_eq!(pl.items[0].name, "Road trip");
    assert_eq!(lib.playlist_tracks(5).await.unwrap()[0].title, "One");

    for sel in [
        Selection::Playlist { id: 5 },
        Selection::Genre { id: 4 },
        Selection::Year { year: 1999 },
        Selection::Year { year: 0 },
        Selection::Folder { id: 12 },
    ] {
        for placement in [
            Placement::PlayNow,
            Placement::PlayNext,
            Placement::AddToQueue,
        ] {
            players
                .enqueue(me.clone(), sel, placement, None)
                .await
                .unwrap();
        }
    }
    for placement in [
        Placement::PlayNow,
        Placement::PlayNext,
        Placement::AddToQueue,
    ] {
        players
            .play_favourite(me.clone(), "f0.0".into(), placement)
            .await
            .unwrap();
    }

    let calls = h.lms.rpc_calls();
    for want in [
        json!([
            "",
            [
                "albums",
                "0",
                "120",
                "tags:aljySw",
                "sort:new",
                "search:live",
                "genre_id:4",
                "year:1999",
                "artist_id:3",
                "compilation:1",
                "role_id:ALBUMARTIST",
                "library_id:ab12"
            ]
        ]),
        json!(["", ["musicfolder", "0", "200", "folder_id:12", "tags:cdt"]]),
        json!([me, ["playlistcontrol", "cmd:load", "year:0"]]),
        json!([me, ["favorites", "playlist", "insert", "item_id:f0.0"]]),
    ] {
        assert!(calls.contains(&want), "{want} not sent");
    }
    // The file's path in the folder listing never reaches the app.
    assert!(!format!("{top:?}").contains("/srv/"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apps_radio_and_plugins_open_through_the_gateway() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let menus = Menus::new(s.clone());

    let apps = menus.apps().await.unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].cmd, "someapp");
    assert_eq!(
        apps[0].icon.as_deref(),
        Some("plugins/SomeApp/html/images/icon.png")
    );
    let radios = menus.radios().await.unwrap();
    assert_eq!(radios[0].cmd, "someradio");

    // Plugins from the home menu, except what is listed as an app already.
    let plugins = menus
        .plugins(me.clone(), vec!["someapp".into()])
        .await
        .unwrap();
    let ids: Vec<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["randomplay", "pluginSomePlugin"]);
    assert_eq!(plugins[0].children[0].kind, MenuItemKind::Action);

    // An app's top menu, then its search box, then something to play.
    let top = menus
        .open(me.clone(), apps[0].menu.clone(), 0, 100, None)
        .await
        .unwrap();
    assert_eq!(top.items[0].kind, MenuItemKind::Search);
    assert_eq!(top.items[1].kind, MenuItemKind::Playable);
    let search = top.items[0].go.clone().unwrap();
    menus
        .open(me.clone(), search.clone(), 0, 100, Some("abba gold".into()))
        .await
        .unwrap();
    let play = top.items[1].play.clone().unwrap();
    // (The fake LMS answers every command of the app with its menu.)
    assert!(matches!(
        menus.run(me.clone(), play, None).await.unwrap(),
        MenuOutcome::Menu { .. }
    ));

    // A plugin entry's menu, and its suitability (it only says it needs setting up).
    let plugin = plugins[1].go.clone().unwrap();
    assert_eq!(
        menus.probe(me.clone(), plugin).await,
        Suitability::NeedsSetup {
            message: "Log in to Some Plugin in its settings".into()
        }
    );
    assert_eq!(
        menus.probe(me.clone(), apps[0].menu.clone()).await,
        Suitability::Usable
    );
    // A random-mix entry runs a learned action.
    let mix = plugins[0].children[0].go.clone().unwrap();
    menus.run(me.clone(), mix, None).await.unwrap();

    let calls = h.lms.rpc_calls();
    assert!(calls.contains(&json!([
        me,
        [
            "someapp",
            "items",
            "0",
            "100",
            "item_id:0",
            "menu:someapp",
            "search:abba gold"
        ]
    ])));
    assert!(calls.contains(&json!([me, ["someapp", "playlist", "play", "item_id:1"]])));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_menus_offer_plugin_actions_and_never_paths() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let menus = Menus::new(s.clone());

    for (kind, id) in [
        (InfoKind::Track, 7),
        (InfoKind::Album, 7),
        (InfoKind::Artist, 3),
        (InfoKind::Genre, 4),
        (InfoKind::Year, 1999),
        (InfoKind::Folder, 12),
        (InfoKind::Playlist, 5),
    ] {
        let page = menus
            .open(me.clone(), info_menu(kind, id).unwrap(), 0, 100, None)
            .await
            .unwrap();
        // The gateway drops the file's location; the rest arrives.
        assert_eq!(page.items.len(), 3, "{kind:?}");
        assert!(page.items.iter().all(|i| !i.text.contains("/srv/")));
    }
    let page = menus
        .open(
            me.clone(),
            info_menu(InfoKind::Track, 7).unwrap(),
            0,
            100,
            None,
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].kind, MenuItemKind::Folder);
    assert_eq!(page.items[2].kind, MenuItemKind::Text);
    // "More like this" (a plugin's action) runs through the gateway's learned actions.
    let like = page.items[1].go.clone().unwrap();
    assert_eq!(like.next_window.as_deref(), Some("nowPlaying"));
    let before = h.lms.rpc_calls().len();
    menus.run(me.clone(), like, None).await.unwrap();
    // Only that reached LMS (besides the gateway refreshing its list of apps).
    let sent: Vec<Value> = calls_since(&h, before)
        .into_iter()
        .filter(|c| !matches!(c[1][0].as_str(), Some("apps" | "radios")))
        .collect();
    assert_eq!(sent, vec![json!([me, ["someplugin", "mix", "track_id:7"]])]);
    // A library link from the menu opens as a menu.
    let link = page.items[0].go.clone().unwrap();
    menus.open(me.clone(), link, 0, 100, None).await.unwrap();
}

/// Actions LMS never offered, or offered ones changed, are refused by the gateway; malformed
/// ones never leave the device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_menu_actions_are_refused() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let menus = Menus::new(s.clone());
    // Learn the context menu's actions first.
    let page = menus
        .open(
            me.clone(),
            info_menu(InfoKind::Track, 7).unwrap(),
            0,
            100,
            None,
        )
        .await
        .unwrap();
    let like = page.items[1].go.clone().unwrap();

    let before = h.lms.rpc_calls().len();
    let forged = [
        // Never offered.
        MenuAction {
            cmd: vec!["someplugin".into(), "wipe".into()],
            params: vec![],
            next_window: None,
            opens_menu: false,
            needs_input: false,
        },
        // Offered, but for another track.
        MenuAction {
            params: vec![MenuParam {
                key: "track_id".into(),
                value: "8".into(),
            }],
            ..like.clone()
        },
        // An extra parameter.
        MenuAction {
            params: vec![
                MenuParam {
                    key: "track_id".into(),
                    value: "7".into(),
                },
                MenuParam {
                    key: "folder".into(),
                    value: "x".into(),
                },
            ],
            ..like.clone()
        },
        // Server administration, whatever a menu says.
        MenuAction {
            cmd: vec!["pref".into(), "audiodir".into()],
            params: vec![],
            next_window: None,
            opens_menu: false,
            needs_input: false,
        },
    ];
    for a in forged {
        assert!(
            matches!(
                menus.run(me.clone(), a.clone(), None).await,
                Err(ApiError::Forbidden { .. })
            ),
            "{a:?} was not refused"
        );
    }
    // An app the gateway doesn't know.
    assert!(matches!(
        menus
            .open(
                me.clone(),
                app_menu("unknownapp".into()).unwrap(),
                0,
                10,
                None
            )
            .await,
        Err(ApiError::Forbidden { .. })
    ));
    for call in calls_since(&h, before) {
        let cmd = call[1][0].as_str().unwrap_or("");
        assert!(
            matches!(cmd, "apps" | "radios" | "menu"),
            "{call} reached LMS"
        );
    }

    // Malformed: refused on the device.
    let before = h.lms.rpc_calls().len();
    let url = MenuAction {
        params: vec![MenuParam {
            key: "url".into(),
            value: "file:///srv/music".into(),
        }],
        ..like.clone()
    };
    assert!(matches!(
        menus.run(me.clone(), url, None).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        menus.run("../x".into(), like.clone(), None).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        menus.open(me.clone(), like, 0, 10, None).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(
        calls_since(&h, before)
            .iter()
            .all(|c| c[1][0] != "someplugin"),
        "a malformed action reached LMS"
    );
    // A listener may not control someone else's player through menus either.
    assert!(matches!(
        menus
            .open(
                HOME_PLAYER.into(),
                info_menu(InfoKind::Track, 7).unwrap(),
                0,
                10,
                None
            )
            .await,
        Err(ApiError::Forbidden { .. })
    ));
}

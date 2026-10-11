//! Browsing, remote control and artwork against the real gateway: every command the app
//! can send must pass the gateway's allow-list, and refusals must surface as `Forbidden`.

mod common;

use anywhere_mobile::{
    AlbumSort, ApiError, Art, MusicLibrary, Placement, PlaybackMode, Players, RepeatMode,
    Selection, ShuffleMode,
};
use common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_calls_parse_through_the_gateway() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let lib = MusicLibrary::new(s.clone());

    let page = lib.albums(0, 100, AlbumSort::Title, None).await.unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].title, "First Album");
    assert_eq!(
        page.items[0].artwork.as_deref(),
        Some("music/abcdef12/cover")
    );
    assert_eq!(page.items[0].artist_id, Some(3));

    let tracks = lib.album_tracks(7).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].duration_secs, Some(61.5));
    assert_eq!(tracks[1].track_number, Some(2));

    assert_eq!(
        lib.artists(0, 50, None).await.unwrap().items[0].name,
        "Artist"
    );
    assert_eq!(lib.genres(0, 50).await.unwrap().items[0].name, "Jazz");
    let found = lib.search("first".into(), 10).await.unwrap();
    assert_eq!(found.albums[0].id, 7);
    assert_eq!(found.tracks[0].id, Some(1));
    assert_eq!(found.artists[0].id, 3);
    assert_eq!(lib.album(7).await.unwrap().unwrap().id, 7);
    assert_eq!(lib.artist_albums(3).await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_command_the_app_sends_passes_the_allow_list() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let lib = MusicLibrary::new(s.clone());
    let players = Players::new(s.clone());

    for sort in [
        AlbumSort::Title,
        AlbumSort::Artist,
        AlbumSort::Year,
        AlbumSort::Newest,
        AlbumSort::Random,
    ] {
        lib.albums(500, 500, sort, Some("AC/DC Live 1990/91".into()))
            .await
            .unwrap();
    }
    lib.album(7).await.unwrap();
    lib.album_tracks(7).await.unwrap();
    lib.artists(0, 500, Some("beatles".into())).await.unwrap();
    lib.artist_albums(12).await.unwrap();
    lib.genres(0, 100).await.unwrap();
    lib.search("abba gold".into(), 20).await.unwrap();
    players.list().await.unwrap();

    let p = || me.clone();
    let status = players.status(p()).await.unwrap();
    assert_eq!(status.mode, PlaybackMode::Playing);
    assert_eq!(status.volume, 40);
    players.queue(p(), 0, 100).await.unwrap();
    players.play(p()).await.unwrap();
    players.set_paused(p(), true).await.unwrap();
    players.set_paused(p(), false).await.unwrap();
    players.toggle_pause(p()).await.unwrap();
    players.stop(p()).await.unwrap();
    players.next(p()).await.unwrap();
    players.previous(p()).await.unwrap();
    players.jump_to(p(), 3).await.unwrap();
    players.seek(p(), 83.25).await.unwrap();
    players.set_volume(p(), 55).await.unwrap();
    players.toggle_mute(p()).await.unwrap();
    for m in [ShuffleMode::Off, ShuffleMode::Songs, ShuffleMode::Albums] {
        players.set_shuffle(p(), m).await.unwrap();
    }
    for m in [RepeatMode::Off, RepeatMode::One, RepeatMode::All] {
        players.set_repeat(p(), m).await.unwrap();
    }
    players.set_power(p(), true).await.unwrap();
    for sel in [
        Selection::Album { id: 7 },
        Selection::Track { id: 1 },
        Selection::Artist { id: 3 },
    ] {
        for placement in [
            Placement::PlayNow,
            Placement::PlayNext,
            Placement::AddToQueue,
        ] {
            players.enqueue(p(), sel, placement, Some(1)).await.unwrap();
        }
    }

    // LMS saw exactly what the app built (the gateway passes every argument as text), for
    // the app's own player.
    let calls = h.lms.rpc_calls();
    for want in [
        json!([
            me,
            ["playlistcontrol", "cmd:load", "album_id:7", "play_index:1"]
        ]),
        json!([me, ["time", "83.3"]]),
        json!([me, ["mixer", "volume", "55"]]),
        json!([
            "",
            [
                "albums",
                "500",
                "500",
                "tags:aljyS",
                "sort:new",
                "search:AC/DC Live 1990/91"
            ]
        ]),
    ] {
        assert!(calls.contains(&want), "{want} not sent");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusals_surface_as_forbidden() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let players = Players::new(s.clone());
    let before = h.lms.rpc_calls().len();

    // Commands outside the allow-list.
    for cmd in [
        json!(["pref", "audiodir", "?"]),
        json!(["readdirectory", 0, 10, "folder:/"]),
        json!(["playlist", "play", "file:///srv/music/x.flac"]),
        json!(["rescan"]),
        json!(["albums", 0, 100000]),
    ] {
        let raw = cmd.as_array().unwrap().clone();
        assert!(
            matches!(s.rpc_raw("", raw).await, Err(ApiError::Forbidden { .. })),
            "{cmd} was not refused"
        );
    }
    // A player this listener may not control.
    assert!(matches!(
        players.play(HOME_PLAYER.into()).await,
        Err(ApiError::Forbidden { .. })
    ));
    // Someone else's phone.
    let (bob, _) = {
        let store = MemStore::new();
        let b = h.session(store.clone());
        b.login("bob".into(), PW.into(), None).await.unwrap();
        (b, store)
    };
    let bobs_player = bob.current().unwrap().device.player_id;
    assert!(matches!(
        players.set_volume(bobs_player, 100).await,
        Err(ApiError::Forbidden { .. })
    ));
    // Nothing refused reached LMS (the gateway may refresh its list of apps meanwhile).
    for call in &h.lms.rpc_calls()[before..] {
        let cmd = call[1][0].as_str().unwrap_or("");
        assert!(
            matches!(cmd, "apps" | "radios" | "menu"),
            "{call} reached LMS"
        );
    }

    // Malformed ids never leave the device.
    for bad in ["", "../../x", "aa:bb:cc:dd:ee", "AA-BB-CC-DD-EE-01"] {
        assert!(matches!(
            players.play(bad.into()).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        MusicLibrary::new(s.clone()).album_tracks(-1).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        players
            .enqueue(
                s.current().unwrap().device.player_id,
                Selection::Album { id: 0 },
                Placement::PlayNow,
                None
            )
            .await,
        Err(ApiError::InvalidInput { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_players_list_shows_only_permitted_players() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    // The fake LMS lists a home player alice isn't granted: filtered out by the gateway.
    assert!(Players::new(s.clone()).list().await.unwrap().is_empty());
    h.accounts.grant_player("alice", HOME_PLAYER).unwrap();
    let list = Players::new(s.clone()).list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Home One");
    Players::new(s.clone())
        .play(HOME_PLAYER.into())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn artwork_is_fetched_with_the_token_and_capped() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let art = Art::new(s.clone());

    let img = art.fetch("music/abcdef12/cover".into(), 300).await.unwrap();
    assert_eq!(img.bytes, b"IMAGEDATA");
    assert_eq!(img.mime_type, "image/jpeg");
    assert!(
        h.lms
            .http_paths()
            .contains(&"/music/abcdef12/cover_300x300".to_string())
    );
    art.fetch("html/images/radio.png".into(), 300)
        .await
        .unwrap();

    // Over the cap: refused, never truncated.
    let small = Art::with_limit(s.clone(), 1024 * 1024);
    match small.fetch("music/99999999/cover".into(), 300).await {
        Err(ApiError::Server { detail, .. }) => assert!(detail.contains("too large"), "{detail}"),
        other => panic!("expected a size refusal, got {other:?}"),
    }
    // The default cap allows a normal large cover.
    assert_eq!(
        art.fetch("music/99999999/cover".into(), 300)
            .await
            .unwrap()
            .bytes
            .len(),
        HUGE_COVER
    );
    // Not an image (the gateway refuses it too).
    assert!(art.fetch("music/66666666/cover".into(), 300).await.is_err());

    // Hostile keys never reach the network.
    let before = h.lms.http_paths().len();
    for key in [
        "../v1/rpc",
        "music/1/cover/../../../v1/me",
        "music/%2e%2e/cover",
        "music/current/cover",
        "imageproxy/file%3A%2F%2F%2Fetc%2Fpasswd/image",
        "html/images/../../x.png",
    ] {
        assert!(
            matches!(
                art.fetch(key.into(), 300).await,
                Err(ApiError::InvalidInput { .. })
            ),
            "{key}"
        );
    }
    assert_eq!(h.lms.http_paths().len(), before);

    // An expired token is renewed for artwork too.
    s.debug_invalidate_access_token();
    art.fetch("music/abcdef12/cover".into(), 100).await.unwrap();
}

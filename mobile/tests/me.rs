//! The user's history, Home, stations and pinned menus against the real gateway: what the
//! app sends must pass the gateway's checks, stations must start and refill through the
//! allow-list, and pinned menu items must run only while LMS offers them.

mod common;

use anywhere_mobile::me::{
    StationRunner, add_shelf, default_home, layout_problem, move_shelf, refill_due, set_hidden,
};
use anywhere_mobile::{
    ApiError, HomeShelf, InfoKind, Me, MenuStep, Menus, RunnerUpdate, ShelfKind, StationBatch,
    StationSpec, info_menu,
};
use common::*;
use serde_json::Value;

fn sent_since(h: &Harness, before: usize, word: &str) -> Vec<Value> {
    h.lms.rpc_calls()[before..]
        .iter()
        .filter(|p| p[1][0] == word)
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_and_home_round_trip() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = Me::new(s.clone());
    let page = me.history(50, None).await.unwrap();
    assert!(page.enabled);
    assert_eq!(page.retention_days, 396);
    assert!(page.items.is_empty());
    assert!(!me.set_history_enabled(false).await.unwrap());
    assert!(!me.history(10, None).await.unwrap().enabled);
    assert!(me.set_history_enabled(true).await.unwrap());
    assert_eq!(me.clear_history().await.unwrap(), 0);
    assert!(matches!(
        me.history(10, Some("x;drop".into())).await,
        Err(ApiError::InvalidInput { .. })
    ));
    for kind in [ShelfKind::Recent, ShelfKind::Most, ShelfKind::OnThisDay] {
        me.shelf_albums(kind, 10).await.unwrap();
    }
    assert!(me.shelf_albums(ShelfKind::Stations, 10).await.is_err());

    let home = me.home().await.unwrap();
    assert!(home.is_default);
    assert_eq!(home.shelves, default_home().shelves);
    let edited = set_hidden(move_shelf(home, 4, 0), 1, true);
    let saved = me.save_home(edited.clone()).await.unwrap();
    assert!(!saved.is_default);
    assert_eq!(saved.shelves, edited.shelves);
    assert_eq!(me.home().await.unwrap().shelves, edited.shelves);
    // A station shelf must be one of alice's own.
    let stations = me.stations().await.unwrap();
    let with_station = add_shelf(
        saved.clone(),
        HomeShelf {
            kind: ShelfKind::Station,
            id: Some(stations[0].id),
            hidden: false,
        },
    )
    .unwrap();
    me.save_home(with_station.clone()).await.unwrap();
    let not_mine = add_shelf(
        saved,
        HomeShelf {
            kind: ShelfKind::Station,
            id: Some(999_999),
            hidden: false,
        },
    )
    .unwrap();
    assert_eq!(layout_problem(not_mine.clone()), None, "well-formed…");
    assert!(
        matches!(
            me.save_home(not_mine).await,
            Err(ApiError::InvalidInput { .. })
        ),
        "…but not alice's"
    );
    assert!(me.reset_home().await.unwrap().is_default);

    // Bob has his own of everything.
    let store = MemStore::new();
    let bob = h.session(store);
    bob.login("bob".into(), PW.into(), None).await.unwrap();
    let bob_me = Me::new(bob);
    assert!(bob_me.home().await.unwrap().is_default);
    let bobs = bob_me.stations().await.unwrap();
    assert!(bobs.iter().all(|b| stations.iter().all(|a| a.id != b.id)));
    assert!(matches!(
        bob_me.delete_station(stations[0].id).await,
        Err(ApiError::InvalidInput { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stations_start_and_refill_through_the_allow_list() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let player = s.current().unwrap().device.player_id;
    let me = Me::new(s.clone());
    let runner = StationRunner::new(me.clone());
    let stations = me.stations().await.unwrap();
    let kinds: Vec<StationSpec> = stations.iter().map(|x| x.spec.clone()).collect();
    assert_eq!(
        kinds,
        [
            StationSpec::Forgotten,
            StationSpec::Fresh,
            StationSpec::DeepCuts
        ]
    );

    // One genre: LMS's Random Mix, sent through the fixed grammar.
    let jazz = me
        .create_station("Jazz".into(), StationSpec::Genre { genres: vec![4] })
        .await
        .unwrap();
    let before = h.lms.rpc_calls().len();
    let b = runner.start(jazz, player.clone()).await.unwrap();
    assert!(matches!(b, StationBatch::RandomPlay { .. }));
    let sent = sent_since(&h, before, "randomplay");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0][0], player.as_str());
    assert_eq!(
        sent[0][1],
        serde_json::json!(["randomplay", "tracks", "genre_id:4"])
    );
    // LMS refills a Random Mix itself: no batches asked for.
    let before = h.lms.rpc_calls().len();
    assert_eq!(
        runner.on_status(player.clone(), 2, 1, None, vec![]).await,
        RunnerUpdate::Idle
    );
    assert!(sent_since(&h, before, "playlistcontrol").is_empty());

    // Fresh: track ids, loaded then added when the queue runs low.
    let fresh = stations[1].id;
    let before = h.lms.rpc_calls().len();
    let b = runner.start(fresh, player.clone()).await.unwrap();
    let StationBatch::Tracks { ids } = b else {
        panic!("{b:?}")
    };
    assert!(!ids.is_empty());
    let loads = sent_since(&h, before, "playlistcontrol");
    assert_eq!(loads[0][1][1], "cmd:load");
    assert_eq!(runner.current(), Some(fresh));
    // Plenty left: nothing asked for.
    assert!(!refill_due(20, 0));
    assert_eq!(
        runner
            .on_status(player.clone(), 20, 0, ids.first().copied(), vec![])
            .await,
        RunnerUpdate::Idle
    );
    // Low, but the library has nothing new left to offer (served lately): nothing added.
    let before = h.lms.rpc_calls().len();
    let u = runner
        .on_status(player.clone(), 2, 1, ids.last().copied(), ids.clone())
        .await;
    assert_eq!(u, RunnerUpdate::Idle);
    assert!(sent_since(&h, before, "playlistcontrol").is_empty());
    // Another player's status is not this station's business.
    assert_eq!(
        runner
            .on_status(HOME_PLAYER.into(), 1, 0, None, vec![])
            .await,
        RunnerUpdate::Idle
    );
    // The user played something else: the station ends.
    assert_eq!(
        runner
            .on_status(player.clone(), 2, 0, Some(999_999), vec![])
            .await,
        RunnerUpdate::Ended
    );
    assert_eq!(runner.current(), None);
    runner.start(fresh, player.clone()).await.unwrap();
    runner.stop();
    assert_eq!(runner.current(), None);

    // Not on a player alice may not control.
    assert!(matches!(
        runner.start(fresh, HOME_PLAYER.into()).await,
        Err(ApiError::Forbidden { .. })
    ));
    // Bad specs are refused before sending.
    assert!(
        me.create_station(
            "x".into(),
            StationSpec::Decade {
                decade: 1985,
                mine: false
            }
        )
        .await
        .is_err()
    );
    assert!(
        me.create_station("x".into(), StationSpec::Menu)
            .await
            .is_err()
    );
    let decade = me
        .create_station(
            "80s".into(),
            StationSpec::Decade {
                decade: 1980,
                mine: true,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        me.next_batch(decade, player.clone(), true, vec![])
            .await
            .unwrap(),
        StationBatch::Tracks { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn menu_stations_run_only_while_lms_offers_them() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let player = s.current().unwrap().device.player_id;
    let me = Me::new(s.clone());
    let menus = Menus::new(s.clone());
    let runner = StationRunner::new(me.clone());
    let info = info_menu(InfoKind::Track, 1).unwrap();
    let page = menus
        .open(player.clone(), info.clone(), 0, 100, None)
        .await
        .unwrap();
    let mix = page
        .items
        .iter()
        .find(|i| i.text == "More like this")
        .unwrap();
    let play = mix.go.clone().unwrap();
    let steps = vec![MenuStep::Opened {
        action: info.clone(),
        start: 0,
        count: 100,
    }];
    let id = me
        .save_menu_station(
            "More like One".into(),
            steps.clone(),
            play.clone(),
            None,
            player.clone(),
        )
        .await
        .unwrap();
    let before = h.lms.rpc_calls().len();
    let b = runner.start(id, player.clone()).await.unwrap();
    assert_eq!(b, StationBatch::Ran { refill: false });
    let ran = sent_since(&h, before, "someplugin");
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0][0], player.as_str());
    assert_eq!(
        ran[0][1],
        serde_json::json!(["someplugin", "mix", "track_id:7"])
    );

    // Something LMS didn't offer can't be saved.
    let mut forged = play.clone();
    forged.params[0].value = "8".into();
    assert!(matches!(
        me.save_menu_station("x".into(), steps.clone(), forged, None, player.clone())
            .await,
        Err(ApiError::InvalidInput { .. })
    ));
    let mut never = play.clone();
    never.cmd = vec!["playlist".into(), "clear".into()];
    never.params.clear();
    assert!(
        me.save_menu_station("x".into(), steps.clone(), never, None, player.clone())
            .await
            .is_err()
    );
    // A menu (or a search) is not an action.
    assert!(
        me.save_menu_station(
            "x".into(),
            steps.clone(),
            info.clone(),
            None,
            player.clone()
        )
        .await
        .is_err()
    );

    // The admin makes the plugin admin-only: the station is broken, and nothing runs.
    h.accounts
        .set_admin_only_plugins(&["someplugin".to_string()])
        .unwrap();
    let before = h.lms.rpc_calls().len();
    let b = runner.start(id, player.clone()).await.unwrap();
    let StationBatch::Broken { reason } = b else {
        panic!("{b:?}")
    };
    assert!(reason.contains("admins only"), "{reason}");
    assert!(sent_since(&h, before, "someplugin").is_empty());
    let listed = me.stations().await.unwrap();
    assert!(listed.iter().find(|x| x.id == id).unwrap().broken.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_menus_open_as_lms_offers_them_now() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let player = s.current().unwrap().device.player_id;
    let me = Me::new(s.clone());
    let id = me
        .pin_menu("Plugins".into(), vec![MenuStep::HomeMenu], player.clone())
        .await
        .unwrap();
    assert_eq!(me.menus().await.unwrap()[0].id, id);
    let page = me.open_menu(id, player.clone()).await.unwrap();
    let texts: Vec<&str> = page.items.iter().map(|i| i.text.as_str()).collect();
    assert!(texts.contains(&"Some Plugin"), "{texts:?}");
    assert!(
        !page
            .items
            .iter()
            .any(|i| i.go.as_ref().is_some_and(|g| g.cmd[0] == "browselibrary"))
    );
    assert!(matches!(
        me.open_menu(id, HOME_PLAYER.into()).await,
        Err(ApiError::Forbidden { .. })
    ));
    me.delete_menu(id).await.unwrap();
    assert!(me.menus().await.unwrap().is_empty());
    assert!(me.open_menu(id, player).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelves_fetch_albums_and_tracks_by_id_through_the_allow_list() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let lib = anywhere_mobile::MusicLibrary::new(s.clone());
    // In the order asked for, unknown ids left out, each once.
    let albums = lib.albums_by_id(vec![8, 7, 99, 8]).await.unwrap();
    assert_eq!(albums.iter().map(|a| a.id).collect::<Vec<_>>(), [8, 7]);
    let tracks = lib.tracks_by_id(vec![2, 1]).await.unwrap();
    assert_eq!(
        tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        [Some(2), Some(1)]
    );
    let sent: Vec<Value> = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|p| p[1][0] == "albums" || p[1][0] == "titles")
        .map(|p| p[1].clone())
        .collect();
    assert!(
        sent.iter()
            .any(|c| c.to_string().contains("album_id:8,7,99")),
        "{sent:?}"
    );
    assert!(
        sent.iter().any(|c| c.to_string().contains("track_id:2,1")),
        "{sent:?}"
    );
    // Bad ids never leave the app.
    assert!(lib.albums_by_id(vec![0]).await.is_err());
    assert!(lib.tracks_by_id(vec![-3]).await.is_err());
    assert!(lib.albums_by_id(vec![]).await.unwrap().is_empty());
}

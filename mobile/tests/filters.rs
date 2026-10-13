//! Smart filters against the real gateway: what the app sends must pass the gateway's
//! checks (and bad filters are refused before anything is sent), filters are the user's
//! own, results page through `albums` queries only, and a filter can be a Home shelf.

mod common;

use anywhere_mobile::commands::{AlbumSort, ArtistRole};
use anywhere_mobile::me::{add_shelf, filter_problem};
use anywhere_mobile::{ApiError, FilterRule, HomeShelf, Me, ShelfKind};
use common::*;
use serde_json::Value;

fn albums_queries(h: &Harness, since: usize) -> Vec<Value> {
    h.lms.rpc_calls()[since..]
        .iter()
        .filter(|p| p[1][0] == "albums")
        .map(|p| p[1].clone())
        .collect()
}

fn invalid<T: std::fmt::Debug>(r: Result<T, ApiError>) -> bool {
    matches!(r, Err(ApiError::InvalidInput { .. }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filters_round_trip_and_stay_the_users_own() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = Me::new(s);
    assert!(me.filters().await.unwrap().is_empty());
    let rules = vec![
        FilterRule::Genre { genres: vec![9, 4] },
        FilterRule::Artist {
            artist: 3,
            role: Some(ArtistRole::Composer),
        },
        FilterRule::Compilation { compilation: false },
    ];
    let f = me
        .create_filter(" Evening  ".into(), rules.clone(), AlbumSort::Title, 120)
        .await
        .unwrap();
    assert_eq!(f.name, "Evening");
    assert_eq!(
        f.rules[0],
        FilterRule::Genre { genres: vec![4, 9] },
        "the gateway keeps genres in order"
    );
    assert_eq!(f.rules[1..], rules[1..]);
    assert_eq!((f.sort, f.limit), (AlbumSort::Title, 120));
    assert_eq!(me.filters().await.unwrap(), vec![f.clone()]);

    let updated = me
        .update_filter(
            f.id,
            "Seventies".into(),
            vec![FilterRule::Decade { decade: 1970 }],
            AlbumSort::Newest,
            10,
        )
        .await
        .unwrap();
    assert_eq!(updated.id, f.id);
    assert_eq!(updated.rules, vec![FilterRule::Decade { decade: 1970 }]);
    assert_eq!(me.filters().await.unwrap(), vec![updated.clone()]);

    // Bob can't see, change, run or delete it.
    let bob = h.session(MemStore::new());
    bob.login("bob".into(), PW.into(), None).await.unwrap();
    let bob = Me::new(bob);
    assert!(bob.filters().await.unwrap().is_empty());
    assert!(invalid(
        bob.update_filter(
            f.id,
            "Mine now".into(),
            vec![FilterRule::Compilation { compilation: true }],
            AlbumSort::Title,
            5
        )
        .await
    ));
    let before = h.lms.rpc_calls().len();
    assert!(invalid(bob.filter_albums(f.id, 0, 50).await));
    assert!(albums_queries(&h, before).is_empty());
    assert!(invalid(bob.delete_filter(f.id).await));
    assert_eq!(me.filters().await.unwrap(), vec![updated]);

    me.delete_filter(f.id).await.unwrap();
    assert!(me.filters().await.unwrap().is_empty());
    assert!(invalid(me.delete_filter(f.id).await));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn results_page_through_allowed_albums_queries() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = Me::new(s);
    // One query without a decade, in LMS's order.
    let one = me
        .create_filter(
            "Compilations".into(),
            vec![FilterRule::Compilation { compilation: true }],
            AlbumSort::Title,
            50,
        )
        .await
        .unwrap();
    let before = h.lms.rpc_calls().len();
    let page = me.filter_albums(one.id, 0, 50).await.unwrap();
    assert_eq!((page.albums, page.count), (vec![7, 8], 2));
    let sent = albums_queries(&h, before);
    assert_eq!(
        sent,
        vec![serde_json::json!([
            "albums",
            "0",
            "50",
            "compilation:1",
            "sort:album",
            "tags:lay"
        ])]
    );
    let page = me.filter_albums(one.id, 1, 1).await.unwrap();
    assert_eq!((page.albums, page.count), (vec![8], 2));

    // A decade: one query per year, merged without repeats.
    let decade = me
        .create_filter(
            "Nineties".into(),
            vec![FilterRule::Decade { decade: 1990 }],
            AlbumSort::Newest,
            1,
        )
        .await
        .unwrap();
    let before = h.lms.rpc_calls().len();
    let page = me.filter_albums(decade.id, 0, 100).await.unwrap();
    assert_eq!(
        (page.albums, page.count),
        (vec![8], 1),
        "newest first, cut to 1"
    );
    let sent = albums_queries(&h, before);
    assert_eq!(sent.len(), 10);
    for (i, q) in sent.iter().enumerate() {
        assert_eq!(q[3], format!("year:{}", 1990 + i));
    }
    // Pages are checked before sending.
    let before = h.lms.rpc_calls().len();
    assert!(invalid(me.filter_albums(decade.id, 0, 0).await));
    assert!(invalid(me.filter_albums(decade.id, 0, 101).await));
    assert!(invalid(me.filter_albums(decade.id, 501, 10).await));
    assert!(invalid(me.filter_albums(0, 0, 10).await));
    assert!(invalid(me.filter_albums(-5, 0, 10).await));
    assert!(albums_queries(&h, before).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_filters_are_refused_before_sending() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = Me::new(s);
    let good = vec![FilterRule::Decade { decade: 1980 }];
    let calls = h.lms.http_paths().len();
    let cases: Vec<(String, Vec<FilterRule>, u32)> = vec![
        ("".into(), good.clone(), 10),
        ("\u{202e}".into(), good.clone(), 10),
        ("x".repeat(65), good.clone(), 10),
        ("F".into(), vec![], 10),
        ("F".into(), good.clone(), 0),
        ("F".into(), good.clone(), 501),
        ("F".into(), vec![FilterRule::Decade { decade: 1984 }], 10),
        (
            "F".into(),
            vec![FilterRule::Genre {
                genres: (1..=21).collect(),
            }],
            10,
        ),
        (
            "F".into(),
            vec![FilterRule::Genre { genres: vec![2, 2] }],
            10,
        ),
        (
            "F".into(),
            vec![FilterRule::Artist {
                artist: -1,
                role: None,
            }],
            10,
        ),
        ("F".into(), [good.clone(), good.clone()].concat(), 10),
    ];
    for (name, rules, limit) in cases {
        assert!(filter_problem(name.clone(), rules.clone(), limit).is_some());
        assert!(invalid(
            me.create_filter(name.clone(), rules.clone(), AlbumSort::Random, limit)
                .await
        ));
        assert!(invalid(
            me.update_filter(1, name, rules, AlbumSort::Random, limit)
                .await
        ));
    }
    assert_eq!(h.lms.http_paths().len(), calls);
    assert!(me.filters().await.unwrap().is_empty(), "nothing was saved");
    // At most 50 a user: the gateway's refusal comes back as invalid input.
    for i in 0..50 {
        me.create_filter(format!("F{i}"), good.clone(), AlbumSort::Year, 5)
            .await
            .unwrap();
    }
    assert!(invalid(
        me.create_filter("One more".into(), good, AlbumSort::Year, 5)
            .await
    ));
    assert_eq!(me.filters().await.unwrap().len(), 50);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_filter_can_be_a_home_shelf() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = Me::new(s);
    let f = me
        .create_filter(
            "Shelf".into(),
            vec![FilterRule::Compilation { compilation: true }],
            AlbumSort::Random,
            20,
        )
        .await
        .unwrap();
    let home = me.home().await.unwrap();
    let with = add_shelf(
        home.clone(),
        HomeShelf {
            kind: ShelfKind::Filter,
            id: Some(f.id),
            hidden: false,
        },
    )
    .unwrap();
    let saved = me.save_home(with.clone()).await.unwrap();
    assert_eq!(saved.shelves, with.shelves);
    assert_eq!(me.home().await.unwrap().shelves, with.shelves);
    // Not someone else's (or a missing) filter.
    let not_mine = add_shelf(
        home,
        HomeShelf {
            kind: ShelfKind::Filter,
            id: Some(f.id + 1000),
            hidden: false,
        },
    )
    .unwrap();
    assert!(invalid(me.save_home(not_mine).await));
    // A deleted filter's shelf goes.
    me.delete_filter(f.id).await.unwrap();
    let after = me.home().await.unwrap();
    assert!(after.shelves.iter().all(|s| s.kind != ShelfKind::Filter));
}

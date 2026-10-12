//! Ratings and alarms through the real gateway (docs/plans/wave2-design.md §1–2): what the
//! app sends passes the gateway's checks, answers come back checked, and ratings made
//! offline are sent and then dropped from the queue.

mod common;

use anywhere_mobile::{
    AlarmChange, ApiError, OfflineLibrary, RatingStyle, Routine, next_alarm_in, sync_ratings,
};
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ratings_round_trip_and_the_offline_queue_is_sent() {
    let h = Harness::start().await;
    h.lms.add_track(41, "flc", vec![0; 16]);
    let (s, _) = h.alice().await;
    let r = Routine::new(s.clone());
    assert_eq!(r.rating_style().await.unwrap(), RatingStyle::Stars);
    r.set_rating_style(RatingStyle::Heart).await.unwrap();
    assert_eq!(r.rating_style().await.unwrap(), RatingStyle::Heart);

    let rated = r.rate(41, 84).await.unwrap().unwrap();
    assert_eq!(
        (rated.track_id, rated.mine, rated.rating),
        (41, Some(80), Some(80))
    );
    let got = r.ratings(vec![41, 41, -1, 0, 999_999]).await.unwrap();
    assert_eq!(got.len(), 1, "only the library track LMS knows");
    assert_eq!(got[0].mine, Some(80));
    assert!(matches!(
        r.rate(0, 50).await,
        Err(ApiError::InvalidInput { .. })
    ));

    // Offline: queued on the device, then sent; the server checks the key (this one is
    // not the file's, so it is skipped there) and the queue empties either way.
    let root = std::env::temp_dir().join(format!("anywhere-rt-{}", anywhere_gateway::random_id()));
    let lib = OfflineLibrary::open(
        root.to_string_lossy().into(),
        h.server().url(),
        "alice".into(),
    )
    .unwrap();
    assert!(lib.rate_offline(41, "a".repeat(64), 60, 1_700_000_000));
    assert!(!lib.rate_offline(41, "not a key".into(), 60, 1_700_000_000));
    assert_eq!(lib.pending_ratings(), 1);
    let out = sync_ratings(s.clone(), lib.clone()).await.unwrap();
    assert_eq!((out.sent, out.pending, out.unsupported), (1, 0, false));
    assert_eq!(lib.pending_ratings(), 0);
    // Kept across a restart while pending.
    assert!(lib.rate_offline(41, "b".repeat(64), 40, 1_700_000_001));
    let again = OfflineLibrary::open(
        root.to_string_lossy().into(),
        h.server().url(),
        "alice".into(),
    )
    .unwrap();
    assert_eq!(again.pending_ratings(), 1);
    assert_eq!(r.clear_ratings().await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alarms_on_this_phones_player() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let mine = s.current().unwrap().device.player_id;
    let r = Routine::new(s.clone());
    let l = r.alarms(mine.clone()).await.unwrap();
    assert!(l.enabled);
    assert_eq!(l.alarms.len(), 1);
    assert_eq!(l.alarms[0].days, vec![1, 2, 3, 4, 5]);
    assert_eq!(l.alarms[0].sound_title, "Morning radio");
    let radio = l
        .sounds
        .iter()
        .find(|x| x.title == "Morning radio")
        .unwrap()
        .id
        .clone();
    assert_eq!(radio.len(), 16, "an opaque id, never the URL");
    let added = r
        .add_alarm(
            mine.clone(),
            AlarmChange {
                time_secs: Some(6 * 3600),
                days: Some(vec![6, 0]),
                sound_id: Some(radio),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(added.new_id.as_deref(), Some("5e6f7a8b"));
    let sent: Vec<_> = h
        .lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "alarm" && c[1][1] == "add")
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0][1]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "url:http://radio.example.com/stream")
    );
    r.change_alarm(
        mine.clone(),
        "1a2b3c4d".into(),
        AlarmChange {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    r.delete_alarm(mine.clone(), "1a2b3c4d".into())
        .await
        .unwrap();
    r.set_all_alarms(mine.clone(), false).await.unwrap();
    // Refused before anything is sent.
    assert!(matches!(
        r.alarms("not-a-player".into()).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        r.add_alarm(mine.clone(), AlarmChange::default()).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        r.change_alarm(mine.clone(), "../x".into(), AlarmChange::default())
            .await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        r.add_alarm(
            mine.clone(),
            AlarmChange {
                time_secs: Some(1),
                sound_id: Some("http://evil".into()),
                ..Default::default()
            }
        )
        .await,
        Err(ApiError::InvalidInput { .. })
    ));
    // Another player: the gateway refuses.
    assert!(matches!(
        r.alarms("aa:bb:cc:dd:ee:01".into()).await,
        Err(ApiError::Forbidden { .. })
    ));
    let next = next_alarm_in(l.alarms, 1, 6 * 3600).unwrap();
    assert_eq!(next, 3600);
}

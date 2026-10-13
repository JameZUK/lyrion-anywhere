//! Listening stats through the real gateway: what the app asks must pass the gateway's
//! checks, the answer must read whole, and only the user's own plays count.

mod common;

use anywhere_gateway::history::{self, Source, TrackInfo};
use anywhere_mobile::{ApiError, Me, StatsPeriod};
use common::*;

fn seed(h: &Harness, name: &str, track: u64, artist: i64, ago: u64, seconds: f64) {
    let who = h.accounts.user_id_by_name(name).unwrap().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let t = TrackInfo {
        id: track,
        key: format!("{track:064x}"),
        album_id: Some(track as i64 + 100),
        artist_id: Some(artist),
        year: None,
        duration: Some(300.0),
    };
    history::record(&h.accounts, who, &t, now - ago, seconds, Source::Live).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stats_round_trip_through_the_gateway() {
    let h = Harness::start().await;
    seed(&h, "alice", 1, 3, 3600, 200.0);
    seed(&h, "alice", 1, 3, 7200, 200.0);
    seed(&h, "alice", 2, 4, 7300, 100.0);
    seed(&h, "bob", 9, 9, 3600, 200.0);
    let (s, _) = h.alice().await;
    let me = Me::new(s);

    let st = me.stats(StatsPeriod::Days { days: 30 }, 60).await.unwrap();
    assert!(st.enabled);
    assert_eq!(st.retention_days, 396);
    assert_eq!(st.totals.plays, 3, "bob's play is not alice's");
    assert_eq!(st.totals.seconds, 500);
    assert_eq!(
        (st.totals.tracks, st.totals.albums, st.totals.artists),
        (2, 2, 2)
    );
    assert_eq!(st.top_artists[0].id, 3);
    assert_eq!(st.top_artists[0].name, "Artist");
    assert_eq!(st.top_artists[1].name, "", "LMS named someone else");
    assert_eq!(st.top_tracks[0].id, 1);
    assert_eq!(st.top_albums[0].id, 101);
    assert_eq!(st.new_artists.len(), 2);
    assert_eq!(st.by_hour.iter().sum::<u64>(), 3);
    assert_eq!(st.by_weekday.len(), 7);
    assert!(st.by_month.is_empty());
    assert!(st.longest_streak.is_some() && st.busiest_day.is_some());
    assert!(st.from < st.to);

    let all = me.stats(StatsPeriod::All, -840).await.unwrap();
    assert!(!all.by_month.is_empty());
    assert_eq!(all.totals.plays, 3);
    let year = me.stats(StatsPeriod::Year { year: 1999 }, 0).await.unwrap();
    assert_eq!((year.totals.plays, year.by_month.len()), (0, 12));
    me.stats(
        StatsPeriod::Month {
            year: 2026,
            month: 2,
        },
        330,
    )
    .await
    .unwrap();

    // Refused here, before anything is sent.
    for (p, tz) in [
        (StatsPeriod::Days { days: 401 }, 0),
        (
            StatsPeriod::Month {
                year: 2026,
                month: 13,
            },
            0,
        ),
        (StatsPeriod::Year { year: 1969 }, 0),
        (StatsPeriod::All, 900),
    ] {
        assert!(matches!(
            me.stats(p, tz).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }

    // Switched off: nothing shows.
    assert!(!me.set_history_enabled(false).await.unwrap());
    let off = me.stats(StatsPeriod::All, 0).await.unwrap();
    assert!(!off.enabled);
    assert_eq!(off.totals.plays, 0);
    assert!(off.top_artists.is_empty() && off.longest_streak.is_none());
    assert_eq!(off.by_hour, vec![0; 24]);
    // Back on, then cleared: the remembered answer is dropped.
    assert!(me.set_history_enabled(true).await.unwrap());
    assert_eq!(me.stats(StatsPeriod::All, 0).await.unwrap().totals.plays, 3);
    assert_eq!(me.clear_history().await.unwrap(), 3);
    assert_eq!(me.stats(StatsPeriod::All, 0).await.unwrap().totals.plays, 0);
}

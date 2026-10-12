//! The sleep timer's rules (Wave 1 §1), shared by the apps: how long until the end of the
//! current track, album or queue; when an "end of …" timer must be set again; which players
//! a timer is for; the fade-out setting and its curve. No I/O: the app sets LMS's own
//! `sleep <seconds>` through [`crate::Players::set_sleep`], so the timer also works on
//! hardware players and keeps going with the app closed. At the end LMS fades the volume
//! over the last minute (at most), then stops the player and turns it off.
//!
//! The web app has the same rules (web/app/src/lib/lms/sleep.ts); the tests share vectors.

use crate::commands::is_player_id;
use crate::model::{PlayerStatus, Track};

/// Which end of the queue a timer is set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SleepEnd {
    Track,
    Album,
    Queue,
}

/// The fixed lengths offered, in minutes.
pub const SLEEP_MINUTES: [u32; 5] = [15, 30, 45, 60, 90];
/// A custom length: 1 minute to a day.
pub const MAX_CUSTOM_MINUTES: u32 = 24 * 60;
/// Fade-out lengths offered (seconds; 0 is off), the longest, and the default.
pub const FADE_CHOICES: [u32; 6] = [0, 10, 30, 60, 90, 120];
pub const MAX_FADE_SECS: u32 = 120;
pub const DEFAULT_FADE_SECS: u32 = 30;
/// Re-arm when the end has moved by more than this.
pub const REARM_TOLERANCE_SECS: f64 = 5.0;
/// LMS has had this long to report a timer just set before its absence means "cancelled".
pub const CONFIRM_MS: i64 = 10_000;

fn positive(v: Option<f64>) -> Option<f64> {
    v.filter(|d| d.is_finite() && *d > 0.0)
}

/// Seconds until the end of the current track, the current album (the run of tracks from
/// the same album that follows it) or the queue, rounded up. `upcoming` is the queue after
/// the current track, in playing order. `None` when it can't be known: nothing playing, a
/// stream with no length, a track in range without one, (for the queue) a queue longer
/// than what was read, or an end more than a day away (longer than LMS's timer is set).
#[uniffi::export]
pub fn sleep_remaining(end: SleepEnd, status: &PlayerStatus, upcoming: &[Track]) -> Option<u32> {
    let cur = status.current.as_ref()?;
    let length = positive(status.duration_secs).or(positive(cur.duration_secs))?;
    let elapsed = if status.elapsed_secs.is_finite() {
        status.elapsed_secs.max(0.0)
    } else {
        0.0
    };
    let mut left = (length - elapsed).max(0.0);
    let album = cur.album_id.filter(|a| *a > 0);
    match end {
        SleepEnd::Track => return within_a_day(left),
        SleepEnd::Album if album.is_none() => return within_a_day(left),
        SleepEnd::Queue => {
            let index = status.queue_index.unwrap_or(0);
            let after = status.queue_length.saturating_sub(index.saturating_add(1));
            if (upcoming.len() as u64) < after as u64 {
                return None; // only part of the queue was read
            }
        }
        SleepEnd::Album => {}
    }
    for t in upcoming {
        if end == SleepEnd::Album && t.album_id.filter(|a| *a > 0) != album {
            break;
        }
        left += positive(t.duration_secs)?;
    }
    within_a_day(left)
}

/// Whole seconds (rounded up), if within a day.
fn within_a_day(s: f64) -> Option<u32> {
    if !s.is_finite() {
        return None;
    }
    let s = s.max(0.0).ceil();
    (s <= f64::from(crate::commands::MAX_SLEEP_SECS)).then_some(s as u32)
}

/// The seconds to set an "end of …" timer to again, if its end (`ends_at_ms`) has moved by
/// more than the tolerance (a seek, skip, pause or queue edit); otherwise `None`. Never 0
/// (which would cancel it).
#[uniffi::export]
pub fn sleep_rearm(ends_at_ms: i64, remaining: Option<u32>, now_ms: i64) -> Option<u32> {
    let r = remaining?;
    let wanted = now_ms.saturating_add(i64::from(r) * 1000);
    let moved = (wanted.saturating_sub(ends_at_ms)).unsigned_abs();
    (moved as f64 > REARM_TOLERANCE_SECS * 1000.0).then_some(r.max(1))
}

/// Whether an armed timer is over: LMS no longer reports one (it went off, or another
/// controller cancelled it) once it has had time to, or its end has passed.
#[uniffi::export]
pub fn sleep_armed_over(
    set_at_ms: i64,
    ends_at_ms: i64,
    sleep_secs: Option<f64>,
    now_ms: i64,
) -> bool {
    if positive(sleep_secs).is_some() {
        return false;
    }
    now_ms.saturating_sub(set_at_ms) > CONFIRM_MS || now_ms > ends_at_ms.saturating_add(CONFIRM_MS)
}

/// A custom length in minutes, if it is in range (1 minute to a day).
#[uniffi::export]
pub fn sleep_custom_minutes(minutes: i64) -> Option<u32> {
    (1..=i64::from(MAX_CUSTOM_MINUTES))
        .contains(&minutes)
        .then_some(minutes as u32)
}

/// A saved fade-out length, checked: 0–120 seconds, else the default.
#[uniffi::export]
pub fn sleep_fade_sanitise(seconds: i64) -> u32 {
    if (0..=i64::from(MAX_FADE_SECS)).contains(&seconds) {
        seconds as u32
    } else {
        DEFAULT_FADE_SECS
    }
}

/// This device's output gain while a timer runs: full until `fade_secs` are left, then down
/// to silence at the end (an ease-in curve, so the drop sounds even). 1 with no timer.
#[uniffi::export]
pub fn sleep_fade_gain(seconds_left: Option<f64>, fade_secs: f64) -> f32 {
    let Some(left) = seconds_left.filter(|s| !s.is_nan()) else {
        return 1.0;
    };
    if !fade_secs.is_finite() || fade_secs <= 0.0 {
        return 1.0;
    }
    if left <= 0.0 {
        return 0.0;
    }
    if left >= fade_secs {
        return 1.0;
    }
    let x = left / fade_secs;
    (x * x) as f32
}

/// The players a timer is set on: the selected one; with `group`, its sync group too; with
/// `all`, every listed player that is connected. Only well-formed ids, each once, the
/// selected one first.
#[uniffi::export]
pub fn sleep_targets(selected: String, group: Vec<String>, all: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in std::iter::once(selected).chain(group).chain(all) {
        let id = id.trim().to_ascii_lowercase();
        if is_player_id(&id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{RepeatMode, ShuffleMode};
    use crate::model::PlaybackMode;

    fn track(album: i64, duration: Option<f64>) -> Track {
        Track {
            id: Some(album * 100),
            title: "T".into(),
            artist: None,
            album: None,
            album_id: Some(album),
            artist_id: None,
            duration_secs: duration,
            track_number: None,
            artwork: None,
            queue_index: None,
            disc_number: None,
            disc_count: None,
            genre: None,
            format: None,
            year: None,
            quality: None,
        }
    }

    fn status(elapsed: f64, duration: Option<f64>, index: u32, length: u32) -> PlayerStatus {
        PlayerStatus {
            name: "P".into(),
            connected: true,
            powered: true,
            mode: PlaybackMode::Playing,
            elapsed_secs: elapsed,
            duration_secs: duration,
            volume: 50,
            muted: false,
            shuffle: ShuffleMode::Off,
            repeat: RepeatMode::Off,
            queue_index: Some(index),
            queue_length: length,
            can_seek: true,
            current: Some(track(7, Some(200.0))),
            next: None,
            sleep_secs: None,
        }
    }

    #[test]
    fn end_of_track() {
        assert_eq!(
            sleep_remaining(SleepEnd::Track, &status(60.0, Some(200.0), 2, 6), &[]),
            Some(140)
        );
        assert_eq!(
            sleep_remaining(SleepEnd::Track, &status(59.2, Some(200.0), 2, 6), &[]),
            Some(141)
        );
        assert_eq!(
            sleep_remaining(SleepEnd::Track, &status(250.0, Some(200.0), 2, 6), &[]),
            Some(0)
        );
        // The track's own duration when the status has none.
        assert_eq!(
            sleep_remaining(SleepEnd::Track, &status(60.0, None, 2, 6), &[]),
            Some(140)
        );
    }

    #[test]
    fn end_of_album_is_the_run_of_the_same_album() {
        let s = status(60.0, Some(200.0), 2, 6);
        let up = [
            track(7, Some(180.0)),
            track(7, Some(240.0)),
            track(8, Some(300.0)),
            track(7, Some(100.0)),
        ];
        assert_eq!(
            sleep_remaining(SleepEnd::Album, &s, &up),
            Some(140 + 180 + 240)
        );
        assert_eq!(
            sleep_remaining(SleepEnd::Album, &s, &[track(9, Some(100.0))]),
            Some(140)
        );
        assert_eq!(
            sleep_remaining(
                SleepEnd::Album,
                &s,
                &[track(7, Some(180.0)), track(7, None)]
            ),
            None
        );
        let mut no_album = s.clone();
        no_album.current.as_mut().unwrap().album_id = None;
        assert_eq!(sleep_remaining(SleepEnd::Album, &no_album, &up), Some(140));
    }

    #[test]
    fn end_of_queue_needs_the_whole_queue() {
        let up = [
            track(7, Some(180.0)),
            track(8, Some(240.0)),
            track(9, Some(300.0)),
        ];
        assert_eq!(
            sleep_remaining(SleepEnd::Queue, &status(60.0, Some(200.0), 2, 6), &up),
            Some(140 + 720)
        );
        assert_eq!(
            sleep_remaining(SleepEnd::Queue, &status(60.0, Some(200.0), 5, 6), &[]),
            Some(140)
        );
        assert_eq!(
            sleep_remaining(SleepEnd::Queue, &status(60.0, Some(200.0), 2, 900), &up),
            None
        );
        let radio = [track(7, Some(180.0)), track(8, None), track(9, Some(300.0))];
        assert_eq!(
            sleep_remaining(SleepEnd::Queue, &status(60.0, Some(200.0), 2, 6), &radio),
            None
        );
    }

    #[test]
    fn an_end_more_than_a_day_away_is_not_offered() {
        let long = vec![track(8, Some(3600.0)); 30];
        assert_eq!(
            sleep_remaining(SleepEnd::Queue, &status(60.0, Some(200.0), 0, 31), &long),
            None
        );
        assert_eq!(
            sleep_remaining(
                SleepEnd::Queue,
                &status(60.0, Some(200.0), 0, 24),
                &long[..23]
            ),
            Some(140 + 23 * 3600)
        );
    }

    #[test]
    fn nothing_playing_or_no_length_is_unknowable() {
        let mut s = status(60.0, None, 2, 6);
        s.current.as_mut().unwrap().duration_secs = None;
        assert_eq!(sleep_remaining(SleepEnd::Track, &s, &[]), None);
        s.current = None;
        assert_eq!(sleep_remaining(SleepEnd::Track, &s, &[]), None);
    }

    #[test]
    fn hostile_numbers_never_make_a_negative_or_endless_timer() {
        for bad in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -5.0,
            1e308,
            f64::MAX,
        ] {
            for end in [SleepEnd::Track, SleepEnd::Album, SleepEnd::Queue] {
                let mut s = status(bad, Some(bad), u32::MAX, u32::MAX);
                s.current.as_mut().unwrap().duration_secs = Some(bad);
                let r = sleep_remaining(end, &s, &[track(7, Some(bad))]);
                assert!(
                    r.is_none_or(|x| x <= crate::commands::MAX_SLEEP_SECS),
                    "{bad} {end:?} {r:?}"
                );
            }
        }
    }

    #[test]
    fn rearm_only_when_the_end_moves() {
        let set = 1_000_000;
        let ends = set + 140_000;
        assert_eq!(sleep_rearm(ends, Some(140), set), None);
        assert_eq!(sleep_rearm(ends, Some(136), set + 2_000), None);
        assert_eq!(sleep_rearm(ends, Some(40), set), Some(40));
        assert_eq!(sleep_rearm(ends, Some(140), set + 30_000), Some(140));
        assert_eq!(sleep_rearm(ends, Some(0), set + 140_000 - 20_000), Some(1));
        assert_eq!(sleep_rearm(ends, None, set), None);
        assert_eq!(
            sleep_rearm(i64::MIN, Some(u32::MAX), i64::MAX),
            Some(u32::MAX)
        );
    }

    #[test]
    fn armed_is_over_once_lms_stops_reporting_it() {
        let set = 1_000_000;
        let ends = set + 140_000;
        assert!(!sleep_armed_over(set, ends, Some(120.0), set + 20_000));
        assert!(!sleep_armed_over(set, ends, None, set + 3_000));
        assert!(sleep_armed_over(set, ends, None, set + 11_000));
        assert!(sleep_armed_over(set, ends, Some(0.0), set + 11_000));
        assert!(sleep_armed_over(set, ends, Some(f64::NAN), set + 11_000));
    }

    #[test]
    fn custom_lengths_and_fade_setting_are_checked() {
        assert_eq!(sleep_custom_minutes(45), Some(45));
        for bad in [0, -5, 1441, i64::MAX, i64::MIN] {
            assert_eq!(sleep_custom_minutes(bad), None, "{bad}");
        }
        assert_eq!(sleep_fade_sanitise(90), 90);
        assert_eq!(sleep_fade_sanitise(0), 0);
        for bad in [-1, 121, 9_999, i64::MAX, i64::MIN] {
            assert_eq!(sleep_fade_sanitise(bad), DEFAULT_FADE_SECS, "{bad}");
        }
    }

    #[test]
    fn fade_curve() {
        assert_eq!(sleep_fade_gain(None, 30.0), 1.0);
        assert_eq!(sleep_fade_gain(Some(100.0), 30.0), 1.0);
        assert_eq!(sleep_fade_gain(Some(30.0), 30.0), 1.0);
        assert!((sleep_fade_gain(Some(15.0), 30.0) - 0.25).abs() < 1e-6);
        assert_eq!(sleep_fade_gain(Some(0.0), 30.0), 0.0);
        assert_eq!(sleep_fade_gain(Some(-4.0), 30.0), 0.0);
        assert_eq!(sleep_fade_gain(Some(5.0), 0.0), 1.0);
        assert_eq!(sleep_fade_gain(Some(f64::NAN), 30.0), 1.0);
        assert_eq!(sleep_fade_gain(Some(5.0), f64::NAN), 1.0);
        let mut prev = 1.0;
        for i in (0..=60).rev() {
            let g = sleep_fade_gain(Some(f64::from(i) / 2.0), 30.0);
            assert!(g <= prev && (0.0..=1.0).contains(&g));
            prev = g;
        }
    }

    #[test]
    fn targets_are_well_formed_and_distinct() {
        let me = "AA:BB:CC:DD:EE:01".to_string();
        assert_eq!(
            sleep_targets(me.clone(), vec![], vec![]),
            vec!["aa:bb:cc:dd:ee:01"]
        );
        assert_eq!(
            sleep_targets(
                me.clone(),
                vec!["aa:bb:cc:dd:ee:02".into(), "aa:bb:cc:dd:ee:01".into()],
                vec![
                    "aa:bb:cc:dd:ee:03".into(),
                    "../x".into(),
                    "".into(),
                    "all".into(),
                    "aa:bb:cc:dd:ee:04;rescan".into()
                ],
            ),
            vec![
                "aa:bb:cc:dd:ee:01",
                "aa:bb:cc:dd:ee:02",
                "aa:bb:cc:dd:ee:03"
            ]
        );
        assert!(sleep_targets("bad".into(), vec![], vec![]).is_empty());
    }
}

//! Demo-only listening history (tools/demo): `anywhere-gateway admin demo-seed-history`.
//!
//! The documentation screenshots show the listening stats and "Your year in sound", which
//! are worked out from a user's history ([`crate::history`]). A fresh demo has none, and
//! plays can only be recorded by really playing (or, offline, within 90 days), so the demo
//! seeds invented plays into its own scratch data directory with this command.
//!
//! It is never part of a real installation:
//! - **Debug builds only.** This module and its command exist only with
//!   `debug_assertions`; release builds (the plugin's binaries) don't contain them.
//! - **Demo data directories only.** The data directory must hold the marker file
//!   [`MARKER`], which only `tools/demo/run.sh` writes (into a fresh temporary directory).
//! - **Empty history only.** The user must exist and have no history yet, so it never
//!   mixes invented plays with real ones, and running it twice is refused.
//! - **Bounded input:** at most [`MAX_INPUT`] bytes and [`MAX_PLAYS`] plays, each checked
//!   like a recorded play (ids positive and at most 12 digits, a plausible year, played
//!   within the history's retention and not in the future, 0 to 86 400 seconds) and
//!   nothing else in it.

use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::accounts::Accounts;
use crate::history::{self, Source, TrackInfo};
use crate::plays::{MAX_SECONDS, MAX_TRACK_ID};

/// The marker file a demo data directory holds.
pub const MARKER: &str = "demo-data-dir";
/// Largest input, in bytes.
pub const MAX_INPUT: usize = 4 * 1024 * 1024;
/// Most plays seeded at once.
pub const MAX_PLAYS: usize = 20_000;

/// One invented play.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Play {
    track_id: u64,
    album_id: Option<u64>,
    artist_id: Option<u64>,
    year: Option<i64>,
    played_at: u64,
    seconds: f64,
}

/// Why seeding was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The data directory isn't a demo's.
    NotDemo,
    /// No such user.
    NoUser,
    /// The user already has listening history.
    HasHistory,
    /// The input is too large, malformed, or a play is out of bounds.
    BadInput(String),
    Storage(String),
}

fn bad(s: impl Into<String>) -> Refused {
    Refused::BadInput(s.into())
}

fn check(p: &Play, now: u64) -> Result<(), String> {
    let id_ok = |n: u64| (1..=MAX_TRACK_ID).contains(&n);
    if !id_ok(p.track_id) {
        return Err("track_id out of range".into());
    }
    if p.album_id.is_some_and(|n| !id_ok(n)) || p.artist_id.is_some_and(|n| !id_ok(n)) {
        return Err("album_id or artist_id out of range".into());
    }
    if p.year.is_some_and(|y| !(0..=9999).contains(&y)) {
        return Err("year out of range".into());
    }
    let oldest = now.saturating_sub(history::RETENTION.as_secs());
    if p.played_at < oldest || p.played_at > now {
        return Err("played_at outside the history's retention".into());
    }
    if !p.seconds.is_finite() || !(0.0..=MAX_SECONDS).contains(&p.seconds) {
        return Err("seconds out of range".into());
    }
    Ok(())
}

/// The stable key an invented track is counted by (64 hex, like a real one, never equal
/// to one: real keys are salted per installation).
fn demo_key(track_id: u64) -> String {
    hex::encode(Sha256::digest(format!(
        "lyrion-anywhere demo track\n{track_id}"
    )))
}

/// Seed `input` (a JSON array of plays) as `username`'s history. Returns how many were
/// recorded.
pub fn seed(
    accounts: &Accounts,
    data_dir: &Path,
    username: &str,
    input: &[u8],
    now: u64,
) -> Result<usize, Refused> {
    if !data_dir.join(MARKER).is_file() {
        return Err(Refused::NotDemo);
    }
    if input.len() > MAX_INPUT {
        return Err(bad("input too large"));
    }
    let plays: Vec<Play> =
        serde_json::from_slice(input).map_err(|e| bad(format!("not a list of plays: {e}")))?;
    if plays.len() > MAX_PLAYS {
        return Err(bad("too many plays"));
    }
    for (i, p) in plays.iter().enumerate() {
        check(p, now).map_err(|e| bad(format!("play {i}: {e}")))?;
    }
    let storage = |e| Refused::Storage(format!("{e:?}"));
    let user = accounts
        .user_id_by_name(username)
        .map_err(storage)?
        .ok_or(Refused::NoUser)?;
    if history::count(accounts, user).map_err(storage)? > 0 {
        return Err(Refused::HasHistory);
    }
    let mut n = 0;
    for p in &plays {
        let track = TrackInfo {
            id: p.track_id,
            key: demo_key(p.track_id),
            album_id: p.album_id.map(|v| v as i64),
            artist_id: p.artist_id.map(|v| v as i64),
            year: p.year,
            duration: None,
        };
        if history::record(accounts, user, &track, p.played_at, p.seconds, Source::Live)
            .map_err(storage)?
            .is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Role;

    const NOW: u64 = 1_790_000_000;

    /// A scratch directory, removed when dropped.
    struct Dir(std::path::PathBuf);
    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(marker: bool) -> (Dir, Accounts) {
        let dir = Dir(std::env::temp_dir().join(format!("anywhere-demo-{}", crate::random_id())));
        std::fs::create_dir_all(dir.path()).unwrap();
        if marker {
            std::fs::write(dir.path().join(MARKER), "").unwrap();
        }
        let a = Accounts::in_memory().unwrap();
        a.create_user("demo", "demo-password-123", Role::Listener)
            .unwrap();
        (dir, a)
    }

    fn play(track: u64, at: u64) -> String {
        format!(
            r#"{{"track_id":{track},"album_id":1,"artist_id":2,"year":2019,"played_at":{at},"seconds":200}}"#
        )
    }

    fn list(items: &[String]) -> Vec<u8> {
        format!("[{}]", items.join(",")).into_bytes()
    }

    #[test]
    fn seeds_a_demo_users_history() {
        let (dir, a) = setup(true);
        let input = list(&[play(101, NOW - 3600), play(102, NOW - 7200)]);
        assert_eq!(seed(&a, dir.path(), "demo", &input, NOW), Ok(2));
        let user = a.user_id_by_name("demo").unwrap().unwrap();
        let (rows, _) = history::list(&a, user, 10, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].track_id, 101);
        assert_eq!(rows[0].track_key.len(), 64);
        assert_ne!(rows[0].track_key, rows[1].track_key);
    }

    #[test]
    fn refused_outside_a_demo_data_directory() {
        let (dir, a) = setup(false);
        let input = list(&[play(101, NOW - 3600)]);
        assert_eq!(
            seed(&a, dir.path(), "demo", &input, NOW),
            Err(Refused::NotDemo)
        );
        // A directory of that name is not the marker.
        std::fs::create_dir(dir.path().join(MARKER)).unwrap();
        assert_eq!(
            seed(&a, dir.path(), "demo", &input, NOW),
            Err(Refused::NotDemo)
        );
        let user = a.user_id_by_name("demo").unwrap().unwrap();
        assert_eq!(history::count(&a, user).unwrap(), 0);
    }

    #[test]
    fn refused_for_unknown_users_and_existing_history() {
        let (dir, a) = setup(true);
        let input = list(&[play(101, NOW - 3600)]);
        assert_eq!(
            seed(&a, dir.path(), "nobody", &input, NOW),
            Err(Refused::NoUser)
        );
        assert_eq!(seed(&a, dir.path(), "demo", &input, NOW), Ok(1));
        // Never twice, never on top of real history.
        assert_eq!(
            seed(&a, dir.path(), "demo", &input, NOW),
            Err(Refused::HasHistory)
        );
    }

    #[test]
    fn input_is_bounded_and_checked() {
        let (dir, a) = setup(true);
        let refused = |input: &[u8]| {
            matches!(
                seed(&a, dir.path(), "demo", input, NOW),
                Err(Refused::BadInput(_))
            )
        };
        assert!(refused(&vec![b' '; MAX_INPUT + 1]));
        let many: Vec<String> = (0..=MAX_PLAYS as u64).map(|i| play(101, NOW - i)).collect();
        assert!(refused(&list(&many)));
        for bad in [
            &b"not json"[..],
            b"{}",
            b"[1]",
            br#"[{"track_id":101}]"#,
            br#"[{"track_id":0,"played_at":1789990000,"seconds":1}]"#,
            br#"[{"track_id":1000000000000,"played_at":1789990000,"seconds":1}]"#,
            br#"[{"track_id":-1,"played_at":1789990000,"seconds":1}]"#,
            br#"[{"track_id":1,"album_id":0,"played_at":1789990000,"seconds":1}]"#,
            br#"[{"track_id":1,"year":10000,"played_at":1789990000,"seconds":1}]"#,
            br#"[{"track_id":1,"played_at":1790000001,"seconds":1}]"#,
            br#"[{"track_id":1,"played_at":1,"seconds":1}]"#,
            br#"[{"track_id":1,"played_at":1789990000,"seconds":-1}]"#,
            br#"[{"track_id":1,"played_at":1789990000,"seconds":86401}]"#,
            br#"[{"track_id":1,"played_at":1789990000,"seconds":1,"user":"admin"}]"#,
        ] {
            assert!(refused(bad), "{}", String::from_utf8_lossy(bad));
        }
        // One bad play refuses them all: nothing is recorded.
        let mixed = list(&[play(101, NOW - 10), play(0, NOW - 20)]);
        assert!(refused(&mixed));
        let user = a.user_id_by_name("demo").unwrap().unwrap();
        assert_eq!(history::count(&a, user).unwrap(), 0);
    }

    #[test]
    fn arbitrary_input_never_panics() {
        let (dir, a) = setup(true);
        let mut seed_n: u64 = 0x9E37_79B9_7F4A_7C15;
        for len in 0..400usize {
            let mut buf = Vec::with_capacity(len);
            for _ in 0..len {
                seed_n ^= seed_n << 13;
                seed_n ^= seed_n >> 7;
                seed_n ^= seed_n << 17;
                buf.push(
                    b"[]{},:\"0123456789eE.-track_idplayed_at seconds"[(seed_n % 46) as usize],
                );
            }
            let _ = seed(&a, dir.path(), "demo", &buf, NOW);
        }
    }
}

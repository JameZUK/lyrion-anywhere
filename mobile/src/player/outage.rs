//! Controls that work while the server can't be reached (offline controls).
//!
//! LMS owns the queue (SPEC D14), so while the player's link to it is down nothing it
//! decides can happen. What this phone can still do is play what it already holds:
//!
//! - **Play and pause** at once, on this phone, keeping the position.
//! - **Next, previous and jumping** to a queue entry whose audio is on the phone from start
//!   to end: a download of that track, a whole stream in the replay cache that LMS said was
//!   that track from its start ([`super::cache::Entry::whole`]), or a stream spooled whole.
//!   The phone then plays its own copies ("takes over"), through the same audio path, and
//!   carries on into the entries after it while they are on the phone too; at the first one
//!   that isn't, it stops at the end and waits for the server.
//! - **Seeking** within a track it has whole.
//!
//! The queue is what the app last saw of it ([`Snapshot`], in LMS's play order). When the
//! link comes back the phone's state is the truth if the user did anything meanwhile: LMS
//! is told to play the same entry at the same place, paused or not, but only if that queue
//! position still holds that track (see [`super::protocol::restart_commands`]). Without
//! anything done on the phone, nothing changes from before.
//!
//! The rules (what is available, what each control does, what to tell LMS) are pure
//! functions here, so each is unit-tested; the engine applies them.

use std::collections::HashMap;
use std::path::PathBuf;

use anywhere_core::proto::{Strm, encode_server_frame};

use super::cache::Entry;
use super::local::{LocalSource, LocalTrack, Opener, RESTART_SECS, Transport};
use super::protocol::RestartPoint;
use super::queue::LocalRepeat;

/// Most queue entries the app's snapshot may hold (LMS's queue as the app read it).
pub const MAX_SNAPSHOT: usize = 10_000;
/// Within this much of a track's start, "previous" goes to the entry before.
const RESTART_MS: u64 = (RESTART_SECS * 1000.0) as u64;

/// Where a queue entry's audio is on this phone, whole, for playing with no server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LocalAvailability {
    /// Not on the phone (or not all of it).
    None,
    /// Downloaded.
    Downloaded,
    /// Kept from streaming (the replay cache, or a stream received whole).
    OnDevice,
}

/// What this phone's player can do by itself while the server can't be reached. Present in
/// [`super::LocalPlayerStatus`] only while the link is down.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflineControls {
    /// The queue position heard now (or about to be), as far as the phone knows.
    pub index: Option<u32>,
    /// Its LMS track id, when known.
    pub track_id: Option<i64>,
    /// Playing on this phone (false: paused or stopped).
    pub playing: bool,
    /// Position in that track.
    pub elapsed_ms: u64,
    /// Play and pause work.
    pub play_pause: bool,
    /// The next entry is on the phone.
    pub next: bool,
    /// The previous entry (or, past the first seconds, the start of this one) is.
    pub previous: bool,
    /// The track playing is on the phone whole, so it can be sought in.
    pub seek: bool,
    /// The phone plays its own copies of the queue (after a skip, a jump or a track's end).
    pub local: bool,
    /// Stopped at the end of a track: the next one needs the server.
    pub waiting_for_server: bool,
}

/// A control used on the phone while the server can't be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalCmd {
    Play,
    Pause,
    Next,
    Previous,
    /// Play the entry at this queue position.
    Jump(u32),
    /// Go to this position (ms) in the current track.
    Seek(u64),
}

/// LMS's queue as the app last saw it: track ids in play order (LMS's `playlist index`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    /// Positive ids; anything else (a radio stream, an unknown entry) is 0, never on the
    /// phone.
    ids: Vec<i64>,
    /// The position LMS said was current.
    current: Option<usize>,
    repeat: Repeat,
}

/// [`LocalRepeat`], with a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl From<LocalRepeat> for Repeat {
    fn from(r: LocalRepeat) -> Repeat {
        match r {
            LocalRepeat::Off => Repeat::Off,
            LocalRepeat::All => Repeat::All,
            LocalRepeat::One => Repeat::One,
        }
    }
}

impl Snapshot {
    /// From the app (untrusted): at most [`MAX_SNAPSHOT`] entries, odd ids made 0, a
    /// current position that isn't in the list dropped.
    pub(crate) fn new(ids: Vec<i64>, current: Option<u32>, repeat: LocalRepeat) -> Snapshot {
        let ids: Vec<i64> = ids
            .into_iter()
            .take(MAX_SNAPSHOT)
            .map(|id| if valid_id(id) { id } else { 0 })
            .collect();
        let current = current.map(|c| c as usize).filter(|c| *c < ids.len());
        Snapshot {
            ids,
            current,
            repeat: repeat.into(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// The track at queue position `i` (`None`: no such position, or not a library track).
    pub(crate) fn id(&self, i: usize) -> Option<i64> {
        self.ids.get(i).copied().filter(|id| *id > 0)
    }

    pub(crate) fn current(&self) -> Option<usize> {
        self.current
    }

    /// The position whose track is `id`, nearest to `near` (where it was last heard).
    pub(crate) fn position_of(&self, id: i64, near: Option<usize>) -> Option<usize> {
        if let Some(n) = near
            && self.id(n) == Some(id)
        {
            return Some(n);
        }
        let near = near.unwrap_or(0);
        self.ids
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == id && id > 0)
            .min_by_key(|(i, _)| i.abs_diff(near))
            .map(|(i, _)| i)
    }

    /// Where "next" goes from `cur`: the following entry, round to the first under repeat
    /// (as LMS does).
    pub(crate) fn next_of(&self, cur: usize) -> Option<usize> {
        let n = self.ids.len();
        if cur + 1 < n {
            Some(cur + 1)
        } else if self.repeat != Repeat::Off && n > 0 {
            Some(0)
        } else {
            None
        }
    }

    /// Where "previous" goes from `cur` (`None` at the start without repeat).
    pub(crate) fn previous_of(&self, cur: usize) -> Option<usize> {
        let n = self.ids.len();
        if cur >= n {
            None
        } else if cur > 0 {
            Some(cur - 1)
        } else if self.repeat != Repeat::Off {
            Some(n - 1)
        } else {
            None
        }
    }

    /// What plays after `cur` ends by itself: the same track under repeat one, round again
    /// under repeat all, nothing after the last otherwise.
    pub(crate) fn after_end(&self, cur: usize) -> Option<usize> {
        let n = self.ids.len();
        match self.repeat {
            _ if cur >= n => None,
            Repeat::One => Some(cur),
            Repeat::All => Some((cur + 1) % n),
            Repeat::Off => (cur + 1 < n).then_some(cur + 1),
        }
    }

    /// The entries the phone can play in a row from `start` (which must be available): it
    /// and those after it while each is available, round again under repeat all. The repeat
    /// mode for that run: all only when the run is the whole queue (round and round), one
    /// under repeat one (just this track), otherwise none (it stops at the run's end).
    pub(crate) fn run_from(
        &self,
        start: usize,
        available: &dyn Fn(usize) -> bool,
    ) -> Option<(Vec<usize>, LocalRepeat)> {
        if start >= self.ids.len() || !available(start) {
            return None;
        }
        if self.repeat == Repeat::One {
            return Some((vec![start], LocalRepeat::One));
        }
        let mut run = vec![start];
        let mut at = start;
        while let Some(next) = self.after_end(at) {
            if next == start {
                return Some((run, LocalRepeat::All));
            }
            if !available(next) {
                break;
            }
            run.push(next);
            at = next;
        }
        Some((run, LocalRepeat::Off))
    }
}

/// A library track id as LMS gives them.
fn valid_id(id: i64) -> bool {
    (1..=999_999_999_999).contains(&id)
}

/// What "previous" does from where playback is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Back {
    /// Start the current track again.
    Restart,
    /// Go to this queue position.
    To(usize),
}

/// "Previous" as LMS does it: past the first [`RESTART_SECS`], the start of this track;
/// within them, the track before (or this one again at the start of the queue).
pub(crate) fn previous_step(snap: &Snapshot, cur: usize, elapsed_ms: u64) -> Back {
    if elapsed_ms >= RESTART_MS {
        return Back::Restart;
    }
    snap.previous_of(cur).map_or(Back::Restart, Back::To)
}

/// What the gating needs to know, at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Moment {
    /// The queue position heard now.
    pub cur: Option<usize>,
    /// Its track id, when known (it decides over a position that disagrees).
    pub track_id: Option<i64>,
    pub elapsed_ms: u64,
    pub playing: bool,
    /// Something is loaded that play and pause can act on.
    pub loaded: bool,
    /// Playing the phone's own copies.
    pub local: bool,
    pub waiting: bool,
}

/// Which controls work now (see the table in the offline controls spec). `available`: queue
/// position → whether its track is on the phone whole.
pub(crate) fn controls(
    snap: &Snapshot,
    m: Moment,
    available: &dyn Fn(usize) -> bool,
) -> OfflineControls {
    let cur = m.cur.filter(|c| *c < snap.len());
    let here = cur.is_some_and(available);
    let next = cur.and_then(|c| snap.next_of(c)).is_some_and(available);
    let previous = cur.is_some_and(|c| match previous_step(snap, c, m.elapsed_ms) {
        Back::Restart => here,
        Back::To(i) => available(i),
    });
    OfflineControls {
        index: cur.map(|c| c as u32),
        track_id: m.track_id.or_else(|| cur.and_then(|c| snap.id(c))),
        playing: m.playing,
        elapsed_ms: m.elapsed_ms,
        // Waiting at a track's end, play can't start anything (the next needs the server).
        play_pause: m.loaded && !m.waiting,
        next,
        previous,
        seek: here && !m.waiting,
        local: m.local,
        waiting_for_server: m.waiting,
    }
}

/// Where each LMS track is on the phone, whole: downloads first, then streams kept.
#[derive(Default)]
pub(crate) struct Sources {
    pub downloads: HashMap<i64, LocalTrack>,
    /// Whole replay-cache entries by track, with their files.
    pub cache: HashMap<i64, (Entry, PathBuf)>,
    /// Streams spooled whole, by track.
    pub spools: HashMap<i64, LocalTrack>,
}

impl Sources {
    pub(crate) fn availability(&self, id: i64) -> LocalAvailability {
        if !valid_id(id) {
            LocalAvailability::None
        } else if self.downloads.contains_key(&id) {
            LocalAvailability::Downloaded
        } else if self.cache.contains_key(&id) || self.spools.contains_key(&id) {
            LocalAvailability::OnDevice
        } else {
            LocalAvailability::None
        }
    }

    /// The file to play for track `id` (a download, then a kept stream).
    pub(crate) fn track(&self, id: i64) -> Option<LocalTrack> {
        if !valid_id(id) {
            return None;
        }
        if let Some(t) = self.downloads.get(&id) {
            return Some(t.clone());
        }
        if let Some(t) = self.spools.get(&id) {
            return Some(t.clone());
        }
        let (e, path) = self.cache.get(&id)?;
        Some(LocalTrack {
            key: format!("{CACHE_KEY}{}", e.fp.hex()),
            track_id: id,
            path: path.clone(),
            ext: anywhere_core::decode::format_extension(e.format)?.into(),
            duration_secs: 0.0,
            album: None,
        })
    }
}

/// How a [`LocalTrack`]'s key names a replay-cache entry (its file is checked whole before
/// it is played; see the engine's opener).
pub(crate) const CACHE_KEY: &str = "cache:";
/// … a download.
pub(crate) const DOWNLOAD_KEY: &str = "download:";

/// The phone playing its own copies of the queue.
pub(crate) struct Takeover {
    pub source: LocalSource,
    /// The queue position of each track given to `source`.
    positions: Vec<usize>,
    pub last_flush: u64,
    /// The run played to its end: the next entry needs the server.
    pub waiting: bool,
    pub was_playing: bool,
}

impl Takeover {
    /// Start playing the queue at `start`, `ms` in, from the phone's copies. `None` if
    /// `start` isn't on the phone.
    pub(crate) fn start(
        snap: &Snapshot,
        sources: &Sources,
        start: usize,
        ms: u64,
        play: bool,
        opener: Opener,
        seed: u64,
    ) -> Option<Takeover> {
        let mut source = LocalSource::new(opener, Box::new(crate::offline::plays::now_secs), seed);
        // Only entries in a row: one that can't be played ends the run there.
        source.set_stop_at_failure(true);
        let mut t = Takeover {
            source,
            positions: Vec::new(),
            last_flush: 0,
            waiting: false,
            was_playing: false,
        };
        t.play_from(snap, sources, start, ms, play).then_some(t)
    }

    /// Play from queue position `start`, `ms` in (`play`: or paused there). False, and
    /// nothing changed, if it isn't on the phone.
    pub(crate) fn play_from(
        &mut self,
        snap: &Snapshot,
        sources: &Sources,
        start: usize,
        ms: u64,
        play: bool,
    ) -> bool {
        let track_at = |i: usize| snap.id(i).and_then(|id| sources.track(id));
        let Some((run, repeat)) = snap.run_from(start, &|i| track_at(i).is_some()) else {
            return false;
        };
        let tracks: Vec<LocalTrack> = run.iter().filter_map(|i| track_at(*i)).collect();
        if tracks.len() != run.len() {
            return false;
        }
        self.positions = run;
        self.waiting = false;
        self.source.set_queue(tracks, 0, false, play);
        self.source.set_repeat(repeat);
        if ms > 0 {
            self.source.seek(ms);
        }
        true
    }

    /// The queue position audible now (or about to be).
    pub(crate) fn current(&self) -> Option<usize> {
        if self.waiting {
            return self.positions.last().copied();
        }
        self.source
            .current_index()
            .and_then(|i| self.positions.get(i))
            .copied()
    }

    /// The track playing now (its key says where it comes from).
    pub(crate) fn current_track(&self) -> Option<&LocalTrack> {
        let i = self.source.current_index()?;
        self.source.tracks().get(i)
    }

    pub(crate) fn playing(&self) -> bool {
        self.source.transport() == Transport::Playing
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        if self.waiting {
            0
        } else {
            self.source.elapsed_ms()
        }
    }

    /// Once the run has played out (`output_idle`: the output has played every sample it
    /// was given), stop there and wait for the server. True when that happened now.
    pub(crate) fn check_end(&mut self, output_idle: bool) -> bool {
        if !self.playing() || !self.source.queue_exhausted() || !output_idle {
            return false;
        }
        self.source.finish();
        self.waiting = true;
        self.last_flush = self.source.flush_count();
        true
    }

    /// Where LMS should carry on: this entry, here, paused or not.
    pub(crate) fn point(&self, snap: &Snapshot) -> Option<RestartPoint> {
        let index = self.current()?;
        Some(RestartPoint {
            index: u32::try_from(index).ok()?,
            track_id: snap.id(index)?,
            // At the end of the run, the start of the last entry, paused: LMS carries on
            // from there when told to play.
            elapsed_ms: self.elapsed_ms(),
            paused: !self.playing() || self.waiting,
            local: true,
        })
    }
}

/// A `strm` frame as LMS would send it, for the phone's own pause (`p`) and unpause (`u`)
/// of the player LMS was driving. Only these two commands are ever made here.
pub(crate) fn local_strm(command: u8) -> Option<Vec<u8>> {
    if !matches!(command, b'p' | b'u') {
        return None;
    }
    let s = Strm {
        command,
        autostart: b'0',
        format: b'?',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 0,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        // A pause for good (not a sync interval); an unpause now (no start time).
        replay_gain: 0,
        server_port: 0,
        server_ip: 0,
        request: Vec::new(),
    };
    Some(encode_server_frame(*b"strm", &s.encode_payload()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(n: usize, repeat: LocalRepeat) -> Snapshot {
        Snapshot::new((1..=n as i64).map(|i| 100 + i).collect(), Some(0), repeat)
    }

    fn only(set: &'static [usize]) -> impl Fn(usize) -> bool {
        move |i| set.contains(&i)
    }

    #[test]
    fn snapshots_from_the_app_are_cleaned() {
        let s = Snapshot::new(
            vec![5, 0, -3, 1_000_000_000_000, 7],
            Some(9),
            LocalRepeat::Off,
        );
        assert_eq!(s.len(), 5);
        assert_eq!(
            (0..6).map(|i| s.id(i)).collect::<Vec<_>>(),
            vec![Some(5), None, None, None, Some(7), None]
        );
        assert_eq!(
            s.current(),
            None,
            "a current position past the end is dropped"
        );
        let big = Snapshot::new(vec![1; MAX_SNAPSHOT + 10], Some(3), LocalRepeat::All);
        assert_eq!(big.len(), MAX_SNAPSHOT);
        assert_eq!(big.current(), Some(3));
        assert_eq!(Snapshot::default().next_of(0), None);
        assert_eq!(Snapshot::default().previous_of(0), None);
        assert_eq!(Snapshot::default().after_end(0), None);
    }

    #[test]
    fn next_previous_and_the_end_follow_lms() {
        let off = snap(3, LocalRepeat::Off);
        assert_eq!(off.next_of(0), Some(1));
        assert_eq!(off.next_of(2), None, "the end without repeat");
        assert_eq!(off.previous_of(0), None);
        assert_eq!(off.previous_of(2), Some(1));
        assert_eq!(off.previous_of(7), None, "no such position");
        assert_eq!(off.after_end(1), Some(2));
        assert_eq!(off.after_end(2), None);
        let all = snap(3, LocalRepeat::All);
        assert_eq!(all.next_of(2), Some(0), "repeat all wraps");
        assert_eq!(all.previous_of(0), Some(2));
        assert_eq!(all.after_end(2), Some(0));
        let one = snap(3, LocalRepeat::One);
        assert_eq!(one.after_end(1), Some(1), "repeat one plays it again");
        assert_eq!(one.next_of(1), Some(2), "next still moves on");
        assert_eq!(one.next_of(2), Some(0));
    }

    #[test]
    fn previous_restarts_after_the_first_seconds() {
        let s = snap(3, LocalRepeat::Off);
        assert_eq!(previous_step(&s, 1, 0), Back::To(0));
        assert_eq!(previous_step(&s, 1, RESTART_MS - 1), Back::To(0));
        assert_eq!(previous_step(&s, 1, RESTART_MS), Back::Restart);
        assert_eq!(previous_step(&s, 0, 0), Back::Restart, "the first entry");
        assert_eq!(previous_step(&snap(3, LocalRepeat::All), 0, 0), Back::To(2));
    }

    #[test]
    fn runs_cover_what_is_on_the_phone_in_a_row() {
        let s = snap(6, LocalRepeat::Off);
        assert_eq!(
            s.run_from(1, &only(&[1, 2, 4, 5])),
            Some((vec![1, 2], LocalRepeat::Off)),
            "stops before the first entry that isn't"
        );
        assert_eq!(
            s.run_from(3, &only(&[1, 2, 4, 5])),
            None,
            "not on the phone"
        );
        assert_eq!(s.run_from(9, &|_| true), None, "no such position");
        assert_eq!(
            s.run_from(4, &only(&[0, 4, 5])),
            Some((vec![4, 5], LocalRepeat::Off)),
            "no wrap without repeat"
        );
        let all = snap(6, LocalRepeat::All);
        assert_eq!(
            all.run_from(4, &only(&[0, 1, 4, 5])),
            Some((vec![4, 5, 0, 1], LocalRepeat::Off)),
            "repeat all wraps, then stops"
        );
        assert_eq!(
            all.run_from(2, &|_| true),
            Some((vec![2, 3, 4, 5, 0, 1], LocalRepeat::All)),
            "the whole queue: round and round"
        );
        assert_eq!(
            snap(6, LocalRepeat::One).run_from(3, &|_| true),
            Some((vec![3], LocalRepeat::One))
        );
    }

    fn moment(cur: usize, elapsed_ms: u64) -> Moment {
        Moment {
            cur: Some(cur),
            track_id: None,
            elapsed_ms,
            playing: true,
            loaded: true,
            local: false,
            waiting: false,
        }
    }

    #[test]
    fn controls_follow_what_is_on_the_phone() {
        let s = snap(4, LocalRepeat::Off);
        // Nothing on the phone: only play and pause (what is buffered).
        let c = controls(&s, moment(1, 10_000), &|_| false);
        assert!(c.play_pause && !c.next && !c.previous && !c.seek);
        assert_eq!((c.index, c.track_id), (Some(1), Some(102)));
        // The next one is: next works.
        let c = controls(&s, moment(1, 10_000), &only(&[2]));
        assert!(c.next && !c.previous);
        // Past the first seconds, previous restarts this one: it needs this one whole.
        let c = controls(&s, moment(1, 10_000), &only(&[0, 1]));
        assert!(c.previous && c.seek);
        let c = controls(&s, moment(1, 10_000), &only(&[0]));
        assert!(!c.previous, "only the one before is here");
        // Within them, the one before.
        assert!(controls(&s, moment(1, 1_000), &only(&[0])).previous);
        assert!(!controls(&s, moment(1, 1_000), &only(&[1])).previous);
        // The last entry: no next without repeat; with repeat all, the first.
        assert!(!controls(&s, moment(3, 0), &|_| true).next);
        let all = snap(4, LocalRepeat::All);
        assert!(controls(&all, moment(3, 0), &only(&[0])).next);
        // Unknown position: nothing but play and pause.
        let mut m = moment(0, 0);
        m.cur = None;
        let c = controls(&s, m, &|_| true);
        assert!(c.play_pause && !c.next && !c.previous && !c.seek);
        m.cur = Some(99);
        assert_eq!(controls(&s, m, &|_| true).index, None);
        // Nothing loaded: play and pause can't act.
        let mut m = moment(1, 0);
        m.loaded = false;
        assert!(!controls(&s, m, &|_| true).play_pause);
        // Waiting at the end of a track: only moving to an entry on the phone works.
        let mut m = moment(1, 0);
        m.waiting = true;
        let c = controls(&s, m, &only(&[0, 1]));
        assert!(!c.play_pause && !c.seek && c.previous && !c.next && c.waiting_for_server);
    }

    #[test]
    fn availability_prefers_downloads_and_needs_a_real_id() {
        let mut src = Sources::default();
        let t = |id: i64, key: &str| LocalTrack {
            key: key.into(),
            track_id: id,
            path: PathBuf::from(format!("/x/{id}")),
            ext: "flac".into(),
            duration_secs: 1.0,
            album: None,
        };
        src.downloads.insert(5, t(5, "download:a"));
        src.spools.insert(5, t(5, "spool:1"));
        src.spools.insert(6, t(6, "spool:2"));
        assert_eq!(src.availability(5), LocalAvailability::Downloaded);
        assert_eq!(src.track(5).unwrap().key, "download:a");
        assert_eq!(src.availability(6), LocalAvailability::OnDevice);
        assert_eq!(src.availability(7), LocalAvailability::None);
        for bad in [0, -5, i64::MAX] {
            assert_eq!(src.availability(bad), LocalAvailability::None);
            assert!(src.track(bad).is_none());
        }
    }

    #[test]
    fn local_strm_frames_are_only_pause_and_unpause() {
        use anywhere_core::proto::{ServerFrameReader, ServerMsg};
        for c in *b"pu" {
            let f = local_strm(c).unwrap();
            let mut r = ServerFrameReader::default();
            r.push(&f);
            match r.next_msg() {
                Ok(Some(ServerMsg::Strm(s))) => {
                    assert_eq!(s.command, c);
                    assert_eq!(s.replay_gain, 0, "for good, and now");
                }
                other => panic!("{other:?}"),
            }
        }
        for c in [b's', b'q', b'f', b't', b'a', 0, 0xff] {
            assert!(local_strm(c).is_none(), "{c}");
        }
    }

    /// Arbitrary snapshots, positions and availability: nothing panics, and every answer
    /// stays within the queue.
    #[test]
    fn arbitrary_queues_never_panic() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3000 {
            let n = (rnd() % 12) as usize;
            let ids: Vec<i64> = (0..n).map(|_| (rnd() % 7) as i64 - 2).collect();
            let repeat =
                [LocalRepeat::Off, LocalRepeat::All, LocalRepeat::One][(rnd() % 3) as usize];
            let s = Snapshot::new(ids, Some((rnd() % 14) as u32), repeat);
            let mask = rnd();
            let avail = move |i: usize| ((mask >> (i % 64)) & 1) == 1;
            let cur = (rnd() % 14) as usize;
            for f in [s.next_of(cur), s.previous_of(cur), s.after_end(cur)] {
                assert!(f.is_none_or(|i| i < s.len()));
            }
            if let Some((run, _)) = s.run_from(cur, &avail) {
                assert!(run.len() <= s.len().max(1));
                assert!(run.iter().all(|i| *i < s.len() && avail(*i)));
                let mut d = run.clone();
                d.sort();
                d.dedup();
                assert_eq!(d.len(), run.len(), "never the same entry twice");
            }
            let mut m = moment(cur, rnd() % 10_000);
            m.waiting = rnd() % 2 == 0;
            let c = controls(&s, m, &avail);
            assert!(c.index.is_none_or(|i| (i as usize) < s.len()));
        }
    }
    // --- The takeover, on in-memory tracks -----------------------------------------------

    /// A 16-bit stereo WAV whose samples all hold `level` (to tell tracks apart).
    fn wav(rate: u32, frames: usize, level: i16) -> Vec<u8> {
        let data_len = frames * 4;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 4).to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data_len as u32).to_le_bytes());
        for _ in 0..frames * 2 {
            v.extend_from_slice(&level.to_le_bytes());
        }
        v
    }

    const RATE: u32 = 10_000;
    const FRAMES: usize = 20_000; // 2 s a track

    /// Sources for the queue positions in `on` (track `101 + i`), and an opener that plays
    /// track `id` as a WAV at level `id`.
    fn on_phone(on: &[usize]) -> (Sources, Opener) {
        let mut s = Sources::default();
        for i in on {
            let id = 101 + *i as i64;
            s.downloads.insert(
                id,
                LocalTrack {
                    key: format!("{DOWNLOAD_KEY}{id}"),
                    track_id: id,
                    path: PathBuf::from(format!("/nowhere/{id}")),
                    ext: "wav".into(),
                    duration_secs: 2.0,
                    album: None,
                },
            );
        }
        let opener: Opener = Box::new(|t: &LocalTrack| {
            anywhere_core::decode::FileDecoder::open(
                Box::new(std::io::Cursor::new(wav(RATE, FRAMES, t.track_id as i16))),
                Some("wav"),
            )
        });
        (s, opener)
    }

    /// Play `frames` (or to the end), as the output would; the track ids heard, in order.
    fn play(t: &mut Takeover, frames: usize) -> Vec<i64> {
        use super::super::audio::AudioSource;
        let mut heard: Vec<i64> = Vec::new();
        let mut block = vec![0f32; 2048];
        let mut got = 0;
        for _ in 0..100_000 {
            t.source.decode_ahead();
            let want = (frames - got).min(1024);
            if want == 0 {
                break;
            }
            match t.source.pull_track(&mut block[..want * 2]) {
                Some((_, n)) => {
                    let id = (block[0] * 32768.0).round() as i64;
                    if heard.last() != Some(&id) {
                        heard.push(id);
                    }
                    t.source.played(n as u64);
                    got += n;
                }
                None if t.check_end(true) => break,
                None if !t.playing() => break,
                None => {}
            }
        }
        heard
    }

    #[test]
    fn a_takeover_plays_on_through_what_is_on_the_phone_then_waits() {
        let s = snap(5, LocalRepeat::Off);
        let (src, opener) = on_phone(&[1, 2, 4]);
        // Not on the phone: no takeover.
        let (_, o) = on_phone(&[]);
        assert!(Takeover::start(&s, &src, 3, 0, true, o, 1).is_none());
        let mut t = Takeover::start(&s, &src, 1, 0, true, opener, 1).unwrap();
        assert_eq!(t.current(), Some(1));
        // 1 then 2 (in a row), and it stops before 3, which isn't: never 4.
        assert_eq!(play(&mut t, usize::MAX), vec![102, 103]);
        assert!(t.waiting, "waits for the server");
        assert!(!t.playing());
        assert_eq!(t.current(), Some(2), "at the last entry it played");
        let p = t.point(&s).unwrap();
        assert_eq!(
            (p.index, p.track_id, p.paused, p.local),
            (2, 103, true, true)
        );
        // A jump to an entry on the phone starts again from there.
        assert!(t.play_from(&s, &src, 4, 0, true));
        assert!(!t.waiting);
        assert_eq!(play(&mut t, 100), vec![105]);
        // One that isn't: refused, and nothing changes.
        assert!(!t.play_from(&s, &src, 3, 0, true));
        assert_eq!(t.current(), Some(4));
        assert!(t.playing());
    }

    #[test]
    fn a_takeover_pauses_seeks_and_says_where_it_is() {
        let s = snap(3, LocalRepeat::Off);
        let (src, opener) = on_phone(&[0, 1, 2]);
        let mut t = Takeover::start(&s, &src, 1, 0, true, opener, 1).unwrap();
        play(&mut t, RATE as usize / 2);
        assert_eq!(t.elapsed_ms(), 500);
        t.source.pause();
        assert!(!t.playing());
        assert!(
            play(&mut t, 100).is_empty(),
            "paused: nothing more is heard"
        );
        let p = t.point(&s).unwrap();
        assert_eq!((p.index, p.track_id, p.elapsed_ms), (1, 102, 500));
        assert!(p.paused && p.local);
        t.source.play();
        assert!(!t.point(&s).unwrap().paused);
        // A seek within the track, kept by the next point.
        t.source.seek(1_500);
        play(&mut t, 10);
        assert_eq!(t.point(&s).unwrap().elapsed_ms, 1_501);
        // Starting part-way in (a seek before the takeover).
        let (src, opener) = on_phone(&[0, 1, 2]);
        let mut t = Takeover::start(&s, &src, 2, 1_000, false, opener, 1).unwrap();
        assert!(!t.playing());
        t.source.play();
        play(&mut t, 10);
        assert_eq!(t.elapsed_ms(), 1_001);
    }

    #[test]
    fn a_takeover_wraps_under_repeat_all_and_loops_under_repeat_one() {
        let all = snap(3, LocalRepeat::All);
        let (src, opener) = on_phone(&[0, 1, 2]);
        let mut t = Takeover::start(&all, &src, 2, 0, true, opener, 1).unwrap();
        let heard = play(&mut t, FRAMES * 4);
        assert_eq!(heard, vec![103, 101, 102, 103], "round and round");
        assert!(!t.waiting);
        // Only part of the queue: it wraps, then stops at the first gap.
        let (src, opener) = on_phone(&[0, 2]);
        let mut t = Takeover::start(&all, &src, 2, 0, true, opener, 1).unwrap();
        assert_eq!(play(&mut t, usize::MAX), vec![103, 101]);
        assert!(t.waiting);
        assert_eq!(t.current(), Some(0));
        let one = snap(3, LocalRepeat::One);
        let (src, opener) = on_phone(&[1]);
        let mut t = Takeover::start(&one, &src, 1, 0, true, opener, 1).unwrap();
        play(&mut t, FRAMES * 3);
        assert_eq!(t.current(), Some(1), "the same track again and again");
        assert!(!t.waiting);
    }

    #[test]
    fn a_missing_copy_ends_the_run_there() {
        let s = snap(3, LocalRepeat::Off);
        let (src, _) = on_phone(&[0, 1, 2]);
        // The second file can't be opened (deleted meanwhile): the run stops after the first.
        let opener: Opener = Box::new(|t: &LocalTrack| {
            if t.track_id == 102 {
                return Err("the download is missing".into());
            }
            anywhere_core::decode::FileDecoder::open(
                Box::new(std::io::Cursor::new(wav(RATE, FRAMES, t.track_id as i16))),
                Some("wav"),
            )
        });
        let mut t = Takeover::start(&s, &src, 0, 0, true, opener, 1).unwrap();
        assert_eq!(play(&mut t, usize::MAX), vec![101], "never skipped over");
        assert!(t.waiting);
    }
}

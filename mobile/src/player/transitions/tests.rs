use super::*;
use anywhere_core::dsp::transition::{fade_gains, join};
use std::collections::HashMap;

const RATE: u32 = 44_100;

fn facts(album: &str, duration_s: f64) -> TrackFacts {
    TrackFacts {
        album: Some(album.into()),
        duration_s: Some(duration_s),
        edges: None,
    }
}

fn edges(lead: f64, trail: f64, fade_in: Option<f64>, fade_out: Option<f64>) -> EdgeFacts {
    EdgeFacts {
        leading_silence_s: lead,
        trailing_silence_s: trail,
        fade_in_s: fade_in,
        fade_out_s: fade_out,
    }
}

fn cfg(mode: TransitionMode, fade_s: f64) -> TransitionConfig {
    TransitionConfig { mode, fade_s }
}

// --- Planning (pure) ----------------------------------------------------------------------

#[test]
fn off_is_always_gapless() {
    let a = facts("1", 200.0);
    let b = facts("2", 200.0);
    assert_eq!(
        plan(TransitionConfig::default(), Some(&a), Some(&b), RATE, RATE),
        Join::Gapless(GaplessWhy::Off)
    );
}

#[test]
fn the_same_album_stays_gapless() {
    let a = facts("7", 200.0);
    let b = facts("7", 180.0);
    for mode in [TransitionMode::Crossfade, TransitionMode::Smart] {
        assert_eq!(
            plan(cfg(mode, 6.0), Some(&a), Some(&b), RATE, RATE),
            Join::Gapless(GaplessWhy::SameAlbum),
            "{mode:?}"
        );
    }
    // Repeating one track: the same album, so gapless too.
    assert_eq!(
        plan(
            cfg(TransitionMode::Smart, 6.0),
            Some(&a),
            Some(&a),
            RATE,
            RATE
        ),
        Join::Gapless(GaplessWhy::SameAlbum)
    );
}

#[test]
fn an_unknown_album_or_track_is_never_risked() {
    let a = facts("1", 200.0);
    let unknown = TrackFacts {
        album: None,
        ..facts("x", 200.0)
    };
    let c = cfg(TransitionMode::Crossfade, 6.0);
    assert_eq!(
        plan(c, Some(&a), None, RATE, RATE),
        Join::Gapless(GaplessWhy::Unknown)
    );
    assert_eq!(
        plan(c, None, Some(&a), RATE, RATE),
        Join::Gapless(GaplessWhy::Unknown)
    );
    assert_eq!(
        plan(c, Some(&a), Some(&unknown), RATE, RATE),
        Join::Gapless(GaplessWhy::Unknown)
    );
    assert_eq!(
        plan(c, Some(&unknown), Some(&a), RATE, RATE),
        Join::Gapless(GaplessWhy::Unknown)
    );
}

#[test]
fn a_rate_change_is_gapless() {
    let a = facts("1", 200.0);
    let b = facts("2", 200.0);
    assert_eq!(
        plan(
            cfg(TransitionMode::Crossfade, 6.0),
            Some(&a),
            Some(&b),
            44_100,
            96_000
        ),
        Join::Gapless(GaplessWhy::RateChange)
    );
}

#[test]
fn a_fixed_crossfade_uses_the_setting() {
    let a = facts("1", 200.0);
    let b = facts("2", 200.0);
    assert_eq!(
        plan(
            cfg(TransitionMode::Crossfade, 6.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE
        ),
        Join::Crossfade {
            trim_out: 0,
            trim_in: 0,
            overlap: 6 * 44_100
        }
    );
    // Clamped to the supported range; nonsense becomes the default.
    for (asked, got) in [(0.0, 1.0), (60.0, 10.0), (f64::NAN, 6.0), (-3.0, 1.0)] {
        assert_eq!(
            plan(
                cfg(TransitionMode::Crossfade, asked),
                Some(&a),
                Some(&b),
                RATE,
                RATE
            ),
            Join::Crossfade {
                trim_out: 0,
                trim_in: 0,
                overlap: (got * 44_100.0) as u64
            },
            "{asked}"
        );
    }
}

#[test]
fn smart_without_analysis_falls_back_to_the_fixed_crossfade() {
    let a = facts("1", 200.0);
    let mut b = facts("2", 200.0);
    b.edges = Some(edges(1.0, 1.0, None, None));
    assert_eq!(
        plan(
            cfg(TransitionMode::Smart, 5.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE
        ),
        Join::Crossfade {
            trim_out: 0,
            trim_in: 0,
            overlap: 5 * 44_100
        }
    );
}

#[test]
fn smart_trims_silence_and_follows_the_fades() {
    let mut a = facts("1", 200.0);
    let mut b = facts("2", 200.0);
    // A fades out over 4 s, then 1.5 s of silence; B starts after 0.5 s of silence.
    a.edges = Some(edges(0.0, 1.5, None, Some(4.0)));
    b.edges = Some(edges(0.5, 0.0, Some(3.0), None));
    assert_eq!(
        plan(
            cfg(TransitionMode::Smart, 6.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE
        ),
        Join::Crossfade {
            trim_out: (1.5 * 44_100.0) as u64,
            trim_in: (0.5 * 44_100.0) as u64,
            overlap: 4 * 44_100
        }
    );
    // No fade-out: the incoming fade-in decides.
    a.edges = Some(edges(0.0, 0.0, None, None));
    assert!(matches!(
        plan(cfg(TransitionMode::Smart, 6.0), Some(&a), Some(&b), RATE, RATE),
        Join::Crossfade { overlap, .. } if overlap == 3 * 44_100
    ));
    // Two abrupt ends: a short blend.
    b.edges = Some(edges(0.0, 0.0, None, None));
    assert!(matches!(
        plan(cfg(TransitionMode::Smart, 6.0), Some(&a), Some(&b), RATE, RATE),
        Join::Crossfade { overlap, .. } if overlap == 2 * 44_100
    ));
    // A long fade is capped by the setting.
    a.edges = Some(edges(0.0, 0.0, None, Some(9.0)));
    assert!(matches!(
        plan(cfg(TransitionMode::Smart, 3.0), Some(&a), Some(&b), RATE, RATE),
        Join::Crossfade { overlap, .. } if overlap == 3 * 44_100
    ));
}

#[test]
fn hostile_analysis_values_are_tamed() {
    let mut a = facts("1", 200.0);
    let mut b = facts("2", 200.0);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0, 1e300] {
        a.edges = Some(edges(bad, bad, Some(bad), Some(bad)));
        b.edges = Some(edges(bad, bad, Some(bad), Some(bad)));
        match plan(
            cfg(TransitionMode::Smart, 6.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE,
        ) {
            Join::Crossfade {
                trim_out,
                trim_in,
                overlap,
            } => {
                assert!(trim_out <= (MAX_TRIM_S * 44_100.0) as u64, "{bad}");
                assert!(trim_in <= (MAX_TRIM_S * 44_100.0) as u64, "{bad}");
                assert!((44_100..=6 * 44_100).contains(&overlap), "{bad}");
            }
            j => panic!("{bad}: {j:?}"),
        }
    }
    // Hostile durations never make an overlap longer than half a track.
    for d in [f64::NAN, -1.0, 0.0, 0.15, 3.0] {
        let a = facts("1", d);
        let b = facts("2", 200.0);
        match plan(
            cfg(TransitionMode::Crossfade, 6.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE,
        ) {
            Join::Crossfade { overlap, .. } if d.is_finite() && d > 0.0 => {
                assert!(overlap as f64 <= d / 2.0 * 44_100.0 + 1.0, "{d}")
            }
            Join::Crossfade { overlap, .. } => assert_eq!(overlap, 6 * 44_100, "{d}"),
            Join::Gapless(why) => assert_eq!(why, GaplessWhy::TooShort, "{d}"),
        }
    }
}

#[test]
fn very_short_tracks_stay_gapless() {
    let a = facts("1", 0.1);
    let b = facts("2", 200.0);
    assert_eq!(
        plan(
            cfg(TransitionMode::Crossfade, 6.0),
            Some(&a),
            Some(&b),
            RATE,
            RATE
        ),
        Join::Gapless(GaplessWhy::TooShort)
    );
}

#[test]
fn random_facts_plan_within_bounds() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let pick = |r: u64| -> f64 {
        match r % 8 {
            0 => f64::NAN,
            1 => -1.0,
            2 => 0.0,
            3 => 1e9,
            _ => (r % 100_000) as f64 / 1000.0,
        }
    };
    for _ in 0..20_000 {
        let mk = |r: &mut dyn FnMut() -> u64| TrackFacts {
            album: match r() % 4 {
                0 => None,
                n => Some(n.to_string()),
            },
            duration_s: (!r().is_multiple_of(3)).then(|| pick(r())),
            edges: r().is_multiple_of(2).then(|| EdgeFacts {
                leading_silence_s: pick(r()),
                trailing_silence_s: pick(r()),
                fade_in_s: r().is_multiple_of(2).then(|| pick(r())),
                fade_out_s: r().is_multiple_of(2).then(|| pick(r())),
            }),
        };
        let a = mk(&mut next);
        let b = mk(&mut next);
        let mode = match next() % 3 {
            0 => TransitionMode::Off,
            1 => TransitionMode::Crossfade,
            _ => TransitionMode::Smart,
        };
        let rate = [44_100, 48_000, 96_000][(next() % 3) as usize];
        let c = cfg(mode, pick(next()));
        let j = plan(c, Some(&a), Some(&b), rate, rate);
        if let Join::Crossfade {
            trim_out,
            trim_in,
            overlap,
        } = j
        {
            let r = f64::from(rate);
            assert!(trim_out as f64 <= MAX_TRIM_S * r + 1.0);
            assert!(trim_in as f64 <= MAX_TRIM_S * r + 1.0);
            assert!(overlap as f64 <= MAX_FADE_S * r + 1.0);
            assert!(overlap as f64 >= MIN_OVERLAP_S * r - 1.0);
            assert!(
                a.album.is_some() && a.album != b.album,
                "never within an album"
            );
        }
        if a.album.is_some() && a.album == b.album {
            assert!(matches!(j, Join::Gapless(_)));
        }
    }
}

#[test]
fn level_correction() {
    assert_eq!(tail_correction(-3.0, -3.0), 1.0);
    assert!((tail_correction(-6.0, 0.0) - 0.501_187).abs() < 1e-5);
    assert_eq!(tail_correction(f64::NAN, 0.0), 1.0);
    assert!(tail_correction(1e9, 0.0).is_finite());
}

#[test]
fn describes_itself_for_the_signal_path() {
    assert_eq!(TransitionConfig::default().describe(), None);
    assert_eq!(
        cfg(TransitionMode::Crossfade, 6.0).describe().unwrap(),
        "Crossfade 6 s (not within albums)"
    );
    assert_eq!(
        cfg(TransitionMode::Smart, 4.5).describe().unwrap(),
        "Smart transitions, up to 4.5 s (not within albums)"
    );
}

// --- The crossfader over a scripted source ------------------------------------------------

/// A source of whole tracks, handed out in pulls of at most `chunk` frames.
struct Script {
    tracks: Vec<(u32, u32, Vec<f32>)>,
    pos: Vec<usize>,
    /// Tracks the source "has" so far (the rest arrive later, like a stream).
    available: usize,
    chunk: usize,
}

impl Script {
    fn new(tracks: Vec<(u32, u32, Vec<f32>)>) -> Self {
        let n = tracks.len();
        Script {
            tracks,
            pos: vec![0; n],
            available: n,
            chunk: 1000,
        }
    }

    fn current(&self) -> Option<usize> {
        (0..self.available).find(|&i| self.pos[i] < self.tracks[i].2.len())
    }
}

impl AudioSource for Script {
    fn next_audio(&self) -> Option<TrackAudio> {
        let i = self.current()?;
        Some(TrackAudio {
            seq: self.tracks[i].0,
            sample_rate: self.tracks[i].1,
            source: None,
            server_gain: 1.0,
        })
    }

    fn pull_track(&mut self, out: &mut [f32]) -> Option<(u32, usize)> {
        let i = self.current()?;
        let (seq, _, data) = &self.tracks[i];
        let n = ((data.len() - self.pos[i]) / CHANNELS)
            .min(out.len() / CHANNELS)
            .min(self.chunk);
        out[..n * CHANNELS].copy_from_slice(&data[self.pos[i]..self.pos[i] + n * CHANNELS]);
        self.pos[i] += n * CHANNELS;
        Some((*seq, n))
    }

    fn track_audio(&self, seq: u32) -> Option<TrackAudio> {
        self.tracks.iter().find(|t| t.0 == seq).map(|t| TrackAudio {
            seq,
            sample_rate: t.1,
            source: None,
            server_gain: 1.0,
        })
    }

    fn playing_track(&self) -> Option<u32> {
        None
    }

    fn audio_exhausted(&self) -> bool {
        self.available == self.tracks.len() && self.current().is_none()
    }

    fn applied_volume(&self) -> (f32, f32) {
        (1.0, 1.0)
    }

    fn set_fixed_volume(&mut self, _: bool) {}
}

struct Facts {
    by_seq: HashMap<u32, TrackFacts>,
    level: HashMap<u32, f64>,
}

impl TrackFactsSource for Facts {
    fn facts(&self, seq: u32) -> Option<TrackFacts> {
        self.by_seq.get(&seq).cloned()
    }

    fn level_db(&self, seq: u32, _: Option<&TrackAudio>) -> f64 {
        self.level.get(&seq).copied().unwrap_or(0.0)
    }
}

fn tone(frames: usize, value: f32) -> Vec<f32> {
    // A distinct value per frame, so any misplaced frame shows.
    (0..frames)
        .flat_map(|i| {
            let v = value + i as f32 * 1e-6;
            [v, -v]
        })
        .collect()
}

/// Run the crossfader to the end; the output and the source frames reported.
fn drain(t: &mut Transitions, src: &mut Script, info: &Facts) -> (Vec<f32>, u64, Vec<u32>) {
    let mut out = Vec::new();
    let mut src_frames = 0;
    let mut seqs = Vec::new();
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    for _ in 0..1_000_000 {
        t.top_up(src, &mut block);
        match t.pull(src, &mut block, info, false) {
            Some(p) => {
                out.extend_from_slice(&block[..p.frames * CHANNELS]);
                src_frames += p.src_frames;
                if seqs.last() != Some(&p.seq) {
                    seqs.push(p.seq);
                }
            }
            None => {
                if t.exhausted(src) {
                    break;
                }
                // Waiting: the output would run low eventually.
                if let Some(p) = t.pull(src, &mut block, info, true) {
                    out.extend_from_slice(&block[..p.frames * CHANNELS]);
                    src_frames += p.src_frames;
                    if seqs.last() != Some(&p.seq) {
                        seqs.push(p.seq);
                    }
                } else if t.exhausted(src) {
                    break;
                }
            }
        }
    }
    (out, src_frames, seqs)
}

fn two_albums(a_frames: usize, b_frames: usize) -> (Script, Facts) {
    let src = Script::new(vec![
        (0, RATE, tone(a_frames, 0.25)),
        (1, RATE, tone(b_frames, -0.5)),
    ]);
    let info = Facts {
        by_seq: HashMap::from([
            (0, facts("A", a_frames as f64 / 44_100.0)),
            (1, facts("B", b_frames as f64 / 44_100.0)),
        ]),
        level: HashMap::new(),
    };
    (src, info)
}

#[test]
fn off_passes_every_sample_through_untouched() {
    let (mut src, info) = two_albums(50_000, 30_000);
    let expect: Vec<f32> = src.tracks.iter().flat_map(|t| t.2.clone()).collect();
    let mut t = Transitions::new(TransitionConfig::default());
    let (out, frames, seqs) = drain(&mut t, &mut src, &info);
    assert_eq!(out, expect, "bit-exact, gapless");
    assert_eq!(frames, 80_000);
    assert_eq!(seqs, [0, 1]);
    assert!(t.is_empty());
}

#[test]
fn a_crossfade_mixes_exactly_and_accounts_for_every_frame() {
    let a = 300_000;
    let b = 200_000;
    let (mut src, info) = two_albums(a, b);
    let (ta, tb) = (src.tracks[0].2.clone(), src.tracks[1].2.clone());
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 1.0));
    let (out, frames, seqs) = drain(&mut t, &mut src, &info);
    let overlap = 44_100;
    assert_eq!(
        t.last_join(),
        Some(Join::Crossfade {
            trim_out: 0,
            trim_in: 0,
            overlap
        })
    );
    // The same as the core's whole-track join, to f32 precision.
    let fa: Vec<f64> = ta.iter().map(|x| f64::from(*x)).collect();
    let fb: Vec<f64> = tb.iter().map(|x| f64::from(*x)).collect();
    let want = join(
        &fa,
        &fb,
        CHANNELS,
        Transition::Crossfade {
            frames: overlap as usize,
            curve: FadeCurve::EqualPower,
        },
    );
    assert_eq!(out.len(), want.len());
    assert_eq!(out.len() / CHANNELS, a + b - overlap as usize);
    for (i, (o, w)) in out.iter().zip(&want).enumerate() {
        assert_eq!(*o, *w as f32, "sample {i}");
    }
    // Untouched outside the overlap.
    let split = (a - overlap as usize) * CHANNELS;
    assert_eq!(&out[..split], &ta[..split]);
    assert_eq!(
        &out[split + overlap as usize * CHANNELS..],
        &tb[overlap as usize * CHANNELS..]
    );
    // Every source frame is reported once.
    assert_eq!(frames, (a + b) as u64);
    assert_eq!(seqs, [0, 1]);
    assert!(t.is_empty());
}

#[test]
fn the_overlap_follows_the_equal_power_curve() {
    let (a, b) = (200_000, 200_000);
    let mut src = Script::new(vec![
        (0, RATE, vec![1.0; a * CHANNELS]),
        (1, RATE, vec![0.0; b * CHANNELS]),
    ]);
    let info = Facts {
        by_seq: HashMap::from([(0, facts("A", 10.0)), (1, facts("B", 10.0))]),
        level: HashMap::new(),
    };
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let (out, _, _) = drain(&mut t, &mut src, &info);
    let start = a - 88_200;
    for i in (0..88_200).step_by(997) {
        let (g, _) = fade_gains(FadeCurve::EqualPower, i, 88_200);
        assert!(
            (f64::from(out[(start + i) * CHANNELS]) - g).abs() < 1e-6,
            "frame {i}"
        );
    }
}

#[test]
fn same_album_tracks_stay_bit_exact_and_gapless() {
    let mut src = Script::new(vec![
        (0, RATE, tone(100_000, 0.25)),
        (1, RATE, tone(100_000, -0.5)),
        (2, RATE, tone(100_000, 0.125)),
    ]);
    let info = Facts {
        by_seq: HashMap::from([
            (0, facts("A", 3.0)),
            (1, facts("A", 3.0)),
            (2, facts("A", 3.0)),
        ]),
        level: HashMap::new(),
    };
    let expect: Vec<f32> = src.tracks.iter().flat_map(|t| t.2.clone()).collect();
    let mut t = Transitions::new(cfg(TransitionMode::Smart, 6.0));
    let (out, frames, _) = drain(&mut t, &mut src, &info);
    assert_eq!(out, expect);
    assert_eq!(frames, 300_000);
    assert_eq!(t.last_join(), Some(Join::Gapless(GaplessWhy::SameAlbum)));
}

#[test]
fn smart_trims_silence_and_still_counts_it_as_played() {
    let lead = 22_050; // 0.5 s
    let trail = 44_100; // 1 s
    let mut ta = tone(300_000, 0.25);
    ta.extend(vec![0.0; trail * CHANNELS]);
    let mut tb = vec![0.0; lead * CHANNELS];
    tb.extend(tone(200_000, -0.5));
    let (a, b) = (ta.len() / CHANNELS, tb.len() / CHANNELS);
    let mut src = Script::new(vec![(0, RATE, ta.clone()), (1, RATE, tb.clone())]);
    let mut fa = facts("A", a as f64 / 44_100.0);
    fa.edges = Some(edges(0.0, 1.0, None, None));
    let mut fb = facts("B", b as f64 / 44_100.0);
    fb.edges = Some(edges(0.5, 0.0, None, None));
    let info = Facts {
        by_seq: HashMap::from([(0, fa), (1, fb)]),
        level: HashMap::new(),
    };
    let mut t = Transitions::new(cfg(TransitionMode::Smart, 6.0));
    let (out, frames, _) = drain(&mut t, &mut src, &info);
    let overlap = 2 * 44_100; // two abrupt ends
    assert_eq!(
        t.last_join(),
        Some(Join::Crossfade {
            trim_out: trail as u64,
            trim_in: lead as u64,
            overlap
        })
    );
    assert_eq!(
        out.len() / CHANNELS,
        a - trail + b - lead - overlap as usize
    );
    assert_eq!(
        frames,
        (a + b) as u64,
        "trimmed frames still count as played"
    );
    // The audio either side of the overlap is the tracks' own, without the silence.
    let before = (a - trail - overlap as usize) * CHANNELS;
    assert_eq!(&out[..before], &ta[..before]);
    let after = (lead + overlap as usize) * CHANNELS;
    assert_eq!(&out[out.len() - (tb.len() - after)..], &tb[after..]);
}

#[test]
fn levelling_is_corrected_for_the_outgoing_tail() {
    let (a, b) = (100_000, 100_000);
    let mut src = Script::new(vec![
        (0, RATE, vec![0.5; a * CHANNELS]),
        (1, RATE, vec![0.0; b * CHANNELS]),
    ]);
    let info = Facts {
        by_seq: HashMap::from([(0, facts("A", 5.0)), (1, facts("B", 5.0))]),
        // The pipeline will apply B's −6 dB to the mixed blocks; A's own is 0 dB.
        level: HashMap::from([(0, 0.0), (1, -6.0)]),
    };
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 1.0));
    let (out, _, _) = drain(&mut t, &mut src, &info);
    let first_mixed = (a - 44_100) * CHANNELS;
    let (g, _) = fade_gains(FadeCurve::EqualPower, 0, 44_100);
    let want = 0.5 * tail_correction(0.0, -6.0) as f64 * g;
    assert!((f64::from(out[first_mixed]) - want).abs() < 1e-5);
}

#[test]
fn waits_for_the_next_tracks_facts_but_never_starves_the_output() {
    let (mut src, mut info) = two_albums(200_000, 200_000);
    info.by_seq.remove(&1);
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    let mut total = 0;
    // Play until only the hold of track 0 is left and the next track is waiting.
    for _ in 0..100_000 {
        t.top_up(&mut src, &mut block);
        match t.pull(&mut src, &mut block, &info, false) {
            Some(p) => total += p.src_frames,
            None if src.next_audio().map(|a| a.seq) == Some(1) => break,
            None => {}
        }
    }
    assert!(total < 200_000, "the end of track 0 is held");
    // The output runs low: the boundary goes gapless rather than leave a gap.
    let p = t.pull(&mut src, &mut block, &info, true).unwrap();
    assert_eq!(p.seq, 0);
    assert_eq!(t.last_join(), Some(Join::Gapless(GaplessWhy::Unknown)));
    total += p.src_frames;
    let (_, rest, _) = drain(&mut t, &mut src, &info);
    assert_eq!(total + rest, 400_000);
}

#[test]
fn a_late_next_track_releases_the_hold_when_the_output_runs_low() {
    let (mut src, info) = two_albums(200_000, 100_000);
    src.available = 1; // track 1 hasn't arrived
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    let mut total = 0u64;
    while let Some(p) = {
        t.top_up(&mut src, &mut block);
        t.pull(&mut src, &mut block, &info, false)
    } {
        total += p.src_frames;
    }
    assert!(total < 200_000);
    // Low: frames come out even though the track might still be followed by another.
    let p = t.pull(&mut src, &mut block, &info, true).unwrap();
    assert!(p.frames > 0);
}

#[test]
fn reset_forgets_the_hold() {
    let (mut src, info) = two_albums(200_000, 100_000);
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    t.top_up(&mut src, &mut block);
    let _ = t.pull(&mut src, &mut block, &info, true);
    assert!(!t.is_empty());
    t.reset();
    assert!(t.is_empty());
    assert_eq!(t.held_frames(), 0);
}

#[test]
fn turning_off_plays_out_what_is_held_then_bypasses() {
    let (mut src, info) = two_albums(200_000, 100_000);
    let expect: Vec<f32> = src.tracks.iter().flat_map(|t| t.2.clone()).collect();
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    let mut out = Vec::new();
    for _ in 0..5 {
        t.top_up(&mut src, &mut block);
        if let Some(p) = t.pull(&mut src, &mut block, &info, true) {
            out.extend_from_slice(&block[..p.frames * CHANNELS]);
        }
    }
    t.set_config(TransitionConfig::default());
    let (rest, _, _) = drain(&mut t, &mut src, &info);
    out.extend(rest);
    assert_eq!(out, expect, "nothing lost, nothing mixed");
    assert!(!t.active());
}

#[test]
fn a_short_incoming_track_finishes_the_fade_alone() {
    // B is shorter than the overlap would be at its planned length (its duration is
    // understated to the planner), so the fade finishes without it.
    let (a, b) = (200_000, 30_000);
    let mut src = Script::new(vec![
        (0, RATE, vec![0.5; a * CHANNELS]),
        (1, RATE, vec![0.25; b * CHANNELS]),
        (2, RATE, vec![0.125; 100_000 * CHANNELS]),
    ]);
    let info = Facts {
        by_seq: HashMap::from([
            (0, facts("A", 100.0)),
            (
                1,
                TrackFacts {
                    album: Some("B".into()),
                    duration_s: None,
                    edges: None,
                },
            ),
            (2, facts("C", 100.0)),
        ]),
        level: HashMap::new(),
    };
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 2.0));
    let (out, frames, _) = drain(&mut t, &mut src, &info);
    assert_eq!(
        frames,
        (a + b + 100_000) as u64,
        "every frame accounted for"
    );
    assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
}

#[test]
fn random_scripts_never_lose_or_invent_frames() {
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..60 {
        let n = 1 + (next() % 4) as usize;
        let mut tracks = Vec::new();
        let mut info = Facts {
            by_seq: HashMap::new(),
            level: HashMap::new(),
        };
        let mut total = 0u64;
        for seq in 0..n as u32 {
            let frames = 1 + (next() % 120_000) as usize;
            let rate = if next() % 5 == 0 { 48_000 } else { RATE };
            total += frames as u64;
            tracks.push((seq, rate, vec![0.1; frames * CHANNELS]));
            let album = (next() % 3).to_string();
            let mut f = facts(&album, frames as f64 / f64::from(rate));
            if next() % 2 == 0 {
                f.edges = Some(edges(
                    (next() % 3) as f64,
                    (next() % 3) as f64,
                    None,
                    Some((next() % 8) as f64),
                ));
            }
            if next() % 7 != 0 {
                info.by_seq.insert(seq, f);
            }
        }
        let mut src = Script::new(tracks);
        src.chunk = 1 + (next() % 5000) as usize;
        let mode = [
            TransitionMode::Off,
            TransitionMode::Crossfade,
            TransitionMode::Smart,
        ][(next() % 3) as usize];
        let mut t = Transitions::new(cfg(mode, 1.0 + (next() % 9) as f64));
        let (out, frames, _) = drain(&mut t, &mut src, &info);
        assert_eq!(frames, total, "round {round}");
        assert!(out.len() / CHANNELS <= total as usize, "round {round}");
        assert!(out.iter().all(|x| x.is_finite()), "round {round}");
        assert!(t.is_empty(), "round {round}");
    }
}

/// `strm f`: a hold of the dropped next track goes.
#[test]
fn a_trim_drops_a_hold_of_a_later_track() {
    let mut src = Script::new(vec![(1, RATE, tone(50_000, -0.5))]);
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 1.0));
    let mut block = vec![0.0f32; 4096 * CHANNELS];
    t.top_up(&mut src, &mut block);
    assert!(!t.is_empty());
    t.trim_after(0);
    assert!(t.is_empty());
    // A hold of the kept track stays.
    let mut src = Script::new(vec![(0, RATE, tone(50_000, 0.25))]);
    t.top_up(&mut src, &mut block);
    let held = t.held_frames();
    t.trim_after(0);
    assert_eq!(t.held_frames(), held);
}

/// `strm f` during a crossfade into the dropped track: the fade is called off and the
/// outgoing track's own tail plays out, never another sample of the dropped track.
#[test]
fn a_trim_during_a_crossfade_plays_the_outgoing_tail_alone() {
    let (a, b) = (300_000, 200_000);
    let (mut src, info) = two_albums(a, b);
    let ta = src.tracks[0].2.clone();
    let mut t = Transitions::new(cfg(TransitionMode::Crossfade, 1.0));
    let mut block = vec![0.0f32; 1000 * CHANNELS];
    let mut heard = 0usize;
    // Play until the fade has begun.
    loop {
        t.top_up(&mut src, &mut block);
        let p = t.pull(&mut src, &mut block, &info, true).unwrap();
        if p.seq == 1 {
            break;
        }
        heard += p.frames;
    }
    // The core drops track 1.
    src.pos[1] = src.tracks[1].2.len();
    t.trim_after(0);
    let (rest, _, seqs) = drain(&mut t, &mut src, &info);
    assert_eq!(seqs, [0], "only the outgoing track");
    // What follows is the outgoing track's own audio, in order, to its end.
    let n = rest.len();
    assert!(n > 0);
    assert_eq!(&rest[..], &ta[ta.len() - n..]);
    assert!(heard * CHANNELS + n <= ta.len());
    assert!(t.is_empty());
}

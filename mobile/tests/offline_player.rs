//! The offline player end to end: downloaded FLAC files in an offline library, played from
//! the app's own queue through the same engine as streaming (a pull output stands in for
//! AAudio). Covered: gapless, bit-perfect playback across tracks; seeking; next, previous
//! and jump; shuffle and repeat; plays recorded for sync-back; a missing file skipped and
//! reported; DSP and the signal path; stop and shutdown.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_mobile::offline::index::serialise;
use anywhere_mobile::offline::store::scope_name;
use anywhere_mobile::{
    DownloadFormat, DownloadOrigin, DspSettings, LocalPlayback, LocalPlayerConfig, LocalRepeat,
    NotBitPerfect, OfflineLibrary, OfflinePlayer, OfflinePlayerObserver, OfflinePlayerStatus,
    OfflineTrack, OutputKind, SignalPathInfo, SignalVerdict, TransitionMode, VISUAL_FRAME_BYTES,
};
use common::flac;
use common::wait_for;

const SERVER: &str = "https://music.example.com";

#[derive(Default)]
struct Obs {
    statuses: Mutex<Vec<OfflinePlayerStatus>>,
    signal: Mutex<Option<SignalPathInfo>>,
}

impl OfflinePlayerObserver for Obs {
    fn on_status(&self, status: OfflinePlayerStatus) {
        self.statuses.lock().unwrap().push(status);
    }

    fn on_signal_path(&self, path: Option<SignalPathInfo>) {
        *self.signal.lock().unwrap() = path;
    }
}

struct Track {
    key: String,
    file: Vec<u8>,
    pcm: Vec<f32>,
}

/// Write `files` as downloaded tracks (album 7) and open the library.
fn library(files: Vec<Vec<u8>>, rate_secs: f64) -> (Arc<OfflineLibrary>, Vec<Track>, PathBuf) {
    let albums = vec![Some(7); files.len()];
    library_with(files, rate_secs, &albums)
}

/// [`library`], with each track's album id.
fn library_with(
    files: Vec<Vec<u8>>,
    rate_secs: f64,
    albums: &[Option<i64>],
) -> (Arc<OfflineLibrary>, Vec<Track>, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "anywhere-op-{}",
        anywhere_mobile::offline::plays::new_key()
    ));
    let scope = root.join(scope_name(SERVER, "alice"));
    std::fs::create_dir_all(scope.join("tracks")).unwrap();
    let mut entries = Vec::new();
    let mut tracks = Vec::new();
    for (i, file) in files.into_iter().enumerate() {
        let key = format!("{:064x}", 0xabc0 + i);
        std::fs::write(scope.join("tracks").join(format!("{key}.flac")), &file).unwrap();
        entries.push(OfflineTrack {
            key: key.clone(),
            track_id: 100 + i as i64,
            title: format!("Track {i}"),
            artist: "Band".into(),
            album: "Record".into(),
            album_id: albums[i],
            track_number: Some(i as u32 + 1),
            duration_secs: rate_secs,
            format: DownloadFormat::Original,
            codec: "flc".into(),
            ext: "flac".into(),
            size: file.len() as u64,
            downloaded_at: 1_800_000_000,
            origin: DownloadOrigin::Manual,
        });
        let (_, pcm) = flac::decode(&file);
        tracks.push(Track { key, file, pcm });
    }
    std::fs::write(scope.join("index.json"), serialise(&entries)).unwrap();
    let lib = OfflineLibrary::open(
        root.to_string_lossy().into_owned(),
        SERVER.into(),
        "alice".into(),
    )
    .unwrap();
    assert_eq!(lib.tracks().len(), tracks.len());
    (lib, tracks, scope)
}

/// Audio pulled, in runs of one sample rate.
type Runs = Arc<Mutex<Vec<(u32, Vec<f32>)>>>;

struct Rig {
    player: Arc<OfflinePlayer>,
    obs: Arc<Obs>,
    pulled: Runs,
    stop: Arc<AtomicBool>,
}

impl Rig {
    fn start(lib: &Arc<OfflineLibrary>, dsp: DspSettings) -> Rig {
        let player = OfflinePlayer::new(
            lib.clone(),
            LocalPlayerConfig {
                output: OutputKind::Pull,
                max_sample_rate: 192_000,
                cache_dir: None,
                read_ahead: true,
                dsp,
            },
        );
        let obs = Arc::new(Obs::default());
        player.start(obs.clone()).unwrap();
        let pulled: Runs = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (p, out, st) = (player.clone(), pulled.clone(), stop.clone());
        tokio::spawn(async move {
            while !st.load(Ordering::SeqCst) {
                let b = p.read_pcm(4096);
                let n = b.audio_samples as usize;
                if n > 0 {
                    let mut runs = out.lock().unwrap();
                    match runs.last_mut() {
                        Some((r, s)) if *r == b.sample_rate => s.extend_from_slice(&b.samples[..n]),
                        _ => runs.push((b.sample_rate, b.samples[..n].to_vec())),
                    }
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
        Rig {
            player,
            obs,
            pulled,
            stop,
        }
    }

    fn audio(&self) -> Vec<f32> {
        self.pulled
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, s)| s.iter().copied())
            .collect()
    }

    fn status(&self) -> OfflinePlayerStatus {
        self.player.status()
    }

    async fn wait_samples(&self, n: usize) {
        for _ in 0..3000 {
            if self.audio().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("only {} samples of {n}", self.audio().len());
    }

    fn clear(&self) {
        self.pulled.lock().unwrap().clear();
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.player.shutdown();
    }
}

fn song(secs: f64, seed: u64) -> Vec<u8> {
    flac::encode(
        44_100,
        16,
        &flac::noise((44_100.0 * secs) as usize, 16, seed),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_downloads_gaplessly_and_bit_perfectly_and_records_the_plays() {
    let (lib, tracks, _) = library(vec![song(6.0, 1), song(5.5, 2)], 6.0);
    let rig = Rig::start(&lib, DspSettings::default());
    let keys: Vec<String> = tracks.iter().map(|t| t.key.clone()).collect();
    assert_eq!(
        rig.player
            .play_tracks(keys.clone(), 0, false, true)
            .unwrap(),
        2
    );
    let want: Vec<f32> = tracks.iter().flat_map(|t| t.pcm.iter().copied()).collect();
    rig.wait_samples(want.len()).await;
    // The engine stops at the end of the queue.
    wait_for("the end of the queue", || {
        rig.status().playback == LocalPlayback::Stopped
    })
    .await;
    let got = rig.audio();
    assert_eq!(
        got.len(),
        want.len(),
        "nothing added or lost between the tracks"
    );
    assert!(got == want, "bit-exact, gapless");
    assert_eq!(
        rig.pulled.lock().unwrap().len(),
        1,
        "one output run: no reopen at the boundary"
    );
    // Both plays were recorded for sync-back.
    wait_for("the plays", || lib.pending_plays() == 2).await;
    let statuses = rig.obs.statuses.lock().unwrap().clone();
    assert!(
        statuses
            .iter()
            .any(|s| s.index == Some(1) && s.playback == LocalPlayback::Playing)
    );
    assert!(statuses.iter().all(|s| s.last_error.is_none()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_signal_path_matches_streaming() {
    let (lib, tracks, _) = library(vec![song(2.0, 3)], 2.0);
    let rig = Rig::start(&lib, DspSettings::default());
    rig.player
        .play_tracks(vec![tracks[0].key.clone()], 0, false, true)
        .unwrap();
    wait_for("the signal path", || {
        rig.obs.signal.lock().unwrap().is_some()
    })
    .await;
    let sp = rig.obs.signal.lock().unwrap().clone().unwrap();
    assert_eq!(sp.source_codec, "FLAC");
    assert_eq!(sp.source_rate, 44_100);
    assert_eq!(sp.source_bits, Some(16));
    assert!(sp.engine_bit_perfect);
    assert!(!sp.dsp_active);
    assert_eq!(sp.output_rate, 44_100);
    assert_ne!(sp.verdict, SignalVerdict::LossySource);
    // The same DSP as streaming: a preamp changes the samples and says so.
    rig.player.set_dsp(DspSettings {
        enabled: true,
        preamp_db: -6.0,
        ..DspSettings::default()
    });
    wait_for("processing", || {
        rig.obs
            .signal
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.dsp_active)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seek_skip_jump_pause_shuffle_and_repeat() {
    let (lib, tracks, _) = library(vec![song(4.0, 4), song(4.0, 5), song(4.0, 6)], 4.0);
    let rig = Rig::start(&lib, DspSettings::default());
    let keys: Vec<String> = tracks.iter().map(|t| t.key.clone()).collect();
    rig.player
        .play_tracks(keys.clone(), 1, false, true)
        .unwrap();
    rig.wait_samples(2000).await;
    assert_eq!(rig.status().index, Some(1));
    assert_eq!(rig.audio()[..2000], tracks[1].pcm[..2000]);

    // Seek: the audio from there, sample-accurately.
    rig.player.seek_to(2_000);
    wait_for("the seek", || rig.status().elapsed_ms >= 2_000).await;
    rig.clear();
    rig.wait_samples(20_000).await;
    let got = rig.audio();
    let at = 88_200 * 2;
    let pos = tracks[1].pcm[at..]
        .windows(64)
        .position(|w| w == &got[..64])
        .expect("audio from the seek position");
    assert!(pos < 44_100, "within half a second of 2 s ({pos})");

    // Next, previous (near the start: the track before) and jump.
    rig.player.next();
    wait_for("track 2", || rig.status().index == Some(2)).await;
    rig.player.previous();
    wait_for("back to track 1", || rig.status().index == Some(1)).await;
    rig.player.jump_to(0);
    wait_for("track 0", || rig.status().index == Some(0)).await;

    // Pause: silence, and the position holds.
    rig.player.pause();
    wait_for("paused", || rig.status().playback == LocalPlayback::Paused).await;
    // The output may still finish the buffer it was playing as the pause lands: let the
    // position settle, then it must hold.
    let mut held = rig.status().elapsed_ms;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let now = rig.status().elapsed_ms;
        if now == held {
            break;
        }
        held = now;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(rig.status().elapsed_ms, held);
    rig.player.play();
    wait_for("playing", || {
        rig.status().playback == LocalPlayback::Playing
    })
    .await;

    // Shuffle keeps the current track; repeat is reported.
    rig.player.set_shuffle(true);
    rig.player.set_repeat(LocalRepeat::All);
    wait_for("shuffled", || {
        let s = rig.status();
        s.shuffle && s.repeat == LocalRepeat::All && s.order.first() == Some(&0)
    })
    .await;
    let s = rig.status();
    let mut order = s.order.clone();
    order.sort();
    assert_eq!(order, vec![0, 1, 2]);
    assert_eq!(s.keys, keys);

    // Stop forgets the queue.
    rig.player.stop();
    wait_for("stopped", || {
        let s = rig.status();
        s.playback == LocalPlayback::Stopped && s.index.is_none()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_files_are_skipped_and_unknown_keys_refused() {
    let (lib, tracks, scope) = library(vec![song(1.0, 7), song(1.0, 8)], 1.0);
    let rig = Rig::start(&lib, DspSettings::default());
    // Unknown and malformed keys are left out; none at all is refused.
    assert!(
        rig.player
            .play_tracks(
                vec!["../../etc/passwd".into(), "ab".repeat(32)],
                0,
                false,
                true
            )
            .is_err()
    );
    // The first file vanished after the library was opened (e.g. storage cleared).
    std::fs::remove_file(scope.join("tracks").join(format!("{}.flac", tracks[0].key))).unwrap();
    let keys = vec![tracks[0].key.clone(), "zz".into(), tracks[1].key.clone()];
    assert_eq!(rig.player.play_tracks(keys, 0, false, true).unwrap(), 2);
    rig.wait_samples(tracks[1].pcm.len()).await;
    assert_eq!(rig.audio(), tracks[1].pcm);
    let statuses = rig.obs.statuses.lock().unwrap().clone();
    assert!(
        statuses.iter().any(|s| s
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("missing"))),
        "the skip is reported"
    );
    let _ = tracks[0].file.len();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeat_one_loops_without_a_gap() {
    let (lib, tracks, _) = library(vec![song(1.0, 9)], 1.0);
    let rig = Rig::start(&lib, DspSettings::default());
    rig.player.set_repeat(LocalRepeat::One);
    rig.player
        .play_tracks(vec![tracks[0].key.clone()], 0, false, true)
        .unwrap();
    rig.player.set_repeat(LocalRepeat::One);
    let n = tracks[0].pcm.len();
    rig.wait_samples(n * 2 + 1000).await;
    let got = rig.audio();
    assert_eq!(got[..n], tracks[0].pcm[..]);
    assert_eq!(got[n..2 * n], tracks[0].pcm[..], "the loop is seamless");
}

/// Transitions on, tracks from two albums: they crossfade (1 s, equal power), every frame of
/// both is accounted for (both plays recorded), and the signal path says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crossfades_between_albums() {
    let (lib, tracks, _) = library_with(vec![song(6.0, 1), song(5.5, 2)], 6.0, &[Some(7), Some(8)]);
    let dsp = DspSettings {
        transitions: TransitionMode::Crossfade,
        crossfade_s: 1.0,
        ..DspSettings::default()
    };
    let rig = Rig::start(&lib, dsp);
    let keys: Vec<String> = tracks.iter().map(|t| t.key.clone()).collect();
    rig.player.play_tracks(keys, 0, false, true).unwrap();
    let overlap = 44_100 * 2; // samples
    let (a, b) = (&tracks[0].pcm, &tracks[1].pcm);
    let total = a.len() + b.len() - overlap;
    rig.wait_samples(total).await;
    wait_for("the end of the queue", || {
        rig.status().playback == LocalPlayback::Stopped
    })
    .await;
    let got = rig.audio();
    assert_eq!(got.len(), total, "the tracks overlap by exactly 1 s");
    let split = a.len() - overlap;
    assert!(got[..split] == a[..split], "untouched before the overlap");
    assert!(got[split + overlap..] == b[overlap..], "untouched after it");
    // In the overlap: the equal-power mix of the two.
    for i in (0..overlap / 2).step_by(509) {
        let t = (i as f64 + 0.5) / (overlap / 2) as f64;
        let (go, gi) = (
            (std::f64::consts::FRAC_PI_2 * t).cos(),
            (std::f64::consts::FRAC_PI_2 * t).sin(),
        );
        let want = f64::from(a[split + 2 * i]) * go + f64::from(b[2 * i]) * gi;
        assert!(
            (f64::from(got[split + 2 * i]) - want).abs() < 1e-6,
            "frame {i}"
        );
    }
    wait_for("the plays", || lib.pending_plays() == 2).await;
    let path = rig.player.signal_path();
    if let Some(p) = path {
        assert!(!p.bit_perfect);
        assert!(p.not_bit_perfect.contains(&NotBitPerfect::Crossfade));
    }
}

/// Transitions on, tracks from the same album: still bit-exact and gapless.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transitions_keep_an_album_gapless() {
    let (lib, tracks, _) = library(vec![song(6.0, 3), song(5.5, 4)], 6.0);
    let dsp = DspSettings {
        transitions: TransitionMode::Smart,
        crossfade_s: 6.0,
        ..DspSettings::default()
    };
    let rig = Rig::start(&lib, dsp);
    let keys: Vec<String> = tracks.iter().map(|t| t.key.clone()).collect();
    rig.player.play_tracks(keys, 0, false, true).unwrap();
    let want: Vec<f32> = tracks.iter().flat_map(|t| t.pcm.iter().copied()).collect();
    rig.wait_samples(want.len()).await;
    wait_for("the end of the queue", || {
        rig.status().playback == LocalPlayback::Stopped
    })
    .await;
    assert!(rig.audio() == want, "bit-exact, gapless within the album");
    wait_for("the plays", || lib.pending_plays() == 2).await;
}

/// The visualiser (Wave 2 §4.1): off by default, frames only while asked for and while
/// audio is heard, each of the fixed size, and none once turned off. (The pull output
/// here takes audio faster than real time, so each phase plays the track through.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_visualiser_gives_frames_only_while_asked_and_as_audio_is_heard() {
    let (lib, tracks, _) = library(vec![song(4.0, 9)], 4.0);
    let rig = Rig::start(&lib, DspSettings::default());
    let keys: Vec<String> = tracks.iter().map(|t| t.key.clone()).collect();
    let play_through = |collect: bool| {
        let (rig, keys) = (&rig, keys.clone());
        async move {
            rig.clear();
            rig.player.play_tracks(keys, 0, false, true).unwrap();
            let mut got = Vec::new();
            for _ in 0..3000 {
                if let Some(f) = rig.player.visual_frame() {
                    got.push(f);
                }
                if rig.audio().len() >= 4 * 44_100 * 2
                    && rig.status().playback == LocalPlayback::Stopped
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(if collect { 1 } else { 5 })).await;
            }
            got
        }
    };
    assert!(play_through(false).await.is_empty(), "off by default");
    rig.player.set_visualiser(true);
    let got = play_through(true).await;
    assert!(got.len() >= 3, "only {} frames", got.len());
    for f in &got {
        assert_eq!(f.len(), VISUAL_FRAME_BYTES);
        let v: Vec<f32> = f
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        assert!(v.iter().all(|x| x.is_finite()));
        // Noise fills the spectrum: some band is well above the floor.
        assert!(v[..32].iter().any(|b| *b > 0.3), "{:?}", &v[..32]);
    }
    // Stopped at the end: nothing more is heard, so no new frames.
    let _ = rig.player.visual_frame();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.player.visual_frame(),
        None,
        "no frames once nothing plays"
    );
    // Off again: none, even while playing.
    rig.player.set_visualiser(false);
    assert!(play_through(false).await.is_empty(), "off again");
}

/// Loudness compensation follows the phone's media volume (downloads play on this phone),
/// and the signal path says so; an unknown volume falls back to the player's own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loudness_follows_the_phones_volume() {
    let (lib, tracks, _) = library(vec![song(3.0, 5)], 3.0);
    let rig = Rig::start(
        &lib,
        DspSettings {
            enabled: true,
            loudness: true,
            ..DspSettings::default()
        },
    );
    rig.player.set_loudness_reference_db(Some(20.0));
    assert_eq!(rig.player.loudness_reference_db(), Some(20.0));
    rig.player
        .play_tracks(vec![tracks[0].key.clone()], 0, false, true)
        .unwrap();
    let stage = |want: &'static str| {
        let obs = rig.obs.clone();
        move || {
            obs.signal
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|s| s.stages.iter().any(|st| st.name == want))
        }
    };
    wait_for(
        "loudness following the phone",
        stage("Loudness +5.0 dB bass, +2.0 dB treble (following the phone's volume)"),
    )
    .await;
    // A Bluetooth headset with its own volume: the player's level alone (full: none).
    rig.player.set_loudness_reference_db(None);
    wait_for(
        "loudness on the player level",
        stage("Loudness (none at full volume)"),
    )
    .await;
    // Odd values from the host are cleaned up, never passed on as they are.
    rig.player.set_loudness_reference_db(Some(f64::NAN));
    assert_eq!(rig.player.loudness_reference_db(), None);
    rig.player.set_loudness_reference_db(Some(f64::INFINITY));
    assert_eq!(rig.player.loudness_reference_db(), Some(120.0));
    rig.player.set_loudness_reference_db(Some(-3.0));
    assert_eq!(rig.player.loudness_reference_db(), Some(0.0));
}

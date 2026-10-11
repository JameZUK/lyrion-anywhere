//! Loudness levelling: the gain that brings a track (or its album) to a target loudness.
//!
//! The loudness comes either from the gateway's analysis (EBU R128 integrated loudness and
//! true peak, see [`crate::analysis`]) or from ReplayGain tags. ReplayGain 2.0 gains are
//! relative to a reference of -18 LUFS, so a tag's gain converts to loudness as
//! `loudness = -18 - gain`.
//!
//! Attenuation is always applied in full. A boost is capped twice:
//! - by [`LevellingConfig::max_boost_db`];
//! - by the peak: the boosted peak may exceed the limiter's ceiling by at most
//!   [`LevellingConfig::limiter_headroom_db`] (0 means "never rely on the limiter", so the
//!   boosted track never reaches the ceiling at all).
//!
//! ReplayGain peaks are usually *sample* peaks, which can under-read the true peak by up
//! to about 3 dB; the true-peak limiter after this stage catches the difference.

/// The ReplayGain 2.0 reference loudness.
pub const REPLAYGAIN_REFERENCE_LUFS: f64 = -18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LevellingMode {
    #[default]
    Track,
    /// Album gain where known, otherwise track gain.
    Album,
}

/// A loudness measurement of a track or album.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Integrated loudness, LUFS.
    pub loudness_lufs: f64,
    /// Peak level in dBFS (true peak from analysis; usually sample peak from tags).
    pub peak_db: f64,
}

impl Measurement {
    /// From ReplayGain tags: gain in dB and peak as a linear amplitude (1.0 = full scale).
    pub fn from_replaygain(gain_db: f64, peak_linear: f64) -> Option<Self> {
        if !gain_db.is_finite() || !(-60.0..=60.0).contains(&gain_db) {
            return None;
        }
        // A missing or nonsensical peak is treated as full scale (the safe assumption).
        let peak_db = if peak_linear.is_finite() && peak_linear > 0.0 && peak_linear < 1e3 {
            super::gain_to_db(peak_linear)
        } else {
            0.0
        };
        Some(Measurement {
            loudness_lufs: REPLAYGAIN_REFERENCE_LUFS - gain_db,
            peak_db,
        })
    }

    /// From analysis: integrated loudness (LUFS) and true peak (dBTP).
    pub fn from_analysis(loudness_lufs: f64, true_peak_dbtp: f64) -> Option<Self> {
        if !loudness_lufs.is_finite() || !(-100.0..=30.0).contains(&loudness_lufs) {
            return None;
        }
        let peak_db = if true_peak_dbtp.is_finite() {
            true_peak_dbtp.clamp(-200.0, 60.0)
        } else {
            0.0
        };
        Some(Measurement {
            loudness_lufs,
            peak_db,
        })
    }
}

/// What is known about the loudness of the track being played.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LoudnessInfo {
    pub track: Option<Measurement>,
    pub album: Option<Measurement>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LevellingConfig {
    /// Target loudness, LUFS (e.g. -16 on phones, -18 for ReplayGain-like levels).
    pub target_lufs: f64,
    pub mode: LevellingMode,
    /// Largest boost ever applied, dB.
    pub max_boost_db: f64,
    /// The limiter's ceiling, dBTP.
    pub ceiling_dbtp: f64,
    /// How far a boosted peak may exceed the ceiling, for the limiter to catch, dB.
    pub limiter_headroom_db: f64,
}

impl Default for LevellingConfig {
    fn default() -> Self {
        LevellingConfig {
            target_lufs: -18.0,
            mode: LevellingMode::Track,
            max_boost_db: 12.0,
            ceiling_dbtp: -1.0,
            limiter_headroom_db: 0.0,
        }
    }
}

/// The result of [`decide`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevellingDecision {
    /// Gain to apply, dB.
    pub gain_db: f64,
    /// Gain the target alone asked for, before any cap.
    pub requested_db: f64,
    /// The boost was reduced to protect the peak or by the boost cap.
    pub capped: bool,
    /// Which measurement was used.
    pub used: LevellingMode,
}

/// The levelling gain for `info` under `cfg`, or `None` when nothing is known.
pub fn decide(cfg: &LevellingConfig, info: &LoudnessInfo) -> Option<LevellingDecision> {
    let (m, used) = match (cfg.mode, info.album, info.track) {
        (LevellingMode::Album, Some(a), _) => (a, LevellingMode::Album),
        (_, _, Some(t)) => (t, LevellingMode::Track),
        (LevellingMode::Track, Some(a), None) => (a, LevellingMode::Album),
        _ => return None,
    };
    let target = finite_or(cfg.target_lufs, -18.0).clamp(-70.0, 0.0);
    let max_boost = finite_or(cfg.max_boost_db, 0.0).clamp(0.0, 40.0);
    let headroom = finite_or(cfg.limiter_headroom_db, 0.0).clamp(0.0, 20.0);
    let ceiling = finite_or(cfg.ceiling_dbtp, -1.0).clamp(-20.0, 0.0);

    let requested = target - m.loudness_lufs;
    let mut gain = requested;
    if gain > 0.0 {
        let peak_room = (ceiling - m.peak_db + headroom).max(0.0);
        gain = gain.min(max_boost).min(peak_room);
    }
    let gain = gain.clamp(-super::gain::MAX_GAIN_DB, super::gain::MAX_GAIN_DB);
    Some(LevellingDecision {
        gain_db: gain,
        requested_db: requested,
        capped: gain < requested,
        used,
    })
}

fn finite_or(x: f64, d: f64) -> f64 {
    if x.is_finite() { x } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(lufs: f64, peak: f64) -> LoudnessInfo {
        LoudnessInfo {
            track: Measurement::from_analysis(lufs, peak),
            album: None,
        }
    }

    #[test]
    fn attenuates_loud_tracks_fully() {
        let d = decide(&LevellingConfig::default(), &info(-8.0, 0.5)).unwrap();
        assert!((d.gain_db + 10.0).abs() < 1e-12);
        assert!(!d.capped);
    }

    #[test]
    fn boost_never_pushes_peak_past_ceiling() {
        // Quiet track with a high peak: target asks for +10 dB, but the peak is at -3 dBTP
        // and the ceiling is -1, so only +2 dB is allowed.
        let d = decide(&LevellingConfig::default(), &info(-28.0, -3.0)).unwrap();
        assert!((d.gain_db - 2.0).abs() < 1e-12, "{d:?}");
        assert!(d.capped);
        // With 3 dB of limiter headroom, +5 dB.
        let cfg = LevellingConfig {
            limiter_headroom_db: 3.0,
            ..Default::default()
        };
        assert!((decide(&cfg, &info(-28.0, -3.0)).unwrap().gain_db - 5.0).abs() < 1e-12);
        // A peak already over the ceiling: no boost at all.
        assert_eq!(
            decide(&LevellingConfig::default(), &info(-30.0, 0.5))
                .unwrap()
                .gain_db,
            0.0
        );
    }

    #[test]
    fn boost_cap() {
        let cfg = LevellingConfig {
            max_boost_db: 4.0,
            ..Default::default()
        };
        let d = decide(&cfg, &info(-40.0, -30.0)).unwrap();
        assert_eq!(d.gain_db, 4.0);
    }

    #[test]
    fn replaygain_and_album_mode() {
        // RG gain -6 dB means the track is at -12 LUFS.
        let t = Measurement::from_replaygain(-6.0, 0.9).unwrap();
        assert_eq!(t.loudness_lufs, -12.0);
        let a = Measurement::from_replaygain(-4.0, 0.95).unwrap();
        let i = LoudnessInfo {
            track: Some(t),
            album: Some(a),
        };
        let cfg = LevellingConfig {
            mode: LevellingMode::Album,
            ..Default::default()
        };
        let d = decide(&cfg, &i).unwrap();
        assert_eq!(d.used, LevellingMode::Album);
        assert!((d.gain_db + 4.0).abs() < 1e-12);
        // Album mode falls back to the track; track mode never uses the album if a track exists.
        let only_track = LoudnessInfo {
            track: Some(t),
            album: None,
        };
        assert_eq!(
            decide(&cfg, &only_track).unwrap().used,
            LevellingMode::Track
        );
        assert!(decide(&cfg, &LoudnessInfo::default()).is_none());
    }

    #[test]
    fn hostile_values_are_refused_or_clamped() {
        assert!(Measurement::from_replaygain(f64::NAN, 1.0).is_none());
        assert!(Measurement::from_replaygain(1e6, 1.0).is_none());
        assert_eq!(
            Measurement::from_replaygain(0.0, -1.0).unwrap().peak_db,
            0.0
        );
        assert!(Measurement::from_analysis(f64::INFINITY, 0.0).is_none());
        let cfg = LevellingConfig {
            target_lufs: f64::NAN,
            max_boost_db: f64::INFINITY,
            ceiling_dbtp: 50.0,
            limiter_headroom_db: -5.0,
            mode: LevellingMode::Track,
        };
        let d = decide(&cfg, &info(-60.0, -50.0)).unwrap();
        assert!(d.gain_db.is_finite() && d.gain_db <= 40.0);
    }
}

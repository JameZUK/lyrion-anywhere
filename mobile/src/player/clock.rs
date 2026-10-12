//! Time on the output: mapping between the host's monotonic clock and the output device's
//! frame count, for reporting what is audible and starting audio at an exact time.
//!
//! An output reports a [`Timestamp`]: device frame `frame` was presented (left the speaker,
//! or went out over Bluetooth, where the platform includes that latency) at monotonic time
//! `at_ns`. Android's `AAudioStream_getTimestamp(CLOCK_MONOTONIC)` gives exactly this. The
//! player's clock (its jiffies) is the same `CLOCK_MONOTONIC` ([`mono_ns`]), so from a
//! timestamp:
//! - [`frame_at`]: the device frame being heard at any moment (what is audible now);
//! - [`frame_for`]: the device frame that will be heard at a given moment (when a timed
//!   start's first frame must be placed).
//!
//! The device's sample clock may run slightly fast or slow against the monotonic clock. A
//! fresh timestamp is taken every tick, so the extrapolation spans at most a few tens of
//! milliseconds and the drift over it is negligible (100 ppm over 20 ms is 2 µs). The drift
//! itself shows up to LMS as the player gaining or losing time, which its sync corrects.

/// Device frame `frame` was presented at monotonic time `at_ns`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub frame: i64,
    pub at_ns: i64,
}

/// A timestamp further from now than this is stale or nonsense, and is not used.
const MAX_AGE_NS: i64 = 2_000_000_000;

impl Timestamp {
    /// Usable for mapping at `now_ns`: non-negative, not from the future (beyond a little
    /// scheduling slack) and not stale.
    pub fn usable(&self, now_ns: i64) -> bool {
        self.frame >= 0
            && self.at_ns > 0
            && self.at_ns <= now_ns.saturating_add(50_000_000)
            && now_ns.saturating_sub(self.at_ns) <= MAX_AGE_NS
    }
}

/// The device frame being presented at `now_ns`, at `rate` frames a second. Never before
/// the timestamp's own frame, nor beyond `written` (the frames handed to the device).
pub fn frame_at(ts: Timestamp, rate: u32, now_ns: i64, written: u64) -> u64 {
    let dt = i128::from(now_ns) - i128::from(ts.at_ns);
    let frames = i128::from(ts.frame) + dt * i128::from(rate) / 1_000_000_000;
    frames
        .max(i128::from(ts.frame.max(0)))
        .min(i128::from(written))
        .max(0) as u64
}

/// The device frame that will be presented at `target_ns`, at `rate` frames a second (it may
/// already have been handed to the device, or even heard, if the target is near or past).
pub fn frame_for(ts: Timestamp, rate: u32, target_ns: i64) -> i64 {
    let dt = i128::from(target_ns) - i128::from(ts.at_ns);
    let f = i128::from(ts.frame) + dt * i128::from(rate) / 1_000_000_000;
    f.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// The monotonic clock (`CLOCK_MONOTONIC`, the clock AAudio's timestamps use), in ns.
// `time_t` and `c_long` are 32-bit on some targets (32-bit Android) and 64-bit on others.
#[allow(clippy::useless_conversion)]
#[cfg(unix)]
pub fn mono_ns() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes a `timespec` to the valid pointer it is given;
    // CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    i64::from(ts.tv_sec)
        .saturating_mul(1_000_000_000)
        .saturating_add(i64::from(ts.tv_nsec))
}

/// The monotonic clock, in ns (elsewhere: from the process's first call).
#[cfg(not(unix))]
pub fn mono_ns() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as i64 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;

    #[test]
    fn frame_at_extrapolates_from_the_timestamp() {
        let ts = Timestamp {
            frame: 48_000,
            at_ns: 10_000 * MS,
        };
        assert_eq!(frame_at(ts, 48_000, 10_000 * MS, 1 << 30), 48_000);
        assert_eq!(frame_at(ts, 48_000, 10_020 * MS, 1 << 30), 48_960);
        // Never beyond what was handed over, never before the timestamp's frame.
        assert_eq!(frame_at(ts, 48_000, 10_020 * MS, 48_500), 48_500);
        assert_eq!(frame_at(ts, 48_000, 9_000 * MS, 1 << 30), 48_000);
    }

    /// Bluetooth: the timestamp includes a 250 ms latency. Of 400 ms handed over, only
    /// 150 ms is audible: what is reported, rather than all 400.
    #[test]
    fn bluetooth_latency_is_what_separates_written_from_heard() {
        let rate = 44_100u32;
        let start = 5_000 * MS;
        let latency = 250 * MS;
        // At t, frames written ≈ 400 ms of audio; the device presents frame f at
        // start + f / rate + latency.
        let t = start + 400 * MS;
        let written = (rate as u64) * 400 / 1000;
        let heard_frame = i64::from(rate) * 150 / 1000;
        let ts = Timestamp {
            frame: heard_frame - 441, // 10 ms earlier
            at_ns: t - 10 * MS,
        };
        let f = frame_at(ts, rate, t, written);
        assert_eq!(f as i64, heard_frame);
        // And the frame to start at so that it is heard 300 ms from now.
        let target = frame_for(ts, rate, t + 300 * MS);
        assert_eq!(target, heard_frame + 13_230);
        let heard_at = start + target * 1_000_000_000 / i64::from(rate) + latency;
        assert!((heard_at - (t + 300 * MS)).abs() < MS / 10);
    }

    /// The device's sample clock runs 150 ppm fast. With a timestamp refreshed every 20 ms
    /// the position is still right to well under a millisecond.
    #[test]
    fn clock_drift_between_device_and_host_stays_within_a_frame_or_two() {
        let nominal = 48_000f64;
        let actual = nominal * (1.0 + 150e-6);
        let latency = 0.030;
        let mut worst = 0f64;
        for tick in 0..50_000i64 {
            let now = tick * 20 * MS + 3 * MS;
            let t_ts = now - 7 * MS; // the timestamp is a little old
            let ts_frame = ((t_ts as f64 / 1e9 - latency).max(0.0) * actual) as i64;
            let ts = Timestamp {
                frame: ts_frame,
                at_ns: t_ts,
            };
            let truth = (now as f64 / 1e9 - latency).max(0.0) * actual;
            if truth < 1.0 {
                continue;
            }
            let est = frame_at(ts, nominal as u32, now, u64::MAX) as f64;
            worst = worst.max((est - truth).abs());
        }
        assert!(worst < 3.0, "worst error {worst} frames");
    }

    /// Jitter: the time stamped can be off by a little; the error stays that little.
    #[test]
    fn timestamp_jitter_maps_to_proportional_error() {
        let rate = 48_000;
        let ts = Timestamp {
            frame: 96_000,
            at_ns: 2_000 * MS + 300_000, // 0.3 ms late
        };
        let f = frame_at(ts, rate, 2_010 * MS, u64::MAX) as i64;
        let truth = 96_000 + 480;
        assert!((f - truth).abs() <= 15, "{f}");
    }

    /// Security/robustness: hostile or broken timestamps never panic or overflow.
    #[test]
    fn nonsense_timestamps_are_rejected_and_never_overflow() {
        let now = 50_000 * MS;
        for bad in [
            Timestamp {
                frame: -1,
                at_ns: now,
            },
            Timestamp { frame: 1, at_ns: 0 },
            Timestamp {
                frame: 1,
                at_ns: now + 10_000 * MS,
            },
            Timestamp {
                frame: 1,
                at_ns: now - 10_000 * MS,
            },
        ] {
            assert!(!bad.usable(now), "{bad:?}");
        }
        assert!(
            Timestamp {
                frame: 10,
                at_ns: now - 5 * MS
            }
            .usable(now)
        );
        for frame in [i64::MIN, -1, 0, i64::MAX] {
            for at in [i64::MIN, 0, i64::MAX] {
                for t in [i64::MIN, 0, i64::MAX] {
                    let ts = Timestamp { frame, at_ns: at };
                    let _ = frame_at(ts, u32::MAX, t, u64::MAX);
                    let _ = frame_at(ts, 0, t, 0);
                    let _ = frame_for(ts, u32::MAX, t);
                }
            }
        }
    }

    #[test]
    fn the_monotonic_clock_moves_forward() {
        let a = mono_ns();
        let b = mono_ns();
        assert!(a > 0 && b >= a);
    }
}

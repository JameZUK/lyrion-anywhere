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

/// What an output says about when its frames are heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// The output can't say (a host that pulls PCM itself): frames are taken as heard
    /// when handed over, less any buffer the output reports.
    Unsupported,
    /// The output should say but couldn't this time (AAudio's timestamp call failed).
    Unavailable,
    /// The device's own presentation timestamp.
    At(Timestamp),
}

pub use super::OutputClock;

/// Unusable timestamps are counted over this window (the status reports "per minute").
const ERROR_WINDOW_NS: i64 = 60_000_000_000;
/// No more than this many errors are remembered (a 20 ms tick makes at most 3000 a minute).
const MAX_ERRORS: usize = 4096;
/// After an output opens, its timestamp takes a moment to come: no errors are counted
/// until this has passed.
const OPEN_GRACE_NS: i64 = 1_000_000_000;
/// A usable timestamp whose position strays this far (ms) from where the previous one put
/// the device by now counts as a jump.
const JUMP_MS: i128 = 20;
/// Smoothing of the measured latency, per tick (an exponential moving average).
const LATENCY_SMOOTHING: f64 = 0.1;

/// The output's clock, as the status reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputClockReport {
    /// `None`: no output open.
    pub clock: Option<OutputClock>,
    /// The time between a frame being handed to the device and it being heard (ms).
    pub latency_ms: u32,
    /// Unusable or jumping device timestamps in the last minute.
    pub errors_per_min: u32,
}

/// Works out, once a tick, the device frame being heard: from the output's timestamp when
/// it is usable, otherwise from what was handed over less the device's buffer. Keeps what
/// the status reports about it: which clock is in use, the latency and the timestamp's
/// errors. Runs on the engine's tick, never in the audio callback.
#[derive(Debug, Default)]
pub struct OutputMeter {
    /// An output is open.
    open: bool,
    /// The first tick after the output opened (monotonic ns).
    opened_at: Option<i64>,
    clock: Option<OutputClock>,
    /// The smoothed latency (ms).
    latency_ms: Option<f64>,
    /// The last device frame reported heard (the device clock restarts when an output
    /// opens, and so does this).
    last_heard: u64,
    /// The last usable timestamp's reading: (when, the device frame it put there).
    last_reading: Option<(i64, i64)>,
    /// When each recent error happened (monotonic ns), oldest first.
    errors: std::collections::VecDeque<i64>,
}

impl OutputMeter {
    /// An output (re)opened: its device clock starts at 0. Errors are kept (they are
    /// counted per minute, across reopens).
    pub fn opened(&mut self) {
        *self = OutputMeter {
            open: true,
            errors: std::mem::take(&mut self.errors),
            ..OutputMeter::default()
        };
    }

    /// The output closed (or failed to open).
    pub fn closed(&mut self) {
        *self = OutputMeter {
            errors: std::mem::take(&mut self.errors),
            ..OutputMeter::default()
        };
    }

    /// The device frame heard at `now_ns`, at `rate`, with `written` frames handed to the
    /// device: from what the output says (`pres`) and its buffer (`buffer` frames, if it can
    /// tell). Updates the clock, the latency and the error count. Never goes backwards while
    /// the output stays open, nor beyond `written`.
    pub fn heard(
        &mut self,
        now_ns: i64,
        rate: u32,
        written: u64,
        pres: Presentation,
        buffer: Option<u64>,
    ) -> u64 {
        if !self.open || rate == 0 {
            return written;
        }
        let opened_at = *self.opened_at.get_or_insert(now_ns);
        let counting = now_ns.saturating_sub(opened_at) >= OPEN_GRACE_NS;
        let usable = match pres {
            Presentation::At(ts) if ts.usable(now_ns) => Some(ts),
            Presentation::At(_) | Presentation::Unavailable => {
                if counting {
                    self.error(now_ns);
                }
                None
            }
            Presentation::Unsupported => None,
        };
        let (frame, clock) = match usable {
            Some(ts) => {
                let at = frame_for(ts, rate, now_ns);
                if let Some((then, was)) = self.last_reading {
                    let expected = i128::from(was)
                        + (i128::from(now_ns) - i128::from(then)) * i128::from(rate)
                            / 1_000_000_000;
                    let limit = i128::from(rate) * JUMP_MS / 1000;
                    if counting && (i128::from(at) - expected).abs() > limit {
                        self.error(now_ns);
                    }
                }
                self.last_reading = Some((now_ns, at));
                (frame_at(ts, rate, now_ns, written), OutputClock::Device)
            }
            None => {
                self.last_reading = None;
                let buf = buffer.unwrap_or(0).min(written);
                (written - buf, OutputClock::Estimated)
            }
        };
        let frame = frame.max(self.last_heard);
        self.last_heard = frame;
        let raw = written.saturating_sub(frame) as f64 * 1000.0 / f64::from(rate);
        self.latency_ms = Some(match self.latency_ms {
            Some(was) if self.clock == Some(clock) => was + (raw - was) * LATENCY_SMOOTHING,
            _ => raw,
        });
        self.clock = Some(clock);
        self.prune(now_ns);
        frame
    }

    fn error(&mut self, now_ns: i64) {
        if self.errors.len() >= MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(now_ns);
    }

    fn prune(&mut self, now_ns: i64) {
        while self
            .errors
            .front()
            .is_some_and(|t| now_ns.saturating_sub(*t) > ERROR_WINDOW_NS)
        {
            self.errors.pop_front();
        }
    }

    /// What the status reports.
    pub fn report(&self) -> OutputClockReport {
        let clock = if self.open { self.clock } else { None };
        OutputClockReport {
            clock,
            latency_ms: match clock {
                Some(_) => self.latency_ms.unwrap_or(0.0).round().clamp(0.0, 60_000.0) as u32,
                None => 0,
            },
            errors_per_min: self.errors.len() as u32,
        }
    }
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

    /// A device timestamp 40 ms behind what has been handed over: the clock in use is the
    /// device's and the latency reads 40 ms, settling there through the smoothing.
    #[test]
    fn latency_from_a_usable_timestamp() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let mut report = m.report();
        for tick in 0..100i64 {
            let now = 10_000 * MS + tick * 20 * MS;
            let written = (tick as u64 + 10) * 960; // 20 ms a tick, 200 ms ahead to start
            let heard = written as i64 - 1920; // 40 ms behind
            let ts = Timestamp {
                frame: heard - 240, // stamped 5 ms ago
                at_ns: now - 5 * MS,
            };
            let f = m.heard(now, rate, written, Presentation::At(ts), Some(4800));
            assert_eq!(f as i64, heard);
            report = m.report();
        }
        assert_eq!(report.clock, Some(OutputClock::Device));
        assert_eq!(report.latency_ms, 40);
        assert_eq!(report.errors_per_min, 0);
    }

    /// The latency is smoothed: one odd reading moves it only a little.
    #[test]
    fn the_device_latency_is_smoothed() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let now = 10_000 * MS;
        let ts = |frame| Presentation::At(Timestamp { frame, at_ns: now });
        m.heard(now, rate, 48_000 + 1920, ts(48_000), None); // 40 ms
        m.heard(now, rate, 48_000 + 1920 + 4800, ts(48_000), None); // 140 ms once
        let ms = m.report().latency_ms;
        assert!((45..=55).contains(&ms), "{ms}");
    }

    /// No usable timestamp: what was handed over less the device's buffer, and the latency
    /// is that buffer.
    #[test]
    fn latency_from_the_fallback_is_the_device_buffer() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let f = m.heard(
            10_000 * MS,
            rate,
            48_000,
            Presentation::Unsupported,
            Some(4800),
        );
        assert_eq!(f, 43_200);
        let r = m.report();
        assert_eq!(r.clock, Some(OutputClock::Estimated));
        assert_eq!(r.latency_ms, 100);
        // An output that reports no buffer: what was handed over, no latency known.
        let mut m = OutputMeter::default();
        m.opened();
        assert_eq!(
            m.heard(10_000 * MS, rate, 48_000, Presentation::Unsupported, None),
            48_000
        );
        assert_eq!(m.report().latency_ms, 0);
        // Early on, a buffer larger than what was handed over: nothing heard yet.
        let mut m = OutputMeter::default();
        m.opened();
        assert_eq!(
            m.heard(
                10_000 * MS,
                rate,
                1000,
                Presentation::Unavailable,
                Some(4800)
            ),
            0
        );
    }

    /// The fallback never goes backwards, even when the buffer grows or the timestamp
    /// comes and goes.
    #[test]
    fn the_heard_frame_never_goes_backwards() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let mut last = 0;
        let mut written = 0u64;
        for tick in 0..500i64 {
            let now = 10_000 * MS + tick * 20 * MS;
            written += 960;
            let pres = match tick % 7 {
                0 => Presentation::At(Timestamp {
                    frame: written as i64 - 500,
                    at_ns: now,
                }),
                1 => Presentation::At(Timestamp {
                    frame: 0, // nonsense: far behind
                    at_ns: now,
                }),
                2 => Presentation::Unavailable,
                _ => Presentation::Unsupported,
            };
            let buffer = Some(960 * (tick as u64 % 11));
            let f = m.heard(now, rate, written, pres, buffer);
            assert!(f >= last, "tick {tick}: {f} < {last}");
            assert!(f <= written);
            last = f;
        }
    }

    /// Nothing open: no clock, no latency.
    #[test]
    fn no_output_no_clock() {
        let mut m = OutputMeter::default();
        assert_eq!(m.report().clock, None);
        assert_eq!(
            m.heard(MS, 48_000, 100, Presentation::Unsupported, None),
            100
        );
        assert_eq!(m.report().clock, None);
        m.opened();
        m.heard(MS, 48_000, 4800, Presentation::Unsupported, Some(480));
        assert_eq!(m.report().clock, Some(OutputClock::Estimated));
        m.closed();
        assert_eq!(
            m.report(),
            OutputClockReport {
                clock: None,
                latency_ms: 0,
                errors_per_min: 0,
            }
        );
    }

    /// Unusable timestamps are counted (after the opening grace), over a minute.
    #[test]
    fn unusable_timestamps_are_counted_per_minute() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let start = 100_000 * MS;
        // During the first second after opening, a missing timestamp is normal.
        m.heard(start, rate, 960, Presentation::Unavailable, Some(960));
        m.heard(
            start + 500 * MS,
            rate,
            1920,
            Presentation::Unavailable,
            Some(960),
        );
        assert_eq!(m.report().errors_per_min, 0);
        let stale = Timestamp {
            frame: 10,
            at_ns: start - 5_000 * MS,
        };
        let future = Timestamp {
            frame: 10,
            at_ns: start + 60_000 * MS,
        };
        m.heard(
            start + 1_000 * MS,
            rate,
            3000,
            Presentation::Unavailable,
            None,
        );
        m.heard(
            start + 1_020 * MS,
            rate,
            3000,
            Presentation::At(stale),
            None,
        );
        m.heard(
            start + 1_040 * MS,
            rate,
            3000,
            Presentation::At(future),
            None,
        );
        m.heard(
            start + 1_060 * MS,
            rate,
            3000,
            Presentation::At(Timestamp {
                frame: -1,
                at_ns: start,
            }),
            None,
        );
        assert_eq!(m.report().errors_per_min, 4);
        assert_eq!(m.report().clock, Some(OutputClock::Estimated));
        // An output that can't tell is not in error.
        m.heard(
            start + 1_080 * MS,
            rate,
            3000,
            Presentation::Unsupported,
            None,
        );
        assert_eq!(m.report().errors_per_min, 4);
        // Errors outlive a reopen, and age out after a minute.
        m.opened();
        assert_eq!(m.report().errors_per_min, 4);
        m.heard(
            start + 62_000 * MS,
            rate,
            10,
            Presentation::Unsupported,
            None,
        );
        assert_eq!(m.report().errors_per_min, 0);
    }

    /// A timestamp that jumps (the device position moves by more than the time passed)
    /// is counted too; a steady one is not.
    #[test]
    fn jumping_timestamps_are_counted() {
        let rate = 48_000u32;
        let mut m = OutputMeter::default();
        m.opened();
        let start = 100_000 * MS;
        let mut frame = 0i64;
        for tick in 0..100i64 {
            let now = start + tick * 20 * MS;
            frame += 960;
            if tick == 80 {
                frame += 4800; // 100 ms forward at once
            }
            let ts = Timestamp { frame, at_ns: now };
            m.heard(now, rate, frame as u64 + 4800, Presentation::At(ts), None);
        }
        assert_eq!(m.report().errors_per_min, 1);
        assert_eq!(m.report().clock, Some(OutputClock::Device));
    }

    /// Robustness: any reading, however wild, never panics and stays within what was
    /// handed over.
    #[test]
    fn the_meter_survives_nonsense() {
        let mut m = OutputMeter::default();
        m.opened();
        for &now in &[i64::MIN, -1, 0, 1, i64::MAX] {
            for &rate in &[0u32, 1, 48_000, u32::MAX] {
                for &written in &[0u64, 1, u64::MAX] {
                    for pres in [
                        Presentation::Unsupported,
                        Presentation::Unavailable,
                        Presentation::At(Timestamp {
                            frame: i64::MAX,
                            at_ns: i64::MAX,
                        }),
                        Presentation::At(Timestamp {
                            frame: i64::MIN,
                            at_ns: now,
                        }),
                        Presentation::At(Timestamp {
                            frame: 0,
                            at_ns: now,
                        }),
                    ] {
                        for buffer in [None, Some(0), Some(u64::MAX)] {
                            let _ = m.heard(now, rate, written, pres, buffer);
                            let _ = m.report();
                        }
                    }
                }
            }
            m.opened();
        }
    }

    #[test]
    fn the_monotonic_clock_moves_forward() {
        let a = mono_ns();
        let b = mono_ns();
        assert!(a > 0 && b >= a);
    }
}

//! Audio outputs: where the ring's samples go.
//!
//! - [`AAudioOutput`] (Android only): an AAudio stream whose real-time data callback reads
//!   the ring directly. No JNI per buffer, no allocation and no locks in the callback.
//! - [`PullOutput`]: the host pulls PCM itself through [`crate::LocalPlayer::read_pcm`]
//!   (an `AudioTrack` fed from Kotlin, or tests).
//!
//! An output is opened at the source's sample rate, as 32-bit float for Android's mixer or,
//! for a bit-perfect USB DAC, in the integer format negotiated with the platform (see
//! [`super::negotiate`]). If the source's rate fails, the engine retries at a fallback rate
//! and resamples (float only), and the signal path says so.

use std::sync::{Arc, Mutex};

use super::clock::Timestamp;
use super::negotiate::PcmEncoding;
use super::ring::{Consumer, Ring};

/// An opened output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The stream's actual sample rate.
    pub rate: u32,
    /// The stream's actual sample format.
    pub encoding: PcmEncoding,
    /// Display name of the route, e.g. "AAudio".
    pub device: String,
}

/// Something happened to the output since the last poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputEvent {
    /// The device went away (headphones unplugged, Bluetooth dropped): reopen.
    Disconnected,
}

pub trait AudioOutput: Send {
    /// Open (or reopen) at `rate` in `encoding`, reading from `ring`. Fails rather than
    /// giving another rate or format.
    fn open(
        &mut self,
        ring: &Arc<Ring>,
        rate: u32,
        encoding: PcmEncoding,
    ) -> Result<Opened, String>;
    fn close(&mut self);
    fn poll_event(&mut self) -> Option<OutputEvent>;
    /// When a device frame was (or is being) presented, on the monotonic clock, counting
    /// frames from this stream's open (the ring's device clock is reset then too). `None`
    /// if the output can't say: positions are then taken as heard when handed over.
    fn timestamp(&self) -> Option<Timestamp> {
        None
    }
}

/// The pull slot shared with [`crate::LocalPlayer::read_pcm`].
pub struct PullSlot {
    pub consumer: Option<Consumer>,
    pub rate: u32,
    pub encoding: PcmEncoding,
}

impl Default for PullSlot {
    fn default() -> Self {
        PullSlot {
            consumer: None,
            rate: 0,
            encoding: PcmEncoding::Float,
        }
    }
}

/// Output for hosts that pull PCM themselves. It takes any format: the host reads float
/// with [`crate::LocalPlayer::read_pcm`] or integers with
/// [`crate::LocalPlayer::read_pcm_int`].
pub struct PullOutput {
    slot: Arc<Mutex<PullSlot>>,
}

impl PullOutput {
    pub fn new(slot: Arc<Mutex<PullSlot>>) -> Self {
        PullOutput { slot }
    }
}

impl AudioOutput for PullOutput {
    fn open(
        &mut self,
        ring: &Arc<Ring>,
        rate: u32,
        encoding: PcmEncoding,
    ) -> Result<Opened, String> {
        let mut slot = self.slot.lock().unwrap();
        slot.consumer = None; // give the old handle back first
        slot.consumer = Some(ring.consumer().ok_or("output already in use")?);
        ring.reset_device_clock();
        slot.rate = rate;
        slot.encoding = encoding;
        Ok(Opened {
            rate,
            encoding,
            device: "Host output".into(),
        })
    }

    fn close(&mut self) {
        let mut slot = self.slot.lock().unwrap();
        slot.consumer = None;
        slot.rate = 0;
        slot.encoding = PcmEncoding::Float;
    }

    fn poll_event(&mut self) -> Option<OutputEvent> {
        None
    }
}

#[cfg(target_os = "android")]
pub use aaudio::AAudioOutput;

#[cfg(target_os = "android")]
mod aaudio {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ndk::audio::{
        AudioCallbackResult, AudioContentType, AudioDirection, AudioError, AudioFormat,
        AudioPerformanceMode, AudioSharingMode, AudioStream, AudioStreamBuilder, AudioUsage,
        Clockid,
    };

    use super::{AudioOutput, Opened, OutputEvent, PcmEncoding, Timestamp};
    use crate::player::clock::mono_ns;
    use crate::player::ring::{CHANNELS, Ring};

    /// `AAUDIO_FORMAT_PCM_I24_PACKED` and `AAUDIO_FORMAT_PCM_I32` (API 31). The `ndk`
    /// crate's enum doesn't name them, but carries any value through.
    const AAUDIO_FORMAT_PCM_I24_PACKED: i32 = 3;
    const AAUDIO_FORMAT_PCM_I32: i32 = 4;

    fn aaudio_format(e: PcmEncoding) -> AudioFormat {
        match e {
            PcmEncoding::Float => AudioFormat::PCM_Float,
            PcmEncoding::I16 => AudioFormat::PCM_I16,
            PcmEncoding::I24Packed => AudioFormat::from(AAUDIO_FORMAT_PCM_I24_PACKED),
            PcmEncoding::I32 => AudioFormat::from(AAUDIO_FORMAT_PCM_I32),
        }
    }

    /// An AAudio output stream in shared mode, stereo, at the source's rate:
    /// - 32-bit float with "power saving" performance (the deep-buffer path: right for
    ///   music, and kind to the battery) for Android's mixer;
    /// - the negotiated integer format, with no performance hint, for a bit-perfect USB
    ///   DAC (Android then routes it to the bit-perfect output it set up for us).
    #[derive(Default)]
    pub struct AAudioOutput {
        stream: Option<AudioStream>,
        disconnected: Arc<AtomicBool>,
    }

    // SAFETY: an `AAudioStream` may be controlled from any thread; this struct is only ever
    // used by one thread at a time (the engine's), and the callbacks own their own state.
    unsafe impl Send for AAudioOutput {}

    impl AudioOutput for AAudioOutput {
        fn open(
            &mut self,
            ring: &Arc<Ring>,
            rate: u32,
            encoding: PcmEncoding,
        ) -> Result<Opened, String> {
            self.close();
            let rate_i = i32::try_from(rate).map_err(|_| "bad sample rate")?;
            let mut consumer = ring.consumer().ok_or("output already in use")?;
            // The new stream counts frames from 0: so does the ring's device clock.
            ring.reset_device_clock();
            self.disconnected.store(false, Ordering::Release);
            let flag = self.disconnected.clone();
            let performance = if encoding == PcmEncoding::Float {
                AudioPerformanceMode::PowerSaving
            } else {
                AudioPerformanceMode::None
            };
            let stream = AudioStreamBuilder::new()
                .map_err(|e| format!("AAudio: {e}"))?
                .direction(AudioDirection::Output)
                .sharing_mode(AudioSharingMode::Shared)
                .performance_mode(performance)
                .usage(AudioUsage::Media)
                .content_type(AudioContentType::Music)
                .format(aaudio_format(encoding))
                .channel_count(CHANNELS as i32)
                .sample_rate(rate_i)
                .data_callback(Box::new(move |_stream, data, frames| {
                    let len = usize::try_from(frames).unwrap_or(0) * CHANNELS;
                    if !data.is_null() && len > 0 {
                        // SAFETY: AAudio hands us a buffer of `frames` frames in the format
                        // we asked for and got (`encoding`, `CHANNELS` channels: both are
                        // checked after the open, which fails rather than giving us another
                        // layout). It is valid, suitably aligned for that sample type and
                        // exclusive for this call only.
                        match encoding {
                            PcmEncoding::Float => {
                                let out = unsafe {
                                    std::slice::from_raw_parts_mut(data.cast::<f32>(), len)
                                };
                                consumer.read(out);
                            }
                            PcmEncoding::I16 => {
                                let out = unsafe {
                                    std::slice::from_raw_parts_mut(data.cast::<i16>(), len)
                                };
                                consumer.read_i16(out);
                            }
                            PcmEncoding::I24Packed => {
                                let out = unsafe {
                                    std::slice::from_raw_parts_mut(data.cast::<u8>(), len * 3)
                                };
                                consumer.read_i24_packed(out);
                            }
                            PcmEncoding::I32 => {
                                let out = unsafe {
                                    std::slice::from_raw_parts_mut(data.cast::<i32>(), len)
                                };
                                consumer.read_i32(out, 32);
                            }
                        }
                    }
                    AudioCallbackResult::Continue
                }))
                .error_callback(Box::new(move |_stream, _error: AudioError| {
                    // Any error (usually `Disconnected`) leaves the stream unusable. Never
                    // touch it here: the engine reopens it from its own thread.
                    flag.store(true, Ordering::Release);
                }))
                .open_stream()
                .map_err(|e| format!("AAudio open at {rate} Hz ({encoding:?}): {e}"))?;
            if stream.format() != aaudio_format(encoding)
                || stream.channel_count() != CHANNELS as i32
            {
                return Err(format!(
                    "AAudio gave {:?} instead of {encoding:?}",
                    stream.format()
                ));
            }
            let actual = u32::try_from(stream.sample_rate()).unwrap_or(0);
            if actual != rate {
                // Never play at the wrong speed: the engine falls back and resamples.
                return Err(format!("AAudio opened at {actual} Hz instead of {rate} Hz"));
            }
            stream
                .request_start()
                .map_err(|e| format!("AAudio start: {e}"))?;
            self.stream = Some(stream);
            Ok(Opened {
                rate,
                encoding,
                device: "AAudio".into(),
            })
        }

        fn close(&mut self) {
            if let Some(s) = self.stream.take() {
                let _ = s.request_stop();
                // Dropping closes the stream, which waits for a running callback to return
                // and then frees it (handing the ring's consumer back).
                drop(s);
            }
        }

        fn poll_event(&mut self) -> Option<OutputEvent> {
            (self.stream.is_some() && self.disconnected.swap(false, Ordering::AcqRel))
                .then_some(OutputEvent::Disconnected)
        }

        /// AAudio's presentation timestamp on `CLOCK_MONOTONIC`: the frame heard at a given
        /// time, including the route's latency (Bluetooth too, where Android reports it).
        /// Until the stream is running it has none: then the frames the device has read so
        /// far, as of now (which leaves out only the latency after the mixer).
        fn timestamp(&self) -> Option<Timestamp> {
            let s = self.stream.as_ref()?;
            match s.timestamp(Clockid::Monotonic) {
                Ok(t) => Some(Timestamp {
                    frame: t.frame_position,
                    at_ns: t.time_nanoseconds,
                }),
                Err(_) => {
                    let read = s.frames_read();
                    (read >= 0).then(|| Timestamp {
                        frame: read,
                        at_ns: mono_ns(),
                    })
                }
            }
        }
    }

    impl Drop for AAudioOutput {
        fn drop(&mut self) {
            self.close();
        }
    }
}

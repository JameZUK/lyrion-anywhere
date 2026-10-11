//! Audio outputs: where the ring's samples go.
//!
//! - [`AAudioOutput`] (Android only): an AAudio stream whose real-time data callback reads
//!   the ring directly. No JNI per buffer, no allocation and no locks in the callback.
//! - [`PullOutput`]: the host pulls PCM itself through [`crate::LocalPlayer::read_pcm`]
//!   (an `AudioTrack` fed from Kotlin, or tests).
//!
//! An output is opened at the source's sample rate. If that fails, the engine retries at a
//! fallback rate and resamples, and the signal path says so.

use std::sync::{Arc, Mutex};

use super::ring::{Consumer, Ring};

/// An opened output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The stream's actual sample rate.
    pub rate: u32,
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
    /// Open (or reopen) at `rate`, reading from `ring`.
    fn open(&mut self, ring: &Arc<Ring>, rate: u32) -> Result<Opened, String>;
    fn close(&mut self);
    fn poll_event(&mut self) -> Option<OutputEvent>;
}

/// The pull slot shared with [`crate::LocalPlayer::read_pcm`].
#[derive(Default)]
pub struct PullSlot {
    pub consumer: Option<Consumer>,
    pub rate: u32,
}

/// Output for hosts that pull PCM themselves.
pub struct PullOutput {
    slot: Arc<Mutex<PullSlot>>,
}

impl PullOutput {
    pub fn new(slot: Arc<Mutex<PullSlot>>) -> Self {
        PullOutput { slot }
    }
}

impl AudioOutput for PullOutput {
    fn open(&mut self, ring: &Arc<Ring>, rate: u32) -> Result<Opened, String> {
        let mut slot = self.slot.lock().unwrap();
        slot.consumer = None; // give the old handle back first
        slot.consumer = Some(ring.consumer().ok_or("output already in use")?);
        slot.rate = rate;
        Ok(Opened {
            rate,
            device: "Host output".into(),
        })
    }

    fn close(&mut self) {
        let mut slot = self.slot.lock().unwrap();
        slot.consumer = None;
        slot.rate = 0;
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
    };

    use super::{AudioOutput, Opened, OutputEvent};
    use crate::player::ring::{CHANNELS, Ring};

    /// An AAudio output stream in shared mode, 32-bit float, stereo, "power saving"
    /// performance (large buffers on the deep-buffer path: right for music, and kind to the
    /// battery).
    #[derive(Default)]
    pub struct AAudioOutput {
        stream: Option<AudioStream>,
        disconnected: Arc<AtomicBool>,
    }

    // SAFETY: an `AAudioStream` may be controlled from any thread; this struct is only ever
    // used by one thread at a time (the engine's), and the callbacks own their own state.
    unsafe impl Send for AAudioOutput {}

    impl AudioOutput for AAudioOutput {
        fn open(&mut self, ring: &Arc<Ring>, rate: u32) -> Result<Opened, String> {
            self.close();
            let rate_i = i32::try_from(rate).map_err(|_| "bad sample rate")?;
            let mut consumer = ring.consumer().ok_or("output already in use")?;
            self.disconnected.store(false, Ordering::Release);
            let flag = self.disconnected.clone();
            let stream = AudioStreamBuilder::new()
                .map_err(|e| format!("AAudio: {e}"))?
                .direction(AudioDirection::Output)
                .sharing_mode(AudioSharingMode::Shared)
                .performance_mode(AudioPerformanceMode::PowerSaving)
                .usage(AudioUsage::Media)
                .content_type(AudioContentType::Music)
                .format(AudioFormat::PCM_Float)
                .channel_count(CHANNELS as i32)
                .sample_rate(rate_i)
                .data_callback(Box::new(move |_stream, data, frames| {
                    let len = usize::try_from(frames).unwrap_or(0) * CHANNELS;
                    if !data.is_null() && len > 0 {
                        // SAFETY: AAudio hands us a buffer of `frames` frames in the format
                        // we asked for and got (float, `CHANNELS` channels: both are set
                        // exactly, so the open fails rather than giving us another
                        // layout). It is valid and exclusive for this call only.
                        let out =
                            unsafe { std::slice::from_raw_parts_mut(data.cast::<f32>(), len) };
                        consumer.read(out);
                    }
                    AudioCallbackResult::Continue
                }))
                .error_callback(Box::new(move |_stream, _error: AudioError| {
                    // Any error (usually `Disconnected`) leaves the stream unusable. Never
                    // touch it here: the engine reopens it from its own thread.
                    flag.store(true, Ordering::Release);
                }))
                .open_stream()
                .map_err(|e| format!("AAudio open at {rate} Hz: {e}"))?;
            if stream.format() != AudioFormat::PCM_Float
                || stream.channel_count() != CHANNELS as i32
            {
                return Err("AAudio gave an unexpected format".into());
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
    }

    impl Drop for AAudioOutput {
        fn drop(&mut self) {
            self.close();
        }
    }
}

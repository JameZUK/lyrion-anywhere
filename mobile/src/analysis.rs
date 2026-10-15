//! Track analysis from the gateway (`GET /v1/analysis/track/{id}?format=binary`): the
//! waveform for the seek bar, and the R128 loudness for levelling.
//!
//! The binary form is decoded by the core's decoder ([`TrackAnalysis::from_bytes`]), which
//! treats it as untrusted (fuzz-tested), and the answer is size-capped before decoding. So
//! the app never parses analysis bytes itself: it gets a plain record.
//!
//! The gateway answers 202 while it is still analysing; that is
//! [`AnalysisOutcome::Pending`] with the suggested wait. 403, 404, 413, 415, 422 and 504
//! mean there is no analysis for the track ([`AnalysisOutcome::Unavailable`]); the app
//! keeps its plain seek bar and the engine falls back to ReplayGain tags.

use std::sync::Arc;
use std::time::Duration;

use anywhere_core::analysis::TrackAnalysis;
use anywhere_core::analysis::encode::MAX_BINARY_BYTES;
use anywhere_core::dsp::levelling::Measurement;
use hyper::StatusCode;

use crate::error::ApiError;
use crate::session::{Core, Session};
use crate::transport;

pub const BINARY_TYPE: &str = "application/vnd.lyrion-anywhere.analysis";

/// What the app needs from an analysis.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrackAnalysisInfo {
    pub duration_s: f64,
    pub sample_rate: u32,
    /// Integrated loudness, LUFS (`None` for silence).
    pub integrated_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    /// Waveform buckets (0 = silence, 255 = full scale); `peak` and `rms` have the same
    /// length.
    pub peak: Vec<u8>,
    pub rms: Vec<u8>,
    /// Silence at the start and end, seconds (for smart transitions).
    #[uniffi(default = 0.0)]
    pub leading_silence_s: f64,
    #[uniffi(default = 0.0)]
    pub trailing_silence_s: f64,
    /// The length of a deliberate fade-in or fade-out, seconds (`None`: none).
    #[uniffi(default = None)]
    pub fade_in_s: Option<f64>,
    #[uniffi(default = None)]
    pub fade_out_s: Option<f64>,
}

impl From<TrackAnalysis> for TrackAnalysisInfo {
    fn from(a: TrackAnalysis) -> Self {
        TrackAnalysisInfo {
            duration_s: a.duration_s(),
            sample_rate: a.sample_rate,
            integrated_lufs: a.integrated_lufs,
            loudness_range_lu: a.loudness_range_lu,
            true_peak_dbtp: a.true_peak_dbtp,
            leading_silence_s: a.leading_silence_s,
            trailing_silence_s: a.trailing_silence_s,
            fade_in_s: a.fade_in.fade.then_some(a.fade_in.length_s),
            fade_out_s: a.fade_out.fade.then_some(a.fade_out.length_s),
            peak: a.waveform.peak,
            rms: a.waveform.rms,
        }
    }
}

impl TrackAnalysisInfo {
    /// The loudness measurement for levelling, if the track has one.
    pub fn measurement(&self) -> Option<Measurement> {
        Measurement::from_analysis(self.integrated_lufs?, self.true_peak_dbtp.unwrap_or(0.0))
    }

    /// The track's ends, for smart transitions.
    pub fn edges(&self) -> crate::player::transitions::EdgeFacts {
        crate::player::transitions::EdgeFacts {
            leading_silence_s: self.leading_silence_s,
            trailing_silence_s: self.trailing_silence_s,
            fade_in_s: self.fade_in_s,
            fade_out_s: self.fade_out_s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AnalysisOutcome {
    Ready {
        analysis: TrackAnalysisInfo,
    },
    /// Still being analysed: ask again after this long.
    Pending {
        retry_after_ms: u64,
    },
    /// No analysis for this track (refused, not a library file, unsupported, or analysis
    /// is off on this server).
    Unavailable {
        reason: String,
    },
}

/// The path for a track id (positive, at most 12 digits, as the gateway accepts).
pub fn analysis_path(track_id: i64) -> Result<String, ApiError> {
    if !(1..=999_999_999_999).contains(&track_id) {
        return Err(ApiError::invalid("bad track id"));
    }
    Ok(format!("v1/analysis/track/{track_id}?format=binary"))
}

/// Seconds from Retry-After, clamped to 1–30 (3 when missing or malformed).
pub fn retry_after_ms(v: Option<&str>) -> u64 {
    let s = v
        .map(str::trim)
        .filter(|s| (1..=6).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3);
    s.clamp(1, 30) * 1000
}

/// One request for a track's analysis.
pub(crate) async fn fetch(core: &Arc<Core>, track_id: i64) -> Result<AnalysisOutcome, ApiError> {
    let path = analysis_path(track_id)?;
    let url = core.server().endpoint(&path);
    core.authed(|token| {
        let req = core
            .http()
            .get(url.clone())
            .header("accept", BINARY_TYPE)
            .bearer_auth(&token);
        async move {
            let resp = req.send().await?;
            let status = resp.status();
            let retry = || {
                retry_after_ms(
                    resp.headers()
                        .get(hyper::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()),
                )
            };
            match status {
                StatusCode::OK => {
                    let ct = resp
                        .headers()
                        .get(hyper::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    if ct != BINARY_TYPE {
                        return Err(ApiError::bad_response("not an analysis"));
                    }
                    let bytes = resp.read_limited(MAX_BINARY_BYTES).await?;
                    let a = TrackAnalysis::from_bytes(&bytes)
                        .map_err(|_| ApiError::bad_response("malformed analysis"))?;
                    Ok(AnalysisOutcome::Ready { analysis: a.into() })
                }
                StatusCode::ACCEPTED | StatusCode::TOO_MANY_REQUESTS => {
                    Ok(AnalysisOutcome::Pending {
                        retry_after_ms: retry(),
                    })
                }
                StatusCode::FORBIDDEN
                | StatusCode::NOT_FOUND
                | StatusCode::PAYLOAD_TOO_LARGE
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
                | StatusCode::UNPROCESSABLE_ENTITY
                | StatusCode::GATEWAY_TIMEOUT => {
                    let e = transport::error_from_response(resp).await;
                    let reason = match e {
                        ApiError::Forbidden { detail }
                        | ApiError::Server { detail, .. }
                        | ApiError::InvalidInput { detail }
                        | ApiError::NotFound { detail }
                        | ApiError::Conflict { detail } => detail,
                        other => other.to_string(),
                    };
                    Ok(AnalysisOutcome::Unavailable { reason })
                }
                _ => Err(transport::error_from_response(resp).await),
            }
        }
    })
    .await
}

/// Ask until the analysis is ready or unavailable, waiting as the gateway suggests, for at
/// most `budget` in all. `None` if it was still pending when the budget ran out.
pub(crate) async fn fetch_waiting(
    core: &Arc<Core>,
    track_id: i64,
    budget: Duration,
) -> Result<Option<TrackAnalysisInfo>, ApiError> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let outcome = tokio::time::timeout(left, fetch(core, track_id))
            .await
            .map_err(|_| ApiError::network("timed out"))??;
        match outcome {
            AnalysisOutcome::Ready { analysis } => return Ok(Some(analysis)),
            AnalysisOutcome::Unavailable { .. } => return Ok(None),
            AnalysisOutcome::Pending { retry_after_ms } => {
                let wake = tokio::time::Instant::now() + Duration::from_millis(retry_after_ms);
                if wake >= deadline {
                    return Ok(None);
                }
                tokio::time::sleep_until(wake).await;
            }
        }
    }
}

/// Track analyses for the app (the waveform seek bar).
#[derive(uniffi::Object)]
pub struct Analysis {
    session: Arc<Session>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Analysis {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Analysis { session })
    }

    /// A track's analysis, one request: ready, pending (ask again after the wait given), or
    /// unavailable. Errors are network trouble and the like (try again later).
    pub async fn track(&self, track_id: i64) -> Result<AnalysisOutcome, ApiError> {
        fetch(self.session.core(), track_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_only_for_valid_ids() {
        assert_eq!(
            analysis_path(42).unwrap(),
            "v1/analysis/track/42?format=binary"
        );
        for bad in [0, -1, i64::MIN, 1_000_000_000_000, i64::MAX] {
            assert!(analysis_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn retry_after_is_bounded() {
        assert_eq!(retry_after_ms(Some("5")), 5000);
        assert_eq!(retry_after_ms(None), 3000);
        assert_eq!(retry_after_ms(Some("0")), 1000);
        assert_eq!(retry_after_ms(Some("99999")), 30_000);
        assert_eq!(retry_after_ms(Some("-4")), 3000);
        assert_eq!(retry_after_ms(Some("1e9")), 3000);
        assert_eq!(retry_after_ms(Some("12345678901234567890")), 3000);
    }

    #[test]
    fn measurement_from_the_analysis() {
        let mut a = TrackAnalysisInfo {
            duration_s: 10.0,
            sample_rate: 44_100,
            integrated_lufs: Some(-9.5),
            loudness_range_lu: None,
            true_peak_dbtp: Some(0.4),
            peak: vec![],
            rms: vec![],
            leading_silence_s: 0.5,
            trailing_silence_s: 1.25,
            fade_in_s: None,
            fade_out_s: Some(4.0),
        };
        let e = a.edges();
        assert_eq!(
            (
                e.leading_silence_s,
                e.trailing_silence_s,
                e.fade_in_s,
                e.fade_out_s
            ),
            (0.5, 1.25, None, Some(4.0))
        );
        let m = a.measurement().unwrap();
        assert_eq!(m.loudness_lufs, -9.5);
        assert_eq!(m.peak_db, 0.4);
        a.integrated_lufs = None;
        assert!(a.measurement().is_none(), "silence: nothing to level");
        a.integrated_lufs = Some(f64::NAN);
        assert!(a.measurement().is_none());
    }
}

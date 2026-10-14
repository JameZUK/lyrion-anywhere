//! The one error type every exported call returns (Kotlin: `ApiException`).
//!
//! Messages are safe to show and to log: they never contain tokens, passwords or request
//! bodies. Field names avoid `message`, which would clash with `Throwable.message` in Kotlin.

/// Why a call failed, in terms the UI can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ApiError {
    /// The server couldn't be reached, or the connection failed or timed out. Retry later;
    /// the sign-in is kept.
    #[error("network error: {detail}")]
    Network { detail: String },
    /// Not signed in, or the sign-in was refused or has ended (sign in again).
    #[error("not signed in")]
    Unauthorised,
    /// The password was right; repeat the sign-in with a two-step code.
    #[error("two-step code required")]
    TwoStepRequired,
    /// Refused by the server's policy (a command outside the allow-list, a player this
    /// account may not control, …).
    #[error("refused: {detail}")]
    Forbidden { detail: String },
    /// Too many attempts or requests: wait this many seconds.
    #[error("too many requests; retry in {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    /// The music server (LMS) is busy and stopped answering for a while (its music share
    /// stalled, say); the gateway refused the request at once rather than queue it. Nothing
    /// was done: try again after `retry_after_secs`. The gateway itself is reachable.
    #[error("the music server is busy; retry in {retry_after_secs} s")]
    ServerBusy { retry_after_secs: u64 },
    /// The server (or the music server behind it) failed or answered nonsense.
    #[error("server error {status}: {detail}")]
    Server { status: u16, detail: String },
    /// The request was refused locally (or by the server) as malformed.
    #[error("invalid input: {detail}")]
    InvalidInput { detail: String },
}

impl ApiError {
    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        ApiError::InvalidInput {
            detail: detail.into(),
        }
    }

    pub(crate) fn network(detail: impl Into<String>) -> Self {
        ApiError::Network {
            detail: detail.into(),
        }
    }

    /// The gateway's "music server is busy" refusal: a 503 whose JSON body says
    /// `"busy": true`. A 503 without it keeps its old meaning (see [`Self::from_status`]).
    pub(crate) fn busy(retry_after: Option<u64>) -> Self {
        ApiError::ServerBusy {
            retry_after_secs: retry_after.unwrap_or(2).clamp(1, 300),
        }
    }

    pub(crate) fn bad_response(detail: impl Into<String>) -> Self {
        ApiError::Server {
            status: 0,
            detail: detail.into(),
        }
    }

    /// Map an HTTP failure. `error` is the gateway's `{"error": …}` text, if any.
    pub(crate) fn from_status(
        status: u16,
        error: Option<&str>,
        totp_required: bool,
        retry_after: Option<u64>,
    ) -> Self {
        let detail = clean_detail(error.unwrap_or(""), status);
        match status {
            401 | 403 if totp_required => ApiError::TwoStepRequired,
            401 => ApiError::Unauthorised,
            403 => ApiError::Forbidden { detail },
            408 => ApiError::Network { detail },
            429 => ApiError::RateLimited {
                retry_after_secs: retry_after.unwrap_or(5).clamp(1, 86_400),
            },
            503 if retry_after.is_some() => ApiError::RateLimited {
                retry_after_secs: retry_after.unwrap_or(1).clamp(1, 86_400),
            },
            400..=499 => ApiError::InvalidInput { detail },
            _ => ApiError::Server { status, detail },
        }
    }

    /// A refusal by the server (sign out), as opposed to a failure to reach it (keep the
    /// sign-in and retry).
    pub(crate) fn is_refusal(&self) -> bool {
        matches!(
            self,
            ApiError::Unauthorised
                | ApiError::Forbidden { .. }
                | ApiError::InvalidInput { .. }
                | ApiError::TwoStepRequired
        )
    }
}

/// The server's error text, cut down to something short and printable.
fn clean_detail(error: &str, status: u16) -> String {
    let s: String = error
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if s.trim().is_empty() {
        format!("HTTP {status}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_actionable_errors() {
        assert_eq!(
            ApiError::from_status(401, Some("invalid credentials"), false, None),
            ApiError::Unauthorised
        );
        assert_eq!(
            ApiError::from_status(401, None, true, None),
            ApiError::TwoStepRequired
        );
        // A password change asks for the code with 403 (the token itself is fine).
        assert_eq!(
            ApiError::from_status(403, Some("two-step code required"), true, None),
            ApiError::TwoStepRequired
        );
        assert_eq!(
            ApiError::from_status(403, Some("wrong"), false, None),
            ApiError::Forbidden {
                detail: "wrong".into()
            }
        );
        assert_eq!(
            ApiError::from_status(403, Some("command not allowed"), false, None),
            ApiError::Forbidden {
                detail: "command not allowed".into()
            }
        );
        assert_eq!(
            ApiError::from_status(429, None, false, Some(900)),
            ApiError::RateLimited {
                retry_after_secs: 900
            }
        );
        // No Retry-After: a sensible default, never zero.
        assert_eq!(
            ApiError::from_status(429, None, false, Some(0)),
            ApiError::RateLimited {
                retry_after_secs: 1
            }
        );
        assert_eq!(
            ApiError::from_status(503, None, false, Some(1)),
            ApiError::RateLimited {
                retry_after_secs: 1
            }
        );
        assert!(matches!(
            ApiError::from_status(502, None, false, None),
            ApiError::Server { status: 502, .. }
        ));
        assert!(matches!(
            ApiError::from_status(413, None, false, None),
            ApiError::InvalidInput { .. }
        ));
        assert!(matches!(
            ApiError::from_status(408, None, false, None),
            ApiError::Network { .. }
        ));
    }

    #[test]
    fn server_text_is_cleaned_and_capped() {
        let long = format!("a\u{7}b\r\n{}", "x".repeat(500));
        match ApiError::from_status(403, Some(&long), false, None) {
            ApiError::Forbidden { detail } => {
                assert!(detail.starts_with("ab"));
                assert!(detail.chars().count() <= 200);
                assert!(!detail.chars().any(char::is_control));
            }
            other => panic!("{other:?}"),
        }
        match ApiError::from_status(500, Some(" "), false, None) {
            ApiError::Server { detail, .. } => assert_eq!(detail, "HTTP 500"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn only_refusals_end_a_sign_in() {
        assert!(ApiError::Unauthorised.is_refusal());
        assert!(!ApiError::network("x").is_refusal());
        assert!(
            !ApiError::RateLimited {
                retry_after_secs: 1
            }
            .is_refusal()
        );
        assert!(!ApiError::bad_response("x").is_refusal());
    }
}

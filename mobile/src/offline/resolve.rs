//! "Continue on <player>" (SPEC §8.4): back online, the offline queue is handed to an LMS
//! player. Downloads are kept by stable key, and LMS may have renumbered its tracks since
//! (a full rescan), so each queued track's id is checked first.
//!
//! - **How:** `GET /v1/download/track/{id}/info` for the id the track was downloaded with.
//!   The gateway answers with the stable key of the file that id names now. The id is used
//!   only when that key is the download's own: the same file, still in the library.
//! - **Unresolved:** an id LMS no longer knows (404), or one that names another file now, is
//!   `None`: the caller skips that track and says how many were skipped.
//! - **Refused:** anything else (signed out, downloads not allowed, the server unreachable,
//!   too many requests) stops the whole lookup with that error, so nothing half-checked is
//!   ever sent to a player.
//! - **Whose:** only the signed-in account's own library is accepted (review M14): another
//!   account's or server's is refused with [`ApiError::WrongAccount`] before anything is
//!   asked, and a queued track that library doesn't hold is `None` without asking.
//! - Every id sent to a player afterwards is one the gateway confirmed for this user; ids and
//!   keys that aren't well formed are `None` without asking.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use crate::error::ApiError;
use crate::offline::download::parse_info;
use crate::offline::index::valid_key;
use crate::offline::store::OfflineLibrary;
use crate::session::{Core, Session};
use crate::transport;

/// Most tracks looked up at once (a longer queue is cut by the caller first).
pub const MAX_RESOLVE: usize = 2000;
/// Lookups in flight at once.
const PARALLEL: usize = 4;
/// Largest `/info` answer read.
const MAX_INFO: usize = 64 * 1024;

/// A downloaded track to look up: the id it was downloaded with and its stable key.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OfflineTrackRef {
    pub track_id: i64,
    pub key: String,
}

/// `/info` for one id: `Ok(None)` when the server doesn't know it (404).
pub trait InfoApi {
    fn info(
        &self,
        id: i64,
    ) -> impl std::future::Future<Output = Result<Option<Value>, ApiError>> + Send;
}

/// The id is plausible: a positive id of at most 12 digits, as the gateway takes.
fn plausible(id: i64) -> bool {
    id > 0 && id < 1_000_000_000_000
}

/// Each track's current LMS id, in the order given (`None`: not in the library as that
/// file any more). Fails as a whole on the first error other than "unknown id".
pub async fn resolve<A: InfoApi + Sync>(
    api: &A,
    tracks: &[OfflineTrackRef],
) -> Result<Vec<Option<i64>>, ApiError> {
    use futures_util::StreamExt;
    if tracks.len() > MAX_RESOLVE {
        return Err(ApiError::invalid("too many tracks to look up"));
    }
    // Each id is asked about once, however often it is queued.
    let mut ids: Vec<i64> = Vec::new();
    for t in tracks {
        if plausible(t.track_id) && valid_key(&t.key) && !ids.contains(&t.track_id) {
            ids.push(t.track_id);
        }
    }
    let lookups = ids.into_iter().map(|id| async move {
        let key = match api.info(id).await? {
            None => None,
            // An answer that doesn't fit the contract confirms nothing.
            Some(v) => parse_info(&v, id).ok().map(|i| i.key),
        };
        Ok::<_, ApiError>((id, key))
    });
    let mut stream = futures_util::stream::iter(lookups).buffered(PARALLEL);
    let mut now: HashMap<i64, Option<String>> = HashMap::new();
    while let Some(r) = stream.next().await {
        let (id, key) = r?;
        now.insert(id, key);
    }
    Ok(tracks
        .iter()
        .map(|t| match now.get(&t.track_id) {
            Some(Some(k)) if *k == t.key => Some(t.track_id),
            _ => None,
        })
        .collect())
}

/// The real [`InfoApi`]: the gateway, with the session's token (renewed once on a 401).
struct HttpInfo {
    core: Arc<Core>,
}

impl InfoApi for HttpInfo {
    async fn info(&self, id: i64) -> Result<Option<Value>, ApiError> {
        let url = self
            .core
            .server()
            .endpoint(&format!("v1/download/track/{id}/info"));
        self.core
            .authed(|token| {
                let req = self.core.http().get(url.clone()).bearer_auth(&token);
                async move {
                    let resp = req.send().await?;
                    let status = resp.status();
                    if status.as_u16() == 404 {
                        return Ok(None);
                    }
                    if !status.is_success() {
                        return Err(transport::error_from_response(resp).await);
                    }
                    let body = resp.read_limited(MAX_INFO).await?;
                    Ok(Some(serde_json::from_slice(&body).unwrap_or(Value::Null)))
                }
            })
            .await
    }
}

/// Look downloaded tracks up for "Continue on <player>": each one's current LMS id, in the
/// order given, or `None` where the library no longer has that file (see the module notes).
/// `library` must be the signed-in account's own (else [`ApiError::WrongAccount`], or
/// `Unauthorised` signed out, before anything is asked), and only tracks it holds are
/// looked up: any other is `None`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn resolve_offline_tracks(
    session: Arc<Session>,
    library: Arc<OfflineLibrary>,
    tracks: Vec<OfflineTrackRef>,
) -> Result<Vec<Option<i64>>, ApiError> {
    crate::offline::owned_by(&session, &library).await?;
    if tracks.len() > MAX_RESOLVE {
        return Err(ApiError::invalid("too many tracks to look up"));
    }
    let held: HashSet<String> = library.tracks().into_iter().map(|t| t.key).collect();
    let ours: Vec<OfflineTrackRef> = tracks
        .iter()
        .filter(|t| held.contains(&t.key))
        .cloned()
        .collect();
    let api = HttpInfo {
        core: session.core().clone(),
    };
    let mut found = resolve(&api, &ours).await?.into_iter();
    Ok(tracks
        .iter()
        .map(|t| {
            if held.contains(&t.key) {
                found.next().flatten()
            } else {
                None
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn key(n: i64) -> String {
        format!("{n:064x}")
    }

    fn info(id: i64, key: &str) -> Value {
        json!({"id": id, "key": key, "title": "t", "original": {"format": "flc", "size": 1, "ext": "flac"},
            "formats": ["original"]})
    }

    /// Ids 1–9 are in the library with key(id); 5 was renumbered (now another file);
    /// 404 for anything else. `fail` makes that id's lookup fail.
    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<i64>>,
        fail: Option<(i64, ApiError)>,
    }

    impl InfoApi for Fake {
        async fn info(&self, id: i64) -> Result<Option<Value>, ApiError> {
            self.asked.lock().unwrap().push(id);
            if let Some((f, e)) = &self.fail
                && *f == id
            {
                return Err(e.clone());
            }
            Ok(match id {
                5 => Some(info(5, &key(55))),
                // Malformed: the id isn't the one asked for.
                7 => Some(info(70, &key(7))),
                1..=9 | 0xab => Some(info(id, &key(id))),
                _ => None,
            })
        }
    }

    fn r(id: i64, k: String) -> OfflineTrackRef {
        OfflineTrackRef {
            track_id: id,
            key: k,
        }
    }

    #[tokio::test]
    async fn confirms_ids_in_order_and_skips_the_rest() {
        let api = Fake::default();
        let got = resolve(
            &api,
            &[
                r(3, key(3)),
                r(42, key(42)), // gone from the library
                r(1, key(1)),
                r(5, key(5)),  // renumbered: another file now
                r(2, key(99)), // not this download's file
                r(7, key(7)),  // an answer that doesn't fit
                r(3, key(3)),  // queued twice
            ],
        )
        .await
        .unwrap();
        assert_eq!(got, vec![Some(3), None, Some(1), None, None, None, Some(3)]);
        let mut asked = api.asked.lock().unwrap().clone();
        asked.sort();
        assert_eq!(asked, vec![1, 2, 3, 5, 7, 42], "each id asked about once");
    }

    #[tokio::test]
    async fn malformed_ids_and_keys_are_never_sent() {
        let api = Fake::default();
        let got = resolve(
            &api,
            &[
                r(0, key(0)),
                r(-4, key(4)),
                r(1_000_000_000_000, key(1)),
                r(1, "../../etc".into()),
                r(0xab, key(0xab).to_uppercase()),
                r(2, String::new()),
            ],
        )
        .await
        .unwrap();
        assert!(got.iter().all(Option::is_none));
        assert!(api.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn any_other_failure_stops_the_lookup() {
        for e in [
            ApiError::Unauthorised,
            ApiError::Forbidden {
                detail: "downloads are not allowed for this account".into(),
            },
            ApiError::network("down"),
            ApiError::RateLimited {
                retry_after_secs: 5,
            },
        ] {
            let api = Fake {
                fail: Some((2, e.clone())),
                ..Fake::default()
            };
            let got = resolve(&api, &[r(1, key(1)), r(2, key(2)), r(3, key(3))]).await;
            assert_eq!(got, Err(e));
        }
    }

    #[tokio::test]
    async fn too_many_tracks_are_refused_before_asking() {
        let api = Fake::default();
        let many: Vec<_> = (1..=MAX_RESOLVE as i64 + 1).map(|i| r(i, key(i))).collect();
        assert!(matches!(
            resolve(&api, &many).await,
            Err(ApiError::InvalidInput { .. })
        ));
        assert!(api.asked.lock().unwrap().is_empty());
        assert_eq!(resolve(&api, &[]).await.unwrap(), Vec::<Option<i64>>::new());
    }
}

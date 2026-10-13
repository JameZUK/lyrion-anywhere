//! Offline (milestone A5; SPEC D6, D14, §8.3–8.5): downloads into app-private storage, the
//! offline index, plays recorded while offline and sent back later, and smart downloads.
//!
//! - [`OfflineLibrary`]: what is on the device for one user on one server (no network);
//! - [`Downloader`]: the download manager (network), feeding the library;
//! - [`sync_plays`]: sends plays recorded offline to `POST /v1/plays`;
//! - [`plan_smart`]: which smart downloads to fetch or drop.
//!
//! Playing downloads is [`crate::OfflinePlayer`], through the same engine as streaming.

pub mod download;
pub mod index;
pub mod plays;
pub mod store;

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use crate::error::ApiError;
use crate::session::{Core, Session};
use crate::transport;

pub use download::{
    DownloadFailure, DownloadItem, DownloadObserver, DownloadPolicy, DownloadRequest,
    DownloadState, Downloader, DownloadsSnapshot, FormatChoice, NetworkKind,
};
pub use index::{DownloadFormat, DownloadOrigin, OfflineTrack};
pub use plays::{PlayRecord, PlayVerdict, PlaysSyncOutcome};
pub(crate) use store::scope_name;
pub use store::{OfflineAlbum, OfflineLibrary, remove_offline_data};

/// `POST /v1/plays` over the session (the real [`plays::PlaysApi`]).
struct HttpPlays {
    core: Arc<Core>,
}

/// Largest answer read.
const MAX_PLAYS_ANSWER: usize = 1024 * 1024;

impl plays::PlaysApi for HttpPlays {
    async fn send(&self, body: Value) -> Result<Value, plays::SendError> {
        use plays::SendError;
        let url = self.core.server().endpoint("v1/plays");
        let post = |token: &str| {
            self.core
                .http()
                .post(url.clone())
                .bearer_auth(token)
                .json(&body)
        };
        let token = self.core.access_token().await.map_err(SendError::Failed)?;
        let mut resp = post(&token).send().await.map_err(SendError::Failed)?;
        if resp.status().as_u16() == 401 {
            let fresh = self
                .core
                .renew(Some(&token))
                .await
                .map_err(SendError::Failed)?;
            resp = post(&fresh).send().await.map_err(SendError::Failed)?;
        }
        match resp.status().as_u16() {
            404 | 405 => Err(SendError::Unsupported),
            s if (200..300).contains(&s) => {
                let bytes = resp
                    .read_limited(MAX_PLAYS_ANSWER)
                    .await
                    .map_err(SendError::Failed)?;
                Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
            }
            _ => Err(SendError::Failed(
                transport::error_from_response(resp).await,
            )),
        }
    }
}

/// Send the plays recorded offline (SPEC D6). Safe to call often: plays are sent once
/// (each has an idempotency key), and kept when the server can't take them (offline, or an
/// older server without `/v1/plays`).
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_plays(
    session: Arc<Session>,
    library: Arc<OfflineLibrary>,
) -> Result<PlaysSyncOutcome, ApiError> {
    if !session.has_sign_in() {
        return Err(ApiError::Unauthorised);
    }
    let api = HttpPlays {
        core: session.core().clone(),
    };
    Ok(plays::flush(&api, || library.plays(), |keys| library.drop_plays(keys)).await)
}

/// What smart downloads should change.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmartPlan {
    /// Tracks to download (with their smart origin).
    pub download: Vec<DownloadRequest>,
    /// Smart downloads no longer wanted (stable keys), to remove.
    pub remove: Vec<String>,
}

/// Plan smart downloads: `wanted` is what the smart lists hold now (recently added albums'
/// tracks, favourites), by LMS track id. Smart downloads no longer wanted are removed (the
/// user's own downloads are never touched); wanted tracks not on the device are fetched.
/// The storage cap is the download manager's to enforce.
#[uniffi::export]
pub fn plan_smart(library: Arc<OfflineLibrary>, wanted: Vec<DownloadRequest>) -> SmartPlan {
    let have = library.tracks();
    let wanted_ids: HashSet<i64> = wanted.iter().map(|w| w.track_id).collect();
    let remove = have
        .iter()
        .filter(|t| t.origin.evictable() && !wanted_ids.contains(&t.track_id))
        .map(|t| t.key.clone())
        .collect();
    let have_ids: HashSet<i64> = have.iter().map(|t| t.track_id).collect();
    let mut seen = HashSet::new();
    let download = wanted
        .into_iter()
        .filter(|w| w.track_id > 0 && !have_ids.contains(&w.track_id) && seen.insert(w.track_id))
        .map(|w| DownloadRequest {
            origin: if w.origin == DownloadOrigin::Manual {
                DownloadOrigin::Recent
            } else {
                w.origin
            },
            ..w
        })
        .collect();
    SmartPlan { download, remove }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::index::tests::sample;

    #[test]
    fn smart_plans_never_touch_the_users_own() {
        let root = store::tests::temp_root();
        let lib = OfflineLibrary::open(root, "https://s".into(), "u".into()).unwrap();
        let mut manual = sample(1);
        manual.track_id = 1;
        let mut old_smart = sample(2);
        old_smart.track_id = 2;
        old_smart.origin = DownloadOrigin::Recent;
        let mut kept_smart = sample(3);
        kept_smart.track_id = 3;
        kept_smart.origin = DownloadOrigin::Favourite;
        for t in [&manual, &old_smart, &kept_smart] {
            std::fs::write(lib.final_path(&t.key, &t.ext), vec![0u8; t.size as usize]).unwrap();
            lib.add(t.clone()).unwrap();
        }
        let req = |id, origin| DownloadRequest {
            track_id: id,
            album_id: None,
            origin,
        };
        let plan = plan_smart(
            lib.clone(),
            vec![
                req(3, DownloadOrigin::Favourite),
                req(4, DownloadOrigin::Recent),
                req(4, DownloadOrigin::Recent),
                req(5, DownloadOrigin::Manual),
                req(-1, DownloadOrigin::Recent),
            ],
        );
        assert_eq!(plan.remove, vec![old_smart.key.clone()]);
        assert_eq!(
            plan.download,
            vec![
                req(4, DownloadOrigin::Recent),
                req(5, DownloadOrigin::Recent)
            ]
        );
        // Nothing wanted: every smart download goes, the user's own stays.
        let plan = plan_smart(lib, vec![]);
        assert_eq!(plan.remove.len(), 2);
        assert!(!plan.remove.contains(&manual.key));
    }
}

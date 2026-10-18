//! The Cloudflare API client and the *create* exchange (docs/plans/cloudflare-tunnel.md,
//! "The create exchange"): with an API token the admin enters, find the zone, create a
//! tunnel, point its ingress at the gateway, add the DNS record and fetch the tunnel token.
//!
//! The API token is used for this one exchange: it is held in memory (an [`ApiToken`], whose
//! `Debug` shows nothing) and dropped when the exchange ends, never written or logged. Every
//! request carries it as a bearer credential, has 30 s, and may answer at most 1 MiB. Any
//! failure after the tunnel was created deletes it again (best effort) before reporting.

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};

use super::scrub;
use super::token::{encode_base64, valid_tunnel_token};

/// Cloudflare's API.
pub const API_BASE: &str = "https://api.cloudflare.com/client/v4";
/// Longest one request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest answer read.
pub const MAX_BODY: usize = 1024 * 1024;
/// Pages of zones read at most (50 each).
pub const MAX_ZONE_PAGES: u64 = 5;

/// What the API token needs, as Cloudflare's dashboard names the permissions.
pub const SCOPES: &str =
    "Account › Cloudflare Tunnel › Edit, Zone › DNS › Edit and Zone › Zone › Read";

/// The exchange's steps, as the status's `step` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Zone,
    Create,
    Configure,
    Dns,
    Token,
}

impl Step {
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Zone => "zone",
            Step::Create => "create",
            Step::Configure => "configure",
            Step::Dns => "dns",
            Step::Token => "token",
        }
    }

    fn doing(self) -> &'static str {
        match self {
            Step::Zone => "finding the domain's zone",
            Step::Create => "creating the tunnel",
            Step::Configure => "pointing the tunnel at the gateway",
            Step::Dns => "adding the DNS record",
            Step::Token => "fetching the tunnel token",
        }
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Why a hostname was refused (a sentence for the admin).
pub fn check_hostname(raw: &str) -> Result<String, &'static str> {
    const SHAPE: &str = "The hostname must be a full domain name such as music.example.com, without https://, a port or a path.";
    let h = raw.to_ascii_lowercase();
    if !(4..=253).contains(&h.len()) || h.ends_with('.') {
        return Err(SHAPE);
    }
    let labels: Vec<&str> = h.split('.').collect();
    if labels.len() < 2 {
        return Err(SHAPE);
    }
    for l in &labels {
        let ok = (1..=63).contains(&l.len())
            && l.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !l.starts_with('-')
            && !l.ends_with('-');
        if !ok {
            return Err(SHAPE);
        }
    }
    // An IPv4 address (or anything ending in a numeric label, which no real domain does).
    if h.parse::<std::net::Ipv4Addr>().is_ok()
        || labels
            .last()
            .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("The hostname must be a domain name, not an IP address.");
    }
    for reserved in ["trycloudflare.com", "cfargotunnel.com"] {
        if h == reserved || h.ends_with(&format!(".{reserved}")) {
            return Err(
                "The hostname cannot be under trycloudflare.com or cfargotunnel.com: use a domain of your own on Cloudflare.",
            );
        }
    }
    Ok(h)
}

/// The API token's shape: 20–128 of `[A-Za-z0-9_-]`.
pub fn valid_api_token(t: &str) -> bool {
    (20..=128).contains(&t.len())
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A Cloudflare account, zone or DNS record id: 32 hex characters.
pub fn valid_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The tunnel's name: `lyrion-anywhere-` and the hostname with dots as dashes, at most 64
/// characters in all.
pub fn tunnel_name(hostname: &str) -> String {
    let mut name = format!("lyrion-anywhere-{}", hostname.replace('.', "-"));
    name.truncate(64);
    while name.ends_with('-') {
        name.pop();
    }
    name
}

/// The zone for `hostname`: the one whose name is its longest suffix (equal to it, or
/// preceded by a dot). Zones are `(name, id, account id)`.
pub fn choose_zone<'a>(
    hostname: &str,
    zones: &'a [(String, String, String)],
) -> Option<&'a (String, String, String)> {
    zones
        .iter()
        .filter(|(name, _, _)| {
            let name = name.as_str();
            !name.is_empty()
                && (hostname == name
                    || hostname
                        .strip_suffix(name)
                        .is_some_and(|rest| rest.ends_with('.')))
        })
        .max_by_key(|(name, _, _)| name.len())
}

/// The API token, held only for the exchange. Its `Debug` hides it.
pub struct ApiToken(String);

impl ApiToken {
    pub fn new(t: String) -> Self {
        ApiToken(t)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiToken([masked])")
    }
}

/// One exchange's inputs.
#[derive(Debug)]
pub struct Create {
    pub api_token: ApiToken,
    pub hostname: String,
    pub replace_dns: bool,
    /// The ingress target, e.g. `http://127.0.0.1:9443`.
    pub service: String,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// A failed request: Cloudflare's first error message and code, and the HTTP status.
#[derive(Debug, Clone)]
struct ApiError {
    status: Option<u16>,
    code: Option<i64>,
    message: String,
}

impl ApiError {
    fn missing_permission(&self) -> bool {
        self.status == Some(403) || matches!(self.code, Some(10000) | Some(9109))
    }
}

/// Untrusted text from Cloudflare, made fit to show: no control characters, at most 300
/// characters, scrubbed.
fn clean(s: &str, secrets: &[&str]) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).take(300).collect();
    scrub::scrub_with(s.trim(), secrets)
}

struct Api<'a> {
    client: &'a reqwest::Client,
    base: &'a str,
    token: &'a ApiToken,
}

impl Api<'_> {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, ApiError> {
        let secrets = [self.token.expose()];
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.token.expose())
            .timeout(REQUEST_TIMEOUT);
        if let Some(b) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(b.to_string());
        }
        let resp = req.send().await.map_err(|e| ApiError {
            status: None,
            code: None,
            message: format!(
                "could not reach Cloudflare's API ({})",
                clean(&e.without_url().to_string(), &secrets)
            ),
        })?;
        let status = resp.status();
        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError {
                status: Some(status.as_u16()),
                code: None,
                message: format!(
                    "Cloudflare's answer was cut short ({})",
                    clean(&e.without_url().to_string(), &secrets)
                ),
            })?;
            if bytes.len() + chunk.len() > MAX_BODY {
                return Err(ApiError {
                    status: Some(status.as_u16()),
                    code: None,
                    message: "Cloudflare's answer was too large".into(),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let success = v.get("success").and_then(Value::as_bool) == Some(true);
        if status.is_success() && success {
            return Ok(v);
        }
        let first = v.get("errors").and_then(|e| e.get(0));
        let message = first
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(|m| clean(m, &secrets))
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("Cloudflare answered {}", status.as_u16()));
        Err(ApiError {
            status: Some(status.as_u16()),
            code: first.and_then(|e| e.get("code")).and_then(Value::as_i64),
            message,
        })
    }
}

/// The admin-facing sentence for a step that failed.
fn step_failed(step: Step, e: &ApiError) -> String {
    let mut s = format!(
        "The \"{}\" step ({}) failed: {}",
        step.as_str(),
        step.doing(),
        e.message.trim_end_matches('.')
    );
    if e.missing_permission() {
        s.push_str(&format!(
            "; the API token is missing a permission; it needs {SCOPES}"
        ));
    }
    s.push('.');
    s
}

fn bad_answer(step: Step, what: &str) -> String {
    format!(
        "The \"{}\" step ({}) failed: Cloudflare's answer had no usable {what}.",
        step.as_str(),
        step.doing()
    )
}

/// What the exchange produced: only the tunnel token (and the hostname it serves) is kept.
#[derive(Debug)]
pub struct Created {
    pub tunnel_token: String,
}

/// Run the exchange. `on_step` is told each step as it starts. On failure the error is a
/// sentence naming the step, and a tunnel created on the way has been deleted again.
pub async fn create(
    client: &reqwest::Client,
    base: &str,
    req: &Create,
    on_step: impl Fn(Step),
) -> Result<Created, String> {
    let api = Api {
        client,
        base,
        token: &req.api_token,
    };
    let host = req.hostname.as_str();

    // 1. The zone (and its account).
    on_step(Step::Zone);
    let mut zones: Vec<(String, String, String)> = vec![];
    for page in 1..=MAX_ZONE_PAGES {
        let v = api
            .call(
                reqwest::Method::GET,
                &format!("/zones?per_page=50&page={page}"),
                None,
            )
            .await
            .map_err(|e| step_failed(Step::Zone, &e))?;
        let list = v.get("result").and_then(Value::as_array);
        for z in list.into_iter().flatten() {
            let name = z.get("name").and_then(Value::as_str).unwrap_or("");
            let id = z.get("id").and_then(Value::as_str).unwrap_or("");
            let account = z
                .pointer("/account/id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if valid_id(id) && valid_id(account) && check_hostname(name).is_ok() {
                zones.push((
                    name.to_ascii_lowercase(),
                    id.to_string(),
                    account.to_string(),
                ));
            }
        }
        let pages = v
            .pointer("/result_info/total_pages")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        if list.is_none_or(|l| l.is_empty()) || page >= pages {
            break;
        }
    }
    let Some((_, zone, account)) = choose_zone(host, &zones).cloned() else {
        return Err(format!(
            "no zone on this account contains {host} (the API token needs Zone › Zone › Read)"
        ));
    };

    // 2. The tunnel.
    on_step(Step::Create);
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|_| "The gateway could not make a tunnel secret.")?;
    let v = api
        .call(
            reqwest::Method::POST,
            &format!("/accounts/{account}/cfd_tunnel"),
            Some(json!({
                "name": tunnel_name(host),
                "config_src": "cloudflare",
                "tunnel_secret": encode_base64(&secret),
            })),
        )
        .await
        .map_err(|e| step_failed(Step::Create, &e))?;
    let Some(id) = v
        .pointer("/result/id")
        .and_then(Value::as_str)
        .filter(|id| scrub::is_uuid(id.as_bytes()))
        .map(str::to_ascii_lowercase)
    else {
        return Err(bad_answer(Step::Create, "tunnel id"));
    };

    // 3–5, undone on failure.
    match finish(&api, req, &zone, &account, &id, &on_step).await {
        Ok(c) => Ok(c),
        Err(e) => {
            delete_tunnel(&api, &account, &id).await;
            Err(e)
        }
    }
}

/// Delete a tunnel this exchange created (best effort).
async fn delete_tunnel(api: &Api<'_>, account: &str, id: &str) {
    if let Err(e) = api
        .call(
            reqwest::Method::DELETE,
            &format!("/accounts/{account}/cfd_tunnel/{id}?cascade=true"),
            None,
        )
        .await
    {
        tracing::warn!(error = %e.message, "could not delete the half-made Cloudflare tunnel");
    }
}

async fn finish(
    api: &Api<'_>,
    req: &Create,
    zone: &str,
    account: &str,
    id: &str,
    on_step: &impl Fn(Step),
) -> Result<Created, String> {
    let host = req.hostname.as_str();

    on_step(Step::Configure);
    api.call(
        reqwest::Method::PUT,
        &format!("/accounts/{account}/cfd_tunnel/{id}/configurations"),
        Some(json!({"config": {"ingress": [
            {"hostname": host, "service": req.service},
            {"service": "http_status:404"},
        ]}})),
    )
    .await
    .map_err(|e| step_failed(Step::Configure, &e))?;

    on_step(Step::Dns);
    let record = json!({
        "type": "CNAME",
        "name": host,
        "content": format!("{id}.cfargotunnel.com"),
        "proxied": true,
    });
    let existing = api
        .call(
            reqwest::Method::GET,
            &format!("/zones/{zone}/dns_records?name={host}"),
            None,
        )
        .await
        .map_err(|e| step_failed(Step::Dns, &e))?;
    let first = existing
        .get("result")
        .and_then(Value::as_array)
        .and_then(|r| r.first());
    match first {
        Some(_) if !req.replace_dns => {
            return Err(format!(
                "a DNS record for {host} already exists; tick Replace to overwrite it"
            ));
        }
        Some(rec) => {
            let Some(rid) = rec
                .get("id")
                .and_then(Value::as_str)
                .filter(|r| valid_id(r))
            else {
                return Err(bad_answer(Step::Dns, "DNS record id"));
            };
            api.call(
                reqwest::Method::PATCH,
                &format!("/zones/{zone}/dns_records/{rid}"),
                Some(record),
            )
            .await
            .map_err(|e| step_failed(Step::Dns, &e))?;
        }
        None => {
            api.call(
                reqwest::Method::POST,
                &format!("/zones/{zone}/dns_records"),
                Some(record),
            )
            .await
            .map_err(|e| step_failed(Step::Dns, &e))?;
        }
    }

    on_step(Step::Token);
    let v = api
        .call(
            reqwest::Method::GET,
            &format!("/accounts/{account}/cfd_tunnel/{id}/token"),
            None,
        )
        .await
        .map_err(|e| step_failed(Step::Token, &e))?;
    match v.get("result").and_then(Value::as_str) {
        Some(t) if valid_tunnel_token(t) => Ok(Created {
            tunnel_token: t.to_string(),
        }),
        _ => Err(bad_answer(Step::Token, "tunnel token")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_follow_the_rules() {
        for good in [
            "music.example.com",
            "MUSIC.Example.COM",
            "a.io",
            "x-1.y-2.example.co.uk",
            &format!("{}.example.com", "a".repeat(63)),
        ] {
            assert_eq!(
                check_hostname(good).as_deref(),
                Ok(good.to_ascii_lowercase().as_str()),
                "{good}"
            );
        }
        let long = format!("{}.com", vec!["a".repeat(60); 5].join("."));
        assert!(long.len() > 253);
        for bad in [
            "",
            "a.b",
            "localhost",
            "music",
            "music.example.com.",
            ".music.example.com",
            "music..example.com",
            "-music.example.com",
            "music-.example.com",
            "music.example.com:443",
            "music.example.com/path",
            "https://music.example.com",
            "music example.com",
            "music_x.example.com",
            "m\u{fc}sic.example.com",
            &format!("{}.example.com", "a".repeat(64)),
            &long,
            "10.0.0.10",
            "192.0.2.1",
            "music.example.123",
            "[::1]",
            "::1",
            "trycloudflare.com",
            "words-here.trycloudflare.com",
            "cfargotunnel.com",
            "abc.cfargotunnel.com",
            "x.ABC.TRYCLOUDFLARE.COM",
        ] {
            assert!(check_hostname(bad).is_err(), "{bad}");
        }
        // Look-alikes of the reserved names are ordinary domains.
        assert!(check_hostname("mytrycloudflare.com").is_ok());
        // Each refusal is a sentence.
        for bad in ["x", "10.0.0.10", "a.trycloudflare.com"] {
            let e = check_hostname(bad).unwrap_err();
            assert!(e.ends_with('.') && e.starts_with("The hostname"), "{e}");
        }
    }

    #[test]
    fn api_tokens_follow_the_rule() {
        for good in [
            "a".repeat(20),
            "A".repeat(128),
            "AbC_d-E0123456789xyzABCDEF0123456789abcd".into(),
        ] {
            assert!(valid_api_token(&good), "{good}");
        }
        for bad in [
            "a".repeat(19),
            "a".repeat(129),
            format!("{} ", "a".repeat(30)),
            format!("{}=", "a".repeat(30)),
            format!("{}+/", "a".repeat(30)),
            format!("Bearer {}", "a".repeat(30)),
            format!("{}\n", "a".repeat(30)),
            format!("{}\u{e9}", "a".repeat(30)),
            String::new(),
        ] {
            assert!(!valid_api_token(&bad), "{bad:?}");
        }
    }

    #[test]
    fn the_longest_matching_zone_wins() {
        let z = |n: &str, id: char| (n.to_string(), id.to_string().repeat(32), "a".repeat(32));
        let zones = vec![
            z("example.com", '1'),
            z("music.example.com", '2'),
            z("sic.example.com", '3'),
            z("other.org", '4'),
            z("com", '5'),
        ];
        let pick = |h: &str| choose_zone(h, &zones).map(|z| z.0.clone());
        assert_eq!(
            pick("music.example.com").as_deref(),
            Some("music.example.com")
        );
        assert_eq!(
            pick("a.music.example.com").as_deref(),
            Some("music.example.com")
        );
        assert_eq!(pick("listen.example.com").as_deref(), Some("example.com"));
        // A suffix that isn't on a label boundary doesn't count.
        assert_eq!(pick("basic.example.com").as_deref(), Some("example.com"));
        assert_eq!(pick("music.other.org").as_deref(), Some("other.org"));
        assert_eq!(pick("x.example.net"), None);
        assert_eq!(choose_zone("music.example.net", &zones[..4]), None);
        assert_eq!(pick("notexample.com").as_deref(), Some("com"));
        assert_eq!(choose_zone("notexample.com", &zones[..1]), None);
    }

    #[test]
    fn tunnel_names_are_derived_from_the_hostname() {
        assert_eq!(
            tunnel_name("music.example.com"),
            "lyrion-anywhere-music-example-com"
        );
        let long = format!("{}.example.com", "a".repeat(60));
        let name = tunnel_name(&long);
        assert_eq!(name.len(), 64);
        assert!(name.starts_with("lyrion-anywhere-aaaa"));
        // Never ends in a dash after cutting.
        let cut = format!("{}.b.example.com", "a".repeat(47));
        assert!(!tunnel_name(&cut).ends_with('-'));
        assert!(tunnel_name(&cut).len() <= 64);
    }

    #[test]
    fn ids_are_hex() {
        assert!(valid_id(&"0a".repeat(16)));
        assert!(!valid_id(&"0a".repeat(15)));
        assert!(!valid_id(&format!("{}g", "0".repeat(31))));
        assert!(!valid_id("../../zones"));
    }

    #[test]
    fn the_api_token_never_shows_in_debug() {
        let t = ApiToken::new("secret-api-token-value-123".into());
        let c = Create {
            api_token: t,
            hostname: "music.example.com".into(),
            replace_dns: false,
            service: "http://127.0.0.1:9443".into(),
        };
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("secret-api-token"), "{dbg}");
    }

    #[test]
    fn a_failed_step_is_named_and_a_missing_permission_explained() {
        let e = ApiError {
            status: Some(403),
            code: Some(10000),
            message: "Authentication error".into(),
        };
        let s = step_failed(Step::Create, &e);
        assert!(
            s.starts_with("The \"create\" step (creating the tunnel) failed: Authentication error"),
            "{s}"
        );
        assert!(s.contains(SCOPES), "{s}");
        let e = ApiError {
            status: Some(400),
            code: Some(1003),
            message: "Invalid tunnel name.".into(),
        };
        let s = step_failed(Step::Configure, &e);
        assert_eq!(
            s,
            "The \"configure\" step (pointing the tunnel at the gateway) failed: Invalid tunnel name."
        );
        for code in [10000, 9109] {
            let e = ApiError {
                status: Some(400),
                code: Some(code),
                message: "x".into(),
            };
            assert!(e.missing_permission());
        }
    }
}

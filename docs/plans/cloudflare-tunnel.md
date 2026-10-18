# Built-in Cloudflare Tunnel: agreed design

Agreed with the owner on 2026-10-18. Ships as plugin **0.6.0** (a feature, not a fix). This covers the gateway, the LMS plugin's settings page and the documentation; the web and Android apps are untouched.

## Why

Pointing a domain at home, forwarding port 443 and running a reverse proxy with a certificate is the hard part of the set-up, and the first forum testers side-stepped it with Tailscale or a Cloudflare Tunnel by hand. A tunnel needs no open port, no certificate and no proxy: `cloudflared` runs beside the gateway, dials out to Cloudflare, and Cloudflare serves `https://music.example.com` from its edge. Building it in makes the plugin alone a complete remote-access set-up for anyone with a domain on Cloudflare's free plan.

## Choices made

- **Opt-in, never on by default.** Nothing is downloaded or run until the admin presses *Install* on the plugin's settings page. Removing it stops `cloudflared` and deletes the binary and the tunnel token.
- **Where it is controlled:** a *Cloudflare Tunnel* section on the plugin's LMS settings page (an admin job done on the LAN). Status is also shown in the web app's admin area later, read-only.
- **Who does the work: the gateway.** It has the hardened HTTP client, runs as the LMS user, owns the 0700 data folder, and already supervises `ffmpeg`. The plugin's page only sends token-protected `anywhere tunnel …` commands and shows what the gateway reports. The same gateway endpoints serve Docker users from the command line.
- **Both ways to register:**
  1. *I have a tunnel token*: the admin creates the tunnel in Cloudflare's dashboard (*Networks › Tunnels › Create › cloudflared*), sets the public hostname to the gateway, copies the token and pastes it.
  2. *Create it for me*: the admin enters a Cloudflare API token, the account, the zone (domain) and the hostname wanted. The gateway creates the tunnel, sets its ingress to the gateway, adds the DNS record and fetches the tunnel token. **The API token is used for that one exchange and discarded; it is never stored or logged.**
- **Platforms:** Linux x86_64, aarch64 and armhf (the plugin's own three). Docker: a documented `cloudflared` sidecar, since the image is distroless and must stay so. Windows and macOS: not in scope (the plugin has no gateway there yet).
- **Cloudflare's terms:** audio streaming through their proxy is within the current terms for personal use, but it is their bandwidth and their rules; INSTALL says so in one sentence and links to the terms. "Cloudflare Tunnel" is their name, used descriptively; nothing implies a partnership.

## How it works

### Install

- The settings page's *Install* sends `anywhere tunnel install` (token-protected, like every `anywhere` command but `status`), which calls the gateway's loopback admin API.
- The gateway downloads the official `cloudflared` release asset for the host's architecture (`cloudflared-linux-amd64`, `-arm64`, `-armhf`) from GitHub, **at a version pinned in our release**, and checks it against the **SHA-256 pinned beside it** (the three sums are computed at our release time by `tools/build-release.sh` and shipped in the plugin; the gateway refuses anything else). No self-update: `cloudflared` moves only when a Lyrion Anywhere release moves it, and the page offers *Update* then. Size capped (64 MiB), fetched with the gateway's usual timeouts, written to a temporary file and renamed into place at `<data>/tunnel/cloudflared`, mode 0700.
- Outbound requirement: the LMS host must reach `github.com` (install) and Cloudflare's edge on 7844 or 443 (running). The page says what failed when it can't.

### Register

- **Tunnel token route:** the token is validated for shape (base64 JSON with `a`, `t`, `s`), written to `<data>/tunnel/token` (0600), and the tunnel starts.
- **API-token route** (`anywhere tunnel create`): with the entered API token the gateway, in order:
  1. `GET /zones?name=<domain>` to find the zone (and its account if the account wasn't given);
  2. `POST /accounts/{account}/cfd_tunnel` with a name like `lyrion-anywhere-<host>` and `config_src: cloudflare`;
  3. `PUT …/cfd_tunnel/{id}/configurations` with one ingress rule, `<hostname>` → the gateway's listen address, and the catch-all `http_status:404`;
  4. `POST /zones/{zone}/dns_records` for `<hostname>` as a proxied CNAME to `<id>.cfargotunnel.com` (an existing record for that name is reported, not overwritten, unless the admin ticks *Replace*);
  5. `GET …/cfd_tunnel/{id}/token`, stored as above.
  Required token scopes, shown on the page before the field: *Account › Cloudflare Tunnel › Edit*, *Zone › DNS › Edit*, and *Zone › Zone › Read*. A missing scope is reported by name from Cloudflare's error. Each step's failure says which step, and the gateway undoes a half-made tunnel (deletes the tunnel it created) rather than leaving one dangling.
- **Hostname and inputs** are validated strictly (a DNS label set under the zone, no path, no port; account and zone ids as hex), as every other admin input is.
- **The gateway's own settings follow:** `client_ip_header` becomes `CF-Connecting-IP`, `public_url` becomes `https://<hostname>`, and the tunnel's source (the host itself, on loopback or on the listen address) is trusted for that header. If the gateway listens on a LAN address rather than loopback, the ingress targets that address and the host's own addresses are trusted; the page says so.

### Run

- The gateway supervises `cloudflared tunnel run --no-autoupdate --metrics 127.0.0.1:<port> --loglevel info`, with the token in the child's environment (`TUNNEL_TOKEN`), never on its command line; a fixed argument list, no shell, otherwise empty environment; stdout and stderr captured to a scrubbed ring buffer (last 200 lines, tokens and ids masked); restart with the gateway's usual backoff; stopped when the gateway stops; killed if it misbehaves.
- The metrics port is chosen from a small loopback range and told to the page.

### Status and logs

- `anywhere tunnel status` (token-protected) reports: installed version and the version our release pins (so *Update* can be offered), state (`not installed`, `installed`, `starting`, `connected` with the edge-connection count from the readiness endpoint, `reconnecting`, `stopped`, `failed` with the reason), the hostname, uptime, and the last 50 log lines. The page polls it every few seconds while open, as it does the gateway's status.
- *Start*, *Stop*, *Update*, *Remove* buttons; *Remove* asks for confirmation and says it will delete the token. *Copy log* copies the scrubbed lines.

### Try it first: a temporary test tunnel (no account, no key)

Agreed 2026-10-18 as an addition. Cloudflare's *quick tunnel* needs no sign-up: `cloudflared tunnel --url <the gateway>` alone gives a random `https://<words>.trycloudflare.com` address for as long as the process runs, with no guarantee of uptime and a new address each time. The plugin offers it as **Try a temporary tunnel** beside the real set-up:

- **What it does:** installs `cloudflared` if needed (the same pinned download), starts it with `tunnel --no-autoupdate --url http://<listen address> --metrics 127.0.0.1:<port>`, reads the `trycloudflare.com` address from its output, and shows it on the page as a link and as a QR code (the page's existing QR encoder), so a phone can open it at once. While it runs the gateway's public URL, allowed origin and client-address header follow that address; when it stops they revert.
- **A test, and it says so:** a notice above the button and on the running status: *Test only. The address is random, Cloudflare gives no guarantee for it, and this plugin stops it after one hour. For everyday use, register a tunnel below.* The gateway stops it itself after **60 minutes** (a countdown on the page), on *Stop*, when the gateway stops, or when a registered tunnel is started. Nothing is stored for it: no token, no address.
- **Security:** sign-in and everything else apply as on any public address, since it is one. The page makes clear the address is reachable by anyone who has it until it stops.
- **Tests:** the fake `cloudflared` prints a `trycloudflare.com` line and the status shows it; the hour cap stops it (clock injected); the gateway's public URL reverts on stop; the address never lands in the data folder; a registered tunnel starting stops the test one.

## Trust boundary and secrets

- The tunnel token is the only secret kept: `<data>/tunnel/token`, 0600, under the 0700 data folder, as the admin secret is. Shown nowhere; the log scrubber masks anything that looks like one.
- The API token travels once from the admin's browser to LMS's settings page (over the LAN, as every LMS setting does) and from the plugin to the gateway over loopback with the admin secret; the gateway holds it in memory for the registration exchange only.
- `cloudflared` connects outbound only; it opens no listening port except the loopback metrics port.
- The gateway's listen address stays loopback by default; nothing about the tunnel exposes it to the LAN.

## Documentation

- `docs/INSTALL.md`: a new **Option C: built-in Cloudflare Tunnel** beside the reverse-proxy and Docker options, with the two registration routes, the API-token scopes, the terms sentence, and troubleshooting (can't download, can't connect, DNS record exists, hostname not in the zone).
- `docs/SECURITY.md`, `docs/PROTOCOL.md` (the admin endpoints), `docs/DEPLOY.md` (the Docker sidecar), `README.md` (one line in the set-up list), `SPEC.md` (decision D27, the trust boundary; D26 was already taken), `docs/FEATURES.md`, `docs/PLATFORMS.md`.

## Tests

- **Gateway, unit:** the architecture-to-asset mapping; the download verifier refuses a wrong SHA-256, an oversize body, a redirect to another host; token shape validation; input validation for hostnames, ids and the API token; the log scrubber masks tokens and ids; the state machine for every transition.
- **Gateway, integration (fake Cloudflare API and a fake `cloudflared` script, as the download tests use a fake `ffmpeg`):** the full *create* exchange succeeds and stores only the tunnel token; each failing step is reported by name and a half-made tunnel is deleted; a missing scope is named; an existing DNS record is reported and not overwritten without *Replace*; the supervisor starts the fake, reads its readiness endpoint, restarts it after a crash with backoff, stops it on *Stop* and on gateway shutdown, and never puts the token on the command line (asserted from the fake's view of its arguments); *Remove* deletes binary and token; the admin endpoints need the admin secret.
- **Plugin (`plugin/t/`):** every `anywhere tunnel …` command needs the token; the settings page's JS sends the token, polls status, and shows each state; no secret is echoed into the page.
- **Release (`tools/test-release.sh`):** the pinned `cloudflared` version and sums are present and the sums are 64 hex characters.
- **Live, opt-in:** against the owner's own Cloudflare account with a throwaway hostname, once, before release.

## Effort and order

1. Gateway: download, verify, supervise, status, logs, admin endpoints (with tests). About a day.
2. Gateway: the Cloudflare API client and the *create* exchange (with the fake API). About a day.
3. Plugin: settings-page section, commands, JS, tests. Half a day.
4. Docs, release notes, the live check. Half a day.

## Build contract (fixed 2026-10-18, before the build)

The gateway, the plugin and the docs are built against this. Changes go here first.

### Where things live

- Gateway module `gateway/src/cftunnel/` (`tunnel.rs` is the player socket; the name is taken): `mod.rs` (state, supervisor, status, the settings that follow), `download.rs` (pinned fetch and verify), `pins.rs` (the pinned `cloudflared` version and three SHA-256 sums, **compile-time constants**, rewritten by `tools/update-cloudflared-pin.sh <version>`, which downloads the three assets and sums them), `api.rs` (Cloudflare API client and the *create* exchange), `token.rs` (tunnel-token shape), `scrub.rs` (log scrubber). The pins are compiled in rather than shipped as a file beside the plugin so Docker users get them too; `tools/test-release.sh` checks them.
- Data: `<data>/tunnel/` (0700): `cloudflared` (0700), `token` (0600), `state.json` (hostname, installed version, whether the registered tunnel should run at start-up), `pid` (the child's pid, so a stale `cloudflared` left by a killed gateway can be stopped at the next start, after `/proc/<pid>/exe` is checked to be our binary).
- Assets: `https://github.com/cloudflare/cloudflared/releases/download/<version>/cloudflared-linux-{amd64,arm64,armhf}` (x86_64, aarch64, armhf). The download follows at most two redirects, each only to a host ending in `.githubusercontent.com` or `github.com`, over HTTPS, at most 64 MiB, 10 min, verified against the pin before the temporary file is renamed into place. `cloudflared --version` is then run (10 s limit) to confirm it executes; its output gives the installed version.
- `cloudflared` runs with a fixed argument list, no shell, an environment of only `TUNNEL_TOKEN` (registered tunnels), `HOME=<data>/tunnel` and `PATH=/usr/bin:/bin`, stdin closed, stdout and stderr piped into the log ring: registered `tunnel --no-autoupdate --metrics 127.0.0.1:<port> --loglevel info run`; temporary `tunnel --no-autoupdate --metrics 127.0.0.1:<port> --loglevel info --url http://<listen address>`. The metrics port is a free loopback port the gateway picks. Readiness: `GET http://127.0.0.1:<port>/ready` (200 with `readyConnections` ≥ 1 means connected, 503 otherwise), polled every 5 s; the temporary address from `GET /quicktunnel` (`{"hostname": "<words>.trycloudflare.com"}`), falling back to the `https://….trycloudflare.com` line in the output. Restart backoff after an exit: 2, 4, 8 … 60 s, reset after 10 min up. The gateway stops the child on `stop`, on SIGTERM and SIGINT (the gateway now shuts down on SIGTERM as well, which is what the plugin sends), and the temporary tunnel 60 min after it started.
- TLS: the gateway's `reqwest` gains rustls with the `ring` provider and the bundled web roots, the way `mobile/` does it (`cargo deny` already allows the licences). The release Docker build (musl, zig) must still pass.

### Admin API (loopback, admin secret, as every `/admin/v1/…` route)

Errors are `{"error": "…"}` with 400 (bad input), 409 (wrong state, e.g. `install` while running, `start` with nothing registered), 500 (the gateway could not write its data).

| Route | Body | Answer |
|---|---|---|
| `GET /admin/v1/tunnel` | | 200, the status below |
| `GET /admin/v1/tunnel/log` | | 200 `{"log": [up to 200 scrubbed lines, oldest first]}` |
| `POST /admin/v1/tunnel/install` | | 202 `{}` (download started; 204 if the pinned version is already installed) |
| `POST /admin/v1/tunnel/update` | | 202 `{}` (stops a running tunnel, downloads the pinned version, starts it again if it was running) |
| `POST /admin/v1/tunnel/register` | `{"token": "…", "hostname": "music.example.com"}` | 204; the token and hostname are stored and the tunnel starts (installing first if needed, as `quick` does) |
| `POST /admin/v1/tunnel/create` | `{"api_token": "…", "hostname": "music.example.com", "replace_dns": false}` (`replace_dns` optional, false when absent) | 202 `{}`; the exchange runs in the background (`state: "creating"`, `step` names the step); success stores token and hostname and starts; failure is `failed` with `error` naming the step and Cloudflare's message, and a tunnel this gateway created is deleted again |
| `POST /admin/v1/tunnel/start` | | 204 (the registered tunnel; 409 if none) |
| `POST /admin/v1/tunnel/stop` | | 204 (registered or temporary; the registered one then stays stopped across restarts) |
| `POST /admin/v1/tunnel/quick` | | 202 `{}` (installs if needed, then starts the temporary tunnel; 409 while a registered tunnel runs) |
| `POST /admin/v1/tunnel/remove` | | 204 (stops, deletes everything under `<data>/tunnel/`) |

Status (`GET /admin/v1/tunnel`), every key always present:

```json
{
  "supported": true,
  "pinned_version": "2026.10.1",
  "installed_version": "2026.10.1",
  "update_available": false,
  "mode": "registered",
  "state": "connected",
  "hostname": "music.example.com",
  "public_url": "https://music.example.com",
  "connections": 4,
  "uptime": 3600,
  "quick_ends_in": null,
  "progress": null,
  "step": null,
  "error": null,
  "log": ["…up to 50 scrubbed lines, oldest first…"]
}
```

- `supported`: false when the host is not Linux x86_64/aarch64/armhf (then `state` is `"unsupported"` and every POST answers 409).
- `mode`: `"none"`, `"registered"` or `"quick"`.
- `state`: `"unsupported"`, `"not_installed"`, `"installing"` (with `progress: {"bytes": n, "total": n or null}`), `"installed"` (nothing registered, or registered and stopped: then `hostname` is set), `"creating"` (with `step`: `"zone"`, `"create"`, `"configure"`, `"dns"`, `"token"`), `"starting"`, `"connected"`, `"reconnecting"` (running, no edge connection), `"stopped"` (registered, stopped by the admin), `"failed"` (with `error`; the supervisor keeps retrying a crashed child, `failed` is for the download, the exchange, or a child that could not be started at all).
- `public_url`: `https://<hostname>` while a registered tunnel is registered, `https://<words>.trycloudflare.com` while the temporary one runs, else null. The settings page uses it for pairing QR codes while it is set.
- `quick_ends_in`: seconds left of the hour, temporary tunnel only.
- `uptime`: seconds since the child started, else null.

### Inputs

- `hostname`: lower-cased, 4–253 characters, two or more DNS labels of `[a-z0-9-]` (no leading or trailing `-`, 1–63 each), no trailing dot, no port, no path, not an IP address, not under `trycloudflare.com` or `cfargotunnel.com`.
- `token`: the tunnel token Cloudflare's dashboard shows: base64 (standard or URL alphabet, padding optional) of a JSON object with `a` (the account id, 32 hex characters), `t` (the tunnel id, a dashed UUID of 36 characters) and `s` (base64, 32–128 characters); 64–4096 characters in all. (Corrected 2026-10-18: `t` was first written as 32 hex.) Nothing else is accepted and the decoded fields are not stored separately.
- `api_token`: 20–128 characters of `[A-Za-z0-9_-]`. Held in memory for the exchange only, never written or logged (the scrubber masks it should it appear in a line).
- `replace_dns`: boolean; when false an existing DNS record for the hostname is reported (`error: "a DNS record for music.example.com already exists; tick Replace to overwrite it"`) and the created tunnel is deleted again.

### The *create* exchange (`api.rs`, base URL `https://api.cloudflare.com/client/v4`, overridable for tests)

1. `zone`: `GET /zones?per_page=50` (pages followed, at most 5): the zone whose name is the longest suffix of the hostname (equal to it, or preceded by a dot). None: `failed`, "no zone on this account contains music.example.com (the API token needs Zone › Zone › Read)". The account id comes from the zone's `account.id`.
2. `create`: `POST /accounts/{account}/cfd_tunnel` `{"name": "lyrion-anywhere-<hostname with dots as dashes, at most 64 characters>", "config_src": "cloudflare", "tunnel_secret": "<32 random bytes, base64>"}`. The tunnel id (a UUID) comes from `result.id`.
3. `configure`: `PUT /accounts/{account}/cfd_tunnel/{id}/configurations` `{"config": {"ingress": [{"hostname": "<hostname>", "service": "http://<listen address>"}, {"service": "http_status:404"}]}}`.
4. `dns`: `GET /zones/{zone}/dns_records?name=<hostname>`; an existing record is reported as above unless `replace_dns`, in which case it is `PATCH`ed (type CNAME, content `<id>.cfargotunnel.com`, proxied true) and otherwise `POST /zones/{zone}/dns_records` with the same fields.
5. `token`: `GET /accounts/{account}/cfd_tunnel/{id}/token`; `result` is the tunnel token, validated as above and stored.

Every request carries `Authorization: Bearer <api token>`, 30 s each, bodies at most 1 MiB. A non-success answer reports the step and the first `errors[].message` (a 403 or code 10000/9109 adds "the API token is missing a permission; it needs Account › Cloudflare Tunnel › Edit, Zone › DNS › Edit and Zone › Zone › Read"). Any failure after step 2 deletes the tunnel (`DELETE /accounts/{account}/cfd_tunnel/{id}?cascade=true`, best effort) before reporting.

### The gateway's settings follow the tunnel

While a tunnel runs (registered: from `start`, including the automatic start; temporary: once its address is known) the gateway adds `https://<hostname>` to the accepted origins and reads the client address from `CF-Connecting-IP` on requests from the tunnel's source: loopback is already trusted; a non-loopback listen address is trusted as a proxy for the duration. They revert when it stops. The plugin's own *Public URL* and *Client address header* prefs are untouched; the page says they are overridden while the tunnel runs.

### Plugin commands (`Commands.pm`, `Admin.pm`)

All need the page's token (`csrf:`), `tunnel_status` and `tunnel_log` included: `tunnel_status`, `tunnel_log`, `tunnel_install`, `tunnel_update`, `tunnel_register token:… hostname:…`, `tunnel_create api_token:… hostname:… replace_dns:0|1`, `tunnel_start`, `tunnel_stop`, `tunnel_quick`, `tunnel_remove`. The plugin validates the inputs with the rules above before sending, never logs `token` or `api_token`, and never puts them in a result. The page's *Cloudflare Tunnel* section polls `tunnel_status` every 3 s while the page is open, shows each state in words, the log (last lines, *Copy log*), the buttons for the state (*Install*, *Update* when `update_available`, *Start*, *Stop*, *Remove* with confirmation), the two registration forms (token; API token with the three scopes listed above the field), and *Try a temporary tunnel* with its warning, countdown, link and QR code.

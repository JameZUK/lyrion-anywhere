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
- `docs/SECURITY.md`, `docs/PROTOCOL.md` (the admin endpoints), `docs/DEPLOY.md` (the Docker sidecar), `README.md` (one line in the set-up list), `SPEC.md` (decision D26, the trust boundary), `docs/FEATURES.md`, `docs/PLATFORMS.md`.

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

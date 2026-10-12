# Lyrion Anywhere — Specification

_Draft · 2026-10-10_

Lyrion Anywhere gives secure internet access to a home Lyrion Music Server (LMS) through one authenticated HTTPS/WebSocket endpoint that sits safely behind an ordinary reverse proxy. LMS itself is never exposed.

**v1 includes:**
- an LMS plugin;
- a gateway, a small Rust binary the plugin runs;
- a feature-rich web app (an installable PWA) with a built-in browser player and offline downloads.

Native Android and iOS apps come after v1 and use the same documented protocol.

---

## 1. Decisions

| # | Decision | Choice |
|---|---|---|
| D1 | How a remote device plays audio | **Real Slimproto player.** The device appears in LMS as a normal player (gapless, sync groups, full LMS behaviour). |
| D2 | Server build | **Perl LMS plugin + Rust gateway sidecar.** The gateway reuses the core's fuzzed Slimproto parser. Rust gives compile-time data-race safety and no garbage collector (low RAM on small LMS hosts). Changed from Go on 2026-10-10. |
| D3 | Client stack | **Shared Rust core.** Compiled to WebAssembly for the web app, with UniFFI bindings for the later native apps. |
| D4 | Web interface | **Full-featured web app is a v1 goal.** Native apps come later, on the same protocol. |
| D5 | Offline download format | **Chosen by the user per download:** original, or a transcode (Opus/MP3), with a default for each network type. |
| D6 | Offline playback | **Local player that syncs back.** Plays from cache when offline; play counts (and later ratings) are queued and sent to LMS on reconnect. Plays implemented 2026-10-11 (`POST /v1/plays`): a play counts once half the track or 240 s (whichever is less) was heard. |
| D7 | Accounts | **Admin creates all users.** No self sign-up. |
| D8 | Minimum LMS | **9.0+** |
| D9 | Web UI framework | **SvelteKit** (TypeScript, static build served by the gateway) |
| D10 | Transcoding for downloads | **System `ffmpeg`** if installed (not bundled). Without it, only Original is offered. |
| D11 | Licence | **GPL-2.0-or-later** for all components. Compatible with LMS (GPL-2.0) for the plugin; lets Rust binaries include Apache-2.0-only dependencies (e.g. `sync_wrapper` in the axum/tower stack), which are then distributed under GPLv3 terms. Changed from GPL-2.0-only on 2026-10-10. |
| D12 | Account database | **Owned by the gateway** (SQLite; argon2id passwords). The plugin manages accounts through the gateway's loopback-only admin API. Security-critical code stays in one memory-safe, tested component. Decided 2026-10-10. |
| D13 | Web app look | **Built-in theme system**, with a Material Design theme as the flagship, plus Classic (the original look) and High contrast; light/dark/automatic and an accent colour. Palettes are generated in OKLCH from the accent (Material's tonal roles), output as sRGB hex, and tested for WCAG contrast for every theme, mode and accent. LMS's own skins (e.g. Material Skin) are **not** proxied: that would expose LMS's web interface and full command API remotely, against the default-deny design. Clean-room: no skin code is copied. Decided 2026-10-10. |
| D14 | Native playback model | **The phone is a real LMS player when streaming** (Slimproto through the gateway: LMS drives queue, gapless, sync, plugins); **a local queue for downloaded music offline**, with plays synced back (D6). Both use the same Rust audio engine (bit-perfect output, levelling, waveform). Decided 2026-10-11. |
| D15 | Android minimum | **Android 10 (API 29).** Bit-perfect USB DAC output is enabled at runtime on Android 14+ where the device supports it. Decided 2026-10-11. |
| D16 | Native app distribution | **Free, through Google Play, F-Droid (reproducible builds) and signed APKs on GitHub releases.** Decided 2026-10-11. |
| D17 | iOS licensing | **GPL plus an App Store exception** for this project's own code, so the iOS app can be distributed through the App Store. The exception text is added to the licence before the iOS milestone; contributions from then on are accepted under it. Decided 2026-10-11. |
| D18 | Where track analysis runs | **On the gateway**, on demand (`GET /v1/analysis/track/{id}`, §5.7): decoded and analysed with the core, one track at a time, results cached on disk by the stable track key and file version. Open to any signed-in user (not tied to the download permission: the waveform is for anything you can play; decided 2026-10-11). On by default, with an off switch (`--no-analysis`, plugin setting); clients fall back to a plain seek bar and ReplayGain tags. Plan decision N5. Decided 2026-10-11. |
| D19 | Volume with a bit-perfect USB DAC | **Fixed at 100 %: the DAC's own volume sets the level.** While Android's bit-perfect mixer attributes are in force (Android 14+, a DAC that offers them), the phone's player ignores LMS's volume (`audg`), so no sample is scaled; LMS's volume applies again as soon as the route isn't bit-perfect. LMS's server-side ReplayGain and the app's own DSP still apply if turned on, and the signal path then says the route isn't bit-perfect. The engine carries samples as `f32`, which is exact up to 24 bits, so a 32-bit source is never claimed bit-perfect. Decided 2026-10-11 (milestone A2). |
| D20 | Player-first apps | **Each app is a player in its own right.** Web and Android default to playing on the device itself (a per-device setting can choose "Last used" or a specific player instead, e.g. a desktop beside a hi-fi). Controlling other LMS players is secondary: a small output icon opens "Play on…" (this device, the players the user may control, sync groups). While another player is controlled, the UI says so ("Playing on Kitchen", one tap back). **Transfer** moves the queue and position to another player ("Move playback here") or back ("Bring to this device"), with the same recipe on every app (docs/PROTOCOL.md › Transfer). Decided 2026-10-11 by the owner. |
| D21 | Refresh-token reuse grace | **Kept at 60 seconds for all clients.** A used refresh token presented again within 60 s, while none of its replacements has been used, gets another replacement (a lost response or two tabs refreshing at once); outside that, reuse revokes the device (docs/PROTOCOL.md › Authentication). A review (2026-10-11) suggested a few seconds for browser cookies; the owner chose to keep 60 s: tabs already take turns (navigator.locks), phones need slack for suspended or flaky connections, a thief needs the refresh token itself within the window, and whichever side uses its replacement first cuts the other off, revoking the device for both at the next refresh. Not a finding for future reviews. Decided 2026-10-11 by the owner. |
| D22 | Sync with LMS's other players | **Report what is heard; correct at the output.** Our players report elapsed time as frames *presented* (from the platform's output timestamp), extrapolated to each STAT's jiffies on the same monotonic clock. They start a synced group at the exact time asked, allowing for output latency, and apply LMS's pause and skip corrections at the output stage. The plugin leaves LMS's per-player sync settings at their defaults. See docs/PROTOCOL.md › Synchronised playback. Decided 2026-10-12. |

---

## 2. Goals and non-goals

### Goals (v1)

- **Real player:** a browser tab or installed PWA becomes a real LMS player, and also a full controller for every player the user is allowed to use.
- **Full-featured web UI:** library, search, apps/radio, queue, multi-player and sync control, all usable on desktop and phone (§7).
- **Offline downloads:** albums, playlists and tracks can be downloaded to the device and played with no connection (§8).
- **One endpoint:** everything goes over a single TLS endpoint (port 443) through any reverse proxy (Caddy, nginx, Traefik, Cloudflare Tunnel). No VPN needed.
- **Own authentication:** users, devices and revocable tokens that don't depend on LMS's password or HTTP server.
- **Default deny:** only allow-listed LMS commands and URLs are reachable from outside.
- **Survives mobile networks:** brief drops and Wi-Fi/cellular handovers don't remove the player from LMS.
- **Easy install:** from the LMS plugin repository, configured in LMS Settings. A Docker image is also provided.
- **One public API:** the web app uses only the documented public API (§6), so later native apps can do everything it does.

### Non-goals (v1)

- Native Android/iOS apps (post-v1, §10).
- Replacing LMS's own web UI or authentication on the LAN.
- Downloading tracks from streaming services (Qobuz, Tidal, Spotify, etc.) or radio. Only local library files can be downloaded.
- Running without TLS. The gateway refuses non-TLS requests from non-local addresses.
- A hosted multi-tenant relay service. Every install is self-hosted.

---

## 3. Architecture

```
 Internet                     Home network (LAN)
┌─────────────────┐          ┌────────────────────────────────────────────────────────┐
│ Web app (PWA)   │  HTTPS   │ ┌──────────────┐   ┌───────────────────┐  ┌──────────┐ │
│  UI + WASM core │──WSS────▶│ │ Reverse proxy│──▶│ Gateway (Rust)    │─▶│ LMS      │ │
│  player+offline │          │ │ Caddy/nginx  │   │ auth · tokens     │  │ 3483 TCP │ │
├─────────────────┤          │ │ TLS :443     │   │ Slimproto tunnel  │  │ 9000 HTTP│ │
│ Android / iOS   │          │ └──────────────┘   │ RPC allow-list    │  │ LAN-only │ │
│ (post-v1, same  │─────────▶│                    │ stream + download │  ├──────────┤ │
│  protocol)      │          │                    │ serves web app    │◀▶│ Anywhere │ │
└─────────────────┘          │                    └───────────────────┘  │ plugin   │ │
                             │                                           └──────────┘ │
                             └────────────────────────────────────────────────────────┘
```

- Clients only ever speak HTTPS/WSS to the proxy.
- The proxy forwards only to the gateway (`127.0.0.1:9443` by default).
- The gateway is the only thing that talks to LMS's Slimproto (3483), HTTP/JSON-RPC (9000) and CometD.
- The gateway talks to the plugin over a local, shared-secret internal API.
- LMS ports 3483, 9000 and 9090 are never forwarded.

**Why a sidecar (D2).** LMS is a single-threaded Perl event loop. Running WebSockets, password hashing, audio relay and transcoding inside it would slow the whole server. It would also mean bundling CPAN modules for every platform. A separate process keeps bugs in LMS's HTTP server (e.g. the 9.2.0 CVEs, §9) unreachable from outside. The UPnP/Cast bridge plugins already ship helper binaries the same way.

---

## 4. Plugin (`Plugins::LyrionAnywhere`)

**Built (0.1.0, 2026-10-10):** settings (enable, listen address, public URL, trusted proxies, development gateway path), user/role/two-step/grant/device/audit management, `anywhere …` LMS commands, gateway supervision with restart backoff. **Release (2026-10-10):** `tools/build-release.sh` builds the zip and `repo.xml`. It includes static gateways for x86_64/aarch64/armhf Linux, which the plugin checks against `Bin/SHA256SUMS` before running, and the browser player. **Still to come:** a cryptographic release signature (the checksums catch corruption and partial updates, not a malicious repository), macOS/Windows gateways, proxy self-test, device rename, streaming policy, player integration, packaging.

**Trust boundary.** Account management runs through LMS's own JSON-RPC/CLI (`anywhere …` commands) and its settings page. LMS's JSON-RPC (9000) and CLI (9090) are unauthenticated on the LAN by default, and `/jsonrpc.js` accepts cross-origin `text/plain` POSTs, so any web page a LAN user opens could send `anywhere …` commands blind. Every admin command therefore needs `csrf:<token>`: 128 random bits made when LMS starts, kept in memory only, and given only to the settings page, which other sites cannot read. That includes the read-only ones (users, devices with their last addresses and browsers, diagnostics, the audit log), since 2026-10; only `anywhere status` needs no token. Someone with direct LAN access to an open LMS can still read the page and so manage accounts, just as they can already control LMS; the settings page says to turn on LMS's password protection. Prefs get the same validation through LMS's `pref` command as through the page, and the development gateway path is honoured only when LMS runs with `ANYWHERE_ALLOW_GATEWAY_PATH=1`. Remote users can never reach these commands: the gateway's RPC allow-list reserves `anywhere`.

### Settings page (in LMS)

- Enable/disable, public URL, a proxy self-test, and a warning if LMS ports are reachable from the public URL.
- Users: create/disable/delete, role (`listener` / `admin`), permitted players, download permission and allowed formats. These are managed through the gateway's admin API (D12); the plugin stores no account data itself.
- Devices: list, rename and revoke. Device pairing via QR code (public URL + one-time code, valid 10 minutes), for the later native apps.
- Policy: default streaming quality (Wi-Fi/cellular), and the server-side transcode cache size.
- Audit log viewer.

### Gateway supervision

- Picks the right binary for the OS/architecture and verifies its release signature.
- Starts, stops, health-checks and restarts it.

### Gateway admin API and LMS-side helpers (127.0.0.1 only, shared secret generated at install)

The plugin calls the gateway's **admin API**, which listens on a separate loopback-only port that's never proxied, for account and device management. The gateway calls back into the plugin only for LMS-side work:

- User/device lookup, permissions and policy.
- Resolving a track ID to a library file. Only tracks in the LMS library are resolved; the gateway never sees a path a client supplied.
- Applying offline play events and ratings to LMS (§8.4).
- Reporting which track a gateway player is about to stream (used later for cache-served playback, §11).

### Player integration

For players that connect through the gateway, the plugin:

- turns off direct streaming, so LMS always proxies radio and service streams and the device never needs to reach the LAN;
- applies the user's transcoding/bitrate policy as player prefs;
- leaves LMS volume alone for "this device" players: the browser player applies it itself where it can, and holds it at 100% where it can't (§7.2, "Volume on this device"). _Changed 2026-10-11:_ previously planned as always fixed at 100%.

### Storage

- State lives in the LMS prefs directory.
- Account data (users, devices, tokens, audit log) lives in the gateway's SQLite database (D12), not in LMS prefs.

---

## 5. Gateway

A single static Rust binary (tokio/axum), sharing the Slimproto parser and stream-request policy with `core/` so the security filter and the clients can never disagree. It is run by the plugin or standalone (Docker), and listens on `127.0.0.1:9443` by default.

### 5.1 Slimproto tunnel (`WS /v1/player`)

1. The client opens the WebSocket and sends an access token as its first message. Unauthenticated sockets are closed after 5 s.
2. The gateway opens a dedicated TCP connection to `127.0.0.1:3483` for that session. Slimproto frames then pass through unchanged as binary WebSocket messages.
3. The gateway inspects and enforces these message types:
   - **Client frames:** only what a software player sends reaches LMS: HELO, STAT, RESP, META, DSCO, `BYE!`, and SETD only as the answer to an LMS name query. IR, BUTN, KNOB, BODY, UREQ, SHUT and anything else end the session as a policy violation.
   - **HELO:** the MAC must equal the one assigned to that device (a locally administered `02:…` address), and the device ID must be 12 (software player). Capabilities are reduced to well-formed `key=value`/codec tokens (printable ASCII, bounded length).
   - **SETD (name):** a name that looks like a URL or path, or is too long, is replaced by the device's name.
   - **strm:** when `server_ip` is 0 or LMS's own address, the client fetches audio via `GET /v1/stream/{session}`. The gateway replays the request to `127.0.0.1:9000` only if the path is `/stream.mp3?player=<that session's MAC>`.
   - **serv:** dropped, so a device can't be redirected to another server.
4. **Resume** (implemented; full detail in `docs/PROTOCOL.md`):
   - **Grace period:** if the socket drops, the gateway keeps the session and the LMS connection for a grace period (60 s by default). LMS never sees the player leave.
   - **Heartbeats:** while the client is away, the gateway reports status to LMS on the player's behalf every second. It replays the client's last STAT with the clock advanced. Without this, LMS drops a silent player after about 15 s, which the live dropout test showed. It does the same for a client whose socket is still open but has sent nothing for 8 s (a socket that died unnoticed), and clients give up on a socket that has brought nothing for 20–25 s and resume.
   - **Redelivery:** frames for the client are numbered and kept in a bounded log. On resume, the client says how many it received and everything after that is re-sent, which covers frames lost in flight.
   - **Stream resume:** downloads resume by byte offset. The client acknowledges received bytes, and the gateway never reads more than one replay window (4 MiB) beyond the last ack. A resume therefore always falls inside the window, and LMS isn't drained ahead of time.
   - **One session per device:** a new session for a device ends its previous one.
   - **Slow clients:** a socket whose outgoing queue fills is closed (1013) and handled like a dropped connection, so the client resumes.
   - **Stream lifetime:** ending a session stops its stream relay at once. A relay also ends after 60 s without data from LMS, or 60 s without an ack while the client's window is full.

### 5.2 Control (`POST /v1/rpc`, `WS /v1/events`)

`/v1/rpc` accepts LMS JSON-RPC requests. Each is checked against an allow-list of commands for the user's role, and against the players the user may address.

- **Listener:**
  - library browse/search (artists, albums, tracks, genres, years, composers, `musicfolder`);
  - favorites, and apps/radio menus (SlimBrowse);
  - `playlistcontrol` by library IDs, saved playlist CRUD;
  - transport, volume, sleep, `status`, `sync`/`unsync` among permitted players;
  - track info menus.
- **Admin (opt-in):** also rescan. Per-player settings are not exposed over the internet.
- **Player permissions (implemented):**
  - admins control every player;
  - listeners control their own signed-in devices plus home players an admin explicitly grants (least privilege, so an account given to a friend can't take over the house's speakers).
- **Always denied:**
  - `readdirectory`, `pref`, `restartserver`, `debug`/`logging`;
  - `serverstatus` fields beyond the permitted players and library totals (no server version, last scan, address or identity, and no players on other servers);
  - any `playlist play/add/insert` with a `file://` or non-library URL;
  - free text (search terms, playlist names, typed input) holding a path, a `file:` URL, a URL to a local host, a bare LAN address, or any URL other than http(s). Public http(s) URLs are allowed, since some apps search by feed URL.
- **Local hosts:** one check, shared by response filtering, request validation, events and the image proxy, decides what is "on the home network": private, loopback, link-local, CGNAT and benchmarking addresses in any spelling a URL parser or `inet_aton` accepts (`2130706433`, `0x7f000001`, `127.1`, IPv6 zone ids, IPv4-mapped IPv6), single-label names, local suffixes (`.lan`, `.local`, `.localhost`, `.home.arpa`, …), wildcard-DNS services that answer with an address taken from the name (`nip.io`, `sslip.io`, `localtest.me`, …), and names embedding a private IPv4 address as labels (`10.0.0.5.example`, `ip-10-0-0-5`).
- **Events:** the gateway subscribes to LMS locally (CometD) and forwards only events for permitted players. It pushes them over `WS /v1/events`.
- **Artwork:** `GET /v1/art/…` proxies only fixed patterns (`/music/{id}/cover…`, `/imageproxy/…`, plugin icons). An image-proxy URL must have been handed out by LMS in an image field, must not name a local host, and must not resolve (from the gateway, with a 2 s limit) to a local address. That last check can't stop DNS rebinding between the gateway's lookup and LMS's own, and a name that doesn't resolve is judged by its name alone.

### 5.3 Downloads (§8)

- **Original format:** after a library and permission check, the gateway streams the original file. It does this by proxying LMS's per-track download handler (`/music/{id}/download`, verified on 9.1.1: returns the original file with `Range`/206 support). Gotcha: a nonexistent track ID makes LMS hang rather than 404, so the gateway first checks that the ID is a positive integer and looks it up with LMS JSON-RPC (`titles 0 1 track_id:<id>`, with a time limit). Only a track LMS knows is ever fetched, with time limits on connecting, on the response headers and on every read.
- **Transcoded:** the gateway runs the system `ffmpeg` (D10; found on `PATH`, or given with `--ffmpeg` / the plugin's *ffmpeg path* setting) to Opus 160 kb/s (Ogg) or MP3 320 kb/s CBR, with title, artist, album and track number tags (and, for MP3, the file's embedded cover art). ffmpeg gets a fixed argument list with no shell, an empty environment, a demuxer forced from a fixed table of LMS file types, `-protocol_whitelist file`, files the gateway named in its own cache directory, an output size cap and a time limit. Results are kept in a size-capped LRU cache on disk (default 2 GiB, `--download-cache-mb`), so repeat downloads cost nothing.
- **Ranges and limits:** all download responses support a single HTTP `Range`, so interrupted downloads resume. Each user may run at most 3 downloads at once (429 beyond). Downloads are audited at most once per track and device an hour.
- **Retries:** LMS sometimes fails the next request right after clients abort large downloads. The track lookup and the download request are both plain reads, so one that fails at the connection level or with a 5xx is tried once more after a short pause, and every upstream failure is logged with its cause.

### 5.4 Web app hosting

- The gateway serves the built web app (static files) at `/`.
- It sets strict security headers on every response (refusals included): CSP with no inline script, `frame-ancestors 'none'`, `Referrer-Policy`, and a `Permissions-Policy` that turns off camera, microphone, geolocation, payment, USB and similar features (fullscreen and autoplay stay available to the app itself). HSTS is set via the proxy.
- API responses (`/v1/`, except artwork, which has its own policy) carry `Cache-Control: no-store`. Signing out sends `Clear-Site-Data: "cache"`; offline downloads (IndexedDB) are kept.

### 5.5 Deployment

- The gateway trusts `X-Forwarded-For`/`-Proto` only from configured proxy addresses. It rejects non-TLS requests from remote addresses.
- It reads `X-Forwarded-For` from the right, skipping trusted proxies, so the client address can't be chosen by the client even when a proxy appends to the header.
- Example Caddy config: `music.example.com { reverse_proxy 127.0.0.1:9443 }`.
- nginx is a first-class target and gets a tested example in the docs. The example:
  - passes `Upgrade`/`Connection` headers for the two WebSocket paths;
  - turns off `proxy_buffering` for `/v1/stream` and `/v1/download`;
  - sets long `proxy_read_timeout` values for WebSockets.
- **Behind Cloudflare:**
  - The gateway can trust `CF-Connecting-IP` when configured to (`--client-ip-header CF-Connecting-IP`, or the plugin's *Client address header* setting). It is honoured only on requests from a trusted proxy.
  - Cloudflare's proxy supports WebSockets but adds per-request latency. The resume buffer (§5.1) absorbs this.
  - Cloudflare's terms restrict serving large volumes of non-HTML media through its proxy. The docs will advise a DNS-only (unproxied) record for heavy streaming/download use.
- **Proxy on another host or VLAN:** the gateway listens on a LAN address instead of localhost, with an allow-list of proxy source addresses. The docs describe the firewall rule needed.
- Docker image: read-only root filesystem, non-root user, no capabilities.
- **Small hosts:** the gateway targets under 50 MB RSS when idle. Transcode concurrency defaults to 1 so `ffmpeg` can't starve LMS.

### 5.6 LMS integration notes (known gotchas)

- **Startup:** LMS can take over 2 minutes to open JSON-RPC after a restart. The gateway retries with backoff and reports "server starting" rather than failing.
- **Downloads:** never use `/stream.mp3?Track=<id>` for downloads. For a non-player client the transcoder streams with no length limit, which once turned a 23 MB track into 106 GiB. Use `/music/<id>/download`, which sets `Content-Length`.
- **Unknown IDs:** a nonexistent track ID on `/music/<id>/download` hangs rather than 404ing. The gateway always applies its own timeouts.
- **Track IDs are not stable:** LMS renumbers them on rescan. Anything stored long-term (offline downloads, pinned items, queued offline events) uses a stable key (§8.3).
- **`playlistcontrol`:** it applies asynchronously and silently drops unknown IDs. Clients must not assume an immediately consistent queue.
- **Plugin:**
  - It must never block the event loop. Use async HTTP and timers only.
  - A leftover backup copy of a plugin inside `Plugins/` silently breaks the real one. The docs warn about this.
  - Settings handlers trim and validate every submitted pref and reject CR/LF.
  - CLI commands invoked from menus without a player are registered with `needsClient=0`.

### 5.7 Track analysis (D18)

- **What:** `GET /v1/analysis/track/{id}` gives a library track's waveform (1,000 buckets of peak and RMS), EBU R128 loudness, loudness range, true and sample peak, leading and trailing silence and fade shapes, as JSON or a compact binary form (`docs/PROTOCOL.md`). The web app and the Android app draw the waveform as the seek bar; the phone's player levels loudness from it.
- **How:** the gateway fetches the original as downloads do (§5.3, same lookup, permission and time limits), saves it to a temporary file and decodes it with the core on a blocking thread (seekable, so MP4 files with the index at the end work), analysing in one pass and constant memory.
- **Limits:** one analysis at a time by default, a queue cap, per-user limits (429), maximum size (1 GiB) and duration (2 h) (413), a 5-minute deadline (504). A request waits up to 10 s, then gets 202 while the job carries on. Corrupt files are a clean 422, remembered for 10 minutes; decoder panics are caught.
- **Cache:** the binary form on disk (`<data-dir>/analysis-cache`, 64 MiB by default, LRU), named by the stable track key (§8.3), the file's size and modification time and the analysis version. Responses carry an ETag.

---

## 6. Anywhere protocol (public API v1)

This is the one contract used by the web app now and the native apps later. It is versioned under `/v1`, documented as OpenAPI (HTTP) plus a message spec (WebSockets), and covered by conformance tests.

| Endpoint | Purpose | Auth |
|---|---|---|
| `POST /v1/auth/login` | Username + password (+ TOTP) → session | Password |
| `POST /v1/auth/pair` | Pairing code → device tokens (native apps) | Pairing code |
| `POST /v1/auth/token` | Refresh token → 15-min access token | Refresh token |
| `POST /v1/auth/logout` | Revoke this device and end its sessions | Access token |
| `GET /v1/me` | User, role, permissions, assigned player MAC | Access token |
| `WS /v1/player` | Slimproto tunnel with resume and stream acks (§5.1, `docs/PROTOCOL.md`) | Access token; resume secret |
| `GET /v1/stream/{session}/{n}[?offset=k]` | Audio for the session's n-th stream; resumable | Bearer token |
| `POST /v1/rpc` | Filtered JSON-RPC (§5.2) | Access token |
| `WS /v1/events` | Filtered event push | Access token |
| `GET /v1/art/…` | Artwork | Access token or signed URL |
| `POST /v1/offline/manifest` | Albums/playlists/tracks → track list with sizes, versions, metadata | Access token |
| `GET /v1/download/track/{id}/info` | Track metadata, stable key and the formats this user may download | Access token + download permission |
| `GET /v1/download/track/{id}?format=` | One track file (`original`, `mp3` = MP3 320, `opus` = Opus 160), `Range` supported | Access token + download permission |
| `GET /v1/analysis/track/{id}[?format=json\|binary]` | Waveform, R128 loudness, peaks, silence and fades of a track (§5.7); 202 while it is being analysed | Access token + download permission |
| `POST /v1/plays` | Batched offline plays (idempotent by a client key per play); ratings later (D6) | Access token |
| `GET /v1/account/devices`, `DELETE …/{id}` | User's own sessions/devices | Access token |
| `GET /healthz` | Liveness | None |

Rules:
- The web app may not use any private endpoint.
- New capabilities are added to the protocol first, then to clients.

---

## 7. Web app (v1)

An installable PWA served by the gateway, responsive from phone to desktop. It is both a player and a full controller.

### 7.1 Stack

- SvelteKit + TypeScript (D9), built with `adapter-static` and served by the gateway. No server-side rendering; all data comes from the `/v1` API. Hash routing, so the gateway serves plain files only.
- **Strict CSP kept:**
  - the build moves SvelteKit's inline start-up script into a file, and fails if any inline script, event-handler attribute or `javascript:` URL remains;
  - the gateway allows inline `style` attributes only (`style-src-attr 'unsafe-inline'`), which the framework needs. Style sheets and `<style>` elements stay `'self'`.
- The Rust core is compiled to WebAssembly. It handles Slimproto, buffering, resume, and decoding with Symphonia (FLAC, ALAC, MP3, AAC-LC, Vorbis, WAV/AIFF) plus Opus.
- Audio output goes through an `AudioWorklet`, or, on iOS and Android, an `<audio>` element fed lossless fMP4 segments through (Managed) Media Source, about 8 s ahead, which keeps playing in the background.
- The Media Session API gives lock-screen/notification controls and hardware media keys.
- A service worker serves the app shell offline. Offline audio and its metadata (and album covers) are stored in IndexedDB, partitioned per user, never in the service worker's caches. _Changed 2026-10-10 from OPFS:_ Safari 17 can't write OPFS files from the page, and the service worker deletes old caches on every update, so IndexedDB Blobs (stored in ~1 MiB pieces, so downloads resume) are the one store that works on every target.
- **Sign-out holds:** token renewals or restores started before a sign-out are discarded. If the logout call can't reach the gateway, a flag in local storage stops the next page load from restoring the session from the refresh cookie; that load shows the sign-in form and finishes the logout (refresh, then logout) in the background. A successful sign-in clears the flag.

### 7.2 Features

**Playback and players**
- "This browser" as an LMS player: gapless, seek, and survives network drops via resume.
- Player switcher: this browser plus every permitted home player. Sync/unsync groups (`sync <player>` for the player that joins, `sync -` to leave); the player button names the group ("Lounge + Study"). Only permitted players are listed or targeted.
- Per-player volume, mute, power, and sleep timer.
- **Volume on this device** (_decided 2026-10-11_): automatic, with a setting to override, kept per browser.
  - **Automatic (default):** where the browser can set its output volume, LMS's volume is applied on the device, so any controller (this app, Material Skin, another phone) changes what is heard. LMS sends linear gains (`audg`, already on LMS's volume curve); they are applied as given, after the queued audio, so a change is heard at once. Web Audio uses a gain in the AudioWorklet, the media element its `volume`. Changes ramp over 30 ms, so they don't click.
  - **Where it can't** (iPhone/iPad: a media element's `volume` is read-only and reads 1; found by a feature test, not the user agent), the device plays at full scale and its own buttons set the level. LMS's volume is held at 100% for this player: if a controller moves it, the device sets it back (at most every 3 s; a mute is left alone). Now playing and the player bar show "Use this device's volume buttons" instead of the slider.
  - **Override in Settings:** Automatic / Follow LMS volume / Device buttons only. "Follow LMS volume" is shown disabled, with the reason, where it can't work.
- **Seek:** LMS sends a flush and the new stream back to back (about 2 ms apart for MP3). The player drops its queued audio on the flush itself, counted by the core, not on a timer that may never see the stopped state in between. In media-element mode the play head is then moved to where the new audio starts, so the browser also drops audio it had already decoded. After a seek, or from stopped, playback starts once 0.5 s is decoded if the stream is arriving at least twice as fast as it plays (a library file, not live radio), and the 8 s queue fills while playing. The connection log records each seek's timing (new stream, first bytes, started, queued, heard).
- Transfer playback between players (move the queue and position).

**Now playing and queue**
- Large art view, track info (format, bitrate, source) and progress.
- Queue: drag-to-reorder (from a handle; Alt+Up/Down and move buttons for keyboard and screen readers), swipe left or tap to remove with Undo (re-added by track id, then moved back), clear, and save as playlist. Shuffle/repeat. Edits show at once, are sent in order, and are reverted if LMS refuses them; status refreshes are held back mid-gesture.
- "Play next", "Add to queue" and "Play now" everywhere.

**Library and discovery**
- Artists, album artists, composers, albums (grid/list, sort, filter), tracks, genres, years, folders, new music, random mix.
- LMS "My Music" parity: New Music (in the sidebar), Albums with sorting, Artists by role (album artists by default, all artists, and composers, conductors and bands where the library has them), Genres (albums and artists), Years, Compilations, Random Albums (shuffle again), and the Music Folder (browsed and played by folder id only; paths never reach the app).
- Virtual libraries: when LMS reports any, the Library page offers a picker; the choice narrows the browse lists and is remembered per user on the device.
- Global search with instant results grouped by type.
- Saved playlists: create, edit, reorder, delete.
- Favorites, including adding to and removing from them.
- Apps and radio: LMS SlimBrowse menus are rendered generically, so installed LMS plugins work without web-app changes.
- Track/album/artist info menus from LMS plugins (bios, reviews, lyrics where available).

**Offline (§8)**
- Download buttons on track/album/playlist, a downloads page with progress, and storage usage.
- Offline mode with a local queue and browse of downloaded content.

**Account and admin**
- Change password, enable TOTP, and list/revoke own devices and sessions.
- Admins can manage users, devices and the audit log (the same functions as the LMS settings page).

**Quality of life**
- Light/dark/system theme and keyboard shortcuts.
- Accessible (WCAG 2.2 AA target) and keyboard-navigable.
- Streaming quality per network type, where the browser exposes it (Network Information API); otherwise a manual setting.

### 7.3 Browser constraints (known risks)

- **Mobile Safari:** may suspend `AudioWorklet` playback when the screen locks.
  - **Mitigation:** a fallback output path that feeds decoded audio to an `<audio>` element via Managed Media Source (iOS 17.1+), which iOS keeps playing in the background.
  - A spike in Phase 0 settles which path ships.
  - **Seen live (2026-10-10):** on iPad Safari the whole page stopped when backgrounded (no reconnect attempt at all), and was sometimes reloaded on return. Android resumed within 10 s.
  - **First mitigation (shipped):**
    - a silent looping `<audio>` element runs alongside the AudioWorklet while this device plays, started in the same tap;
    - the AudioContext is resumed when the page returns;
    - page lifecycle events are logged in a log that survives reloads;
    - after a reload, the app offers "Resume playing here".
  - Managed Media Source stays the fallback if real-device tests show this isn't enough.
- **Storage eviction:**
  - Browsers may evict offline storage. The app requests `navigator.storage.persist()` and shows quota and persistence status.
  - On iOS, offline downloads are only reliable when the PWA is installed to the home screen. The app says so.

---

## 8. Offline downloads

### 8.1 What can be downloaded

- Single tracks, albums, playlists, and artists (all albums), from local library files only.
- **Pinned items:** albums and playlists can be pinned. A pinned playlist re-syncs whenever the device is online: new tracks are downloaded and removed tracks deleted.

### 8.2 Format (D5)

- Per download, the user picks:
  - **Original**: bit-perfect, largest.
  - **Opus 160**: about 1.2 MB/min (an Opus 128 option was dropped: one Opus quality keeps the cache simple).
  - **MP3 320** (constant bit rate): for compatibility.
- Settings hold separate defaults for Wi-Fi and cellular, and a "download on Wi-Fi only" toggle (on by default).
- If the server has no `ffmpeg`, only Original is offered.
- The admin can restrict formats per user.

### 8.3 Download manager (Rust core)

- Persistent queue with 2 concurrent downloads, `Range` resume, retry with backoff, and pause/resume all.
- Each file is verified against the size/version from the manifest.
- **Stable identity:** LMS track IDs change on rescan, so downloads are keyed by a stable track key.
  - The key is the hex SHA-256 of a random gateway secret (made once, kept with the accounts) and the track's library URL. It survives rescans and never reveals the file's path; it changes only if the file moves or the account database is recreated.
  - The manifest (and `GET /v1/download/track/{id}/info`) returns both the current track ID and the stable key. On reconnect, the client re-resolves keys to current IDs before re-syncing or sending offline events.
- A track's version is derived from its library modification time and size. When the library file changes, the cached copy is refreshed on the next sync.
- Optional storage cap per device. Pinned items are never evicted automatically; the user is warned instead.
- Metadata and artwork are stored alongside the audio, so offline browse works fully.

### 8.4 Offline playback and sync-back (D6)

- When the gateway is unreachable, or the user turns on Offline mode, the app switches to a local player:
  - It uses the same decoder pipeline, with gapless playback, its own queue, and browse of downloaded content.
- **Plays (implemented 2026-10-11):**
  - Each play is logged locally, per user, when it ends (finished, skipped or stopped). The log holds the track ID, stable track key, start time and seconds actually heard, with a client-made idempotency key.
  - Back online, plays are sent in batches to `POST /v1/plays` (`docs/PROTOCOL.md` › Offline plays) and kept until the server answers for each.
  - The gateway checks each play:
    - the token;
    - shape and ranges;
    - start time within the last 90 days and not in the future (5 minutes' skew allowed);
    - a local library track, still the same file;
    - the per-user limits.
  - Keys are deduped per user, so a retry never counts twice.
  - **Counting rule (as scrobblers use):** a play counts if at least half the track, or 240 s, was heard, whichever is less (240 s when the duration is unknown).
  - **No overlap:** a play of a track the user already has a counted play of, starting less than that counting time before or after it, is refused (`overlap`): it can't be a separate listen (2026-10 security review).
  - The gateway forwards each play to the plugin's `anywhere play_record` over LMS JSON-RPC on loopback, signed with HMAC-SHA256 under the admin secret both already share. The plugin raises LMS's play count and sets last played (if later) through `Slim::Schema::TrackPersistent`, as LMS does for its own players.
- **Ratings** (not yet): rating changes will be queued the same way.
- Back online, the app offers to send the offline queue to an LMS player ("continue on <player>").
- _Web app (2026-10-10, 2026-10-11):_
  - the local player: an `<audio>` element on blob: URLs, with Media Session controls;
  - offline mode;
  - play logging in IndexedDB, with sync-back and backoff. The Downloads page shows "N plays waiting to sync".
- _Android, A6 (2026-10-11):_ Android Auto browses the library and the downloads and plays on this phone (D20); "Start radio" and "More like this" use AudioMuse-AI's menus or LMS's Don't Stop The Music; optional crossfades between albums (off by default, gapless and bit-perfect otherwise). See docs/plans/native-apps.md › A6.
- _Android, A5 (2026-10-11):_ downloads (a Rust download manager, app-private storage), offline mode, and the local queue played through the same Rust engine as streaming (D14), with the Media3 session. Plays are logged and sent to `POST /v1/plays` as on the web. A revoked device deletes its downloads on its next connection. See docs/plans/native-apps.md › A5.

### 8.5 Security

- Downloads need the per-user download permission, which also lists the formats the user may choose. It is on, with every format, by default for admins and listeners alike (they can already play the whole library; a download is the same audio kept for later). Admins turn it off or limit the formats per user in the plugin settings. It is read on every request, so a change applies at once.
- Files live in the app's private storage (IndexedDB in the browser; app sandbox later on native).
- In the browser, downloads are kept per user: another account signing in on the same browser doesn't see them. On sign-out the app asks whether to remove them (keeping is the default; removal is one tap).
- Offline mode before sign-in (server unreachable) offers only the downloads of the account last signed in on this device; signing out forgets that account, so after a sign-out downloads are shown again only after signing in.
- Revoking a device stops further downloads and ends running ones within seconds. Files already on a device can't be remotely wiped in the browser; the native apps will delete them on their next connection.

---

## 9. Authentication and security

### 9.1 Identities and tokens

- **Users:** created by the admin (D7), each with a role and permitted players. Passwords use argon2id. Passkeys (WebAuthn) come in a later phase.
- **Two-step verification (implemented):**
  - **Opt-in per user:** TOTP (RFC 6238) that users turn on themselves (with their password) by scanning a QR code into an authenticator app, then confirming a code.
  - **Codes:** single use, with ±30 s tolerance. Wrong codes count towards lockouts.
  - **Turning it off** needs the password and a current code.
  - **Admin:** can reset it for a user. Requiring it for all users is a possible later policy option.
- **Device sessions:** every browser login or app pairing creates a device record with its own assigned player MAC (a browser signing in again reuses its device). At most 20 active devices per user; another sign-in revokes the least recently seen. At most 10 new devices per user an hour. A revoked device's LMS player is removed (`client forget`), and the record is deleted after 30 days. Device names are cleaned (letters, digits, spaces and basic punctuation only; no markup, control or bidi characters; 64 characters at most).
  - **Renaming (2026-10-11):** users rename their own device (`PATCH /v1/devices/self`: web app Settings › This device, Android Settings), admins any device (plugin settings, through the admin API). Names are 1–64 characters, cleaned, and unique per user. The LMS player follows: renamed at once with LMS's `name` command if connected, otherwise at its next connection.
  - **Client type and last address (2026-10-11):** clients report their kind (`pwa`, `browser` or `android`) and a short platform string at sign-in (the web app at every renewal too); the gateway also keeps the User-Agent, cleaned and cut short. Each device's last client address (resolved behind trusted proxies) and time are recorded at sign-in and renewal, and on use at most once a minute. Only admins see these, in the plugin's device list; listeners have no device list. A revoked device's address and User-Agent are cleared at once.
- **Refresh tokens:**
  - valid for the sign-in lifetime, 90 days by default (1–365, set by the admin; was 30 until 2026-10-11), and rotated on every use, so a device in regular use stays signed in; reusing an old one revokes the device (theft detection). Within 60 s of a use, while no replacement has been used, the old one may be presented again (a lost response, or two tabs refreshing together) and gets a further replacement; the first replacement used supersedes the others. See `docs/PROTOCOL.md` for the trade-off;
  - in the browser, held in an `HttpOnly; Secure; SameSite=Strict` cookie scoped to `/v1/auth`;
  - in native apps, held in the Keystore/Keychain.
- **Access tokens:** last 15 minutes. They are opaque random values: the gateway keeps them in memory (hashed) mapped to user, device, role and MAC, and clients keep them in memory only. The gateway is the only verifier, so signed tokens would add a format to get wrong and gain nothing. Revocation is instant.
- **Stream, art and download tokens:** short-lived signed URLs bound to session/track, used where a header can't be set.
- **CSRF:**
  - Only `/v1/auth/*` reads the cookie, and it requires a custom header plus an `Origin` check.
  - All other APIs use bearer tokens, so they can't be triggered cross-site.
- **Revocation:** revoking a device or user in settings closes its sockets immediately and rejects its tokens.

### 9.2 Threat model

| Threat | Mitigation |
|---|---|
| Internet scanner hits LMS bugs (9.2.0 path traversal, `readdirectory`, XSS: CVE-2026-50230…50235) | LMS ports never exposed; gateway forwards only allow-listed commands and path patterns |
| Password guessing | Per-IP (IPv6 per /64) hard lockout, and a per-user limit that refuses the addresses that failed and holds all others to 2 attempts a minute (so guessing from many addresses gains little, and nobody can lock a user out); counted before the password is checked; at most 4 concurrent password checks; exponential lockout; TOTP; generic error messages |
| Stolen device / token | Per-device revocation; refresh-token rotation with reuse detection; 15-min access tokens |
| Impersonating another player | HELO MAC must match device's assigned MAC; stream path must match session's MAC |
| Reading arbitrary files | Track IDs resolved only through the library by the plugin; `file://` URLs rejected in control commands |
| XSS in the web app (e.g. hostile tag metadata) | Framework escaping; strict CSP, no inline script; metadata never rendered as HTML |
| Sniffing on hostile Wi-Fi | TLS via the proxy; gateway rejects non-TLS remote requests |
| Resource exhaustion | Caps on sessions per device, frame/body size, concurrent streams and downloads, transcode jobs; event sockets per device (4) and user (16), oldest closed first; artwork requests in flight per user (8) and overall (32), images streamed rather than buffered; LMS JSON-RPC answers read up to 16 MiB; 15 s limits on request heads, idle keep-alive and request bodies; at most 1024 open connections (`--max-connections`) and 64 per non-proxy address |
| LAN user creating remote accounts | Plugin commands are reachable only through LMS itself (LAN, LMS password protection); `anywhere` is reserved in the gateway allow-list so remote users can't call it; admin API on loopback with a secret |
| Misconfigured proxy exposing the LAN | Gateway binds to localhost by default; plugin self-test checks LMS ports aren't publicly reachable |

### 9.3 Hardening

- Structured audit log of logins, pairings, revocations, denied commands and downloads. Kept for one year, at most the newest 100,000 entries (2026-10).
- Pinned dependencies. Signed releases; the plugin verifies the gateway signature.
- An external security review is a release gate for v1 (§11).

---

## 10. Native apps (post-v1)

The detailed plan (scope, architecture, audio engine, track analysis, milestones and open decisions) is in [`docs/plans/native-apps.md`](docs/plans/native-apps.md).

Android (Kotlin + Jetpack Compose) and iOS (Swift + SwiftUI) are thin native UIs over the same Rust core (UniFFI) and the same protocol (§6). They reach feature parity with the web app, and add:

- device pairing by QR code;
- system background playback (Media3 `MediaSessionService` on Android, audio background mode + `MPNowPlayingInfoCenter` on iOS);
- Android Auto and CarPlay;
- downloads in the app sandbox, removed automatically when the device is revoked;
- true network-type detection for quality switching.

---

## 11. Milestones

The **v1 release** is Phases 0–3.

0. **Spike.** _Closed 2026-10-10; see `docs/phase0-report.md`._ The core played as a real LMS player natively, through the gateway behind a TLS proxy, and in desktop and Android browsers, with gapless transitions. The native test client replaced the planned squeezelite shim. The screen-locked phone test moves to Phase 1.
1. **Gateway + plugin.** Delivered in this order:
   1. resume and automatic client reconnect (the first phone test lost the player on a weak-signal dropout);
   2. accounts, pairing and sessions;
   3. the RPC allow-list, events and art;
   4. the plugin settings page and gateway supervision (_done 2026-10-10; signature check and packaging remain_);
   5. the screen-locked phone tests (Android, iOS);
   6. nginx config and the Docker image (_done 2026-10-10: `deploy/`, `Dockerfile`, `docs/DEPLOY.md`_).
   - **Gate:** an external security review, plus scanner tests proving LMS paths are unreachable.
   - **Gate:** a phone keeps playing through a 30 s network dropout and 4 minutes with the screen locked.
2. **Web app.** All of §7.2 except offline, as an installable PWA. _First slice done 2026-10-10:_
   - sign-in with two-step verification; the player switcher with "this device" built in;
   - Now playing (art, transport, seek, volume, shuffle/repeat); the queue (jump, reorder, remove, clear);
   - albums, artists, album and artist pages, and search, with play now, play next and add everywhere;
   - Media Session controls; light/dark theme; phone and desktop layouts.
   - Favourites (folders, play/next/add) and saved playlists: play, filter, and for admins create, rename, delete, reorder and remove tracks. Listeners can't change shared playlists (enforced by the gateway).
   - **Gate:** usability pass on phone and desktop; WCAG AA audit.
3. **Offline downloads.** Server download/transcode/manifest/events endpoints; the web download manager, offline mode and sync-back.
   - **Gate:** a pinned 500-track playlist downloads, survives a restart, plays offline, and syncs plays back. **→ v1 release.**
4. **Android app.**
5. **iOS app.**
6. **Later:**
   - Cache-served streaming: when LMS plays a track the device already has cached, play it from the cache to save data. Needs a spike on keeping LMS's stream state consistent.
   - Smart auto-cache of recently played tracks.
   - Passkeys.

---

## 12. Open questions

- [ ] **Downloads:** confirm streaming-service tracks are excluded. Proposed: yes, for licensing/ToS reasons.
- [ ] **Name:** "Lyrion Anywhere". Check with the Lyrion project before listing in the plugin repository.
- [ ] **Native app distribution (post-v1):** free vs paid. App Store review of a bring-your-own-server app needs a demo server.

---

## 13. Testing strategy (standing rule)

Every component is tested for security, stability and reliability, and security comes first. No change lands without tests, and CI runs the whole suite on every push. A release is blocked by any failing or skipped test.

| Layer | Security | Stability | Reliability |
|---|---|---|---|
| Rust core | Fuzzing (`cargo-fuzz`) and arbitrary-input tests for the Slimproto parser and decoder; must never panic or allocate unbounded memory | Unit tests for the player state machine, framing edge cases and rate changes | Gapless/resume/back-pressure scenario tests; WASM build tested in a headless browser |
| Gateway (Rust) | Negative tests per endpoint: no/expired/forged/revoked tokens, wrong-MAC HELO, forged stream paths, `serv`, every denied RPC command, `file://` URLs, path traversal in art/download routes, oversize frames, rate limits, CSRF/Origin, header spoofing from untrusted sources. A scanner suite proves LMS paths are unreachable. `cargo-fuzz` on frame and request parsers (shared with core) | Unit tests; mutation checks that each security test fails when its protection is removed | Integration tests against a fake LMS (Slimproto + HTTP + CometD): resume within grace, reconnect, timeouts, LMS restart during play |
| Plugin (Perl) | Settings input validation (CR/LF, ranges, enums); internal API secret enforcement; track resolution only from library | `prove` tests with stubbed `Slim::*` modules | Sidecar supervision (crash, restart, signature failure) |
| Web app | CSP compliance; XSS tests with hostile tag metadata; auth flows (login, TOTP, logout, revoked session) | Component tests (Vitest) | Playwright end-to-end tests against gateway + fake LMS; offline mode and sync-back |
| Live (opt-in) | Proxy self-test against a real deployment | — | Muted dedicated test player on a real LMS, config from gitignored `dev/local.env` |

Other rules:
- Security fixes get a regression test that fails before the fix.
- Dependency audits (`cargo audit`, `cargo deny` for licences and advisories, `npm audit`) run in CI.

## 14. Project conventions

- **Language:** British English in all docs and UI strings.
- **Repo layout:**
  - `core/` (Rust), `cli/` (native test player), `gateway/` (Rust), `plugin/` (Perl), `web/` (SvelteKit), `docs/`
  - `docs/` holds `USAGE.md`, `SETTINGS.md`, `PROTOCOL.md`, `TROUBLESHOOTING.md`, and runbooks, each runbook with a "Gotchas" section.
- **Changelog:** `CHANGELOG.md` with dated entries, newest first.
- **Plugin release:**
  - tag `vX.Y.Z`; `tools/build-release.sh` builds the plugin zip, gateway binaries and `repo.xml`, and is run by GitHub Actions
  - the version is single-sourced and stamped into `install.xml`, the plugin's VERSION constant and the manifest
- **No homelab details in the repo:** real hostnames, IPs and credentials live in gitignored local files (`.env.local`, `dev/local.*`). Committed examples use placeholders.
- **Clean-room reimplementation:** squeezelite (GPL-3.0) and Material Skin are references for behaviour only. Their code is not copied, and they are credited in the README.
- **Plugin tests:** Perl tests use stubbed `Slim::*` modules (`prove`). Live end-to-end tests drive a muted test player and restore its state afterwards.

## 15. References

- [Lyrion: Slimproto protocol](https://lyrion.org/reference/slimproto-protocol/)
- [Lyrion: CLI / JSON-RPC commands](https://lyrion.org/reference/cli/general/)
- [Lyrion wiki: Remote streaming](https://wiki.lyrion.org/index.php/Remote_streaming.html)
- [LMS remote access with Tailscale (forum)](https://forums.lyrion.org/forum/user-forums/logitech-media-server/113997-lms-remote-access-safe-secure-and-free-with-tailscale-mesh-vpn)
- [OpenCVE: Lyrion Music Server CVEs](https://app.opencve.io/cve/?product=lyrion_music_server&vendor=lyrion)
- [GHSA-rxf9-fjjc-h697 / CVE-2026-50230](https://github.com/advisories/GHSA-rxf9-fjjc-h697)

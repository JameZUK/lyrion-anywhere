# Changelog

Dated entries, newest first.

## 2026-10-10

### Phase 1: LMS plugin
- **`Plugins::LyrionAnywhere` 0.1.0** (LMS 9.0+):
  - settings for the gateway (enable, listen address, public URL, trusted proxies), each validated before it is saved;
  - users, roles, two-step reset, pairing codes, home-player grants, devices and the audit log, all through the gateway's admin API;
  - matching `anywhere …` LMS commands.
- **Gateway supervision:** picks the binary for the platform and runs it directly (no shell), with the admin API on loopback only. It restarts after crashes with backoff (5, 10, 30, 60 s) and stops when disabled.
- **Gateway:**
  - `POST` forms of the admin deletes, for LMS's HTTP client;
  - `--log-file`;
  - `anywhere` added to the reserved RPC commands, so remote users can never manage accounts.
- **Settings page:** every name is set as text, never parsed as HTML, because device names and audit details come from remote users. Validation messages that echo input are escaped. The page warns that anyone who can open LMS settings can manage remote accounts.
- **Tests:**
  - 140 Perl tests against stubbed LMS modules, including a contract test that drives the real gateway binary;
  - 5 settings-page tests, including a full render of the page with hostile names;
  - a new CI job, in which the contract test may not be skipped.
- **Bugs found by tests:** a missing `Slim::Utils::OSDetect` import, and an XSS in the settings error echo.
- **Live, in a throwaway LMS 9 container:**
  - the plugin loaded and the gateway started from the settings page;
  - the account commands worked, and sign-in through the gateway worked;
  - the gateway was restarted about 6 s after `kill -9`;
  - disabling it in settings stopped it;
  - a device named with an HTML payload was stored and shown as plain text.

### Phase 1: artwork and events
- **`/v1/art/…`:**
  - fixed path patterns for covers, the current cover (permitted players only), and plugin/built-in icons;
  - image-proxy URLs only if LMS itself handed them out, so a user can't make LMS fetch arbitrary home-network URLs (SSRF);
  - only real images are relayed, size-capped, with a sandboxing CSP (inert SVGs).
- **`WS /v1/events`:**
  - one LMS CLI listener shared by all subscribers;
  - events filtered to the caller's permitted players;
  - only playback-related event types; URLs and paths redacted;
  - revocation and permission changes applied within 30 s.
- **Bugs found and fixed:**
  - Image-proxy art could never load: axum decoded `%2F` in the path. Art paths are now validated raw, which also blocks encoded traversal.
  - The CLI still sent its old default MAC after the switch to accounts. The gateway correctly refused it; the CLI now uses the gateway-assigned identity.
- **Tests:** 18 new; all 8 art/events protections confirmed by mutation checks (one test gap found and closed).
- **Live:** real covers and icons served, LAN-address image proxy refused, unpermitted current-cover refused, events delivered to the right subscribers.

### Phase 1: browse and control (`/v1/rpc`)
- **Default-deny allow-list** for LMS JSON-RPC. Every argument is validated; only app commands learned from LMS's own menus are allowed; URLs and file paths are refused in every form; rescan is admin-only.
- **Player permissions:** admins control everything; listeners control their own devices plus admin-granted home players (admin API for grants).
- **Response filtering:**
  - removes LAN addresses, server identifiers, `file://` paths and stream URLs;
  - player lists show only permitted players.
- **Tests:** 40 new (policy matrix, 20,000 random inputs, integration against a fake LMS proving refused commands never reach it, filtering, permissions, app learning, grants). All 11 policy protections are confirmed by mutation checks.
- **Live (read-only):**
  - browsing returned 10,498 albums;
  - no file paths or addresses leaked;
  - `readdirectory`, `pref` and an unpermitted real player were refused and logged.
- **Found:** LMS drops the connection when a plugin command targets a never-connected player; the gateway reports 502.
- **Incident:** the build machine's disk filled during mutation testing and truncated one uncommitted source file. It was restored, and the mutation script now verifies every restore and reports build failures separately.

### Phase 1: optional two-step verification
- **TOTP, opt-in per user** (RFC 6238, verified against the RFC test vectors).
  - Users turn it on from the account panel by scanning a QR code (rendered by the gateway as SVG) or typing the key, then confirming a code.
  - Login then needs a code as well as the password.
  - Codes are single use. Wrong codes count towards lockouts.
  - A wrong password never reveals whether two-step is on.
  - Turning it off needs the password and a code.
  - Admins can reset it for a lost authenticator (admin API and `anywhere-gateway admin reset-totp`).
- **Schema:** migrations are tracked with `PRAGMA user_version`.
- **Tests:** 22 new two-step tests (RFC vectors, enrolment, login, replay, lockout, disable, reset, migration, endpoints, browser helpers). Mutation checks cover every two-step protection.
- **Live:** enrolment by QR, login challenge, replay rejected, fresh code accepted.

### Phase 1: accounts
- **Account store:** the gateway now owns accounts (spec D12), in SQLite:
  - users, with argon2id passwords;
  - devices, each with its own player MAC;
  - refresh tokens that rotate on every use, where reuse revokes the device;
  - single-use pairing codes and an audit log.
- **Public auth API:** `/v1/auth/login|pair|refresh|logout` and `/v1/me`.
  - CSRF header and origin checks.
  - The refresh token is held in an HttpOnly cookie for browsers.
  - Lockouts per username and per client.
- **Admin API** on a separate loopback-only listener, with an admin secret. `anywhere-gateway admin` commands for bootstrap.
- **Revocation** (logout, device revoke, user disable/delete, refresh reuse) ends live player sessions immediately.
- The Phase 0 devices file is gone.
- **Browser player:** sign-in page, silent sign-in via the refresh cookie, background token renewal, and device reuse across sign-ins.
- **CLI:** `--gateway` signs in with `--user`. The password comes from `ANYWHERE_PASSWORD` or stdin.
- **Tests:** 30 new auth tests (store, endpoints, admin API, revocation). Mutation checks cover every account protection.


### Phase 1: resume after network dropouts
- **Session survives drops:** the gateway session now outlives the client socket. A dropped connection is held for a 60 s grace period, and LMS never sees the player leave.
- **Heartbeats:** the gateway answers LMS on the player's behalf while it is away. Without this, LMS dropped the player after about 15 s, as found in a live test.
- **Redelivery:** gateway → client frames are numbered and redelivered on resume. A test race showed that frames written into a dying socket were silently lost.
- **Stream resume:** downloads resume by byte offset. Acknowledgement-paced relaying keeps the gateway within a 4 MiB window of what the client has confirmed. A live test showed network buffers otherwise absorbing whole tracks.
- **One session per device:** a newer connection for a device replaces the older one.
- **Browser player reconnects** automatically with backoff, queues outgoing messages while disconnected, and resumes broken downloads.
- **Live result:** a 20 s total outage against a real LMS. LMS stayed connected with its clock running, the session and download resumed, and page and LMS elapsed time matched afterwards (62.0 s).
- `docs/PROTOCOL.md` documents the player protocol.

### Phase 0: spike (closed)
- The Rust player core, Rust gateway, WASM browser player and native test CLI. See `docs/phase0-report.md`.
- Relicensed to GPL-2.0-or-later.

# Changelog

Dated entries, newest first.

## 2026-10-10

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

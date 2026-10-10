# Lyrion Anywhere player protocol (v1)

The protocol a client (web app, native app or test CLI) uses to act as an LMS player through the gateway. It is implemented by `gateway/` and, on the client side, by `web/spike/player-worker.js` and `cli/src/gateway.rs`.

## Endpoints

| Endpoint | Purpose |
|---|---|
| `WS /v1/player` | Slimproto tunnel, plus session control |
| `GET /v1/stream/<session>/<n>[?offset=k]` | Audio for the session's n-th stream |

Requirements for both endpoints:
- They must be reached over TLS through a trusted reverse proxy. Plain HTTP is accepted only from loopback.
- WebSocket upgrades must be same-origin or from an explicitly allowed origin. Clients that send no `Origin` (native apps) are accepted.

## Authentication

Accounts live in the gateway (spec D12). Every `/v1/auth/*` request must carry `X-Anywhere-Client: 1`, and any `Origin` must be same-origin or explicitly allowed. That's the CSRF defence for the cookie-carrying endpoints.

| Endpoint | Body | Result |
|---|---|---|
| `POST /v1/auth/login` | `{"username","password","device_name","device_id"?,"refresh"?: "cookie"\|"body"}` | `{access_token, expires_in, user, device}`, plus the refresh token (see below) |
| `POST /v1/auth/pair` | `{"code","device_name"}` | Same, with `refresh_token` in the body (native apps) |
| `POST /v1/auth/refresh` | none (cookie), or `{"refresh_token"}` | A new access token and a **rotated** refresh token |
| `POST /v1/auth/logout` | bearer access token | 204; revokes this device and ends its sessions |
| `GET /v1/me` | bearer access token | `{user, device}` |
| `GET /v1/account` | bearer access token | `{username, role, totp_enabled}` |
| `POST /v1/account/totp/setup` | bearer access token | `{secret, uri, qr_svg}`: a new secret, active once confirmed |
| `POST /v1/account/totp/confirm` | `{"code"}` | 204; two-step is now on |
| `POST /v1/account/totp/disable` | `{"password","code"}` | 204; two-step is now off |

- **Access tokens:** random, valid for 15 minutes. Use them as `Authorization: Bearer …` for streams and `/v1/me`, and as the first message on the player socket. Keep them in memory only.
- **Refresh tokens:**
  - valid for 30 days and single use; each refresh returns a new one;
  - presenting a used one revokes the whole device, because the token has been copied;
  - browsers get them in an `HttpOnly; Secure; SameSite=Strict` cookie scoped to `/v1/auth`;
  - native apps get them in the body and keep them in the Keystore/Keychain.
- **Two-step verification (optional, per user):** TOTP per RFC 6238 (HMAC-SHA1, 30 s, 6 digits), compatible with standard authenticator apps.
  - **Login:** once a user turns it on, a login with only the right password returns 401 `{"totp_required":true}`. The client then repeats the login with `"totp":"123456"`. A wrong password never reveals whether two-step is on.
  - **Codes:** each code works once; ±1 step is accepted for clock drift. Wrong codes count towards the same lockouts as wrong passwords.
  - **Turning it off** needs the password and a current code.
  - **Lost authenticator:** the admin can reset two-step for a user.
  - **Not affected:** refresh and pairing, because the device was verified when it signed in.
- **Devices:** each login (or pairing) is a device with its own player MAC. A browser can pass its previous `device_id` to keep the same LMS player, as long as the device belongs to the same user.
- **Errors:**
  - wrong credentials always return the same 401;
  - repeated failures per username or per client address return 429 with `Retry-After`, with escalating lockouts;
  - malformed or unknown fields return 400, and bodies over 4 KiB return 413.
- **Revocation:** revoking a device, disabling or deleting a user, logging out, or detecting refresh-token reuse closes that device's live player session immediately (1008, "access revoked").

### Admin API

The admin API is served on a separate listener (default `127.0.0.1:9445`) that must never be put behind the proxy. It is used by the LMS plugin's settings page.

Requests are refused unless they come from loopback, carry no `X-Forwarded-*`/`Forwarded`/`X-Real-IP` headers, and present the admin secret (`<data-dir>/admin.secret`, mode 0600) as a bearer token.

Endpoints:
- `GET/POST /admin/v1/users`
- `DELETE /admin/v1/users/{name}`
- `POST /admin/v1/users/{name}/password`
- `POST /admin/v1/users/{name}/disabled`
- `POST /admin/v1/users/{name}/pairing` (returns a one-time code, valid 10 minutes)
- `POST /admin/v1/users/{name}/totp/reset` (removes two-step for a user who lost their authenticator)
- `GET/POST /admin/v1/users/{name}/players`, `DELETE /admin/v1/users/{name}/players/{mac}` (home players a listener may control)
- `GET /admin/v1/devices`
- `DELETE /admin/v1/devices/{id}`
- `GET /admin/v1/audit`

## Player socket

### Opening

The client's first message is text, sent within 5 s:

- **New session:** the bare access token.
- **Resume:** `{"token":"…","resume":{"session":"…","secret":"…","received":N}}`. `N` is the number of binary messages the client received in this session before the drop.

The gateway replies with a text greeting:

```json
{"session":"<32 hex>","secret":"<32 hex>","mac":"02:…","name":"…","resumed":false}
```

- `mac` and `name` are the player identity assigned to this device. The client must use that MAC in HELO.
- `secret` is needed to resume this session. Treat it like a credential.

### Messages after the greeting

**Client → gateway:**
- **Binary messages:** the client → server Slimproto byte stream, with any chunking.
  - A new session must start with HELO carrying the assigned MAC.
  - A resumed session must not send HELO again.
- **Text messages:** only the stream acknowledgement `{"ack":{"stream":n,"bytes":b}}`. Send one at least every 1 MiB received on stream `n`, and once straight after resuming.

**Gateway → client:**
- One binary message per server → client Slimproto frame, length prefix included, exactly as LMS sent it.
- `serv` (redirect to another server) is never forwarded.

### Policy violations (close 1008, session ended)

Any of these ends the whole session, not just the socket:
- a first frame other than HELO;
- a HELO with another MAC;
- a second HELO;
- a frame over 16 KiB;
- any text message other than a well-formed ack.

### Close codes

| Code | Meaning | Client should |
|---|---|---|
| 1001 | LMS closed the connection, or the session expired | start a new session |
| 1006 | Connection lost (no close frame) | resume |
| 1008 | Unauthorised, or policy violation | stop; don't retry with the same credentials |
| 1013 | Server unavailable, or client too slow | resume (with backoff) |
| 4000 | Another connection for this device took over | stop |
| 4404 | Resume refused: unknown/expired session, wrong secret, or frames no longer available | start a new session |

## Resume

When the socket drops (anything other than a policy close):

1. The gateway keeps the session and its LMS connection for the **resume grace** (default 60 s). LMS never sees the player leave.
2. While detached, the gateway reports to LMS on the player's behalf every second, and answers LMS status polls. It replays the client's last STAT with refreshed timestamps: elapsed time advances by wall time if the client was playing, and stays frozen if it was paused. Without this, LMS drops a silent player after about 15 s.
3. Frames LMS sends meanwhile are kept in a numbered log, together with recently sent frames, which may have been lost in flight. On resume the gateway re-sends every frame after the client's `received` count, in order.
4. The log is bounded (default 1 MiB). If frames the client missed have been discarded, or the client claims frames that were never sent, the resume is refused (4404) and the client starts afresh.
5. A resume while the old socket is still open replaces it. On mobile, dead sockets often linger.
6. A new (non-resume) session for the same device ends any previous session, so LMS never sees two players with one MAC.

Recommended client backoff: about 0.5, 1, 2 and 4 s, then every 5 s, with ±20 % jitter. Stop resuming shortly before the grace expires, then start a new session.

## Streams

- **Numbering:** stream `n` is the n-th `strm s` frame the client received in the session, counting from 1. Refused streams count too.
- **What the gateway fetches:** only the request LMS itself put in that `strm`, after checking it is exactly `GET /stream.mp3?player=<this MAC>` on an LMS address.
- **Ownership:** the upstream LMS connection belongs to the session, not the HTTP request.
  - **Resume:** a broken download resumes with `?offset=k`, where `k` is the number of bytes the client has.
  - **Supersede:** a newer request for the same stream supersedes an older one.
- **Pacing:** the gateway reads from LMS at most one **replay window** (default 4 MiB) beyond the client's last ack. Network buffers therefore can't hold more than the window, and a resume always lands inside it.
- **Responses:**

  | Status | Meaning |
  |---|---|
  | 200 | Audio follows (from `offset`) |
  | 401 | Bad or missing bearer token |
  | 404 | No such session or stream for this device, or the stream was refused by policy |
  | 410 | `offset` is older than the replay window: give up on this stream |
  | 416 | `offset` is beyond what has been sent |

- **Retries:** network errors and 5xx are retryable. Everything else is final; report the stream as failed.

## Browse and control (`POST /v1/rpc`)

Request: `{"player": "<player id or empty>", "cmd": [...]}` with a bearer access token. `cmd` is an LMS command array, for example `["albums", 0, 50, "tags:al"]`.

Response: `{"result": {...}}`, or `{"error": "<reason>"}` with one of these statuses:
- **403:** refused by policy, with the reason;
- **401:** bad token;
- **400:** malformed request;
- **413:** body over 8 KiB;
- **502:** LMS unavailable.

**Default deny.** A command is allowed only if it is in the allow-list and every argument passes validation:
- numbers or printable strings of at most 256 characters;
- at most 16 arguments;
- paging `start count` with `count` ≤ 500;
- `key:value` parameters restricted to known keys per command, each given at most once, with typed values.

| Group | Allowed |
|---|---|
| Library (no player) | `artists`, `albums`, `titles`/`tracks`, `genres`, `years`, `search`, `musicfolder`, `playlists` (list, tracks, new/rename/delete/edit by id), `favorites items`, `apps`, `radios`, `players`, `serverstatus`, `syncgroups ?` |
| Player (permitted players only) | `status`, `play`, `pause`, `stop`, `mode ?`, `time`, `power`, `sleep`, `mixer volume/muting`, `playlist index/jump/move/delete/clear/shuffle/repeat/tracks ?`, `playlistcontrol cmd:load/add/insert/delete` with exactly one id selector, `sync` with another permitted player, `favorites playlist play/add/insert item_id:…` |
| Apps and radio | `<app> items …` and `<app> playlist play/add/insert item_id:…`, only for app commands LMS lists in its `apps`/`radios` menus. Core command names can never be learned as apps. An unknown app triggers at most one menu refresh a minute. |
| Admin role only | `rescan` |

Everything else is refused, notably:
- `readdirectory`, `pref`, `playerpref`;
- server restart and stop, `debug`/`logging`, `button`/`display`;
- any form taking a URL or file path, such as `playlist play/add/insert/load` and `playlistcontrol url:`.

**Player permissions:**
- **Admins** may control every player.
- **Listeners** may control their own signed-in devices, plus home players an admin has granted (`/admin/v1/users/{name}/players`).

**Response filtering:**
- removed everywhere: `ip`, `uuid`, `httpport`, `url` (file paths and stream credentials), `mac`, `seq_no`, `remote_url` and `path`;
- player lists show only players the caller may control, with counts adjusted, and other-server counts removed.

## Artwork (`GET /v1/art/<path>`)

Bearer access token required. Browsers fetch art with `fetch()` and display it through blob URLs, so credentials never appear in image URLs.

Allowed paths (validated on the raw, still-encoded path):

| Path | Notes |
|---|---|
| `music/<id>/cover[_WxH[_m]][.ext]`, `music/<id>/thumb…` | Numeric or hex track id |
| `music/current/cover…?player=<mac>` | Only for players the caller may control; not cached |
| `imageproxy/<url-encoded http(s) URL>/image[_WxH[_m]][.ext]` | Only for image URLs LMS itself returned in an earlier `/v1/rpc` response. Anything else would let a user make LMS fetch arbitrary URLs on the home network (SSRF). |
| `plugins/<Name>/html/…`, `plugins/cache/icons/…`, `html/images/…` | Image files only (`png`, `jpg`, `gif`, `svg`); no `..`, `//` or `%` encodings |

Rules for what is relayed:
- only JPEG/PNG/GIF/WebP/SVG responses, at most 20 MiB;
- every response carries `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`, so an SVG can't run script on the gateway's origin.

## Events (`WS /v1/events`)

**Connecting:** the first message is the access token, as text. The gateway replies `{"subscribed":true}`, then sends `{"player":"<mac>","event":[...]}` for the caller's permitted players and `{"player":null,"event":["rescan",...]}` for library rescans. `{"resync":true}` means some events were missed, so re-query state.

**What is forwarded:** only `playlist`, `mixer`, `power`, `client`, `sync`, `pause`, `play`, `stop`, `time` and `newmetadata` events. Any token that looks like a URL or path is blanked. Preference changes (`prefset`) and everything else are never forwarded.

**Rules:**
- Clients may send nothing after the token. Any message closes the socket (1008).
- Revocation and permission changes are re-checked every 30 s: a revoked device is disconnected, and new grants take effect.

**Upstream:** the gateway keeps one connection to the LMS CLI (`--lms-cli-port`, default 9090) while anyone is subscribed, and reconnects with backoff.

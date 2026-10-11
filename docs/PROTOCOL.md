# Lyrion Anywhere player protocol (v1)

The protocol a client (web app, native app or test CLI) uses to act as an LMS player through the gateway. It is implemented by `gateway/` and, on the client side, by `web/app/static/player/player-worker.js`, `mobile/src/player/` (the native apps) and `cli/src/gateway.rs`.

## Endpoints

| Endpoint | Purpose |
|---|---|
| `WS /v1/player` | Slimproto tunnel, plus session control |
| `GET /v1/stream/<session>/<n>[?offset=k]` | Audio for the session's n-th stream |

Requirements for both endpoints:
- They must be reached over TLS through a trusted reverse proxy. Plain HTTP is accepted only from loopback.
- WebSocket upgrades must be same-origin or from an explicitly allowed origin. Clients that send no `Origin` (native apps) are accepted.

### Connection limits (all endpoints)

- A request head must arrive within 15 s, and an idle keep-alive connection is closed after 15 s. The connection is closed without a response.
- A request body (every endpoint takes at most a few KiB of JSON) must arrive whole within 15 s, or the request gets 408 and the connection is closed. Bodies over 1 MiB get 413 whatever the endpoint.
- At most 1024 connections are open at once (`--max-connections`); beyond that, new connections wait to be accepted. A peer that is neither loopback nor a trusted proxy may hold at most 64. Behind a reverse proxy every connection comes from the proxy, so per-client connection limits belong there.
- WebSockets and streamed responses (audio, downloads, artwork) are not affected by these time limits.
- The server speaks HTTP/1.1 (with WebSocket upgrades).

## Authentication

Accounts live in the gateway (spec D12). Every `/v1/auth/*` request must carry `X-Anywhere-Client: 1`, and any `Origin` must be same-origin or explicitly allowed. That's the CSRF defence for the cookie-carrying endpoints.

| Endpoint | Body | Result |
|---|---|---|
| `POST /v1/auth/login` | `{"username","password","device_name","device_id"?,"refresh"?: "cookie"\|"body"}`, plus the optional client fields | `{access_token, expires_in, user, device}`, plus the refresh token (see below) |
| `POST /v1/auth/pair` | `{"code","device_name"}`, plus the optional client fields | Same, with `refresh_token` in the body (native apps) |
| `POST /v1/auth/refresh` | none (cookie), or `{"refresh_token"}`; either may add the optional client fields | A new access token and a **rotated** refresh token |
| `POST /v1/auth/logout` | bearer access token | 204; revokes this device and ends its sessions. Sends `Clear-Site-Data: "cache"` |
| `GET /v1/me` | bearer access token | `{user, device, features}` (`features.analysis`: the server offers track analysis) |
| `GET /v1/account` | bearer access token | `{username, role, totp_enabled, downloads: {enabled, formats}}` (`formats`: what this user may download that this server can produce; see [Downloads](#downloads)) |
| `POST /v1/account/totp/setup` | `{"password"}` | `{secret, uri, qr_svg}`: a new secret, active once confirmed. A wrong password is 401 and counts towards a lockout of its own (per user), separate from sign-in |
| `POST /v1/account/totp/confirm` | `{"code"}` | 204; two-step is now on |
| `POST /v1/account/totp/disable` | `{"password","code"}` | 204; two-step is now off |
| `PATCH /v1/devices/self` | `{"name"}`, bearer access token | `{id, name}`: renames the caller's own device (see **Devices**) |

- **Access tokens:** random, valid for 15 minutes. Use them as `Authorization: Bearer …` for streams and `/v1/me`, and as the first message on the player socket. Keep them in memory only.
- **Refresh tokens:**
  - single use; each refresh returns a new one, valid for the sign-in lifetime (90 days by default, 1–365 set by the admin in the plugin settings). A device in use therefore stays signed in; one unused for the whole lifetime must sign in again;
  - presenting a used one revokes the whole device, because the token has been copied (audit event `refresh.reused`, and the 401 carries `Clear-Site-Data: "cache"`);
  - **grace:** within 60 seconds of a token's use, while none of its replacements has been used, presenting it again is a lost response (a phone suspended mid-request) or a concurrent refresh (two tabs restoring at once). Another replacement is issued *alongside* the first, which stays valid (audit event `refresh.retried`), so whichever one the client keeps works. At most 8 replacements per token; beyond that the refresh is refused (401) without revoking;
  - **siblings:** the first replacement to be used wins. Its unused siblings are superseded at once: their access tokens stop working, and presenting one later is reuse, so the device is revoked. Presenting the original after any of its replacements was used is reuse too;
  - **trade-off:** a thief who copied a refresh token and replays it within 60 seconds of the owner's refresh also gets a valid replacement. They can't both keep the device: whichever side uses its replacement first cuts off the other's access token immediately, and when the other side next refreshes, the device is revoked for both. Before this change, the second refresh discarded the first replacement, which signed out (and revoked) legitimate browsers whose tabs refreshed together. The window is 60 seconds and the thief needs the refresh token itself;
  - rotation is a single database transaction, so two simultaneous refreshes of one token never look like reuse: one rotates it, the other is served by the grace rule;
  - the web app also takes a cross-tab Web Lock (`anywhere-refresh`, where supported) around each refresh, so its tabs don't present the same cookie at once;
  - clients sign out only when a refresh is refused (4xx). If the server can't be reached, they keep the sign-in and retry with backoff;
  - browsers get them in an `HttpOnly; Secure; SameSite=Strict` cookie scoped to `/v1/auth`;
  - native apps get them in the body and keep them in the Keystore/Keychain.
- **Two-step verification (optional, per user):** TOTP per RFC 6238 (HMAC-SHA1, 30 s, 6 digits), compatible with standard authenticator apps.
  - **Login:** once a user turns it on, a login with only the right password returns 401 `{"totp_required":true}`. The client then repeats the login with `"totp":"123456"`. A wrong password never reveals whether two-step is on.
  - **Codes:** each code works once; ±1 step is accepted for clock drift. Wrong codes count towards the same lockouts as wrong passwords.
  - **Turning it on** needs the account password, so a stolen access token alone can't add an authenticator.
  - **Turning it off** needs the password and a current code.
  - **Lost authenticator:** the admin can reset two-step for a user.
  - **Not affected:** refresh and pairing, because the device was verified when it signed in.
- **Devices:** each login (or pairing) is a device with its own player MAC. A browser can pass its previous `device_id` to keep the same LMS player, as long as the device belongs to the same user (the web app always does). A user has at most 20 active devices: signing in (or pairing) another revokes the least recently seen one and ends its live session (audit event `device.evicted`).
  - At most 10 **new** devices per user per hour (reusing one doesn't count); beyond that, 429 with `Retry-After` (audit event `device.rate_limited`).
  - **Device names** are cleaned rather than refused: letters of any script, digits, spaces and `. , - _ ' ’ ( ) &` are kept, everything else (markup, control, bidi and other format characters such as U+202E or U+200B) is removed, runs of whitespace become one space, and the result is cut to 64 characters. Nothing left: `Browser`. The name a player reports to LMS (`SETD`) is cleaned the same way, and replaced by the device name if nothing usable is left.
  - **Renaming:** `PATCH /v1/devices/self` `{"name"}` renames the device the access token belongs to (there is no way to name another one; it needs `X-Anywhere-Client` like `/v1/auth/*`). Admins rename any device through the admin API. A name must be 1–64 characters once trimmed, and is then cleaned as above; one with nothing usable left (e.g. only markup or format characters) is refused with 400, as is one already used (ignoring case) by another of the same user's devices. Other users may use the same name. Access tokens carry the new name at once. If the device has a live player session, the gateway renames its LMS player with LMS's `name` command (LMS then tells the player, as when a player is renamed in LMS) and answers later name queries with it; otherwise the new name reaches LMS in the greeting and name reply when the device next connects. Audit event `device.renamed`.
  - **Client fields** (optional, on login, pair and refresh): `"client_kind"`: `"pwa"` (the web app in a standalone display mode, i.e. installed), `"browser"` (a browser tab) or `"android"`; anything else is a 400. `"client_platform"`: a short description such as `"Safari on iPadOS"` or `"Pixel 8"`, at most 48 characters (longer is a 400), cleaned like a device name. Fields left out keep what the device reported before. The gateway also keeps the `User-Agent` header, cut to 200 characters of printable ASCII, without `<`, `>`, `"`, `'`, `&`, backslashes and backticks. Clients from before these fields send none; a client meeting an older gateway (which refuses unknown fields as `malformed request`) may repeat a sign-in without them, as the Android app does. The web app sends them on every sign-in and renewal; the Android app on sign-in and pairing.
  - **Last seen:** each device's last client address (as resolved under **Client address**, the full address rather than the /64 used for lockouts) and time are recorded at sign-in, pairing and refresh, and on authenticated API requests and player connections at most once a minute per device. Only the admin API's device list shows them, with the client fields; listeners can't see them for any device, their own included (the apps have no device list). Revoking a device clears its address and User-Agent at once; its row (name and client type) goes with the other revoked devices.
  - When a device is revoked (any of the ways under **Revocation**), the gateway asks LMS to forget its player (`client forget`), after ending the session; best effort, failures are logged. Revoked devices are deleted after 30 days.
- **Errors:**
  - wrong credentials always return the same 401;
  - repeated failures return 429 with `Retry-After`, with escalating lockouts (5 failures in 15 minutes; 15 minutes, doubling up to a day). An attempt counts from the moment it starts, so parallel guesses can't get past the limit:
    - **per client address**, a hard lock: that address may not try again until it ends. IPv6 clients are counted per /64;
    - **per username**, a soft one, so nobody can lock someone else out: once tripped, the addresses that failed for that username are refused, and all others share 2 attempts a minute. The right password (and code) from an address that hasn't failed therefore still signs in, while spreading guesses over many addresses buys no more than that budget. Usernames that can't exist get no entry of their own;
    - **pairing codes** have the same soft limit across all clients, as well as the per-address one;
  - at most 4 password checks run at once; beyond that, requests wait briefly and then get 503 with `Retry-After: 1`;
  - malformed or unknown fields return 400, and bodies over 4 KiB return 413.
- **Revocation:** revoking a device, disabling or deleting a user, an admin password reset, logging out, the device limit, or detecting refresh-token reuse closes that device's live player session immediately (1008, "access revoked"), then removes its LMS player.
- **Client address:** for lockouts and audit, the gateway uses the TCP peer, unless the peer is a trusted proxy (loopback or `--trusted-proxy`). Then, if `--client-ip-header` names a header (e.g. `CF-Connecting-IP`) and it holds one valid address, that; otherwise `X-Forwarded-For` read from the right, skipping trusted proxies: the first address no trusted proxy vouches beyond. Entries further left are written by the client and ignored, so proxies may append to the header.
- **Response headers:** every `/v1/` response except artwork (and downloads, which set `private, no-store`) carries `Cache-Control: no-store` (track analyses set `private, no-cache`, with an ETag). All responses, the "TLS required" 403 included, carry the security headers: CSP, `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy`, `Cross-Origin-Opener-Policy` and a `Permissions-Policy` that turns off camera, microphone, geolocation, payment, USB and other features the app doesn't use.
- **Origins:** an `Origin` is accepted if it is the request's own host over HTTPS, or listed with `--allowed-origin`. `http://` same-origin is accepted only on plain-HTTP requests, which the gateway serves only on loopback without proxy headers (local development).
- **`Clear-Site-Data: "cache"`** on sign-out (and on the reuse refusal) clears cached artwork and responses from the browser. It leaves IndexedDB alone, so offline downloads stay; browsers may also empty the service worker's caches, which it refills.

### Pairing links (native apps)

A pairing code can be handed to a phone as a QR code holding a link:

```
lyrion-anywhere://pair?server=<percent-encoded server address>&code=<pairing code>
```

- `code` is required: 4–32 letters and digits once spaces and dashes are removed (case-insensitive), then redeemed with `POST /v1/auth/pair`.
- `server` is optional (`https://music.example.com`). Apps validate it exactly as a typed address: `https` only (plain `http` only for loopback), no user name or password, query, fragment or path.
- Any other parameter, a repeated parameter or a fragment makes the link invalid. A bare code (no link) is also accepted.

### Admin API

The admin API is served on a separate listener (default `127.0.0.1:9445`) that must never be put behind the proxy. It is used by the LMS plugin's settings page.

Requests are refused unless they come from loopback, carry no `X-Forwarded-*`/`Forwarded`/`X-Real-IP` headers, and present the admin secret (`<data-dir>/admin.secret`, mode 0600) as a bearer token.

Endpoints:
- `GET/POST /admin/v1/users`
- `DELETE /admin/v1/users/{name}`
- `POST /admin/v1/users/{name}/password` (also revokes the user's devices, signing them out everywhere)
- `POST /admin/v1/users/{name}/disabled`
- `POST /admin/v1/users/{name}/pairing` (returns a one-time code, valid 10 minutes)
- `POST /admin/v1/users/{name}/totp/reset` (removes two-step for a user who lost their authenticator)
- `POST /admin/v1/users/{name}/downloads` `{"enabled": bool, "formats"?: ["original","mp3","opus"]}` (download permission; `formats` must name one to three of these and is kept as it was when left out). `GET /admin/v1/users` lists `downloads` and `download_formats` per user, and `GET /admin/v1/settings` lists the `download_formats` this server can produce
- `GET/POST /admin/v1/users/{name}/players`, `DELETE /admin/v1/users/{name}/players/{mac}` (home players a listener may control)
- `GET /admin/v1/devices`: each active device's `id, username, name, mac, created_at, last_seen`, and `last_addr` (the client address it was last seen from), `client_kind`, `client_platform` and `user_agent` (each `null` until known; see **Devices** under Authentication)
- `PATCH /admin/v1/devices/{id}` `{"name"}` (also `POST /admin/v1/devices/{id}/name`): rename any device, with the same rules as `PATCH /v1/devices/self`. Returns `{id, name}`; 404 for an unknown or revoked device
- `GET /admin/v1/devices/{id}/diagnostics` (the device's latest diagnostics report)
- `DELETE /admin/v1/devices/{id}`
- `GET/POST /admin/v1/settings`: `{"session_days"}` sets the sign-in lifetime (1–365 days). The GET also returns `session_days_default` (90), `session_days_min`, `session_days_max` and the `download_formats` this server can produce
- `GET /admin/v1/audit`

LMS's HTTP client cannot send `DELETE` or `PATCH`, so the rename has a `POST` form (above) and each delete has one too: `/admin/v1/users/{name}/delete`, `/admin/v1/users/{name}/players/{mac}/remove` and `/admin/v1/devices/{id}/revoke`.

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
  - A new session must start with HELO carrying the assigned MAC and device ID 12 (software player).
  - A resumed session must not send HELO again.
  - Only the frames a software player sends are accepted: HELO, STAT, RESP, META, DSCO, `BYE!`, and SETD as the answer to an LMS name query (`setd` id 0 with no data), one answer per query.
  - The gateway rewrites two things before LMS sees them:
    - HELO capabilities keep only well-formed tokens: printable ASCII, comma-separated, each a bare codec name (`flc`) or `Key=value`, at most 64 tokens and 1 KiB in all;
    - a SETD name that is empty, longer than 64 characters, has control characters, or looks like a URL or path is replaced by the device's name.
- **Text messages:** only the stream acknowledgement `{"ack":{"stream":n,"bytes":b}}`. Send one at least every 1 MiB received on stream `n`, and once straight after resuming.

**Gateway → client:**
- One binary message per server → client Slimproto frame, length prefix included, exactly as LMS sent it.
- `serv` (redirect to another server) is never forwarded.
- **Client notes:**
  - On a seek LMS sends `strm q` and the new `strm s` back to back (about 2 ms apart for MP3), so a client sees them together. Drop queued audio on the `strm q` itself, not on a later check of the player state.
  - `audg` carries LMS's volume as linear gains (16.16) on LMS's own curve. A client applying it should do so at its output, after any queued audio.

### Policy violations (close 1008, session ended)

Any of these ends the whole session, not just the socket:
- a first frame other than HELO;
- a HELO with another MAC;
- a second HELO;
- a HELO with a device ID other than 12;
- any client opcode outside the list above (IR, BUTN, KNOB, BODY, UREQ, SHUT, …), a SETD LMS didn't ask for, or a malformed STAT or DSCO;
- a frame over 16 KiB;
- any text message other than a well-formed ack.

### Close codes

| Code | Meaning | Client should |
|---|---|---|
| 1001 | LMS closed the connection, or the session expired | start a new session. A browser can't tell the gateway's 1001 from one it made itself (for example when suspending a page), so the web player tries to resume first and starts a new session on 4404 |
| 1006 | Connection lost (no close frame) | resume |
| 1008 | Unauthorised, or policy violation | stop; don't retry with the same credentials |
| 1013 | Server unavailable, or client too slow | resume (with backoff) |
| 4000 | Another connection for this device took over | stop |
| 4404 | Resume refused: unknown/expired session, wrong secret, or frames no longer available | start a new session |

## Resume

When the socket drops (anything other than a policy close):

1. The gateway keeps the session and its LMS connection for the **resume grace** (default 60 s). LMS never sees the player leave.
2. While detached, the gateway reports to LMS on the player's behalf every second, and answers LMS status polls. It replays the client's last STAT with refreshed timestamps: elapsed time advances by wall time if the client was playing, and stays frozen if it was paused. Without this, LMS drops a silent player after about 15 s.
   - It does the same for a socket that is still attached but has sent nothing for 8 s. A working client answers LMS's status polls every 5 s, so silence means its socket may have died without either side noticing (a phone changing networks). The heartbeats stop as soon as the client sends anything.
   - **Clients:** LMS polls every player every 5 s, so a working socket never stays quiet for long. Treat about 20–25 s with nothing received as a dead socket: abandon it and resume. Count only what arrives; your own messages going out prove nothing.
3. Frames LMS sends meanwhile are kept in a numbered log, together with recently sent frames, which may have been lost in flight. On resume the gateway re-sends every frame after the client's `received` count, in order.
4. The log is bounded (default 1 MiB). If frames the client missed have been discarded, or the client claims frames that were never sent, the resume is refused (4404) and the client starts afresh.
5. A resume while the old socket is still open replaces it (close 4000). On mobile, dead sockets often linger. From that moment nothing the old socket sends reaches LMS.
6. A client that can't keep up with LMS's frames (its outgoing queue fills) has its socket closed with 1013 and is treated exactly like a dropped connection: heartbeats and the grace period start, and it can resume.
7. A new (non-resume) session for the same device ends any previous session, so LMS never sees two players with one MAC.

Recommended client backoff: about 0.5, 1, 2 and 4 s, then every 5 s, with ±20 % jitter. Stop resuming shortly before the grace expires, then start a new session.

## Streams

- **Numbering:** stream `n` is the n-th `strm s` frame the client received in the session, counting from 1. Refused streams count too.
- **What the gateway fetches:** only the request LMS itself put in that `strm`, after checking it is exactly `GET /stream.mp3?player=<this MAC>` on an LMS address.
- **Ownership:** the upstream LMS connection belongs to the session, not the HTTP request.
  - **Resume:** a broken download resumes with `?offset=k`, where `k` is the number of bytes the client has.
  - **Supersede:** a newer request for the same stream supersedes an older one.
- **Pacing:** the gateway reads from LMS at most one **replay window** (default 4 MiB) beyond the client's last ack. Network buffers therefore can't hold more than the window, and a resume always lands inside it.
- **Ending:** a response ends cleanly only when LMS finished the stream cleanly and the client has had every byte. Every other end is an error (the connection breaks without a clean end), so a client never mistakes it for the end of the track:
  - the session ends (for example, the device is revoked), or a newer request for the stream takes over;
  - LMS sends nothing for 60 s, or its stream fails. Later requests for that stream fail too, after replaying what was received;
  - the client has a full window unacknowledged and sends no ack for 60 s (for example, it is paused, or its buffer is full). It fetches again with `?offset=` when it wants more.
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
- **502:** LMS unavailable, or its answer is over 16 MiB (the read stops at the limit).

**Default deny.** A command is allowed only if it is in the allow-list and every argument passes validation:
- numbers or printable strings of at most 256 characters;
- at most 16 arguments;
- paging `start count` with `count` ≤ 500;
- `key:value` parameters restricted to known keys per command, each given at most once, with typed values;
- free-text values (`search:`, `term:`, playlist names, an app's `search:`, text typed into a learned search box) refused if they hold an absolute or Windows path, a `file:` URL, a URL to a local host (see below), a bare LAN address such as `10.0.0.5:9000`, or any URL other than http(s). Public http(s) URLs pass (some apps search by feed URL), as does ordinary text such as "AC/DC" or "Live 1990/91".

| Group | Allowed |
|---|---|
| Library (no player) | `artists`, `albums`, `titles`/`tracks`, `genres`, `years`, `search`, `musicfolder`, `libraries` (no arguments), `playlists` (list, tracks), `favorites items`, `apps`, `radios`, `players`, `serverstatus`, `syncgroups ?` |

Library browse parameters ("My Music"), and the values each takes:

| Parameter | Commands | Value |
|---|---|---|
| `artist_id`, `album_id`, `track_id` | `albums`, `artists`, `titles`, `genres` | ids: digits, or a comma-separated list of them |
| `genre_id` | `albums`, `artists`, `titles` (ids); `genres` (one id) | digits |
| `year` | `albums`, `artists`, `titles`, `genres` | digits (0 is "no year") |
| `folder_id` | `musicfolder` | one id, digits only. Never `url:` or a path: LMS would read (or scan) that location |
| `role_id` | `artists`, `albums` | one of `ALBUMARTIST`, `ARTIST`, `COMPOSER`, `CONDUCTOR`, `BAND`, `TRACKARTIST` (LMS fails outright on a role it doesn't know) |
| `compilation` | `albums` | `0` or `1` |
| `library_id` | `albums`, `artists`, `titles`, `genres`, `years` | an LMS virtual library id: 1–32 hex digits |
| `sort` | `albums`, `titles` | `album`, `artflow`, `artistalbum`, `yearalbum`, `yearartistalbum`, `new`, `random`, `title`, `tracknum`, `albumtrack`, `artisttrack`, `id` |
| `hasAlbums` | `years` | `0` or `1` |
| `tags` | all of them | letters and digits only |

A `musicfolder` answer lists each entry's `url` (its path on the server): it is stripped like every `url`, and a name that holds a path is dropped (see the response filter below).
| Player (permitted players only) | `status`, `play`, `pause`, `stop`, `mode ?`, `time`, `power`, `sleep`, `mixer volume/muting`, `playlist index/jump/move/delete/clear/shuffle/repeat/tracks ?`, `playlistcontrol cmd:load/add/insert/delete` with exactly one id selector, `sync` with another permitted player, `favorites playlist play/add/insert item_id:…` |
| Apps and radio | `<app> items …` and `<app> playlist play/add/insert item_id:…`, only for app commands LMS lists in its `apps`/`radios` menus. Core command names can never be learned as apps. An unknown app triggers at most one menu refresh a minute. |
| Admin role only | `rescan`; changing saved playlists (`playlists new/rename/delete/edit`) |

**Plugin and context menus (learned actions).** LMS menus (apps, radio, track/album/artist info, plugin menus) offer actions: commands with parameters, which plugins define. A command outside the fixed allow-list is allowed only if **LMS offered exactly that action, in a menu the gateway relayed to the same device in the last hour**:
- requests must match a learned action word for word and parameter for parameter, except text typed into a search box (`__TAGGEDINPUT__`);
- request values are printable, at most 256 characters, and never URLs or paths;
- actions are learned after response filtering, and an action whose parameters held a path or LAN URL is removed whole, so it can never be learned or shown;
- server-level commands (preferences, rescan, restart, `client`, display/IR, …) never run this way;
- actions that change shared data (`favorites`, `jivefavorites`, `playlists`, presets) need the admin role;
- paging (`start count`) is required for `… items` menus, optional for a navigation (a `go` action without `nextWindow`, which opens a submenu in LMS's own clients), and refused for any other action;
- at most 4,000 learned actions per device, expiring after an hour, and learned actions are kept for at most 1,000 devices (the least recently used go first).

**Plugin entries (home menu).** Plugins also add entries to the player's home menu (My Music, Radio, Extras), not only to `apps`/`radios`. `menu <start> <count> [direct:1]` (permitted players only) returns that menu cut down to library and plugin entries: anything under the settings nodes, player power, and entries the app shows itself (library, favourites, search) are dropped before the device sees them, as are entries not reachable from a top-level node. Their actions are then learned like any menu's.

**Admin-only plugins.** An admin can reserve plugins (by command word, e.g. `audiomuseai`) for admins, on the plugin settings page (`GET/POST /admin/v1/plugins` on the admin API; the GET lists what LMS offers as apps, radio and home-menu entries). For listeners, these plugins are removed from app lists, the home menu and context menus, and their commands are refused (403), however they are reached.

Context menus start from `trackinfo`/`albuminfo`/`artistinfo`/`genreinfo`/`yearinfo`/`playlistinfo`/`folderinfo items` with a library id.

Everything else is refused, notably:
- `readdirectory`, `pref`, `playerpref`;
- server restart and stop, `debug`/`logging`, `button`/`display`;
- any form taking a URL or file path, such as `playlist play/add/insert/load` and `playlistcontrol url:`.

**Player permissions:**
- **Admins** may control every player.
- **Listeners** may control their own signed-in devices, plus home players an admin has granted (`/admin/v1/users/{name}/players`).

**Response filtering:**
- removed everywhere by key: `ip`, `uuid`, `httpport`, `url` (file paths and stream credentials), `mac`, `seq_no`, `remote_url`, `favorites_url` and `path`;
- removed everywhere by value, whatever the key, when the text contains (anywhere, not only at the start):
  - a `file:` URL;
  - a URL of any scheme (`http`, `smb`, `nfs`, `ftp`, `rtmp`, …) whose host is local, directly or as an `imageproxy/` target;
  - a bare local IPv4 address in dotted-quad form or a local IPv6 address, with or without a port (`10.0.0.5:9000`, `[fe80::1]:9000`, `fe80::1%eth0`);
  - an absolute filesystem path of two or more segments (LMS's own artwork and icon paths excepted);
  - a Windows drive or UNC path.

  Slashes inside words ("AC/DC", "1990/91") don't count, nor do numbers such as "1999", "10.5", "v1.2.3", "10:30" or a player's MAC address. Bare addresses in text are matched only as four dotted decimal parts (so "127.1" in a title is left alone), and `0.x.x.x` is ignored there as version-like;
- **local hosts** are: private, loopback, link-local, CGNAT (100.64/10), benchmarking (198.18/15), multicast and reserved IPv4, and `0.0.0.0/8`; IPv6 loopback, unique-local, link-local, site-local and multicast, IPv4-mapped/compatible/NAT64 forms of a local IPv4 address, and zone ids; IPv4 hosts in any form a URL parser or `inet_aton` accepts (`2130706433`, `0x7f000001`, `127.1`, `0177.0.0.1`, percent-encoded dots); a host ending in a number that isn't a valid address; single-label names (`nas`, `localhost`); names ending `.local`, `.lan`, `.home`, `.internal`, `.intranet`, `.localdomain`, `.localhost` or `.home.arpa`; wildcard-DNS services that resolve to an address named in the host (`nip.io`, `sslip.io`, `xip.io`, `localtest.me`, `lvh.me`, `vcap.me`, `traefik.me`, `local.gd`, `localhost.direct`, `lacolhost.com`); and names embedding a private IPv4 address as four labels or four dashed parts (`10.0.0.5.example.com`, `ip-10-0-0-5.example.com`);
- menu entries (`item_loop`) whose text is such a value, like track info's `Location: …` and `URL: …` lines, are dropped whole;
- sync information (`syncgroups ?`, `sync ?`, and `sync_*` fields in `status`) lists only players the caller may control; groups with none are dropped;
- players on other servers (`other_players_loop`, `sn_players_loop`, `other player count`, `sn player count`, and any key starting `other_player`, `sn_player`, `other player` or `sn player`) are removed from every response;
- player lists (`players`, `serverstatus`) show only players the caller may control, with counts adjusted, and lose the server's `version` and `lastscan` (the web app uses neither).

## Transfer (move playback between players)

Clients move what one player is playing (its queue, the current track and position, shuffle order and repeat) to another player, start it there and stop the first. The web app offers it as *Move here* and *Bring here*, and the Android app as *Move playback here* and *Bring to this phone* (planned and run in `mobile/src/transfer.rs`); native apps must use the same recipe. It uses only commands already on the allow-list above, all through `POST /v1/rpc`, so the gateway's player permissions apply to every step: the caller must be allowed to control **both** players, or the first step that names the other one is refused (403) and nothing more is sent.

Below, `S` is the source player (the one playing now) and `T` the target. Both are player ids (MAC addresses) taken from the caller's `players` list. Clients check that each is a well-formed MAC before sending anything, and that `S` ≠ `T`.

### How LMS's `sync` behaves (LMS 9.1, verified)

- `<X> sync <Y>` makes **Y join X's group**. Y first leaves any group it was in, and takes the queue X's group plays: Y's own queue is replaced. If X is playing, Y starts in step with it; if X is paused or stopped, Y is too.
- `<X> sync -` takes X out of its group. X **stops**, but keeps a copy of the queue at the same track (time 0). The players left in the group carry on without a gap.
- A target that is switched off joins the group but stays silent, so it must be switched on first.
- The target keeps its own volume. Shuffle order and repeat come with the queue.
- Radio streams and app items move like library tracks: LMS copies the queue entries themselves, and the target opens the stream afresh.

The order matters: sending `<T> sync <S>` instead would replace the **source's** queue with the target's (empty, typically), and lose it.

### Recipe 1: group, then leave (use this)

1. `S`: `status - 1 tags:…`. Note `mode` and `playlist_tracks`. If `playlist_tracks` is 0 there is nothing to move: stop and tell the user.
2. `T`: `power 1`.
3. `S`: `sync <T>`. T joins S's group and takes its queue.
4. `T`: `status - 1`, until `sync_master` or `sync_slaves` lists `S` **and** `playlist_tracks` equals S's. The web app checks up to 6 times, 250 ms apart; LMS 9.1 has it done by the first check. If this never happens, send `T`: `sync -` and use recipe 2.
5. `S`: `sync -`. S leaves the group and stops; T carries on.
6. `S`: `stop` (it is stopped already; this makes sure).
7. Start T as S was:
   - `play`: nothing to do, T is playing;
   - `pause`: `T`: `pause 0`;
   - `stop`: `T`: `play`.
8. Control T from now on.

If any step fails after step 3, send `T`: `sync -` (best effort), so the two are never left grouped by accident, and report the error.

**Measured against LMS 9.1** with two test players: T played on from the same point (31.5 s at the start, 33.8 s on T 2 s later), with nothing lost. The browser player, through a loopback gateway, moved to a test player and back in under 300 ms of client time per direction.

**Edge cases:**
- **T was in another group:** it leaves that group when it joins (LMS does this itself); the others there carry on with their own queue.
- **S was in a group with others (C):** T joins that group, and after step 5 T and C play on together. The whole group's playback has moved from S to T.
- **T's own queue** is replaced. The web app says so in the *Play on…* sheet.
- **Queue length:** no limit beyond LMS's own; the queue is never sent through the client.

### Recipe 2: copy the list (fallback)

Only if LMS won't group the two (step 4 never succeeds). It can move **library tracks only**: the gateway never reveals URLs, so radio, app and other remote items (no `id`, a negative `id`, or `remote: 1`) can't be loaded by id. Refuse the move and tell the user to start those on the target instead.

1. `S`: `status - 1 tags:…` → `mode`, `time`, `playlist_cur_index`, `playlist_tracks`, `playlist repeat`. Then `S`: `status 0 <playlist_tracks> tags:…` for the queue in play order. Refuse if the queue is empty, if it has more than 500 entries (the gateway pages at most 500), if fewer entries came back than `playlist_tracks`, or if any entry is remote. If `mode` is `play`, add the seconds since the status was read to `time`.
2. `T`: `power 1`, then `playlist shuffle 0` (the list is copied in play order already, and must not be shuffled again), then `playlist repeat <r>` (S's repeat, 0–2).
3. Split the track ids, in order, into lists whose comma-separated form is at most **200 characters** (the gateway's limit for an id list).
   - First list: `T`: `playlistcontrol cmd:load track_id:<ids> play_index:<i>`. Use `play_index:<i>` if the current index `i` falls in this first list, else `play_index:0`.
   - Each further list: `T`: `playlistcontrol cmd:add track_id:<ids>`.
   - If `i` was beyond the first list: `T`: `playlist index <i>`.
4. If `time` ≥ 2: `T`: `time <floor(time)>`.
5. `S`: `stop`.
6. Control T from now on.

Loading always starts T playing, whatever S was doing.

## Diagnostics (`POST /v1/diagnostics`)

A device reports how it plays, for troubleshooting by an admin: browser capabilities, the chosen playback mode and why, player status, and its recent connection log. Bearer token required.
- **Limits:** bodies over 32 KiB get 413, and anything not a JSON object gets 400.
- **Sanitising:** the gateway keeps only a sanitised copy: depth ≤ 4, ≤ 200 keys or items per level, strings ≤ 600 characters with control characters removed, simple keys only.
- **Storage:** the latest report per device, with at most one stored update every 10 s (`202` when skipped). It is deleted when the device is revoked.
- **Reading:** admins use `GET /admin/v1/devices/{id}/diagnostics` (the LMS plugin shows it per device).

## Artwork (`GET /v1/art/<path>`)

Bearer access token required. Browsers fetch art with `fetch()` and display it through blob URLs, so credentials never appear in image URLs.

Allowed paths (validated on the raw, still-encoded path):

| Path | Notes |
|---|---|
| `music/<id>/cover[_WxH[_m]][.ext]`, `music/<id>/thumb…` | Numeric or hex track id |
| `music/current/cover…?player=<mac>` | Only for players the caller may control; not cached |
| `imageproxy/<url-encoded http(s) URL>/image[_WxH[_m]][.ext]` | Only for image URLs LMS itself returned in an image field (`image`, `icon`, `icon-id`, `artwork_url`, `coverart`, `cover`) of an earlier, filtered `/v1/rpc` response, and never for a URL on the home network or the server itself: the host must not be a local host (as defined under response filtering, which covers single-label names and wildcard-DNS names such as `10.0.0.5.nip.io`), and must not resolve, from the gateway within 2 s, to any local address. The lookup can't stop DNS rebinding (LMS resolves the name again when it fetches), and a name that doesn't resolve is judged by the name alone. Anything else would let a user make LMS fetch arbitrary URLs on the home network (SSRF): text such as track titles and player names can be set by users, so it never counts. |
| `plugins/<Name>/html/…`, `plugins/cache/icons/…`, `html/images/…` | Image files only (`png`, `jpg`, `gif`, `svg`); no `..`, `//` or `%` encodings |

Rules for what is relayed:
- only JPEG/PNG/GIF/WebP/SVG responses, at most 20 MiB: a larger declared `Content-Length` gets 502, and a body without one that runs past the limit is cut off (the response fails rather than ending cleanly). Images are streamed, never held in memory whole;
- each user may have 8 artwork requests in flight, and everyone together 32. Further requests wait up to 10 s for a free slot, then get 429 with `Retry-After: 2`;
- the whole LMS exchange, body included, has a 20 s limit;
- every response carries `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`, so an SVG can't run script on the gateway's origin.

## Downloads

Offline downloads of library tracks (spec §5.3, §8). Bearer access token required (401 without a valid one: missing, expired, logged out or revoked). Unlike `/v1/auth/*`, no `X-Anywhere-Client` header is needed: like `/v1/rpc` and `/v1/art`, these take no cookies, so there is nothing to forge cross-site.

### `GET /v1/download/track/{id}/info`

```json
{"id": 123, "key": "<64 hex>", "title": "…", "artist": "…", "album": "…", "album_id": 7,
 "tracknum": 3, "duration": 241.2,
 "original": {"format": "flc", "content_type": "audio/flac", "size": 23456789, "ext": "flac"},
 "formats": ["original", "mp3", "opus"]}
```

- `key`: a stable identifier that survives LMS rescans (which renumber track ids): the hex SHA-256 of a random gateway secret and the track's library URL. It never reveals the file's path. Store downloads by `key`, and re-resolve keys to current ids after a rescan.
- `original.format` is LMS's file type (`flc`, `mp3`, `mp4`, `alc`, `ops`, …). `album_id`, `tracknum`, `duration` and `original.size` are `null` when LMS doesn't know them.
- `formats`: what this user may download this track as. `original` (when permitted) is always offered; `mp3` and `opus` only when the server has `ffmpeg` with the encoder, the user's permission includes them, and the file is of a type the gateway transcodes.
- Responses carry `Cache-Control: private, no-store`.

### `GET /v1/download/track/{id}?format=original|mp3|opus`

The audio (`format` defaults to `original`; any other query is 400).

| Format | What | Content type, extension |
|---|---|---|
| `original` | The library file, relayed from LMS's `/music/<id>/download` (never `/stream.mp3`, which can stream for ever) | From LMS's type, e.g. `audio/flac`, `.flac` |
| `mp3` | MP3, 320 kb/s CBR, ID3v2.3 tags, embedded cover art kept | `audio/mpeg`, `.mp3` |
| `opus` | Opus, 160 kb/s, in Ogg, with tags | `audio/ogg`, `.opus` |

Headers: `Content-Type`, `Content-Length`, `Accept-Ranges: bytes`, `Cache-Control: private, no-store`, and `Content-Disposition: attachment; filename="03 Artist - Title.flac"`. The quoted name is ASCII only (quotes, slashes, backslashes and control characters replaced); when the real name has other characters, an RFC 5987 `filename*=UTF-8''…` follows with the percent-encoded Unicode name (bidi controls removed).

**Ranges:** one `Range: bytes=a-b`, `bytes=a-` or `bytes=-n` gets 206 with `Content-Range: bytes a-b/total`. A malformed range, several ranges, or one past the end gets 416 (with `Content-Range: bytes */total` when the size is known). Resume an interrupted download from the bytes you have.

**Transcodes** are made on first request (the response starts once ffmpeg has finished, which can take a while for a long track), then kept in an LRU cache on disk and served from it, `Range` included. A transcode that fails is 502; one that runs too long (10 minutes) is 504.

### Refusals

| Status | `error` | When |
|---|---|---|
| 401 | `invalid credentials` | No valid access token |
| 403 | `downloads are not allowed for this account` | The admin turned this user's downloads off |
| 403 | `this format is not allowed for this account` | The format isn't among the user's permitted ones |
| 404 | `not found` | The id isn't a positive integer of at most 12 digits, or LMS doesn't know it |
| 404 | `not downloadable` | Not a local library file (a streaming-service or radio track) |
| 404 | `format not available` | No ffmpeg (or no encoder) for this format, or a file type that isn't transcoded |
| 400 | `unknown format` | Any query other than `format=original\|mp3\|opus` |
| 416 | `bad range` | See Ranges |
| 429 | `too many downloads at once` | The user already has 3 downloads running; `Retry-After: 5` |
| 502 / 504 | `music server unavailable` / `music server timed out` | LMS failed or didn't answer in time |

**Safety:**
- An id is checked to be a positive integer and looked up with LMS JSON-RPC (`titles 0 1 track_id:<id>`, 10 s limit) before `/music/<id>/download` is ever requested: for an unknown id, that handler hangs rather than answering.
- A track lookup or download request that LMS fails at the connection level, or answers with a 5xx, is tried once more after 250 ms (both are plain reads); a second failure is 502. Every upstream failure is logged with its cause (no tokens or file paths).
- LMS must answer a download within 20 s and never pause for more than 30 s mid-file; an original is relayed without buffering, never beyond its `Content-Length`, and a body cut short fails rather than looking complete.
- A running download re-checks every 5 s that its device is still signed in, so revoking a device (or disabling its user) ends it.
- The permission is read on every request, so an admin's change applies at once.
- Each download is logged in the audit log (event `download`, detail `track <id> <format>`), at most once per track and device an hour, so resumes and retries don't flood it.
- ffmpeg is run with a fixed argument list (no shell) and an empty environment, its input demuxer forced from a fixed table of LMS file types, `-protocol_whitelist file`, MOV external references off, and input and output files the gateway itself named in its cache directory. At most one transcode runs at once, each output is capped in size, and leftovers are removed.

## Offline plays (`POST /v1/plays`)

Plays made with no connection, from downloaded tracks, sent back when the device is online again (spec D6). Each play the server accepts raises the track's LMS play count and sets its last-played time, as if it had played on a player. Bearer access token required; no download permission is needed. Like `/v1/rpc`, it takes no cookies, so no `X-Anywhere-Client` header is needed.

**Request:** a JSON array of 1 to 200 plays, at most 96 KiB:

```json
[{"key": "01790000000-1b9d6bcd-bbfd-4b2d-9b5d-ab8dfbbd4bed", "track_id": 123,
  "played_at": 1790000000, "seconds": 250.5, "track_key": "<64 hex>"}]
```

| Field | Rule |
|---|---|
| `key` | The idempotency key: made by the client once per play, kept with it, sent unchanged on every retry. 8 to 64 characters of `A-Z a-z 0-9 _ -` (a UUID will do). |
| `track_id` | The LMS track id: a JSON integer, 1 to 999 999 999 999 (not a string, not `1.0`). |
| `played_at` | When playback started, in Unix seconds: a JSON integer. |
| `seconds` | Seconds actually heard (not counting seeks or pauses): a number, 0 to 86 400. |
| `track_key` | Optional, recommended: the download's stable `key` from `/v1/download/track/{id}/info`. If present, the track must still be that file. This protects against a full rescan having given the id to another track. |

Any other field, or a wrong type, makes that play `invalid`.

**Answer (200):** one result per play, in the order sent:

```json
{"results": [{"key": "…", "result": "recorded"},
             {"key": "…", "result": "refused", "reason": "too_short"},
             {"key": "…", "result": "retry", "reason": "server_unavailable"}],
 "retry_after": 60}
```

| `result` | Meaning | Client |
|---|---|---|
| `recorded` | LMS counted it | Delete it |
| `duplicate` | Already counted (an earlier upload got through) | Delete it |
| `refused` | It will never count; see `reason` | Delete it |
| `retry` | Not now; see `reason` | Keep it, and send again after `retry_after` seconds (present whenever any result is `retry`) |

`key` echoes the play's key, or `null` if it wasn't a valid key.

`refused` reasons:
- `invalid`: a malformed play;
- `in_future`: `played_at`, or `played_at + seconds`, is more than 5 minutes ahead of the server's clock;
- `too_old`: more than 90 days ago;
- `not_found`: LMS doesn't know the track;
- `not_library`: not a local library file (radio, streaming services);
- `track_changed`: `track_key` names another file;
- `too_short`: not enough was heard (below).

`retry` reasons:
- `server_unavailable`: LMS didn't answer;
- `server_busy`: LMS is scanning the library;
- `rate_limited`: the hourly allowance is used up;
- `in_progress`: another upload is recording this key right now;
- `busy`: this upload ran out of time;
- `server_error`: the plugin refused or failed;
- `unsupported`: the LMS plugin is missing or too old.

**The counting rule (the scrobblers' rule):** a play counts if `seconds` is at least half the track's duration or 240 seconds, whichever is less. If LMS doesn't know the duration, 240 seconds is needed. Clients may send every play: the server applies the rule.

**What LMS records:** `playcount` + 1, and `lastplayed` = `played_at` if that is later than the current value. This is what LMS does when a track starts on a player, so it never moves `lastplayed` back. LMS's own players count a play when it starts, so their counts and these can differ slightly for skipped tracks.

**Idempotency:** keys are kept per user (not per device, so a device that signs in again still dedupes) for as long as their plays could be accepted (90 days, plus a day). At most 50 000 are kept per user (the newest). Retrying with the same key is always safe. A play whose upload failed before LMS answered is retried by the gateway's next attempt. The plugin also remembers recent keys, so even then it isn't counted twice.

**Other answers:**

| Status | `error` | When |
|---|---|---|
| 400 | `malformed plays` | Not a JSON array, or empty |
| 401 | `invalid credentials` | No valid access token |
| 413 | `too many plays` | More than 200 plays, or a body over 96 KiB |
| 429 | `another upload of plays is running` | One upload at a time per user; `Retry-After: 5` |
| 429 | `too many uploads; try again shortly` | More than 30 uploads a minute per user; `Retry-After: 60` |
| 503 | `play sync is not available` | The gateway has no key to sign plays with |

**Limits:**
- At most 1000 recorded plays an hour per user (beyond that, `retry` / `rate_limited`).
- Each upload spends at most 30 s talking to LMS; plays not reached are `retry` / `busy`.
- Once LMS fails to answer, the rest of the upload is `retry` / `server_unavailable` without further attempts.
- Successful uploads are audited (event `plays`, detail `recorded <n>`).

**How the gateway reaches LMS:**
- LMS has no command that counts a play. The plugin adds one, `anywhere play_record track_id:<id> played_at:<unix> key:<k> ts:<unix> sig:<hex>`.
- The gateway sends it over LMS's JSON-RPC on loopback, like all its LMS calls.
- `key` is the first 32 hex characters of SHA-256 of the user id, a newline and the client's key, so keys are scoped to the user.
- `sig` is the hex HMAC-SHA256, keyed with the gateway's admin secret, of `lyrion-anywhere play_record v1`, the track id, `played_at`, `key` and `ts`, each on its own line. The plugin already reads that secret from the gateway's data directory to call the admin API.
- The plugin refuses anything unsigned, wrongly signed, or with a `ts` more than 2 minutes off. It then checks the track is a local library file, and records the play through LMS's schema (`Slim::Schema::TrackPersistent`), as LMS does for a player. While a scan runs, it answers `busy`.
- Clients can't send `anywhere` commands through `/v1/rpc`: the allow-list reserves them.

**Clients:**
- Queue each play locally, per user, as it ends (finished, skipped or stopped), with a fresh key.
- Send in batches of up to 200, oldest first, when online. Keep each play until it is answered `recorded`, `duplicate` or `refused`.
- Back off on network errors, 5xx and 429 (honouring `Retry-After`), and stop on 401 until signed in again.
- If a whole upload is 400 or 413, split it to find the play the server can't read, and drop only that one.

## Track analysis (`GET /v1/analysis/track/{id}`)

A library track's waveform and loudness, for waveform seek bars and loudness levelling (plan §4, milestone A3). Computed by the gateway with the core (`core::analysis`) the first time it is asked for, then cached. `/v1/me` says whether a server offers it: `"features": {"analysis": true}`.

**Who may ask:** any signed-in user: a bearer access token (401 without a valid one). The waveform is available for everything a user can play, so it does not depend on the download permission. Only local library files: streams and radio are 404. No `X-Anywhere-Client` header is needed.

**Query and encoding:** nothing, `?format=json` or `?format=binary`; anything else is 400 `unknown parameter`. Without a query, `Accept: application/vnd.lyrion-anywhere.analysis` (not `q=0`) asks for the binary form; otherwise the answer is JSON.

**JSON** (`application/json`), about 3 KiB:

```json
{"version":1,"sample_rate":44100,"channels":2,"frames":7938000,
 "integrated_lufs":-9.84,"loudness_range_lu":5.2,"max_momentary_lufs":-6.1,"max_short_term_lufs":-7.9,
 "true_peak_dbtp":0.31,"sample_peak_dbfs":-0.01,"leading_silence_s":0.0,"trailing_silence_s":1.25,
 "fade_in":{"slope_db_per_s":0.4,"start_lufs":-12.0,"end_lufs":-10.2,"length_s":0.0,"fade":false},
 "fade_out":{"slope_db_per_s":-4.1,"start_lufs":-10.0,"end_lufs":-38.6,"length_s":6.3,"fade":true},
 "invalid_samples":0,"waveform":{"peak":"<base64>","rms":"<base64>"}}
```

- Loudness per EBU R128 / ITU-R BS.1770-4 (LUFS, LU), true peak in dBTP (4× oversampled), sample peak in dBFS, all rounded to 0.01. `null` where there is nothing to measure (silence, or too short for a loudness range). Mono files are measured as played: dual mono.
- `waveform.peak` and `waveform.rms`: the same number of buckets (1,000 for any track long enough), each a byte from 0 (silence) to 255 (full scale), linear, base64-encoded.
- Fades are the least-squares loudness slope over the first and last 8 s of audio; `fade` marks a deliberate one.

**Binary** (`application/vnd.lyrion-anywhere.analysis`), about 2.1 KiB, little-endian:

```text
"LAAN"  u16 version (1)
u32 sample rate    u16 channels    u64 frames
f32 ×6  integrated LUFS, loudness range LU, max momentary, max short-term, true peak dBTP,
        sample peak dBFS   (NaN = null)
f32 ×2  leading, trailing silence (s)
edge ×2 fade in, fade out: f32 slope dB/s, f32 start LUFS (NaN = null), f32 end LUFS (NaN = null),
        f32 length s, u8 fade (0 or 1)
u64     invalid (non-finite) samples
u16 N   (at most 10,000)   u8 ×N peak   u8 ×N RMS
```

Clients must treat both forms as untrusted: check the magic, the version, every length (no trailing bytes, at most 20,250 bytes in all), that every float is finite and within ±10⁶ except the NaN "null"s, and that the fade flags are 0 or 1. The web app (`web/app/src/lib/analysis/format.ts`) and the core (`TrackAnalysis::from_bytes`, used by the native apps) do so and are fuzz-tested.

**Caching:** responses carry an `ETag` (different for the two encodings), `Vary: Accept` and `Cache-Control: private, no-cache`; `If-None-Match` gets 304. On the server, results are kept on disk (`<data-dir>/analysis-cache`, 64 MiB by default, least recently used removed first), named by a hash of the stable track key (as for downloads: salted, survives rescans), the file's size and modification time as LMS reports them, and the analysis version. A changed file is analysed afresh.

**While it runs:** a request waits up to 10 s for the result. If it isn't ready by then, the answer is **202** `{"status":"pending"}` with `Retry-After: 3`, and the analysis carries on: ask again. Requests for a file being analysed join that job (one analysis per file at a time). A cached result is never limited.

**Limits:**
- One analysis runs at a time (the rest queue, at most 16 in all). One user may have 2 analyses of their own queued or running, and 8 analysis requests open; beyond either, **429** with `Retry-After: 5`.
- Files over 1 GiB, or tracks over 2 hours, are **413**: refused before fetching when LMS's size or duration says so, otherwise while fetching or decoding.
- An analysis (fetch and decode together) that takes over 5 minutes is abandoned: **504** `analysis took too long`.
- Decoding and analysis run on a blocking thread, never on the async runtime; the deadline is checked between packets. A decoder panic is caught.
- Formats: FLAC, MP3, AAC and ALAC in MP4 (the index may be at the end: the file is read whole and seekable), Ogg Vorbis, Ogg Opus and WAV. Other types (WMA, APE, WavPack, DSD, AIFF, …) are **415** before anything is fetched.
- A file that can't be decoded (corrupt, hostile, or truncated to nothing) is **422** `the audio could not be analysed`. 413, 422 and 504 are remembered for 10 minutes, so the same file isn't fetched and decoded again meanwhile.

**Other answers:** 404 `not found` (a bad or unknown id, as for downloads), 404 `not analysable` (not a library file), 404 `analysis is not available on this server` (turned off with `--no-analysis` or the plugin's setting), 502/504 `music server unavailable`/`timed out` (not remembered).

## Events (`WS /v1/events`)

**Connecting:** the first message is the access token, as text. The gateway replies `{"subscribed":true}`, then sends `{"player":"<mac>","event":[...]}` for the caller's permitted players and `{"player":null,"event":["rescan",...]}` for library rescans. `{"resync":true}` means some events were missed, so re-query state.

**What is forwarded:** only `playlist`, `mixer`, `power`, `client`, `sync`, `pause`, `play`, `stop`, `time` and `newmetadata` events. Any token that looks like a URL or path, or that response filtering would remove (such as a bare LAN address like `10.0.0.5:9000`), is blanked. Preference changes (`prefset`) and everything else are never forwarded.

**Rules:**
- Clients may send nothing after the token. Any message closes the socket (1008).
- Revocation and permission changes are re-checked every 30 s: a revoked device is disconnected, and new grants take effect.
- A device may hold 4 event sockets and a user 16 across all their devices. A new socket beyond either limit closes the oldest one (of that device first, otherwise of that user) with 1008 `replaced by a newer connection`. Clients must not reconnect after it: the web app treats every 1008 other than `unauthorised` as final (the page keeps polling), so two tabs can't keep evicting each other.

**Upstream:** the gateway keeps one connection to the LMS CLI (`--lms-cli-port`, default 9090) while anyone is subscribed, and reconnects with backoff.

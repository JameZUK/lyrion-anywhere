# Lyrion Anywhere player protocol (v1)

The protocol a client (web app, native app or test CLI) uses to act as an LMS player through the gateway. It is implemented by `gateway/` and, on the client side, by `web/app/static/player/player-worker.js` and `cli/src/gateway.rs`.

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
| `POST /v1/auth/login` | `{"username","password","device_name","device_id"?,"refresh"?: "cookie"\|"body"}` | `{access_token, expires_in, user, device}`, plus the refresh token (see below) |
| `POST /v1/auth/pair` | `{"code","device_name"}` | Same, with `refresh_token` in the body (native apps) |
| `POST /v1/auth/refresh` | none (cookie), or `{"refresh_token"}` | A new access token and a **rotated** refresh token |
| `POST /v1/auth/logout` | bearer access token | 204; revokes this device and ends its sessions |
| `GET /v1/me` | bearer access token | `{user, device}` |
| `GET /v1/account` | bearer access token | `{username, role, totp_enabled, downloads: {enabled, formats}}` (`formats`: what this user may download that this server can produce; see [Downloads](#downloads)) |
| `POST /v1/account/totp/setup` | `{"password"}` | `{secret, uri, qr_svg}`: a new secret, active once confirmed. A wrong password is 401 and counts towards the lockout |
| `POST /v1/account/totp/confirm` | `{"code"}` | 204; two-step is now on |
| `POST /v1/account/totp/disable` | `{"password","code"}` | 204; two-step is now off |

- **Access tokens:** random, valid for 15 minutes. Use them as `Authorization: Bearer …` for streams and `/v1/me`, and as the first message on the player socket. Keep them in memory only.
- **Refresh tokens:**
  - single use; each refresh returns a new one, valid for the sign-in lifetime (30 days by default, 1–365 set by the admin in the plugin settings). A device in use therefore stays signed in; one unused for the whole lifetime must sign in again;
  - presenting a used one revokes the whole device, because the token has been copied. One exception: within 60 seconds of its use, while the token that replaced it is still unused, the response was lost (a phone suspended mid-request). The unused replacement is then discarded and a fresh one issued (audit event `refresh.retried`). The discarded replacement is remembered: if it is presented later, two parties held tokens for the device, so it is revoked as for any reuse;
  - rotation is a single database transaction, so two simultaneous refreshes of one token never look like reuse: one rotates it, the other is served by the grace rule;
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
- **Devices:** each login (or pairing) is a device with its own player MAC. A browser can pass its previous `device_id` to keep the same LMS player, as long as the device belongs to the same user. A user has at most 20 active devices: signing in (or pairing) another revokes the least recently seen one and ends its live session (audit event `device.evicted`).
- **Errors:**
  - wrong credentials always return the same 401;
  - repeated failures per username or per client address return 429 with `Retry-After`, with escalating lockouts. An attempt counts from the moment it starts, so parallel guesses can't get past the limit. IPv6 clients are counted per /64, and usernames that can't exist get no entry of their own;
  - at most 4 password checks run at once; beyond that, requests wait briefly and then get 503 with `Retry-After: 1`;
  - malformed or unknown fields return 400, and bodies over 4 KiB return 413.
- **Revocation:** revoking a device, disabling or deleting a user, an admin password reset, logging out, or detecting refresh-token reuse closes that device's live player session immediately (1008, "access revoked").

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
- `GET /admin/v1/devices`
- `GET /admin/v1/devices/{id}/diagnostics` (the device's latest diagnostics report)
- `DELETE /admin/v1/devices/{id}`
- `GET /admin/v1/audit`

LMS's HTTP client cannot send `DELETE`, so each delete also has a `POST` form: `/admin/v1/users/{name}/delete`, `/admin/v1/users/{name}/players/{mac}/remove` and `/admin/v1/devices/{id}/revoke`.

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
- **Ending:** a response ends (and its LMS connection is dropped) when:
  - the session ends (for example, the device is revoked);
  - LMS sends nothing for 60 s (the response fails, so the client sees a network error);
  - the client has a full window unacknowledged and sends no ack for 60 s. It can fetch again with `?offset=`.
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
- `key:value` parameters restricted to known keys per command, each given at most once, with typed values.

| Group | Allowed |
|---|---|
| Library (no player) | `artists`, `albums`, `titles`/`tracks`, `genres`, `years`, `search`, `musicfolder`, `playlists` (list, tracks), `favorites items`, `apps`, `radios`, `players`, `serverstatus`, `syncgroups ?` |
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
- removed everywhere by value, whatever the key, when the text contains (anywhere, not only at the start): a `file:` URL; a URL whose host is private, loopback, link-local or a local name (`.local`, `.lan`, `.home.arpa`, …), directly or as an `imageproxy/` target; an absolute filesystem path of two or more segments (LMS's own artwork and icon paths excepted); or a Windows drive or UNC path. Slashes inside words ("AC/DC", "1990/91") don't count;
- menu entries (`item_loop`) whose text is such a value, like track info's `Location: …` and `URL: …` lines, are dropped whole;
- sync information (`syncgroups ?`, `sync ?`, and `sync_*` fields in `status`) lists only players the caller may control; groups with none are dropped;
- player lists show only players the caller may control, with counts adjusted, and other-server counts removed.

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
| `imageproxy/<url-encoded http(s) URL>/image[_WxH[_m]][.ext]` | Only for image URLs LMS itself returned in an image field (`image`, `icon`, `icon-id`, `artwork_url`, `coverart`, `cover`) of an earlier, filtered `/v1/rpc` response, and never for a URL on the home network or the server itself. Anything else would let a user make LMS fetch arbitrary URLs on the home network (SSRF): text such as track titles and player names can be set by users, so it never counts. |
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

## Events (`WS /v1/events`)

**Connecting:** the first message is the access token, as text. The gateway replies `{"subscribed":true}`, then sends `{"player":"<mac>","event":[...]}` for the caller's permitted players and `{"player":null,"event":["rescan",...]}` for library rescans. `{"resync":true}` means some events were missed, so re-query state.

**What is forwarded:** only `playlist`, `mixer`, `power`, `client`, `sync`, `pause`, `play`, `stop`, `time` and `newmetadata` events. Any token that looks like a URL or path is blanked. Preference changes (`prefset`) and everything else are never forwarded.

**Rules:**
- Clients may send nothing after the token. Any message closes the socket (1008).
- Revocation and permission changes are re-checked every 30 s: a revoked device is disconnected, and new grants take effect.
- A device may hold 4 event sockets and a user 16 across all their devices. A new socket beyond either limit closes the oldest one (of that device first, otherwise of that user) with 1008 `replaced by a newer connection`. Clients must not reconnect after it: the web app treats every 1008 other than `unauthorised` as final (the page keeps polling), so two tabs can't keep evicting each other.

**Upstream:** the gateway keeps one connection to the LMS CLI (`--lms-cli-port`, default 9090) while anyone is subscribed, and reconnects with backoff.

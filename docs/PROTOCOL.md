# Lyrion Anywhere player protocol (v1)

The protocol a client (web app, native app or test CLI) uses to act as an LMS player through the gateway. It is implemented by `gateway/` and, on the client side, by `web/app/static/player/player-worker.js`, `mobile/src/player/` (the native apps) and `cli/src/gateway.rs`.

## Versions

The player protocol has a whole-number version, stated by both sides: the gateway in `GET /v1/me` (`protocol`) and the client in the player socket's sign-in form (`"protocol"`). A step means the two sides must move together: there are no compatibility shims (a standing rule since 2026-10-25: doing it properly overrules backwards compatibility). Everything outside the player socket (sign-in, `/v1/rpc`, `/v1/events`, downloads) is unchanged by a step unless its section says so, so a mismatched pair still controls other players.

| Protocol | Gateway | Clients | What changed |
|---|---|---|---|
| 1 | plugin 0.1 to 0.7.2 | web app to 0.7.2, Android to 1.16.0 | The protocol as first released, with the optional sign-in fields (`quality`, `grace`, `keepalive`) announced through `features`. |
| 2 | plugin 0.7.3 | web app 0.7.3, Android 1.17.0 | `"protocol"` is required in the sign-in form (a client without it is closed with 4426 `update the app`); the gateway sends a `stream` message for every stream it relays (what LMS served, what the gateway sent); `GET /v1/me` carries `protocol`; the devices lists carry each device's `protocol`. |

A client whose `/v1/me` shows a lower `protocol` than it needs should not open the player socket and should tell the user which plugin version the server needs (the apps say "update the plugin to 0.7.3 or later"); a gateway that closes with 4426 is telling the user to update the app.

## Endpoints

| Endpoint | Purpose |
|---|---|
| `WS /v1/player` | Slimproto tunnel, plus session control |
| `GET /v1/stream/<session>/<n>[?offset=k][&limit=m]` | Audio for the session's n-th stream |

Other routes on the public listener:
- `GET /healthz`: returns `ok` with no authentication, for the proxy's or a container's health check. It says nothing about LMS or any user.
- Every other path is the static web app when the gateway serves one (`--static-dir`), else 404. The API routes are the `/v1/…` ones in this document; the admin API is on a separate listener (below). Files under `/_app/immutable/` carry their content hash in their name and are cached for a year; every other file (the page, the service worker, the player's worker, worklet and WebAssembly, which keep their names) is `Cache-Control: no-cache` with a strong `ETag` that is a hash of its content, so `If-None-Match` gets 304 only for the very same bytes, and `If-Modified-Since` alone never does: every file in a release zip carries the date 2020-01-01, and the page's size usually survives a release, so a date (or a date-and-size tag) cannot tell two releases' copies apart.

Requirements for both endpoints:
- They must be reached over TLS through a trusted reverse proxy. Plain HTTP is accepted only from loopback, and from the admin's trusted networks (`--trusted-network`, private networks only, D28): a direct request, with no `X-Forwarded-For` or `X-Forwarded-Proto`, from a peer inside one. A trusted-network peer that sends either header is refused unless it is a trusted proxy saying `https`. Everything else gets 403 `TLS required`. With no trusted networks, only loopback.
- WebSocket upgrades must be same-origin or from an explicitly allowed origin. Clients that send no `Origin` (native apps) are accepted.

### Connection limits (all endpoints)

- A request head must arrive within 15 s, and an idle keep-alive connection is closed after 15 s. The connection is closed without a response.
- A request body (every endpoint takes at most a few KiB of JSON) must arrive whole within 15 s, or the request gets 408 and the connection is closed. Bodies over 1 MiB get 413 whatever the endpoint, and `/v1/rpc` bodies over 8 KiB as soon as that much has arrived (or is declared).
- A `/v1/` path (other than artwork's, which is checked as it stands) whose percent-escapes don't decode to UTF-8 names nothing: 404 `{"error":"not found"}`, before any other check, like any other unknown resource (2026-10-18; before, some endpoints answered 400).
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
| `GET /v1/me` | bearer access token | `{user, device, protocol, features}` (`protocol`: the gateway's player-protocol version, 2 since plugin 0.7.3, see *Versions*; `features.analysis`: the server offers track analysis; `features.resume_grace`: the player socket's sign-in takes `grace`; `features.change_password`: `POST /v1/account/password` is there; `features.devices`: *Your devices* (`GET /v1/devices` and friends) is there; `features.save_queue`: `POST /v1/players/{player}/queue/save` is offered) |
| `GET /v1/account` | bearer access token | `{username, role, totp_enabled, downloads: {enabled, formats}}` (`formats`: what this user may download that this server can produce; see [Downloads](#downloads)) |
| `POST /v1/account/totp/setup` | `{"password"}` | `{secret, uri, qr_svg}`: a new secret, active once confirmed. A wrong password is 401 and counts towards a lockout of its own (per user), separate from sign-in |
| `POST /v1/account/totp/confirm` | `{"code"}` | 204; two-step is now on |
| `POST /v1/account/totp/disable` | `{"password","code"}` | 204; two-step is now off |
| `POST /v1/account/password` | `{"current","new","code"?,"sign_out_others"?}`, bearer access token | `{signed_out, other_devices}`: the caller's own password is changed (see **Changing your password**) |
| `GET /v1/devices` | bearer access token | `{devices: [{id, name, client_kind, client_platform, created_at, last_seen, current}]}`: the caller's own signed-in devices (see **Your devices**) |
| `DELETE /v1/devices/{id}` | bearer access token | 204: signs out another of the caller's own devices (see **Your devices**) |
| `POST /v1/devices/sign-out-others` | bearer access token | `{signed_out}`: signs out all the caller's devices but this one (see **Your devices**) |
| `PATCH /v1/devices/self` | `{"name"}`, bearer access token | `{id, name}`: renames the caller's own device (see **Devices**) |
| `POST /v1/devices/pair` | `{"password"}`, bearer access token | `{code, expires_in}`: a one-time pairing code for another device of the caller's own (see [Pairing links](#pairing-links-native-apps)) |

- **Access tokens:** random, valid for 15 minutes. Use them as `Authorization: Bearer …` for streams and `/v1/me`, and as the first message on the player socket. Keep them in memory only.
- **Refresh tokens:**
  - single use; each refresh returns a new one, valid for the sign-in lifetime (90 days by default, 1–365 set by the admin in the plugin settings). A device in use therefore stays signed in; one unused for the whole lifetime must sign in again;
  - presenting a used one revokes the whole device, because the token has been copied (audit event `refresh.reused`, and the 401 carries `Clear-Site-Data: "cache"`);
  - **grace:** within 60 seconds of a token's use, while none of its replacements has been used, presenting it again is a lost response (a phone suspended mid-request) or a concurrent refresh (two tabs restoring at once). Another replacement is issued *alongside* the first, which stays valid (audit event `refresh.retried`), so whichever one the client keeps works. At most 8 replacements per token; beyond that the refresh is refused (401) without revoking;
  - **siblings:** the first replacement to be used wins. Its unused siblings are superseded at once: their access tokens stop working, and presenting one later is reuse, so the device is revoked. Presenting the original after any of its replacements was used is reuse too;
  - **trade-off:** a thief who copied a refresh token and replays it within 60 seconds of the owner's refresh also gets a valid replacement. They can't both keep the device: whichever side uses its replacement first cuts off the other's access token immediately, and when the other side next refreshes, the device is revoked for both. Before this change, the second refresh discarded the first replacement, which signed out (and revoked) legitimate browsers whose tabs refreshed together. The window is 60 seconds and the thief needs the refresh token itself. This was reviewed and kept deliberately (SPEC D21), so it is not a finding;
  - rotation is a single database transaction, so two simultaneous refreshes of one token never look like reuse: one rotates it, the other is served by the grace rule;
  - the web app also takes a cross-tab Web Lock (`anywhere-refresh`, where supported) around each refresh, so its tabs don't present the same cookie at once;
  - **diagnostics:** a refresh may carry `X-Anywhere-Refresh-Reason` (`load`, `timer`, `401`, `events`, `player`, `retry`, `signout`) and `X-Anywhere-Page` (a random id per page load). They are only logged, beside an 8-hex-digit fingerprint of the token presented and of the one issued (a hash of the stored hash, never the token), so a reuse can be traced to the copy of the app that sent it. Values other than 1–24 lower-case letters, digits and `-` are logged as `-`;
  - **limits:** a device may renew at most 30 times a minute; beyond that the refresh is refused with 429 and `Retry-After`, without using the token up. A device keeps at most 8 live access tokens (the oldest are dropped), and the gateway keeps only the newest 32 used refresh tokens per device for reuse detection, plus none that have expired;
  - clients sign out only when the gateway itself refuses a refresh: a 401 carrying its JSON `error`. Anything else (429, 5xx, or a proxy's or CDN's own error page, of any status) leaves the sign-in, and the native apps' downloads, in place; they retry with backoff;
  - browsers get them in an `HttpOnly; Secure; SameSite=Strict` cookie scoped to `/v1/auth`. The one exception is a plain-HTTP request from a trusted network (D28): there the cookie is set, and cleared on logout, without `Secure`, since browsers drop a `Secure` cookie sent over plain HTTP from anything but `localhost`. On loopback and on every HTTPS request it is unchanged;
  - native apps get them in the body and keep them in the Keystore/Keychain.
- **Two-step verification (optional, per user):** TOTP per RFC 6238 (HMAC-SHA1, 30 s, 6 digits), compatible with standard authenticator apps.
  - **Login:** once a user turns it on, a login with only the right password returns 401 `{"totp_required":true}`. The client then repeats the login with `"totp":"123456"`. A wrong password never reveals whether two-step is on. The login without a code counts against the address's sign-in lock like a failure (audit event `login.totp_required`), so the password can't be checked without limit; the sign-in with the code clears it again. It doesn't count against the username's lock, so the owner's two-step sign-in works while someone else's wrong guesses throttle the username.
  - **Codes:** each code works once; ±1 step is accepted for clock drift. Wrong codes count towards the same lockouts as wrong passwords.
  - **Turning it on** needs the account password, so a stolen access token alone can't add an authenticator.
  - **Turning it off** needs the password and a current code.
- **Changing your password** (`POST /v1/account/password`): a signed-in user changes their own password. Admins set anyone's with `POST /admin/v1/users/{name}/password`, which signs that user out everywhere.
  - **Body:** `{"current": "…", "new": "…", "code": "123456", "sign_out_others": false}`. `code` is needed only when two-step is on; `sign_out_others` is optional (default `false`). Unknown fields, duplicates, wrong types and bodies over 4 KiB are refused (400, or 413 for size). Like the `/v1/auth/*` calls, it needs `X-Anywhere-Client` and an allowed `Origin`, and only a bearer access token is accepted, never a cookie.
  - **The new password** must pass the same policy as an admin's (10–1024 characters) and differ from the current one; otherwise 400 with the reason in `error` (`password must be at least 10 characters`, `password too long`, `the new password must be different from the current one`). These are checked before the current password, so they cost no attempt.
  - **Wrong credentials** are **403**, not 401, so clients don't take them for an expired token and send the same guess again after renewing: `{"error":"the current password or two-step code is wrong"}`, or `{"error":"two-step code required","totp_required":true}` when two-step is on and no code was sent. 401 means only a missing, expired or revoked access token.
  - **Rate limits:** a wrong password or code counts as a failed sign-in, against the account-change lock of this user and device (shared with two-step setup and `POST /v1/devices/pair`; another of the user's devices has its own, so a stolen access token can't lock the owner out of account changes everywhere), the caller's address (which also stops its sign-ins) and the username's sign-in lock. The right password without a needed code counts against the first two, as at sign-in. Five failures lock them: 429 with `Retry-After` (seconds), as at sign-in. Failed sign-ins from that address lock this call too. A success clears only this user's own failures from the address.
  - **On success:** 200 `{"signed_out": 0, "other_devices": [{"id", "name", "client_kind", "client_platform", "last_seen"}]}` with `Cache-Control: no-store`. The caller's device stays signed in. The user's other devices stay signed in too (each holds its own tokens, not the password) and are listed so the user can see them; with `"sign_out_others": true` they are revoked instead and counted in `signed_out`. Pairing codes the user had made and not yet used stop working.
  - **Audit:** `user.password_changed` (detail `self`; an admin's reset has an empty detail) and `password.change_failed` (or `password.change_totp_required` when the code was missing) with the client address. The passwords are never logged.
  - **Detecting it:** `GET /v1/me` lists `"change_password": true` under `features`; clients hide the form on older gateways.
  - **Lost authenticator:** the admin can reset two-step for a user.
  - **Not affected:** refresh and pairing, because the device was verified when it signed in.
- **Devices:** each login (or pairing) is a device with its own player MAC. A browser can pass its previous `device_id` to keep the same LMS player, as long as the device belongs to the same user (the web app always does). A user has at most 20 active devices: signing in (or pairing) another revokes the least recently seen one and ends its live session (audit event `device.evicted`).
  - At most 10 **new** devices per user per hour (reusing one doesn't count); beyond that, 429 with `Retry-After` (audit event `device.rate_limited`).
  - **Device names** are cleaned rather than refused: letters of any script, digits, spaces and `. , - _ ' ’ ( ) &` are kept, everything else (markup, control, bidi and other format characters such as U+202E or U+200B) is removed, runs of whitespace become one space, and the result is cut to 64 characters. Nothing left: `Browser`. The name a player reports to LMS (`SETD`) is cleaned the same way, and replaced by the device name if nothing usable is left.
  - **Renaming:** `PATCH /v1/devices/self` `{"name"}` renames the device the access token belongs to (there is no way to name another one; it needs `X-Anywhere-Client` like `/v1/auth/*`). Admins rename any device through the admin API. A name must be 1–64 characters once trimmed, and is then cleaned as above; one with nothing usable left (e.g. only markup or format characters) is refused with 400, as is one already used (ignoring case) by another of the same user's devices. Other users may use the same name. Access tokens carry the new name at once. If the device has a live player session, the gateway renames its LMS player with LMS's `name` command (LMS then tells the player, as when a player is renamed in LMS) and answers later name queries with it; otherwise the new name reaches LMS in the greeting and name reply when the device next connects. Audit event `device.renamed`.
  - **Your devices** (`features.devices` in `GET /v1/me`; clients hide the page on older gateways): a signed-in user sees and signs out their own devices, never anyone else's, whatever their role (admins manage other users' devices through the admin API).
    - `GET /v1/devices`: `{"devices": [{"id", "name", "client_kind", "client_platform", "created_at", "last_seen", "current"}]}`, the caller's signed-in devices oldest first, with `Cache-Control: no-store`. `current` is `true` for the device the access token belongs to. `last_seen` (Unix seconds, or `null`) is rounded down to the minute; `client_kind` and `client_platform` are `null` when the device never said. Nothing else: no addresses, MACs, User-Agents or tokens.
    - `DELETE /v1/devices/{id}`: signs out one of the caller's other devices: it is revoked (its refresh and access tokens stop working at once), its live player session ends, and LMS is asked to forget its player, as for any revocation. 204 on success. Another user's device, one that doesn't exist or is already signed out, and an id that isn't a plain positive number are all `404 {"error":"no such device"}`, so a user can't learn which ids exist. The calling device itself is refused with `409`: signing out here is `POST /v1/auth/logout`, which also clears the browser's refresh cookie and cached data.
    - `POST /v1/devices/sign-out-others` (no body): signs out every device of the caller's but the one asking. `{"signed_out": n}` with `Cache-Control: no-store`.
    - **Protection:** a bearer access token only, never a cookie (401 otherwise). The two that sign out need `X-Anywhere-Client` and an allowed `Origin`, like `/v1/auth/*` (403 otherwise). No password is asked for: signing a device out only takes access away, and the device asking can't be signed out this way. Together the three calls allow 30 requests a minute per device asking; beyond that, 429 with `Retry-After`. Per device, not per user, so someone holding a stolen device's token can't use up the allowance its owner needs, on another device, to sign it out.
    - **Audit:** `device.revoked` with the user's name and detail `self from <id>`, naming the device that did it (an admin's revocation has neither). Signing out other devices with a password change is audited the same way.
  - **Client fields** (optional, on login, pair and refresh): `"client_kind"`: `"pwa"` (the web app in a standalone display mode, i.e. installed), `"browser"` (a browser tab) or `"android"`; anything else is a 400. `"client_platform"`: a short description such as `"Safari on iPadOS"` or `"Pixel 8"`, at most 48 characters (longer is a 400), cleaned like a device name. Fields left out keep what the device reported before. The gateway also keeps the `User-Agent` header, cut to 200 characters of printable ASCII, without `<`, `>`, `"`, `'`, `&`, backslashes and backticks. Clients from before these fields send none; a client meeting an older gateway (which refuses unknown fields as `malformed request`) may repeat a sign-in without them, as the Android app does. The web app sends them on every sign-in and renewal; the Android app on sign-in and pairing.
  - **Last seen:** each device's last client address (as resolved under **Client address**, the full address rather than the /64 used for lockouts) and time are recorded at sign-in, pairing and refresh, and on authenticated API requests and player connections at most once a minute per device. Only the admin API's device list shows the address and User-Agent; a user's own list (**Your devices**) has the client fields and the last-seen time to the minute, never an address. Revoking a device clears its address and User-Agent at once; its row (name and client type) goes with the other revoked devices.
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
- **Origins:** an `Origin` is accepted if it is the request's own host over HTTPS, or listed with `--allowed-origin`. `http://` same-origin is accepted only on plain-HTTP requests, which the gateway serves only without proxy headers, on loopback (local development) or from a trusted network (D28), so the web app served at `http://10.8.0.1:9443` needs no `--allowed-origin`.
- **`Clear-Site-Data: "cache"`** on sign-out (and on the reuse refusal) clears cached artwork and responses from the browser. It leaves IndexedDB alone, so offline downloads stay; browsers may also empty the service worker's caches, which it refills.

### Pairing links (native apps)

A pairing code can be handed to a phone as a QR code holding a link, or as the link itself (tapped in a message):

```
lyrion-anywhere://pair?server=<percent-encoded server address>&code=<pairing code>
```

For example `lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK`.

- `code` is required: 4–32 letters and digits once spaces and dashes are removed (case-insensitive), then redeemed with `POST /v1/auth/pair`. The gateway makes 10-character codes from `A–Z` (without `I` and `O`) and `2–9`.
- `server` is optional (`https://music.example.com`). Apps validate it exactly as a typed address: `https`, or plain `http` only to a literal private address (`http://10.8.0.1:9443`, never a name, D28; the Android app warns before signing in) or, in development builds of the Android app, to loopback. No user name or password, query, fragment or path.
- The link itself is exactly `lyrion-anywhere://pair?…`: no user name, password or port on `pair`, no path, no fragment. Any other parameter, a repeated one or one without a name makes the link invalid. A bare code (no link) is also accepted when typed.
- **The QR code** holds the link as UTF-8 in byte mode, error correction level M, version 10 or smaller (at most 213 bytes). The web app and the plugin's settings page make it with the project's own encoder (`web/app/src/lib/qr/qr.js`), with a quiet zone of four modules.
- **Where codes come from:**
  - an admin: *Add a device* next to a user in the plugin's settings (admin API `POST /admin/v1/users/{name}/pairing`), or `anywhere-gateway admin pair NAME`. The plugin builds the link from its *Public URL*: an `https://` address with nothing after the host, or `http://` with a literal private address and an optional port (a gateway reached over a VPN, D28). Without one of these (an `http://` name or public address included) it shows the code alone and says why;
  - the user themselves: the web app's *Settings › Account › Add another device*, which calls `POST /v1/devices/pair` and builds the link from the page's own address.
- **`POST /v1/devices/pair`** `{"password"}`, with a bearer access token and `X-Anywhere-Client` (like `/v1/auth/*`). Returns `{code, expires_in}` (600) with `Cache-Control: no-store`.
  - The code is always for the token's own user: there is no field to name another (unknown fields are a 400).
  - The account password is required, so a stolen access token alone (15 minutes) can't be turned into a lasting sign-in on another device. A wrong one is 401 and counts towards the per-user lock that also guards two-step changes (429 with `Retry-After` once locked; sign-in isn't affected).
  - At most 5 codes an hour per user, and none while the user is at the new-device limit (10 an hour, see **Devices**), since such a code couldn't be redeemed: 429 with `Retry-After`. These are checked before the password.
  - Codes are single use and valid for 10 minutes, wherever they were made; redeeming one is subject to the pairing lockouts (per client address, and across all clients). Only a hash is stored; expired codes are deleted as new ones are made.
  - Audit events: `pairing.created` (detail `admin` or `self`, with the device that asked), `pairing.rate_limited`, `pairing.self_failed` (wrong password), and `pairing.ok` / `pairing.failed` on redemption. The code itself is never logged or audited.
- **What apps do with a link:** it only fills in the pairing screen. The Android app shows the server (and the one it would replace) and pairs only when the user confirms; if a sign-in is kept, it first asks whether to sign out (downloads are kept). A link that isn't valid is ignored, and the user told.

### Admin API

The admin API is served on a separate listener (default `127.0.0.1:9445`) that must never be put behind the proxy. It is used by the LMS plugin's settings page.

Requests are refused unless they come from loopback, carry no `X-Forwarded-*`/`Forwarded`/`X-Real-IP` headers, and present the admin secret (`<data-dir>/admin.secret`, mode 0600) as a bearer token.

Endpoints:
- `GET/POST /admin/v1/users`
- `DELETE /admin/v1/users/{name}`
- `POST /admin/v1/users/{name}/password` (also revokes the user's devices, signing them out everywhere)
- `POST /admin/v1/users/{name}/disabled`
- `POST /admin/v1/users/{name}/pairing` (returns `{code, expires_in}`: a one-time code, valid 10 minutes; see [Pairing links](#pairing-links-native-apps))
- `POST /admin/v1/users/{name}/totp/reset` (removes two-step for a user who lost their authenticator)
- `POST /admin/v1/users/{name}/downloads` `{"enabled": bool, "formats"?: ["original","mp3","opus"]}` (download permission; `formats` must name one to three of these and is kept as it was when left out). `GET /admin/v1/users` lists `downloads` and `download_formats` per user, and `GET /admin/v1/settings` lists the `download_formats` this server can produce
- `GET/POST /admin/v1/users/{name}/players`, `DELETE /admin/v1/users/{name}/players/{mac}` (home players a listener may control)
- `GET /admin/v1/devices`: each active device's `id, username, name, mac, created_at, last_seen`, and `last_addr` (the client address it was last seen from), `client_kind`, `client_platform` and `user_agent` (each `null` until known; see **Devices** under Authentication)
- `PATCH /admin/v1/devices/{id}` `{"name"}` (also `POST /admin/v1/devices/{id}/name`): rename any device, with the same rules as `PATCH /v1/devices/self`. Returns `{id, name}`; 404 for an unknown or revoked device
- `GET /admin/v1/devices/{id}/diagnostics` (the device's latest diagnostics report)
- `DELETE /admin/v1/devices/{id}`
- `GET/POST /admin/v1/settings`: `{"session_days"}` sets the sign-in lifetime (1–365 days). The GET also returns `session_days_default` (90), `session_days_min`, `session_days_max` and the `download_formats` this server can produce
- `GET /admin/v1/audit`
- `GET/POST /admin/v1/plugins`: the plugins reserved for admins (see **Admin-only plugins** under [Browse and control](#browse-and-control-post-v1rpc)); the GET also lists what LMS offers as apps, radio and home-menu entries
- The built-in Cloudflare Tunnel (docs/plans/cloudflare-tunnel.md, SPEC D27). Errors are `{"error": "…"}`, a sentence for the admin: 400 bad input, 409 wrong state (busy installing or creating, `start` with nothing registered, `quick` while a registered tunnel runs, `install` of a new version while running, or any POST when the tunnel is unsupported or turned off with `--no-tunnel`), 500 the gateway could not write its data. A 202 answers `{}`.
  - `GET /admin/v1/tunnel`: the status, every key always present: `supported` (bool; false on a platform other than Linux x86_64/aarch64/armhf, with `--no-tunnel`, or when the tunnel's own listener could not be opened), `pinned_version`, `installed_version` (string or null), `update_available` (bool), `mode` (`none`, `registered`, `quick`), `state` (`unsupported`, `not_installed`, `installing`, `installed`, `creating`, `starting`, `connected`, `reconnecting`, `stopped`, `failed`), `hostname`, `public_url` (`https://<hostname>` while a tunnel is registered, the `trycloudflare.com` address while the temporary one runs, else null), `connections` (edge connections, 0 when not connected), `uptime` (seconds since `cloudflared` started, or null), `quick_ends_in` (seconds left of the temporary tunnel's hour, or null), `progress` (`{bytes, total}` while installing, `total` may be null), `step` (`zone`, `create`, `configure`, `dns` or `token` while creating), `error` (set when `failed`, and when `unsupported` because the tunnel's own listener could not be opened: a sentence naming its address), `tunnel_listen` (the tunnel's own listener as bound, e.g. `"127.0.0.1:9444"`, the service `cloudflared` is given; null when there is none) and `log` (the last 50 scrubbed lines of `cloudflared`'s output, oldest first). A registered tunnel the admin stopped is `stopped` with `mode: "registered"` and its hostname; after a gateway restart it is `installed`, still registered
  - `GET /admin/v1/tunnel/log`: `{"log": [...]}`, up to 200 scrubbed lines, oldest first
  - `POST /admin/v1/tunnel/install`: 202, the pinned `cloudflared` is downloaded from GitHub and checked against the SHA-256 compiled into the gateway; 204 if it is already installed (or `--cloudflared` names a system one)
  - `POST /admin/v1/tunnel/update`: 202, stops a running tunnel, installs the pinned version and starts it again; 204 when it is already the pinned version
  - `POST /admin/v1/tunnel/register` `{"token", "hostname"}`: 204, the tunnel token (from Cloudflare's dashboard) and hostname are stored and the tunnel starts, installing first if needed
  - `POST /admin/v1/tunnel/create` `{"api_token", "hostname", "replace_dns"?}`: 202, the gateway creates the tunnel through Cloudflare's API in the background (`state: "creating"`, `step` names the step), stores only the tunnel token and hostname and starts it; a failure is `failed` with the step and Cloudflare's message in `error`, and a tunnel it created is deleted again. The API token (scopes: Account › Cloudflare Tunnel › Edit, Zone › DNS › Edit, Zone › Zone › Read) is held in memory for the exchange only. `replace_dns` (default false) overwrites an existing DNS record for the hostname
  - `POST /admin/v1/tunnel/start`: 204, starts the registered tunnel (stopping a temporary one); 409 if none is registered
  - `POST /admin/v1/tunnel/stop`: 204, stops the registered or temporary tunnel; a registered one then stays stopped across gateway restarts
  - `POST /admin/v1/tunnel/quick`: 202, installs if needed and starts a temporary `trycloudflare.com` tunnel, which stops itself after one hour and is never stored or restarted
  - `POST /admin/v1/tunnel/remove`: 204, stops the tunnel and deletes everything under `<data-dir>/tunnel/` (the binary and the token)

  `cloudflared` sends the tunnel's requests to the gateway's tunnel listener (`--tunnel-listen`, default `127.0.0.1:9444`, loopback only, not opened with `--no-tunnel`), which serves the same routes as the public listener. Only requests on it have the client address read from `CF-Connecting-IP` (a single valid address; otherwise the `X-Forwarded-For` rule), and only they are accepted from `https://<hostname>` while a tunnel runs. The public listener behaves as configured whether a tunnel runs or not.

LMS's HTTP client cannot send `DELETE` or `PATCH`, so the rename has a `POST` form (above) and each delete has one too: `/admin/v1/users/{name}/delete`, `/admin/v1/users/{name}/players/{mac}/remove` and `/admin/v1/devices/{id}/revoke`.

Any other path on the admin listener is 404, and only once the request has passed the checks above (without the secret it is refused like any other).

## Player socket

### Opening

The client's first message is text, sent within 5 s:

- **New session:** the bare access token, or `{"token":"…"}`.
- **Resume:** `{"token":"…","resume":{"session":"…","secret":"…","received":N}}`. `N` is the number of binary messages the client received in this session before the drop.
- **Protocol version (required):** either form carries `"protocol":2` (a positive whole JSON number; see *Versions*). A form without it, with a lower number or with anything else (`"2"`, `2.0`, `-1`, `0`, `true`, `null`, an array, an object) is an old or broken client: the gateway checks the token first, then closes with **4426** `update the app` without opening (or resuming) a session, and audits `player.too_old` with the value given. A higher number is accepted: the newer client decides what to make of an older gateway.
- **Stream level (optional):** either form may add `"quality":"original"|"high"|"low"`, the level of the streams to come (see *Lower-data streams* below). An unknown level is taken as `original`, never refused. Gateways before this feature ignore the field.
- **Resume grace (optional):** either form may add `"grace":S`, the seconds the client would like the session held after its socket drops (see *Resume*). `S` must be a whole number from 0 to 3600; anything else (a string, a fraction such as `60.0`, a negative or larger number, `null`, …) is a policy violation (close 1008): a new session isn't opened, and a resumable session (right device and secret) is ended. The gateway grants `min(S, its maximum)`; the maximum is 600 s unless the operator sets `--max-resume-grace` (60–3600 s). Without the field, a new session gets the default of 60 s and a resumed one keeps what it was granted before. Send it only to a gateway whose `GET /v1/me` lists `"resume_grace": true` under `features`; older gateways ignore it and hold sessions for 60 s.
- **Text keepalives (optional):** either form may add `"keepalive":true`. The gateway then also sends the text message `{"keepalive":true}` every 5 s (see *Keepalives* below). Anything but a JSON `true` means no. Ask only when `GET /v1/me` lists `"lms_busy": true` under `features`; older gateways ignore it.

The gateway replies with a text greeting:

```json
{"session":"<32 hex>","secret":"<32 hex>","mac":"02:…","name":"…","resumed":false,"grace":60}
```

- `mac` and `name` are the player identity assigned to this device. The client must use that MAC in HELO.
- `secret` is needed to resume this session. Treat it like a credential.
- `grace` is the resume grace granted, in whole seconds (older gateways leave it out: assume 60). A client should stop resuming a little before it runs out and start a new session instead.
- **A new player starts at full volume.** LMS gives a player it has never seen a volume of 50. The first time a device's player attaches, as soon as LMS knows it (its first frame after HELO), the gateway sends LMS `mixer volume 100` for that player, once per device (remembered in the gateway's database, so a level chosen later is kept; devices that existed before this behaviour are left alone). A client has nothing to do.

### Messages after the greeting

**Client → gateway:**
- **Binary messages:** the client → server Slimproto byte stream, with any chunking.
  - A new session must start with HELO carrying the assigned MAC and device ID 12 (software player).
  - A resumed session must not send HELO again.
  - Only the frames a software player sends are accepted: HELO, STAT, RESP, META, DSCO, `BYE!`, and SETD as the answer to an LMS name query (`setd` id 0 with no data), one answer per query.
  - The gateway rewrites two things before LMS sees them:
    - HELO capabilities keep only well-formed tokens: printable ASCII, comma-separated, each a bare codec name (`flc`) or `Key=value`, at most 64 tokens and 1 KiB in all;
    - a SETD name that is empty, longer than 64 characters, has control characters, or looks like a URL or path is replaced by the device's name.
- **Text messages:**
  - The stream acknowledgement `{"ack":{"stream":n,"bytes":b}}`. Send one at least every 1 MiB received on stream `n`, and once straight after resuming.
  - The stream level `{"quality":"original"|"high"|"low"}`, for the streams that follow (the next `strm s`). Only to a gateway whose `GET /v1/me` lists `"stream_quality": true` under `features`: older gateways end the session on it.

**Gateway → client:**
- One binary message per server → client Slimproto frame, length prefix included, exactly as LMS sent it.
- `serv` (redirect to another server) is never forwarded.
- **The stream message** (protocol 2): one text message for every stream the gateway relays for this session, sent as soon as the gateway has LMS's response for it (so during the first bytes, after the client asked for the stream), and sent again for the stream in progress right after a resume greeting:

  ```json
  {"stream":{"seq":12,"served":{"codec":"flac","rate":96000,"bits":24,"channels":2,"gain_db":-3.2},"sent":{"mode":"opus","kbps":160,"level":"high"}}}
  ```

  - `seq` is the count of `strm s` frames the gateway has forwarded in this session, from 1, so the client ties the message to the stream it is fetching; frames replayed after a resume are not counted again.
  - `served` is LMS's stream as it reached the gateway: `codec` from the `strm` format byte (`flac`, `pcm`, `mp3`, `ogg`, `aac`, `alac`, `opus`, or `unknown`); keys may come in any order; for FLAC, `rate`, `bits` and `channels` read from the STREAMINFO block at the head of the stream (a headerless stream, such as LMS's bare-frame seek, leaves them `null`); for PCM, from the `strm` fields; `gain_db` is LMS's server ReplayGain for the track, absent when there is none. Anything unknown is `null`.
  - `sent` is what the gateway did: `{"mode":"passthrough"}`, or `{"mode":"opus","kbps":160,"level":"high"}` / `{"mode":"opus","kbps":96,"level":"low"}` when it re-encoded the stream (see *Lower-data streams*).
  - It is text, so it is never counted in `received`, like the keepalive. It is at most 512 bytes and carries only the tokens and numbers above. The clients use it for the quality badge and the signal-path diagram (SPEC §7.2, D30).
- A refused `strm s` (another address, or a request that isn't this player's own LMS stream) is still forwarded so the client stays in step, but with its address zeroed and its HTTP request removed; fetching it is a 404.
- **Client notes:**
  - On a seek LMS sends `strm q` and the new `strm s` back to back (about 2 ms apart for MP3), so a client sees them together. Drop queued audio on the `strm q` itself, not on a later check of the player state.
  - `strm f` is not a full flush. LMS sends it when the next track, already streamed, is deleted or moved in its queue, and it keeps the current track playing. Drop only what came after the audible track, keep playing, and answer `STMf`. That includes audio already handed to the output: the core reports it through `trim_count()` / `trim()`, and the web player's WASM host through `take_trim()`.
  - `audg` carries LMS's volume as linear gains (16.16) on LMS's own curve. A client applying it should do so at its output, after any queued audio.

### Keepalives

The gateway pings every player socket every 5 s, whatever LMS is doing. When LMS is blocked (see *When LMS is busy*), the socket is otherwise quiet, and a client that treats a quiet socket as dead would give up on a working connection. Pings prove the gateway is there; they say nothing about LMS.

- A client that sees pings (native apps) can count them as traffic.
- Browsers don't show pings to scripts, so a web client asks for text keepalives at sign-in (`"keepalive":true`) and gets `{"keepalive":true}` every 5 s as well. It is not a greeting (no `session`) and not a frame (not binary): it only refreshes the client's "last heard" time and is never counted in `received`.
- The client's pongs are not taken as the player speaking: a browser answers pings even while its scripts are suspended, and the gateway must keep reporting to LMS for such a client (as for a detached client, see *Resume*).

### Policy violations (close 1008, session ended)

Any of these ends the whole session, not just the socket:
- a first frame other than HELO;
- a HELO with another MAC;
- a second HELO;
- a HELO with a device ID other than 12;
- any client opcode outside the list above (IR, BUTN, KNOB, BODY, UREQ, SHUT, …), a SETD LMS didn't ask for, or a malformed STAT or DSCO;
- a frame over 16 KiB;
- any text message other than a well-formed ack or level: one of those two fields exactly, with a known level, at most 256 bytes;
- a sign-in `grace` that isn't a whole number from 0 to 3600 (see *Opening*).

### Close codes

| Code | Meaning | Client should |
|---|---|---|
| 1001 | LMS closed the connection, or the session expired | start a new session. A browser can't tell the gateway's 1001 from one it made itself (for example when suspending a page), so the web player tries to resume first and starts a new session on 4404 |
| 1006 | Connection lost (no close frame) | resume |
| 1008 | Unauthorised, or policy violation | stop; don't retry with the same credentials |
| 1013 | Server unavailable, or client too slow | resume (with backoff) |
| 4000 | Another connection for this device took over | stop |
| 4404 | Resume refused: unknown/expired session, wrong secret, or frames no longer available | start a new session |
| 4426 | The client is too old for this gateway: its sign-in form had no `protocol`, or a lower one than the gateway requires (see *Versions*) | stop; tell the user to update the app. Never retry with the same form |

## Resume

When the socket drops (anything other than a policy close):

1. The gateway keeps the session and its LMS connection for the **resume grace**: 60 s, or what the client asked for at sign-in, up to the server's maximum (600 s by default; see *Opening*). LMS never sees the player leave.
   - A longer grace changes nothing else: the session still holds one LMS connection, a new (non-resume) session for the device still ends it at once, revoking the device ends it at once, and the frame log below keeps its bound.
2. While detached, the gateway reports to LMS on the player's behalf every second, and answers LMS status polls. It replays the client's last STAT with refreshed timestamps: elapsed time advances by wall time if the client was playing, and stays frozen if it was paused or stopped (`STMp`, `STMf`, `STMn`). Without this, LMS drops a silent player after about 15 s.
   - **The play point never looks stalled:** the elapsed time keeps pace with the clock, past the end of the track LMS knows and after an `STMu`, because a client with a rolling cache plays on into the next track while it is away. Until 2026-10-15 it stopped at the track's end while the clock ran on; to LMS the player looked stalled, and in a sync group LMS sent it a `skipAhead` every 2 s for minutes and corrected the other members against it. How far it runs is bounded by the grace.
   - **Sync corrections aren't kept:** LMS's `strm a` (skip ahead), `strm p` with an interval (pause for) and `strm u` with a start time, sent while the client is away, are dropped rather than queued for the resume: they describe the moment they were sent, and replayed late they would make the player jump by their sum. `strm p` and `strm u` with 0 (pause, unpause now), `strm q`, `strm f`, `strm s`, `audg`, `setd` and the rest are kept as before. Status polls (`strm t`) are answered by the gateway, as above.
   - It does the same for a socket that is still attached but has sent nothing for 8 s. A working client answers LMS's status polls every 5 s, so silence means its socket may have died without either side noticing (a phone changing networks). The heartbeats stop as soon as the client sends anything.
   - **Stale heartbeats are dropped:** for 5 s after its last heartbeat, the gateway forwards no client `STMt` whose jiffies are behind that heartbeat's: LMS has already been told a later clock. A client resuming after a gap may send what it held all at once; LMS re-bases its clock for the player on each `STMt`, so a backlog of them, a second apart in jiffies but arriving together, throws a sync group out (live, 2026-10-15: 257 re-basings in one millisecond, then 178 `skipAhead` corrections in 15 minutes). Every other frame, however old (`STMs`, `STMd`, `STMu`, …), is forwarded. Clients should not hold heartbeats in the first place: send a fresh one on resuming.
   - **Clients:** LMS polls every player every 5 s, so a working socket never stays quiet for long. Treat about 20–25 s with nothing received as a dead socket: abandon it and resume. Count only what arrives; your own messages going out prove nothing.
3. Frames LMS sends meanwhile are kept in a numbered log, together with recently sent frames, which may have been lost in flight. On resume the gateway re-sends every frame after the client's `received` count, in order.
4. The log is bounded (default 1 MiB). If frames the client missed have been discarded, or the client claims frames that were never sent, the resume is refused (4404) and the client starts afresh.
5. A resume while the old socket is still open replaces it (close 4000). On mobile, dead sockets often linger. From that moment nothing the old socket sends reaches LMS.
6. A client that can't keep up with LMS's frames (its outgoing queue fills) has its socket closed with 1013 and is treated exactly like a dropped connection: heartbeats and the grace period start, and it can resume.
7. A new (non-resume) session for the same device ends any previous session, so LMS never sees two players with one MAC.

Recommended client backoff: about 0.5, 1, 2 and 4 s, then every 5 s, with ±20 % jitter. Stop resuming shortly before the grace expires, then start a new session.

What the Android player does (`mobile/src/player/engine.rs`), as an example:
- **Grace:** it asks for `"grace":600` when `/v1/me` lists `resume_grace`, and resumes until 5 s before the grace in the greeting runs out (55 s when the greeting has none, or an out-of-range one).
- **Stream downloads:** a broken download is retried from its offset, with the backoff above, until the granted grace has run out since the first failure in a row (60 s without a grant). A 4xx other than an expired token fails the track at once.
- **No network:** the app reports a loss only once no default network has come back for 1.5 s, and the engine keeps a live socket through a loss that ends within 2 s more. Android reports the default network lost and back within a moment whenever it picks it again, and dropping the socket for that cost a reconnect about once a minute (2026-10-15). A loss that lasts closes the socket with a WebSocket close frame, never just dropping it. The engine then stops trying until the network is back and then tries at once with the backoff started again (the grace still counts from the close). A "gone, back" while already reconnecting (the app's *reconnect now*) also retries at once. Stopping sends `BYE!` and then a close frame, and a close from the gateway is answered before the socket is let go.
- **While detached:** its STATs wait in an outbox of 2000 frames (the oldest goes when it is full). `STMt` heartbeats are never held: the gateway speaks for the player meanwhile, and the player sends a fresh one within a second of resuming (held ones, sent together, threw a sync group out; see step 2 above). `STMs`, `STMd`, `STMo`, `STMr`, `STMu` and the rest are sent in order when the session resumes, with one exception in a sync group: a mid-track `STMo` held through the gap is dropped (see *Synchronised playback › Rejoining after a gap*).

## Streams

- **Numbering:** stream `n` is the n-th `strm s` frame the client received in the session, counting from 1. Refused streams count too.
- **What the gateway fetches:** only the request LMS itself put in that `strm`, after checking it is exactly `GET /stream.mp3?player=<this MAC>` on an LMS address.
- **Ownership:** the upstream LMS connection belongs to the session, not the HTTP request.
  - **Resume:** a broken download resumes with `?offset=k`, where `k` is the number of bytes the client has.
  - **First bytes only:** `?limit=m` (1 to 16 MiB) sends at most `m` bytes from the offset and then ends the response cleanly; the rest follows with `?offset=`, from the same upstream. A `limit` of 0, above 16 MiB or not a whole number is `400`. Older gateways ignore it and send everything. The phone's replay cache uses it to fetch a stream's first 96 KiB, look them up, and fetch nothing more on a hit. So does its use of downloaded originals: an Original FLAC stream whose STREAMINFO matches a download fetches the same first bytes, and the rest comes from the download.
  - **Supersede:** a newer request for the same stream supersedes an older one.
- **FLAC artwork is dropped:** for a stream LMS announced as FLAC (`strm` format `f`), the gateway removes PICTURE and PADDING metadata blocks from the start of the stream, so players needn't download embedded artwork (often 50–500 KB) before the first audio frame. Artwork is still served by `/v1/art`.
  - Only the metadata chain changes: the last-block flag moves to the last block kept; STREAMINFO, SEEKTABLE, VORBIS_COMMENT, CUESHEET, APPLICATION blocks and every audio byte are forwarded unchanged, so the decoded audio is bit-identical.
  - The metadata is held (at most 16 MiB and 1024 blocks) until its end has been checked. Anything unexpected sends the stream on exactly as LMS sent it: no `fLaC` marker (e.g. a stream that starts mid-file), a first block that isn't a 34-byte STREAMINFO, a second STREAMINFO, a reserved or invalid block type, a malformed SEEKTABLE length, too much metadata, no frame sync straight after the last block (a lying last-block flag), or the stream ending first.
  - **Offsets** (`?offset=`, acks, the replay window, `416`) count the stream as the client receives it, stripped. The gateway asks LMS for each stream once and strips it once, so every request for it sees the same bytes, and resuming works as for any stream.
  - **STAT:** LMS counts the bytes it sent, so the gateway adds the bytes it removed to `bytes_received` in the client's STATs once the count is past the stripped metadata.
- **STAT `bytes_received` is clamped** to what the stream's relay has sent the client (then mapped to LMS's count as below). A player that played the rest of a track from its replay cache, or from a downloaded original, reports the whole stream, though the relay read only its start; LMS is never told of bytes it didn't send. The next `strm` cancels the half-read relay as usual.
  - `--keep-flac-pictures` turns this off.
- **Lower-data streams** (docs/plans/streaming-quality.md): when the session's level is `high` or `low` and LMS announces a FLAC stream (`strm` format `f`), the gateway re-encodes it to **Ogg Opus** (160 or 96 kb/s, 48 kHz stereo) on the way through.
  - **What the player sees:** the `strm` it is sent says Ogg Opus: format `u`, PCM parameters `?`.
  - **What is never re-encoded:** other formats pass through as they are, and so does everything when the gateway has no `ffmpeg` with libopus, or is already running `--live-transcode-jobs` encodes (default 4; 0 turns the feature off). The player can tell by the format code in the `strm` it received.
  - **Offsets and STAT:** offsets, acks, the replay window and resume all count the Opus stream; LMS is asked once and the stream encoded once. In STATs, `bytes_received` is turned back into LMS's count, in proportion to what has been fed and produced, and in full once the client has the whole finished stream.
  - **How `ffmpeg` runs:** with a fixed argument list, no shell and an empty environment, the input demuxer forced to FLAC, and only the `pipe` protocol allowed. It is killed when its stream is abandoned.
  - **Deterministic:** the encode is bit-exact (`-fflags +bitexact -flags:a +bitexact`: no version strings, a fixed Ogg stream serial), so the same track at the same level is the same bytes every time and the phone's replay cache can match it.
  - **Failures:** a failed encode ends the stream with an error, never cleanly; if the encoder can't start at all, the response is 502.
- **A FLAC seek is bare frames.** LMS serves a seek into a FLAC file by jumping to the frame boundary (`Slim::Formats::FLAC::findFrameBoundaries`, via Audio::Scan) and streaming from there; its stock `flc flc * *` rule is a pass-through and FLAC, unlike Movie, Wav, Ogg and MP3, has no `getInitialAudioBlock`, so the stream has no `fLaC` marker and no STREAMINFO. The core makes the header from the first frame (checked against the second) before probing (`core::decode`); a `custom-convert.conf` rule that re-encodes through `flac --skip` yields a header-first stream instead, which is also fine. The gateway passes either on unchanged (the FLAC stripper only acts on a stream that begins with `fLaC`).
- **Pacing:** the gateway reads from LMS at most one **replay window** (default 4 MiB) beyond the client's last ack. Network buffers therefore can't hold more than the window. The bytes kept for replay always reach back to the last ack (the read that fills the window may bring up to a chunk more, and the window then grows by that much until the next ack), so a resume from the byte received always lands inside it. A client should acknowledge only what it has consumed, not what it has read ahead: the web player reads a response as it arrives into its own queue, whatever its decoder takes just now (Safari stops delivering a body whose consumer paused), and the window then bounds that queue. It gives up a response that brings nothing for 15 s while its decoder is hungry and fetches again from `?offset=`.
- **Ending:** a response ends cleanly only when LMS finished the stream cleanly and the client has had every byte. Every other end is an error (the connection breaks without a clean end), so a client never mistakes it for the end of the track:
  - the session ends (for example, the device is revoked), or a newer request for the stream takes over;
  - LMS sends nothing for 60 s, or its stream fails. Later requests for that stream fail too, after replaying what was received;
  - the client has a full window unacknowledged and sends no ack for ten minutes (for example, it is paused). A slow client is not idle: one holding 8 MiB of input stops acknowledging for over a minute at CD quality. It fetches again with `?offset=` when it wants more.
  - **A proxy may hide the error.** Something between the gateway and the client (seen through a Cloudflare Tunnel, issue #12) can pass the broken connection on as a clean end. So both players (web and Android) confirm a clean end that brought any bytes with one more request from the offset reached. At the true end the gateway answers it at once with an empty `200`; otherwise the rest of the audio follows. A response that ends cleanly having brought nothing is the end, so a track costs one extra, empty request at most and confirming never loops. A confirmation refused with a final status (an older gateway, a stream gone, `416` past what was sent) counts as the end, as before; so does one that keeps failing with a network error or `5xx` (Android: after 3 retries with the usual backoff; web: after its retry limit).
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
- **503** `{"error":"music server is busy","busy":true}` with `Retry-After: 2`: LMS is blocked (see *When LMS is busy*). The request never reached LMS; retry after the pause, or when `/v1/events` says `{"lms":"ok"}`. Authentication, the body and the policy are checked first, so a request that would be refused anyway still gets its 400/401/403.

**Default deny.** A command is allowed only if it is in the allow-list and every argument passes validation:
- numbers or printable strings of at most 256 characters;
- at most 16 arguments;
- paging `start count` with `count` ≤ 500;
- `key:value` parameters restricted to known keys per command, each given at most once, with typed values;
- free-text values (`search:`, `term:`, playlist names, an app's `search:`, text typed into a learned search box) refused if they hold an absolute or Windows path, a `file:` URL, a URL to a local host (see below), a bare LAN address such as `10.0.0.5:9000`, or any URL other than http(s). Public http(s) URLs pass (some apps search by feed URL), as does ordinary text such as "AC/DC" or "Live 1990/91";
- a URL in typed free text with an explicit port other than 80 or 443 (`http://music.example.com:9000/`) is refused;
- every http(s) URL in a command's text is resolved by the gateway (2 s limit per name, at most four distinct names per command) and the command refused, as above, if any name resolves to a local address (a public name with an A record of `127.0.0.1`, or an IPv4-mapped private AAAA). A name that does not resolve is judged by its name alone, and DNS rebinding between the gateway's lookup and LMS's is not caught.

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
| Player (permitted players only) | `status`, `play`, `pause`, `stop`, `mode ?`, `time`, `power`, `sleep`, `mixer volume/muting`, `playlist index/jump/move/delete/clear/shuffle/repeat/tracks ?`, `playlistcontrol cmd:load/add/insert/delete` with exactly one id selector, `sync` with another permitted player, `favorites playlist play/add/insert item_id:…`; Don't Stop The Music: `dontstopthemusicsetting` (exactly that), and `playerpref plugin.dontstopthemusic:provider <token>|0|?` (the token is letters, digits and `_`, starting with a letter, at most 64); for the device's own player only, `playerpref digitalVolumeControl 0|1|?` (LMS's *Volume Control*: `0` is "Output level is fixed at 100%", `1` enables volume control; no other `playerpref`, and never for another player, permitted or not); Random Mix (part of LMS): `randomplay <tracks\|albums\|contributors\|artists\|year\|works> [genre_id:<id>]`, `randomplay disable` and `randomplayisactive` (see below) |
| Apps and radio | `<app> items …` and `<app> playlist play/add/insert item_id:…`, only for app commands LMS lists in its `apps`/`radios` menus. Core command names can never be learned as apps. An unknown app triggers at most one menu refresh a minute. |
| Admin role only | `rescan`; changing saved playlists (`playlists new/rename/delete/edit`, whose `name:`/`newname:` follow the same rules as a queue save's name; saving the queue goes through `POST /v1/players/{player}/queue/save` instead, and raw `playlist save` is refused) |

**Plugin and context menus (learned actions).** LMS menus (apps, radio, track/album/artist info, plugin menus) offer actions: commands with parameters, which plugins define. A command outside the fixed allow-list is allowed only if **LMS offered exactly that action, in a menu the gateway relayed to the same device in the last hour**:
- requests must match a learned action word for word and parameter for parameter, except text typed into a search box (`__TAGGEDINPUT__`);
- request values are printable, at most 256 characters, and never URLs or paths;
- actions are learned after response filtering, and an action whose parameters held a path or LAN URL is removed whole, so it can never be learned or shown;
- server-level commands (preferences, rescan, restart, `client`, display/IR, …) never run this way;
- actions that change shared data (`favorites`, `jivefavorites`, `playlists`, presets, and Random Mix's server-wide genre and library choices `randomplaychoosegenre`, `randomplaygenreselectall`, `randomplaychooselibrary`) need the admin role;
- paging (`start count`) is required for `… items` menus, optional for a navigation (a `go` action without `nextWindow`, which opens a submenu in LMS's own clients), and refused for any other action;
- at most 4,000 learned actions per device, expiring after an hour, and learned actions are kept for at most 1,000 devices (the least recently used go first).

**Plugin entries (home menu).** Plugins also add entries to the player's home menu (My Music, Radio, Extras), not only to `apps`/`radios`. `menu <start> <count> [direct:1]` (permitted players only) returns that menu cut down to library and plugin entries: anything under the settings nodes, player power, and entries the app shows itself (library, favourites, search) are dropped before the device sees them, as are entries not reachable from a top-level node. Their actions are then learned like any menu's.

**Admin-only plugins.** An admin can reserve plugins (by command word, e.g. `audiomuseai`) for admins, on the plugin settings page (`GET/POST /admin/v1/plugins` on the admin API; the GET lists what LMS offers as apps, radio and home-menu entries). For listeners, these plugins are removed from app lists, the home menu and context menus, and their commands are refused (403), however they are reached. Don't Stop The Music providers named after a reserved plugin (e.g. `PLUGIN_AUDIOMUSEAI_…` for `audiomuseai`) are removed from `dontstopthemusicsetting` and can't be chosen.

**Radio and "more like this" (apps, A6).** Nothing is built in: the apps read the item's own context menu and use AudioMuse-AI's entries there (command words starting `audiomuse`) as learned actions, or play the item with Don't Stop The Music switched on for the player (the provider in use if any; otherwise AudioMuse-AI's, then a similarity provider, then any but random). With neither, there is no radio and the action is hidden.

Context menus start from `trackinfo`/`albuminfo`/`artistinfo`/`genreinfo`/`yearinfo`/`playlistinfo`/`folderinfo items` with a library id.

**Random Mix (`randomplay`), a fixed grammar.** Single-genre stations play as LMS's Random Mix, which keeps the queue topped up itself. Only these shapes pass, on a player the caller may control:
- `randomplay <mode>` with `mode` one of `tracks`, `albums`, `contributors`, `artists`, `year`, `works` (LMS matches the mode as a regular expression, so nothing else is passed on), optionally with one `genre_id:<id>` (a positive number without leading zeros: LMS looks the genre up by id, and fails on 0);
- `randomplay disable`; `randomplayisactive` (no arguments).

Refused: `genres:` (free text, unescaped and split by LMS), `library_id:`, any other parameter or a second `genre_id`, and as server queries. Random Mix's genre and library choices (`randomplaychoosegenre`, `randomplaygenreselectall`, `randomplaychooselibrary`) change server-wide prefs, not the player's: they are not in the allow-list, and as learned menu actions they are admin-only.

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

### Continue on a player: the offline queue (SPEC §8.4)

The same copy recipe, from the device's own downloads rather than a player. No new endpoint or command.

1. For each queued download, `GET /v1/download/track/{id}/info` with the id it was downloaded with (each id once, a few at a time). Use the id only if the answer's `key` is the download's stable key. A 404, or another key, means the track is skipped and counted. Any other answer (401, 403, 429, 5xx, no connection) stops here, with nothing sent.
2. `T`: `power 1`, `playlist shuffle 0`, then the confirmed ids in play order as in step 3 above, starting at the current track (or the next confirmed one if it was skipped).
3. If the position is ≥ 2 s: `T`: `time <floor(position)>`, adding the time taken if the downloads were playing. If they were paused: `T`: `pause 1`.
4. Only now stop the local player. If any step fails, leave it as it was.

## Synchronised playback: how our players keep time

Our players (Android, and browsers) take part in LMS sync groups alongside Squeezeboxes and squeezelite. LMS keeps a group together using three things, and our players do each the way a hardware player does (behaviour checked against the LMS 9.0 source: `Slim::Player::StreamingController`, `Squeezebox2`, `Player`).

**1. Every STAT says what is audible, and when.** We advertise `AccuratePlayPoints=1`, so LMS uses every STAT's `(jiffies, elapsed_milliseconds)` pair as it is: no averaging. It turns the jiffies into server time with an epoch learnt from STAT arrival times (the earliest arrival seen, adjusted for drift). It then works out when each player's track apparently started (`server time − elapsed`).
- **Jiffies** are milliseconds on the player's monotonic clock: `CLOCK_MONOTONIC` on Android, `performance.now()` in the browser worker. Audio timing uses the same clock.
- **Elapsed** counts frames as they are **presented** (heard), not as they are handed to the output. Before 2026-10-12 it counted handed-over frames, so a player sounded late by its output latency: 10–40 ms through a phone's speaker, 150–300 ms over Bluetooth, tens of ms in Web Audio, seconds in iOS media-element mode. LMS never saw that, so never corrected it.
- **Each STAT's elapsed is extrapolated to its own jiffies.** The host reports the frames presented together with the moment that count was exact (`Player::presented(frames, at_ms)`). A STAT sent later adds the time since (at most 1 s, never more than the host holds, and not while paused or in a sync pause).

**2. A timed start.** For a group, LMS sends `strm s` with autostart off. Each player buffers and answers `STMl`. Once every player has, LMS sends each one `strm u` whose 32-bit argument is a start time in that player's jiffies: by default 200 ms ahead (the server's *Synchronised players startup delay*), minus that player's *Player start delay* and *Player audio delay*.
- Our players pre-roll audio into the output while they wait, held silent.
- The output then places the first frame so it is **heard** at that time, to the sample: on Android, the device frame is worked out from the output timestamp; in Web Audio, the context frame.
- If the output's latency is longer than the time left (Bluetooth against LMS's 200 ms), the start can't be made. The player then starts at once and drops the audio that should already have played, so it is still in step.
- A start time in the past, or more than 30 s ahead (a corrupt or hostile frame), starts at once.

**3. Corrections.** About once a second LMS compares the players' apparent start times. If the gap is at least 10 ms (and at least the player's *Minimum synchronisation adjustment*), it corrects the player:
- `strm a` (skip ahead N ms) for a player that is behind;
- `strm p` with an interval (pause N ms) for one that is ahead, when the reference player can't skip.

Our players apply both **at the output stage**, at once:
- a pause inserts N ms of silence, consuming nothing;
- a skip drops the next N ms of queued audio.

Applied anywhere earlier, they would only take effect once the audio queued ahead of the output (400 ms on Android, 300 ms in Web Audio, about 8 s in a media element) had played. A pause shorter than that queue was lost entirely. Intervals over 10 s are capped.

### Per platform

| | Position heard | Timed start | Pause, skip |
|---|---|---|---|
| Android (AAudio) | `AAudioStream_getTimestamp(CLOCK_MONOTONIC)` maps the device frame being heard to the moment. The ring buffer's record of which frames went out where maps that back to the audio. Bluetooth latency is included where Android reports it (Android 10+, most A2DP devices). Until the stream runs: frames read by the device. | The ring holds until the device frame heard at the start time, then starts mid-callback | In the ring's consumer (the audio callback) |
| Android (host-pulled `AudioTrack`) | Frames handed over (no timestamp yet) | The ring starts at the frame handed over at the start time (no latency allowance) | In the ring |
| Web Audio | The worklet reports rendered context frames. The page sends `getOutputTimestamp()` (or `currentTime − outputLatency − baseLatency`) every 250 ms. | The worklet starts at the context frame heard at that time | In the worklet |
| Media element (iOS) | The element's `currentTime` on the appended timeline, read every 100 ms and smoothed against the wall clock, with the time it was read | `play()` on a timer at that time | In the audio before muxing: silence inserted for a pause, decoded audio dropped for a skip. Reported to the core at once, heard once the element has played what was already appended (up to 8 s). Never by seeking or pausing the element: on iPadOS that stalls it for about as long as the correction gained, and LMS asks again at every check. |

**Media-element limits:** the browser doesn't say how long `play()` takes to become audible, nor the route's latency (AirPlay, Bluetooth), so neither is compensated. Expect the same offset LMS has always seen from such a player, plus `play()`'s start-up (tens of ms, varying). LMS's corrections still keep it from drifting, though each is heard only after the audio already appended to the element. If it sounds consistently late, set *Player audio delay* for that player (below).

### LMS settings for our players

*Settings › Player › (the player) › Synchronise*. These are LMS's own, per player. **The plugin changes none of them**: with the position reported as heard, LMS's defaults are right for our players, and changing preferences behind the owner's back (or a hardware player's by mistake) would be worse.

| Setting (LMS pref) | Default | For our players |
|---|---|---|
| Maintain synchronisation (`maintainSync`) | on | Leave on. |
| Minimum synchronisation adjustment (`minSyncAdjust`) | 10 ms | Leave. Raise it (to 20–30 ms) only for a browser in media-element mode if corrections are too frequent. |
| Player audio delay (`playDelay`) | 0 | Leave at 0: our players already allow for their output latency. Set it only for latency the platform doesn't report (a media element on AirPlay, or Bluetooth on a phone that doesn't report it). If the player sounds late, set its lateness in ms. |
| Player start delay (`startDelay`) | 0 | Leave at 0: a start our players can't make is caught up exactly. |
| Network packet latency (`packetLatency`) | 2 ms | Leave. |
| Synchronised players startup delay (server, `syncStartDelay`) | 200 ms | Leave. Raising it lets a Bluetooth phone start without dropping its first few hundred ms. |

### Rejoining after a gap (Android)

When the phone's link to the gateway stalls while it is in a group, the others play on. Once the link is back and audio arrives, the phone gets back in step by itself (`mobile/src/player/engine.rs`):
- **Short stall:** if it has decoded more than the stall (plus 250 ms), it drops that much audio and carries on with the others. It waits up to 2 s for that much to arrive.
- **Longer stall:** it has LMS restart the group where the others are now: `time <t>`, where `t` is where the phone stopped plus the time it has been stalled. LMS flushes and restarts every member there, in step; the others hear a short gap.
  - LMS's current track (`status - 2 tags:xed`) is checked first. Nothing is sent if LMS isn't playing (someone paused or stopped the group), if the current track is no longer the one that stalled (the queue changed, or the group moved on), if `t` is within 1 s of the track's end or past it (LMS moves the group on itself), or if the stall is longer than any session lasts.
  - Not LMS's `time ?`: with the phone as the group's master, it lags at the point LMS last heard from the phone. In the live check, a 14 s stall sent the whole group back about 14 s.
  - Only when the phone never learnt the track's queue position (a radio stream, or LMS didn't answer) does it fall back on `time ?`.
- **The underrun report:** the phone tells LMS `STMo` when its output runs dry mid-track. Sent while the link is down, it waits in the outbox. In a group, the phone drops it on reconnecting (with the `STMr` for its recovery, if one followed), and sends no `STMr` when the audio comes back. While LMS is still streaming the track, a member's `STMo` makes LMS pause the whole group until every member has rebuffered. Arriving after the gap, it no longer describes anything LMS should act on: in the live check, the others went silent for 4 to 15 s.
  - **Still sent:** alone, everything held goes as before (LMS rebuffering one player is harmless); the `STMo` sent in place of `STMu` when everything received has played out with the link down (it is about the end of the track); and any underrun while the link is up, including one after the rejoin.
  - Group membership is as last known before the gap; the phone asks again on reconnecting.
- **Live check:** `mobile/tests/live_sync.rs` (opt-in, against a real LMS) asserts that, after the rejoin, the phone is within 100 ms of the group; LMS doesn't pause the others for more than 1 s; and a restart moves them by at most 1.5 s.

### Measured (simulation)

`mobile/tests/sync.rs` runs two players against a fake LMS that does what LMS does:
- epoch learning from STAT arrivals, with its drift adjustment;
- the timed start;
- skip corrections.

It uses the real core, ring buffer and timestamp maths, in front of simulated outputs. They have latency, sample-clock drift (up to ±120 ppm), monotonic clock drift against the server, timestamp jitter, tick jitter and a jittery network with spikes. Each frame carries its own number, so what each output presents is known exactly. Against a hardware player (2 ms):

| Player | First moment both play | Settled, mean | Settled, worst |
|---|---|---|---|
| Phone speaker (30 ms) | ≤ 5 ms | 0.5–6 ms | ≤ 7 ms |
| Bluetooth (250 ms) | ≤ 5 ms | 1–5 ms | ≤ 7 ms |
| Web Audio-like (60 ms, jittery) | ≤ 11 ms | 1–2 ms | ≤ 3 ms |
| Speaker vs Bluetooth phone, both drifting | 1 ms | 3 ms | 6 ms |
| *Before: Bluetooth, handed-over frames* | *264 ms* | *256 ms* | *264 ms* |
| *Before: speaker, handed-over frames* | *42 ms* | *31 ms* | *36 ms* |

What remains is mostly LMS's own estimate of each player's clock (the network delay of the fastest STAT it has seen). These are simulated figures. On real devices, add the platform timestamp's own error: small on Android's speaker and wired outputs; on Bluetooth, it depends on what the headset reports.

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
| `imageproxy/<url-encoded http(s) URL>/image[_WxH[_m]][.ext]` | Only for image URLs LMS itself returned in an image field (`image`, `icon`, `icon-id`, `artwork_url`, `coverart`, `cover`) of an earlier, filtered `/v1/rpc` response, or as an artist's photo in a `/v1/info/artist/{id}` answer, and never for a URL on the home network or the server itself: the host must not be a local host (as defined under response filtering, which covers single-label names and wildcard-DNS names such as `10.0.0.5.nip.io`), and must not resolve, from the gateway within 2 s, to any local address. The lookup can't stop DNS rebinding (LMS resolves the name again when it fetches), and a name that doesn't resolve is judged by the name alone. Nor does it see redirects: LMS's image proxy fetches the URL and follows any redirect itself, so a public host that redirects to a home-network address is followed; only an `image/*` answer of at most 20 MiB is ever relayed, whatever it came from. Anything else would let a user make LMS fetch arbitrary URLs on the home network (SSRF): text such as track titles and player names can be set by users, so it never counts. |
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

**Transcodes** are made on first request (the response starts once ffmpeg has finished, which can take a while for a long track), then kept in an LRU cache on disk and served from it, `Range` included. A transcode that fails is 502; one that runs too long (10 minutes) is 504. The original is fetched from LMS into the cache directory first and counts against the cache's size while it is there (other files are evicted to make room, and a transcode waits while others' originals hold it), so a track larger than the whole cache is 413 `track too large` for `mp3` and `opus`; it is fetched before a transcode slot is taken, so a slow fetch never holds up anyone else's encode (2026-10-18).

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
| 413 | `track too large` | A transcode of a track larger than the transcode cache |
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
- `too_short`: not enough was heard (below);
- `overlap`: the user already has a counted play of this track starting less than the counting time (below) before or after this one, so it can't be a separate listen.

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

## Your own things (`/v1/me/…`)

Listening history, the customisable Home, stations, pinned menus and smart filters (docs/plans/wave1-design.md §6–8, docs/plans/wave2-design.md §5.6; SPEC D23). Everything is per user: the same on every device of the user's, and never shown to anyone else. Each needs a bearer access token (401 without); a user may make 120 of these requests a minute (429 with `Retry-After` beyond). Answers are JSON with `Cache-Control: private, no-store`; refusals are `{"error": "<reason>"}`. Bodies are JSON objects whose unknown fields are refused (400), at most 4 KiB (413), 256 bytes for `PATCH /v1/me/history`. Ids in paths are plain positive decimal numbers; anything else, and another user's id, is 404.

### Listening history

The gateway records each play that counts (the rule of `POST /v1/plays`: half the track or 240 s, whichever is less) of a local library track:
- **live**, on the user's own devices' players (the gateway follows each player session's status reports; time heard is elapsed time while playing, never more than the player's own clock (each STAT's jiffies) allows, so a seek forward isn't "heard" while reports held back during a network gap and sent together on resume still count; over a whole session, no more than the wall time since its first report plus 10 s is ever counted, whatever the reports claim; a seek that restarts the stream continues the same play; the track is asked of LMS just after it starts). Hardware players and other users' devices are never attributed to anyone;
- **offline**, when `POST /v1/plays` has LMS record a play.

Each play keeps the stable track key, the LMS track, album and artist ids and year at the time, when it started, the seconds heard (updated when a live play ends), and `live` or `offline`. Plays are kept **13 months** (396 days), at most 50,000 per user (the newest). Nothing is recorded while the user has history switched off.

| Request | Answer |
|---|---|
| `GET /v1/me/history[?limit=1–200][&before=<cursor>]` | `{"enabled": bool, "retention_days": 396, "items": [{"track_key", "track_id", "album_id", "artist_id", "year", "played_at", "seconds", "source"}], "next": "<cursor>"\|null}`, newest first (50 by default). The cursor is `<played_at>-<id>` (digits); unknown query parameters are refused (400) |
| `DELETE /v1/me/history` | `{"removed": n}`: all of the user's plays (audited as a count) |
| `PATCH /v1/me/history` `{"enabled": true\|false}` | `{"enabled": …}`: switch recording on or off (on unless switched off). Switching off keeps what was recorded; clear it to delete it |

Clients show titles with `titles 0 <n> track_id:<id,id,…> tags:…` through `/v1/rpc`.

### Shelves

`GET /v1/me/shelves/{recent|most|on_this_day}[?limit=1–50]` → `{"shelf": "<name>", "albums": [<album id>, …]}`, best first (20 by default); library ids only (fetch them with `albums 0 <n> album_id:<id,id,…>`, which LMS answers in its own order). Any other shelf name is 404.
- **recent:** the user's albums by their last play here (their own history only: LMS's `sort:recentlyplayed` is server-wide and is not blended in, so no user sees what others played; since 2026-10-18);
- **most:** the user's albums by their plays over the last 90 days, most first, then the most recently played (their own history only, as for `recent`);
- **on_this_day:** albums the user played within 3 days of this date a year ago (LMS keeps only each track's last play, so this one is the user's own).

LMS before 9.1 has no play-count sorts: the server side is then empty.

### Listening stats

`GET /v1/me/stats[?period=<p>][&tz=<minutes>]` (docs/plans/wave3-design.md §1): the "Your listening" page and "Your year in sound", worked out from the user's own listening history. Nothing new is stored.

- **`period`:** `days-N` (N from 7 to 400, no leading zero; `days-30` by default), `month-YYYY-MM`, `year-YYYY` (years 1970 to 2100) or `all`.
- **`tz`:** minutes east of UTC, −840 to 840 (0 by default), plain decimal (no `+`). Clients send the device's current offset. Hours, days, weekdays, months and streaks are worked out at this fixed offset (no daylight-saving changes within the period).
- **Refused (400, `bad query`):** any other parameter, either one twice, a malformed or out-of-range value, percent-encoding, or a query string over 64 characters.
- **The period:** `days-N` runs from local midnight N − 1 days ago until now; a month or a year from local midnight on its first day to its end, or now if sooner (a future one is empty, `from` = `to`); `all` covers the 396 days the history keeps, until now. A play counts when `from <= played_at < to`.

```json
{
  "enabled": true,
  "retention_days": 396,
  "period": {"from": 1790000000, "to": 1792600000},
  "totals": {"plays": 412, "seconds": 82800, "tracks": 301, "albums": 64, "artists": 41, "days": 27},
  "top_artists": [{"id": 12, "name": "An Example Band", "plays": 40, "seconds": 9100}],
  "top_albums": [{"id": 7, "plays": 22, "seconds": 5200}],
  "top_tracks": [{"id": 9, "plays": 6, "seconds": 1300}],
  "new_artists": [{"id": 31, "name": "Another Example", "first_played": 1791000000}],
  "by_hour": [0, 0, 1, …],
  "by_weekday": [12, 30, …],
  "by_month": [{"month": "2026-10", "plays": 412, "seconds": 82800}],
  "longest_streak": {"days": 9, "from": "2026-10-02", "to": "2026-10-10"},
  "busiest_day": {"date": "2026-10-04", "seconds": 14400}
}
```

- **Counting:**
  - At most 200 000 plays are read (the newest). Seconds are whole numbers.
  - Tracks are counted by the history's stable track key (library ids can change after a rescan); each reports the newest track id seen for it.
  - Albums and tracks are ids only: load names and artwork as for shelves.
- **Top lists:** `top_artists`, `top_albums` and `top_tracks` hold at most 20 each. They are ordered by plays, then seconds, then the latest play, then the lower id.
- **`new_artists`:** at most 20 artists whose first play in the whole kept history falls in the period. The ones played most in the period come first. History reaches back only 396 days, so for `all` and long periods say "first played since …".
- **Artist names:**
  - LMS looks an artist up by one id only, so the gateway asks `artists 0 1 artist_id:<id>` for each artist the answer names: at most 40, four at a time, each within 4 s and all within 8 s.
  - A name is cleaned: control and invisible formatting characters are removed, whitespace is collapsed and it is cut to 200 characters. A name naming a server path or a LAN address is dropped.
  - Names are remembered for an hour (a failed lookup for 2 minutes). A name LMS can't give is `""`.
- **Charts:**
  - `by_hour` (24 entries) and `by_weekday` (7, Monday first) count plays.
  - `by_month` lists every month of the period, with zeros, for `year-*`, `all` and `days-N` with N > 60. Otherwise it is `[]`.
- **Streaks:** `longest_streak` is the longest run of consecutive local days with at least one play. `busiest_day` is the day with the most seconds. On a tie, the most recent wins. Both are `null` with no plays.
- **History switched off:** `enabled: false`, with zeros, empty lists and `null`s (what was kept shows again when it is switched back on).
- **Caching:**
  - An answer is remembered per user, period and `tz` for 5 minutes. A play recorded meanwhile shows once it expires.
  - Clearing the history, switching it on or off, or deleting the account drops the user's answers at once.
  - An answer missing a name because LMS failed is not remembered.
- **Limits:** it counts under the 120-a-minute `/v1/me` allowance.

### Home layout

`GET /v1/me/home` → `{"version": 1, "shelves": [{"type": "recent"}, …], "default": bool}`. Without a saved layout it is the default, in this order: `recent`, `stations`, `new`, `most`, `on_this_day`.

`PUT /v1/me/home` with `{"version": 1, "shelves": [...]}` saves one (answering it as `GET` would); `DELETE /v1/me/home` goes back to the default. A layout is refused (400) unless:
- `version` is 1 and there are no other fields;
- `shelves` is a list of at most 24, each `{"type", "id"?, "hidden"?}` and nothing else;
- `type` is one of `recent`, `stations`, `new`, `most`, `on_this_day`, `random`, `favourites`, `playlists` (no id), or `station`, `menu`, `filter` (one of the user's own stations, pinned menus or smart filters, checked when saved: 400 `no such station or pinned menu` / `no such filter` otherwise) or `genre` (an LMS genre id), which need an `id` (1 to 999 999 999 999);
- `hidden` is `true` or `false`;
- no shelf appears twice, and the whole document is at most 4 KiB.

Shelves of stations, pinned menus or filters deleted since are left out when the layout is read.

### Stations

A station is endless: the client asks for the next batch whenever fewer than about 5 tracks are left after the current one. Each user starts with three built-in stations (forgotten favourites, fresh, deep cuts), which they may delete.

| Request | Answer |
|---|---|
| `GET /v1/me/stations` | `{"stations": [{"id", "name", "kind", "spec", "broken": "<why>"\|null}]}` |
| `POST /v1/me/stations` `{"name", "spec", "player"?}` | 201 with the station; 400 for a bad spec or name; 409 beyond 100 stations and pinned menus; 422 for a menu station LMS doesn't offer now, on `player` |
| `DELETE /v1/me/stations/{id}` | `{"removed": true}`, or 404 |
| `POST /v1/me/stations/{id}/next` `{"player", "start"?, "count"? (1–50, 20), "exclude"? (≤ 200 track ids)}` | see below; 403 for a player the caller may not control; 429 beyond 20 batches a minute |

Specs (`name`: 1–64 characters, cleaned like device names):
- `{"kind": "genre", "genres": [<id>, …]}` (1–5 genres). With one genre, `next` answers `{"mode": "randomplay", "command": ["randomplay", "tracks", "genre_id:<id>"], "refill": false}` for the client to send through `/v1/rpc` (LMS's Random Mix then keeps the queue filled); with several, track batches as below;
- `{"kind": "decade", "decade": 1980, "mine": bool}` (1900–2090; `mine` favours what the user played from it). Random Mix can't filter by year, so this is a track list;
- `{"kind": "forgotten"}`: tracks the user played at least twice but not in the last 60 days (× 3), and tracks of LMS's most played albums it hasn't played lately (× 1);
- `{"kind": "fresh"}`: tracks of the newest albums played at most once, and never by the user;
- `{"kind": "deep_cuts"}`: tracks played at most once, by the artists the user plays most (× 3) and LMS's most played artists (× 1);
- `{"kind": "menu", "steps": [<command>, …], "play": <command>, "add"?: <command>}`: pinned from a menu (below).

Track stations answer `{"mode": "tracks", "tracks": [<track id>, …], "refill": true}`. A batch never holds the user's last 200 plays, the `exclude` ids, or the last 200 tracks the station served. The client loads the first batch (`playlistcontrol cmd:load track_id:…`) and adds the rest (`cmd:add`), splitting id lists to fit 200 characters.

**Stations pinned from menus.** "Save as station" on a mix or generator item stores a reference to an action LMS offered, never a free command:
- `steps`: the menu requests from a root to the menu holding the item (1–4), each with paging. The first must be a menu of the fixed allow-list: `menu …` (the home menu), `apps`, `radios`, `<app> items …` for a listed app, `favorites items …` or a context menu (`trackinfo items …` and the others). Plugin entries in the home menu start from `menu`;
- `play` (and optionally `add`): the item's actions, without paging and never with typed text;
- commands are 1–16 words; each word is at most 256 bytes and passes the free-text check; the first is a plain word.

Every time the station is saved or plays, the gateway replays the steps under the user's own role and players, exactly as `/v1/rpc` would: the first must pass the fixed allow-list; each later one must be an action LMS offered in the previous step's filtered answer; and `play`/`add` must be offered in the last one (the learned-action rules above: never server commands, admin-only actions for admins only, admin-only plugins hidden from listeners). Only then does it run the action, on a player the caller may control:
- start (`"start": true`) runs `play`, answering `{"mode": "ran", "refill": <has add>}`;
- a refill runs `add`, or answers `{"mode": "none", "refill": false}` without one;
- if anything is no longer offered, nothing runs: the answer is 409 `{"error": "<why>", "broken": true}`, and the station is listed with `broken` set until it works again;
- 502 if LMS can't be reached (the station isn't marked broken).

### Pinned menus

"Pin to Home" for a menu stores the steps to it (as above, with the menu itself as the last step).

| Request | Answer |
|---|---|
| `GET /v1/me/menus` | `{"menus": [{"id", "name", "steps", "broken"}]}` |
| `POST /v1/me/menus` `{"name", "steps", "player"}` | 201; 422 if LMS doesn't offer it now, on `player` |
| `DELETE /v1/me/menus/{id}` | `{"removed": true}`, or 404 |
| `POST /v1/me/menus/{id}/open` `{"player"}` | `{"result": <the last menu, filtered as /v1/rpc would>}`; the device may then run what it offers (its actions are learned for the device). 409 `{"broken": true}` if LMS no longer offers it |

### Smart filters

A filter is saved rules that produce albums; every rule must match. Filters are the user's own: another user's id is 404 everywhere, exactly as an id that doesn't exist.

```json
{"id": 3, "name": "Late jazz",
 "rules": [{"field": "genre", "in": [4, 9]}, {"field": "decade", "is": 1960},
           {"field": "artist", "is": 12, "role": "COMPOSER"}, {"field": "compilation", "is": false}],
 "sort": "yearalbum", "limit": 200}
```

| Request | Answer |
|---|---|
| `GET /v1/me/filters` | `{"filters": [<filter>, …]}`, oldest first |
| `POST /v1/me/filters` `{"name", "rules", "sort", "limit"}` | 201 with the filter (with its new `id`); 400 for a bad filter; 409 `too many filters` beyond 50 |
| `PUT /v1/me/filters/{id}` `{"name", "rules", "sort", "limit", "id"?}` | 200 with the filter, replaced; `id` may be sent only if it is the path's; 400 for a bad filter; 404 if it isn't the caller's |
| `DELETE /v1/me/filters/{id}` | `{"removed": true}`, or 404 |
| `GET /v1/me/filters/{id}/albums[?from=0–500][&n=1–100]` | `{"albums": [<album id>, …], "count": <total>}`: the albums from `from` (0 by default), at most `n` (50 by default), and how many the filter produces in all (at most its limit). Clients load details by id (`albums 0 <n> album_id:<id,id,…>`), as for shelves. 400 for any other query parameter or value; 404 if it isn't the caller's; 429 beyond 30 runs a minute or 2 at once (`Retry-After: 5`); 502 if LMS can't be reached |

A filter is refused (400, with the reason) unless:
- it is an object with `name`, `rules`, `sort` and `limit`, and nothing else (`POST` takes no `id`);
- `name` is 1–64 characters, cleaned like device names;
- `rules` is a list of 1–8 rules, each field at most once, each exactly one of:
  - `{"field": "genre", "in": [<genre id>, …]}`: 1–20 distinct ids, each 1 to 999 999 999;
  - `{"field": "decade", "is": 1990}`: a multiple of 10 from 1900 to 2090;
  - `{"field": "artist", "is": <artist id>, "role"?: "<role>"}`: the role one of `ALBUMARTIST`, `ARTIST`, `COMPOSER`, `CONDUCTOR`, `BAND`, `TRACKARTIST`;
  - `{"field": "compilation", "is": true|false}`;
- `sort` is `album` (A–Z), `artistalbum`, `yearalbum`, `new` (recently added) or `random`;
- `limit` is 1–500;
- the body is at most 4 KiB (413 beyond).

Ids are JSON integers (not strings or decimals); a rule with any other key is refused.

**Running a filter.** The gateway builds LMS `albums` queries from the parsed rules (`genre_id:<id,…>`, `artist_id:<id>` with `role_id:<role>`, `compilation:0|1`, `year:<y>`, `sort:<sort>`, `tags:lay`); nothing a client sends is passed on as text, and each query must also pass the `/v1/rpc` allow-list. A decade is one query per year, merged and de-duplicated, then sorted again: A–Z by title, by artist then title, by year (the years in order), newest first by album id (LMS numbers albums as it adds them), or shuffled. Without a decade it is one query in LMS's own order (`random` is shuffled again). The result is cut to the limit. It is kept for 2 minutes: `from=0` always runs the filter afresh (a new shuffle for `random`), and later pages continue the kept result, so a random order holds together while paging. Runs count under the `/v1/me` allowance above, and at most 30 a minute and 2 at once per user (8 at once for everyone); pages served from a kept result don't count as runs.

**Playing a filter** (clients): its first albums, up to 50, through the existing album play path.

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

## Checked queue edits (`/v1/players/{player}/queue/edit`)

LMS's queue commands take a position, so a remove or move sent after someone else changed the queue could hit a different track. LMS has no "this entry" by identity (`playlistcontrol cmd:delete track_id:` removes every copy). The gateway checks and changes in one step instead, one edit at a time per player:

| Request | Body | Answer |
|---|---|---|
| `POST …/queue/edit` | `{"op":"remove","index":i,"track":id}` | 200 `{"done":true}` |
| `POST …/queue/edit` | `{"op":"move","index":i,"to":j,"track":id}` | 200 `{"done":true}` |

- **409 `{"error":"the queue changed"}`:** the entry at `index` isn't `track`, and nothing is changed. Re-read the queue.
- **Refusals:**
  - 403 for a player the caller may not control;
  - 400 for any other body. Positions are 0–100 000, `to` differs from `index`, `track` is a non-zero id, and nothing else is accepted.
- **What remains:** the check and the change are two local LMS commands, a few milliseconds apart. Another controller would have to change the queue in that gap.
- **Detecting it:** `GET /v1/me` lists `"queue_edit": true` under `features`. Clients fall back to their own check against older gateways.

## Save the queue as a playlist (`/v1/players/{player}/queue/save`)

LMS's `playlist save <name>` writes `<name>.m3u` into its playlist folder, named by the client, and silently overwrites a playlist of that name. The RPC policy never passes it on (it stays refused for everyone, admins included). This endpoint does the save with the name checked first:

| Request | Body | Answer |
|---|---|---|
| `POST …/queue/save` | `{"name":"Road trip","replace":false}` | 200 `{"id":12,"name":"Road trip"}` |

- **Who:** admins only, for a player they may control. Saved playlists are shared household data: listeners may play them, not change them (as for `playlists new|rename|delete|edit`). A listener gets 403 `{"error":"changing playlists needs the admin role"}`; a player the caller may not control (or not a player id) is 403 `{"error":"you may not control this player"}`. Requests count towards the per-user `/v1/me/…` allowance.
- **The name:** trimmed, then 1–100 characters (not bytes; any script, sent composed as NFC). Refused with 400:
  - control characters, and invisible ones (zero-width characters and joiners, bidirectional overrides and isolates, line and paragraph separators, the byte-order mark);
  - `/ \ : * ? " < > |` (path separators, characters common filesystems reject, and `:`, which LMS would read as a tagged parameter);
  - `..` anywhere, a leading dot, a trailing dot;
  - Windows device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension);
  - anything the policy's free-text check refuses (file URLs, LAN addresses).
- **The body:** exactly `name` (a string) and optionally `replace` (true or false); anything else is 400, a body over 1 KiB is 413.
- **A name already taken:** the gateway looks for a saved playlist of that name ignoring case (the playlist folder may be on a case-insensitive filesystem), using LMS's `playlists … search:` and comparing exactly. If there is one and `replace` isn't true: 409 `{"error":"a playlist with that name already exists","exists":{"id":5,"name":"Road Trip"}}`, nothing saved. With `replace: true` the existing playlist is overwritten under its own spelling (so a different case doesn't leave two playlists of one name on a case-sensitive filesystem), and keeps its id.
- **An empty queue:** 409 `{"error":"the queue is empty"}`, nothing saved.
- **The answer:** the id LMS gives the saved playlist (its `__playlist_id`; looked up by name if LMS doesn't say), and the name it was saved under. LMS failing to write the file (no playlist folder set, say) is 502.
- **What is saved:** LMS's own `playlist save`: library tracks by their file, and radio or streaming-service entries in the queue by their URL, as LMS's own web pages save them. Those URLs stay on the server: `playlists tracks` is filtered as ever.
- **Concurrency:** one save at a time (names are shared by every player), and none while the gateway edits that player's queue.
- **Detecting it:** `GET /v1/me` lists `"save_queue": true` under `features`. Clients offer the action only then, and only to admins.

## Alarms (`/v1/players/{player}/alarms`)

LMS's own alarms for one player (docs/plans/wave2-design.md §1). They ring on hardware players too, with no app open.

**Who:** anyone who may control the player (admins: every player); any other player is 403. Requests count towards the per-user `/v1/me/…` allowance.

**Endpoints:**

| Request | Body | Does |
|---|---|---|
| `GET …/alarms` | | Lists the alarms, the player's "all alarms" switch and the sounds an alarm can play |
| `POST …/alarms` | `{"time", "days"?, "enabled"?, "repeat"?, "volume"?, "shuffle"?, "sound"?}` | Adds one |
| `PATCH …/alarms/{id}` | any of the same fields | Changes one |
| `DELETE …/alarms/{id}` | | Removes one |
| `PUT …/alarms/enabled` | `{"enabled": true\|false}` | Switches all of the player's alarms on or off |

Every change answers with the new listing; `POST` also gives the new alarm's `id`.

**Values:**
- `time`: seconds after midnight, 0–86 399;
- `days`: 0–6 (0 is Sunday), each once, at least one; every day by default;
- `enabled`, `repeat`: true or false;
- `volume`: 0–100;
- `shuffle`: 0 off, 1 songs, 2 albums;
- `id`: 1–32 lowercase hex;
- unknown fields are refused.

**Sounds:**
- An alarm's sound is an `id` from the listing's `sounds`: `current` (the queue as it is), or 16 hex characters naming one of LMS's own choices (favourites, saved playlists, random mixes, natural sounds), each with a `category` and `title`.
- The gateway turns an id back into LMS's URL by asking LMS for the list again, so only what LMS offers can be set.
- URLs are never accepted from clients and never returned: an alarm whose sound isn't in the list is reported with `"id": null`.

**Listing:**

```json
{"enabled":true,
 "alarms":[{"id":"1a2b3c4d","time":25200,"days":[1,2,3,4,5],"enabled":true,"repeat":true,"volume":40,"shuffle":0,
            "sound":{"id":"9f3c…","category":"playlists","title":"Wake up"}}],
 "sounds":[{"id":"current","category":"current","title":"Current queue"}, …]}
```

## Ratings (`/v1/me/ratings`)

Each user's own ratings, blended with LMS's shared rating (docs/plans/wave2-design.md §2). Private to the user; any signed-in user, for themselves.

- **Values:** 0–100 in steps of 10 (half stars); 0 clears. A heart is 100.
- **Keyed by the stable track key,** so a rating survives a rescan that renumbers the track. Only local library tracks can be rated (streams have no stable key).
- **The blend:** `rating` is (3 × `mine` + `shared`) / 4 where both exist, else whichever exists. `shared` is LMS's own rating (tag `R`), set by Ratings Light or other apps.
- **Written through:** where the Ratings Light plugin is installed, a rating is also set there (`ratingslight setratingpercentnoclient`), so the shared rating follows the last user to rate. Clients can't send that command themselves. Each write has a 3 s limit. An offline batch spends at most 10 s on them and stops at the first one LMS doesn't take (Ratings Light missing or hanging); the user's own ratings are all kept either way.

| Request | Body | Answer |
|---|---|---|
| `GET /v1/me/ratings?tracks=1,2,…` (at most 200) | | `{"ratings":[{"track","mine","shared","rating"}]}`, for the tracks LMS knows |
| `PUT /v1/me/ratings/{track}` | `{"rating": n}` | The same, for that track (404 for a stream or an unknown track) |
| `POST /v1/me/ratings` | `{"ratings":[{"track","key","rating","at"}]}` (at most 500) | `{"applied","skipped"}`. Ratings made offline: `key` must be the track's stable key as LMS knows it now; `at` (Unix seconds) can't be in the future or before 2020; the newest rating of a track wins, so a batch can be sent again. One batch at a time per user: another while one is running is `429` with `Retry-After`, as for `/v1/plays` |
| `DELETE /v1/me/ratings` | | `{"removed": n}` |
| `GET` / `PUT /v1/me/ratings/style` | `{"style":"stars"\|"heart"}` | How the user rates, on every device; `GET` also gives `count` |

## Lyrics (`GET /v1/lyrics/track/{id}`)

The lyrics of a library track, for Now playing (docs/plans/wave1-design.md §9).

**Who may ask:** any signed-in user: a bearer access token (401 without one). It counts towards the per-user `/v1/me/…` allowance (429 with `Retry-After` beyond it). If the admin has made the Music & Artist Information plugin admin-only, anyone else gets the file's own lyrics only, never the plugin's (404 `no lyrics` when the tags have none; since 2026-10-18).

**Request:** the track id only: a positive integer, at most 12 digits, no leading zero. Anything else is 404, and any query string is 400 `unknown parameter`. No path, URL, artist or title is ever taken from the client.

**Where the lyrics come from, in order:**
1. **The file's tags**, as LMS read them (`titles … tags:w`): plain text or LRC.
2. **The Music & Artist Information plugin**, if it is installed: `musicartistinfo lyrics track_id:<id>`. It returns that plugin's local and cached lyrics, and whatever online lyrics services the server's owner has enabled in it. The gateway itself never calls a lyrics service.

**Answer:** 200, already parsed by the core (`core::lyrics`):

```json
{"source":"tags","synced":true,"lines":[{"t":8580,"text":"First line"},{"t":12230,"text":""}]}
```

- `source` is `tags` or `plugin`.
- `synced` says whether the lines have times. `t` is the start of the line in ms; it is 0 for plain lyrics.
- Synced lines are in time order. An empty `text` is an instrumental gap.
- Enhanced-LRC word times, metadata lines (`[ar:…]`) and control characters are removed, and `[offset:…]` is applied.
- **Limits:** at most 4,000 lines, 500 characters a line, from at most 256 KiB of text.
- Lines that look like a server path or a LAN address are dropped, as in every other answer.

**Refusals:**
- 404 `no such track`: an unknown id.
- 404 `no lyrics`: none from either source.
- 429 with `Retry-After`: more than 4 plugin lookups are already running. The plugin may go online, so each lookup also has a 12 s limit.
- 503: LMS didn't answer.

**Caching:** answers, and "no lyrics", are remembered for 15 minutes, for up to 512 tracks. Every answer is `Cache-Control: private, no-store`.

## Artist and album information (`GET /v1/info/artist/{id}`, `GET /v1/info/album/{id}`)

Artist biographies and photos, and album reviews, for the artist and album pages (docs/plans/ux-research.md X5, part 1).

**Who may ask:** any signed-in user: a bearer access token (401 without one). It counts towards the per-user `/v1/me/…` allowance (429 with `Retry-After` beyond it). If the admin has made the Music & Artist Information plugin admin-only, anyone else gets 403 `{"error":"this plugin is for admins only"}`, as `/v1/rpc` answers for it (since 2026-10-18).

**Request:** a library artist or album id only: a positive integer, at most 12 digits, no leading zero. Anything else is 404, and any query string is 400 `unknown parameter`. No name, path or URL is ever taken from the client.

**Where it comes from:** the Music & Artist Information plugin, if it is installed. The gateway sends exactly these commands, nothing else:
- `musicartistinfo biography artist_id:<id>` and `musicartistinfo artistphoto artist_id:<id>` (at the same time);
- `musicartistinfo albumreview album_id:<id>`.

The plugin answers from its cache, local files and whatever online services the server's owner has enabled in it. The gateway itself never calls such a service.

**Answers:** 200, always with every field (`null` when there is nothing):

```json
{"biography":"First paragraph.\n\nSecond paragraph.","photo":"/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image","credits":"Discogs"}
{"review":"A fine record."}
```

- **Text** (`biography`, `review`, `credits`) is plain text, cleaned on the gateway. Clients must still show it as text, never as markup.
  - HTML tags are removed, and so is the content of `script`, `style` and similar elements and of comments. Block elements become line and paragraph breaks.
  - Common entities are decoded, once.
  - Control characters and invisible formatting characters (bidirectional overrides, zero-width characters) are removed.
  - Whitespace is collapsed. Lines are separated by `\n` and paragraphs by `\n\n`.
  - Lines that look like a server path or a LAN address are dropped, as in every other answer.
  - At most 20,000 characters (credits: 120), cut at a word with `…`. At most 512 KiB of the plugin's text is read.
- **Photo:** the plugin names an image on another site. That URL is never handed to clients.
  - If it is an http(s) URL whose host is not on the home network (the image proxy's rules under Artwork), the gateway adds it to the image proxy's allow-list. It answers with the proxy's path, `/v1/art/imageproxy/<url-encoded URL>/image`.
  - Clients fetch that path like any artwork, with their bearer token. They may add a size (`image_600x600`). LMS fetches the image, and only after the image proxy's checks (including the DNS check).
  - Anything else (a local or unusual URL, a path) means `photo` is `null`. `credits` is only given with a photo.

**Refusals:**
- 404 `no such artist` or `no such album`: a malformed id. An id the library doesn't have is a 200 with every field `null`.
- 429 with `Retry-After`: 4 requests are already waiting on the plugin, which may go online. Each lookup also has a time limit (8 s by default).
- **A lookup that fails or runs out of time** is not an error. Its part of the answer is `null`, so the app hides it, and the answer is remembered for 10 minutes. The plugin's online source can hang for one artist, and without this every visit to that artist would wait the full time limit. (Found answers are remembered for 24 hours, and "nothing found" for an hour.)

**Caching:** answers are remembered for 24 hours, and answers with nothing in them for 1 hour, for up to 1,024 artists and albums. Failures, timeouts and half answers are never remembered. A remembered photo is added to the image proxy's allow-list again each time it is answered. Every answer is `Cache-Control: private, no-store`.

## Events (`WS /v1/events`)

**Connecting:** the first message is the access token, as text. The gateway replies `{"subscribed":true}`, then sends `{"player":"<mac>","event":[...]}` for the caller's permitted players and `{"player":null,"event":["rescan",...]}` for library rescans. `{"resync":true}` means some events were missed, so re-query state.

**LMS status:** `{"lms":"busy","since":<Unix ms>}` when LMS stops answering, and `{"lms":"ok"}` when it answers again (see *When LMS is busy*). A client that subscribes while LMS is busy gets the busy message straight after `{"subscribed":true}`; otherwise it should assume LMS is fine. Older clients ignore both.

### When LMS is busy

LMS is a single process. When something blocks it, typically its music folder on network storage that has to spin its disks up before a file opens, it answers nothing at all, often for 30–70 s, and its players hear nothing either. The gateway (from `features.lms_busy`):
- marks LMS busy when any request to it has gone unanswered for 4 s (healthy answers take milliseconds), tells every events subscriber, and logs it;
- refuses new `/v1/rpc` and `/v1/art/…` requests at once with 503 `busy` rather than queue more work inside LMS (artwork since 2026-10-18; its 401 and 404 refusals come first), and doesn't re-read LMS's app menus to refuse an unknown command;
- keeps one cheap request (`serverstatus`) waiting on LMS, so the moment LMS answers it says `{"lms":"ok"}`;
- keeps player sockets alive with keepalives, so players carry on with what they have buffered.

Clients should show the state calmly ("the music server is busy"), not as an error or as being offline: the gateway is reachable. They shouldn't queue up refused commands to replay later.

**What is forwarded:** only `playlist`, `mixer`, `power`, `client`, `sync`, `pause`, `play`, `stop`, `time` and `newmetadata` events. Any token that looks like a URL or path, or that response filtering would remove (such as a bare LAN address like `10.0.0.5:9000`), is blanked, as is the id of any other player the user may not see (a `sync` names the players joined). Preference changes (`prefset`) and everything else are never forwarded.

**Rules:**
- Clients may send nothing after the token. Any message closes the socket (1008).
- Revocation and permission changes are re-checked every 30 s: a revoked device is disconnected, and new grants take effect.
- A device may hold 4 event sockets and a user 16 across all their devices. A new socket beyond either limit closes the oldest one (of that device first, otherwise of that user) with 1008 `replaced by a newer connection`. Clients must not reconnect after it: the web app treats every 1008 other than `unauthorised` as final (the page keeps polling), so two tabs can't keep evicting each other.

**Upstream:** the gateway keeps one connection to the LMS CLI (`--lms-cli-port`, default 9090) while anyone is subscribed, and reconnects with backoff. It sends only `listen 1`, never `login`: a CLI user name and password are not supported. An LMS whose CLI wants a login (password protection on) hangs up without answering, and the gateway logs that by name (`LMS closed its CLI connection without answering`) and keeps retrying; events stop meanwhile.

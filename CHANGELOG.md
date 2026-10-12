# Changelog

Dated entries, newest first.

## 2026-10-12

### Adding devices by QR code, and pairing links
- **Admins: Add a device** next to a user on the plugin's settings page (was *Pairing code*). It shows the one-time code in large letters, a QR code of the `lyrion-anywhere://pair?server=…&code=…` link built from the *Public URL* setting, and a countdown to its expiry. Without a usable `https://` Public URL it shows the code alone and says why. The code is no longer put in the page's status line.
- **Users: Add another device** in the web app (*Settings › Account*): after the account password, a code and QR code for another device of the user's own, from the new `POST /v1/devices/pair` (`docs/PROTOCOL.md` › Pairing links):
  - always for the caller's own user (there's no field to name another; unknown fields are refused);
  - the password is required, so a stolen 15-minute access token can't become a lasting sign-in elsewhere; wrong ones count towards the per-user account-change lock;
  - at most 5 codes an hour per user, and none while the user is at the 10-new-devices-an-hour limit;
  - codes stay single use, valid 10 minutes, stored only as hashes and subject to the pairing lockouts; expired ones are now deleted as new ones are made. Audited (`pairing.created` with `admin` or `self`, `pairing.rate_limited`, `pairing.self_failed`), never with the code. Database migration 9 adds the per-user request log.
- **QR codes made in-house:** a small encoder (byte mode, level M, versions 1–10) in `web/app/src/lib/qr/qr.js`, shared with the plugin's settings page as a generated classic script (`tools/sync-qr.mjs`); no third-party code runs in the browser. It matches the `qrcode` crate module for module for every mask (reference symbols made by a gateway test) and is read back by an independent test decoder.
- **Android: Scan QR code** on the welcome screen as well as the sign-in screen (CameraX and ZXing core, no Google Play Services; the camera is asked for only when scanning). Typing the code still works. Whatever is read is shown first, *Sign in to music.example.com?* with the server it replaces, and sent only when confirmed.
- **Android: pairing links.** Tapping a `lyrion-anywhere://pair` link opens the app's pairing screen with it filled in, and nothing more: the user confirms the server before anything is sent, and if the phone is signed in, is first asked whether to sign out (downloads kept). Links that aren't valid are dropped, and the user told. The intent filter takes VIEW of that scheme and host only, and only the link's text is read.
- **Security:**
  - pairing links with user info or a port on `pair` (`lyrion-anywhere://user@pair?…`) were accepted and ignored; they are now refused (regression test);
  - Android release builds refuse plain `http://` to loopback in typed, scanned and linked addresses (debug builds still allow it, for a gateway on the device). The Rust layer allowed it in every build.
- **Tests:** gateway (own codes for the caller only, the password and its lock, hourly and new-device limits, single use, expiry, lockouts on redemption, CSRF header and origin, signed-out tokens, nothing for another user), the QR encoder (reference symbols for every mask, decoding, arbitrary text, capacity limits, the ISO/IEC 18004 Reed–Solomon example, link and code validation), the plugin's panel (CSRF token, QR content decoded, countdown and expiry, no Public URL, hostile replies) and the web app's (refusals, expiry, no password kept), and on Android the confirmation step, hostile links, the dialogs, accessibility audits and the manifest's intent filter.

## 2026-10-11

### Android: release readiness (A7)
- **Security review and pentest** of everything added since the last one (`docs/security/pentest-2026-10-android-1.0.md`). No critical or high issues. Fixed, each with a regression test:
  - **crash reports could hold secrets:** Android's record of a native crash includes memory dumps, and its readable text went into the shareable report. Now only the backtrace's names are kept, and every report is redacted (tokens, secrets, server and IP addresses, home folders);
  - **play counts could be inflated** by resending one listen under new keys: a play of a track closer than the counting time to one already counted is refused (`overlap`);
  - **the plugin's read-only commands** (devices with their last addresses and User-Agents, the audit log, users) answered anyone on the LAN: they now need the settings page's token too;
  - **device renames** are limited to 10 a minute per device, an unchanged name writes nothing, and renames are audited with who made them; **the audit log is bounded** (one year, 100,000 entries);
  - **analysis time-outs** are remembered only as long as the deadline, not ten minutes for everyone;
  - **no build paths** in the Android native library; stored credentials never print their token; `sync` to the same player in another letter case is refused.
- **Accessibility:** a TalkBack, contrast, large-text and keyboard audit of the app, with fixes (queue and artwork actions reachable by TalkBack, states not shown by colour alone, art-tinted colours meeting contrast, layouts that wrap at 200 % text, keyboard shortcuts for the queue and "More…") and automated Robolectric checks.
- **Privacy policy** (`docs/PRIVACY.md`), linked from Settings › About, with Google Play's Data safety answers.
- **Performance budgets** (`docs/PERFORMANCE.md`), a baseline profile, a smaller Rust library (one codegen unit) and per-ABI APKs: the universal APK is 31.8 MB (was 39.0 MB), per-ABI ones about 16 MB.
- **Distribution:** an app bundle for Google Play with store metadata (`apps/android/fastlane/`); an F-Droid recipe that builds everything from source, including the certificate verifier's Kotlin half (`apps/android/fdroid/`); and a CI job that builds signed APKs and the bundle on `android-v*` tags and publishes a pre-release with checksums. The release checklist is `docs/RELEASING.md`.

### Plugin settings and device management
- **Fixed: the logo didn't load** on the plugin settings page. LMS serves a plugin's images only from paths with an `html/` segment (other image paths are treated as templates, and 404), so it moved to `HTML/EN/plugins/LyrionAnywhere/html/images/logo.png`.
- **Fixed: odd characters** such as "Settings âº Advanced". LMS reads templates as bytes and sends the page as UTF-8, so raw UTF-8 in a template came out encoded twice. The template now uses HTML entities (`&rsaquo;`, `&hellip;`) and the script `\u` escapes; the page's files are ASCII only, checked by the plugin tests and `tools/test-release.sh`.
- **Stay signed in: 90 days by default** (was 30), in the gateway and on the settings page, which shows it even before the gateway answers. The range is still 1–365 days. Installations that never changed it get 90 days from their devices' next renewal.
- **Devices show their last address and client type** (`docs/PROTOCOL.md` › Authentication › Devices):
  - the internet address each device was last seen from, as the reverse proxy passed it on (only trusted proxies are believed), and when. Recorded at sign-in and renewal, and on use at most once a minute per device;
  - what kind of client it is: *Installed web app* (PWA), *Browser tab* or *Android app*, with the browser and system or the phone (e.g. "Safari on iPadOS", "Pixel 8"), and the browser's User-Agent underneath. Clients report this in new optional `client_kind` and `client_platform` fields at sign-in (the web app also at every renewal), strictly checked;
  - **privacy:** only admins see these, in the plugin's device list (the admin API); listeners can't see them for any device. A device's address and User-Agent are cleared as soon as it is signed out or revoked.
  - Database migration 8 adds `last_addr`, `client_kind`, `client_platform` and `user_agent` to devices.
- **Devices can be renamed**, and their LMS player follows (at once if it is connected, using LMS's own `name` command; otherwise when it next connects):
  - admins rename any device in the plugin's device list (admin API `PATCH /admin/v1/devices/{id}`, or `POST …/{id}/name`);
  - users rename their own: web app *Settings › This device*, Android *Settings › This phone* (`PATCH /v1/devices/self`);
  - names are 1–64 characters, cleaned as before (no markup, control or bidi characters), and unique per user.
- **Android:** sign-in and pairing report the app as `android` with the phone's model. Against an older gateway, which refuses the new fields, the app signs in again without them.
- **Tests:**
  - gateway: the migration, address recording and its once-a-minute limit (sign-in, refresh, API use, player connection; forwarded addresses only via a trusted proxy), clearing on revocation, the client fields (bad kinds, long or hostile platforms, unknown fields refused on login, pair and refresh), cleaning fuzzed, renaming (own device, admin, too long, empty, hostile characters, duplicates, another user's device unreachable, no secret, not on the public listener), the LMS name following, and the 90-day default;
  - plugin: the rename operation's validation and CSRF protection, the gateway contract, the page's files ASCII only, every referenced file present (images under `html/`), and the device list rendering hostile names, platforms and User-Agents as text;
  - web app: PWA and iPadOS detection, the client fields on sign-in and renewal, renaming; mobile and Android: the client fields, the older-gateway fallback, renaming and the Settings row.

### Android: car and discovery (A6)
- **Android Auto:** browse *Home* (carry on, new music, favourites, playlists), *Library* (new music, albums, artists, playlists, favourites) and *Downloads*, search, and ask by voice; everything plays on this phone (its player is switched on if needed), or from the downloads when there's no server.
  - The playback service is now a media library service and is **exported**, so every caller is checked: Android Auto and the Google assistants only with their release certificates; the system's own controllers (platform-signed, or holding `MEDIA_CONTENT_CONTROL`) and this app; other trusted controllers (e.g. a watch app with notification access) get play, pause and skip only; anyone else is refused, as is a known package name with the wrong certificate.
  - The tree holds ids and text only, never URLs or paths, and every id coming back from the car is checked against a strict grammar (in the Rust layer, fuzzed) before anything is looked up. No content provider was added (so the car's lists have no artwork yet).
  - Voice "play … on Lyrion Anywhere" from the phone's assistant searches and plays the best match.
- **Start radio** for a track, album or artist, and **More like this**, from what the server has: AudioMuse-AI's mixes and similar lists from LMS's own context menus (run exactly as offered), or LMS's Don't Stop The Music for an endless queue. Hidden when the server offers neither.
- **Smart transitions** (Settings › Sound › Transitions; off by default, so playback stays gapless and bit-perfect): *Crossfade* or *Smart transitions*, 1–10 s, never between tracks of the same album. Smart transitions trim silence and follow each track's own fade from the server's analysis, and fall back to the fixed crossfade without one. For both streaming and downloads; the signal path says "not bit-perfect" and why.
- **Gateway:** Don't Stop The Music's settings menu and its provider setting are allowed for players the user may control, with the provider a strict token and nothing else of `playerpref`. Providers belonging to admin-only plugins are hidden and refused.
- **Tests:**
  - Rust: transition planning (album rule, trims, fade lengths, hostile and random analyses), the crossfader's mix accuracy and exact frame accounting over scripted and random sources, the offline player crossfading between albums and staying bit-exact within one; discovery parsing (fuzzed) and through the real gateway; the browse tree's grammar (fuzzed) and layout; the gateway's new entries, refusals and admin-only providers, unit and end to end.
  - Android: the caller policy (Android Auto with and without its certificate, impostors, a package claimed by another uid, system and transport-only controllers) and its package-manager reading; the library callback; the browse tree (tabs online and offline, paging, refused ids, no URIs, playing on the phone or from downloads, search and voice); the car's source; radio availability and the shelf (view models and Compose); the transitions setting; the manifest (one new exported component, with only the media browser actions).

### Android: offline (A5)
- **Downloads** of albums, playlists and tracks ("Download" on their pages and in "More…"), into app-private storage that is never backed up.
  - The download manager is in the Rust layer: two at a time, resumed with HTTP `Range` after a break, checked against `/info`'s size and stable key, and named by that key (no tokens in file names or the index).
  - It renews an expired sign-in once and retries; a 403 (downloads turned off for the account) is shown and not retried; a 429 waits as the server says; other failures back off and retry.
  - Pause, resume and cancel, each or all; the queue survives a restart; album covers are kept for offline browsing.
  - The format follows the network, as on the web (Original on Wi-Fi and Opus on mobile data by default), with *Wi-Fi only* and a storage cap.
- **Downloads screen:** what is downloading (with progress), what is on the phone (by album), storage used, remove and remove all.
- **Smart downloads (optional):** *Keep recently added albums offline (N)* and *Keep favourites offline* (library tracks and albums among the LMS favourites). Refreshed on Wi-Fi only, inside the storage cap; they never remove the user's own downloads, and a failed refresh removes nothing.
- **Offline mode:** when the server can't be reached (or the phone has no network), the app shows the downloaded library under an *Offline* banner and plays it on the phone; when the server is back it switches back by itself.
- **Playing downloads, through the same engine as streaming:** the app's own queue (play, pause, next, previous, seek, shuffle, repeat), decoded by the Rust engine with the same DSP, levelling, bit-perfect USB output and signal path, and gapless between tracks. The notification, lock screen and Bluetooth buttons work offline too. Downloads and this phone's LMS player never play at once.
- **Plays heard offline** are recorded (with an idempotency key and the download's stable key) in a durable queue and sent to `POST /v1/plays` when back online, after the server's `retry_after` when it asks; an older server without it keeps them for later.
- **Sign-out** asks whether to keep or remove the downloads (keeping is the default); a device the server revoked removes them.
- **Background downloading:** a foreground service of type `dataSync` with a progress notification and *Cancel*, only while downloads run; no WorkManager or other start-up initialiser. On Android 15's time limit, downloads pause and resume when the app is next opened.
- **Tests:**
  - Rust: the seekable file decoder (sample-accurate seeks, hostile and random files); the index parser (fuzzed), the plays queue and its sender (behind a trait, with a fake), smart planning, the play queue (random operations) and the local source (gapless, rate changes, seek, skip, repeat-one, shuffle, missing and broken files);
  - Rust against a scripted gateway: resume after a cut, a 416, a misplaced `Content-Range`, sizes that disagree, more data than announced, 401 renewal, 403, 404, 429 with `Retry-After`, pause and cancel, the storage cap, Wi-Fi only, and the queue across a restart;
  - Rust against the real gateway: a download through `/v1/download/track/{id}`, `Range` resume through it, unknown ids, downloads turned off and on again, a revoked device, and plays through `POST /v1/plays` (recorded, refused for another file's key or too short, a resent key a duplicate); the offline player end to end with real FLAC files (bit-exact and gapless, signal path, controls, plays);
  - Android: the repositories (sign-in, offline owner, revocation, sign-out, smart downloads, plays sending), offline-mode switching, the Media3 player for the local queue, the view models, and Compose tests for the Downloads, offline and storage screens and the sign-out question; the manifest (the download service is private, `dataSync`, and WorkManager's initialiser stays removed).

### Offline plays sync back to LMS (SPEC D6, plan A5)
- **Web app:** each play of a downloaded track is noted on the device when it ends (finished, skipped or stopped), with the seconds actually heard (seeking and pauses don't count).
  - Plays are kept in the browser's storage per user, next to the downloads, and sent to the server in batches when online.
  - Each play stays queued until the server has answered for it, with backoff after failures.
  - The Downloads page shows "N plays waiting to sync".
  - Removing downloads keeps waiting plays. At most 5000 are kept per user (the oldest go first).
- **Gateway: `POST /v1/plays`** (`docs/PROTOCOL.md` › Offline plays). Android will use the same endpoint.
  - Up to 200 plays an upload, each with a client-made key, so a retry never counts twice (keys are kept per user).
  - Strict checks:
    - integer track ids;
    - start time within 90 days and not in the future (5 minutes' skew allowed);
    - 0–24 h heard;
    - no unknown fields;
    - a 96 KiB cap.
  - Only local library tracks count, and with the download's stable key, only if it is still the same file.
  - **The counting rule (as scrobblers use):** half the track or 240 s heard, whichever is less.
  - Each play gets an answer: `recorded`, `duplicate`, `refused` (with the reason) or `retry` (LMS unreachable or scanning, the hourly allowance used).
  - Limits: one upload at a time per user, 30 uploads a minute, 1000 plays an hour, a 30 s budget per upload.
- **Plugin: `anywhere play_record`**. LMS has no command that counts a play, so the plugin adds one.
  - The gateway sends it over LMS JSON-RPC on loopback, signed with HMAC-SHA256 under the admin secret both already share. Unsigned, wrongly signed or stale requests are refused before anything is looked up.
  - It raises the track's play count and sets last played (never backwards) through LMS's own schema, as LMS does when a track plays. It refuses radio, streaming and unknown tracks, and answers `busy` while a scan runs.
  - Recent keys are remembered, so a resend is a duplicate.
- **Tests:**
  - Gateway, against the fake LMS and a fake plugin that checks signatures:
    - success, and dedupe on retry, within an upload and across users;
    - 401, and malformed, oversize and hostile values;
    - future and too-old times;
    - non-library, unknown and renumbered tracks, and the counting rule;
    - plugin refusals;
    - an unreachable or hung LMS;
    - one upload at a time and the rate limits;
    - the plugin command refused through `/v1/rpc`;
    - parser fuzzing and random uploads.
  - Account store: claims, stale claims, pruning and the migration.
  - Plugin (`t/09-plays.t`):
    - recording;
    - duplicates;
    - every signature and freshness refusal;
    - hostile values never reaching the library;
    - scan, statistics-off and database-error states;
    - random input.

    A shared test vector keeps the gateway's and the plugin's signatures in step.
  - Web:
    - queueing per user, batches, answers, backoff and Retry-After;
    - 401, and splitting a bad batch;
    - the queue cap;
    - what the player reports as heard;
    - the Downloads page line.

### Android: moving playback, queue editing, sync groups, tablets, widget, tile and gestures (A4 part 2)
- **Transfer (SPEC D20):** *Move playback here* on each player in *Play on…*, and *Bring to this phone* on the phone's row and the "Playing on …" strip.
  - The same recipe as the web app (`docs/PROTOCOL.md` › Transfer): group then leave, or copy the list of library tracks when LMS won't group the two, with every documented refusal.
  - Planned and run in the Rust layer. Progress shows in the sheet; refusals and errors stay there, with the error's detail, until dismissed.
  - Nothing new on the gateway: every step is an existing command between two players the account may control.
- **Queue editing**, as on the web:
  - swipe left to remove, with *Undo*;
  - drag the handle to reorder;
  - *Clear queue* after a confirmation;
  - tap to play a track.

  TalkBack users get Move up, Move down and Remove on each row.
- **Sync groups:** join or leave the controlled player's group from *Play on…*, sent the right way round (`<selected> sync <other>`).
- **Tablets:**
  - wide windows show albums, artists and playlists beside their lists, and the queue beside Now playing;
  - medium windows use one pane with the navigation rail, with two panes for Now playing when the window is short.
- **Home-screen widget:** artwork, title, artist, play/pause and next, for this phone's player (or the one the app controls). It is redrawn only when that changes, from state the app already has; it never polls or wakes the phone.
- **Quick Settings tile:** play/pause for this phone's player.
- **Gestures:**
  - swipe the mini player up to open Now playing;
  - pull Now playing down to close it;
  - swipe the artwork for next and previous, with a haptic tick.
- **Tests:**
  - Rust: transfer planning and the runner (both recipes, refusals, hostile queues, clean-up after failures); queue-edit and sync commands; Undo;
  - Rust against the real gateway: full transfers both ways and by copying, every step refused for a player the account may not control, and queue edits and sync through the allow-list;
  - Android: view models and Compose for transfer, queue editing, sync, layouts and gestures; the widget's and tile's logic; the manifest (only the widget receiver and the system-bound tile are exported beside the launcher).

### Web app: player-first, and moving playback between players (SPEC D20)
- **This device by default, everywhere** (desktops too). After sign-in the browser's player connects by itself, so it's ready; the first tap anywhere turns the sound on (browsers need a gesture), with no "Play on this device" step. iPhone and iPad keep the media-element mode as before.
  - Only one tab of a browser runs the player (a Web Lock): a second tab that opens controls it rather than taking it over. A tap that starts it takes it over.
  - If this browser can't play, or its player doesn't start (or LMS hasn't listed it after 20 s), the app controls the player used last and says why.
  - **Settings › This device › When the app opens, play on:** *This device* (the default), *The player used last*, or a named player. Kept per user, on this device.
  - A reload keeps what the tab was controlling: this device resumes by itself, a remote player stays selected.
- **No big player picker.** A small output button by the volume (Now playing and the mini bar, phones included) opens *Play on…*:
  - this device, ticked when in use;
  - the other players you may control, with what each is playing;
  - sync groups, as before.
- **"Playing on Lounge"** strip, on Now playing and above the mini bar, whenever another player is in use, with *Back to this device* and *Bring here*.
- **Transfer:** *Move here* on any other player moves the queue, track, position, shuffle and repeat there, starts it, and stops the player it came from. *Bring here* does the reverse.
  - The recipe groups the target with the source and then takes the source out (LMS copies the queue, radio and app items included, and the target plays on without a gap). It falls back to loading the same library tracks by id where LMS won't group the two. Documented in `docs/PROTOCOL.md` ("Transfer") for the native apps.
  - Verified against LMS 9.1 with test players: playing, paused, stopped, a radio stream, a target that was off, a target already in a group, a source in a group.
  - Nothing new on the gateway allow-list: every step is an existing command between two players the caller may control.
- **Fix: sync toggles.** Adding a player to the selected player's group sent the command the wrong way round when the selected player was already grouped. In LMS, `<X> sync <Y>` makes Y join X's group and take its queue, so the selected player was pulled out of its group and onto the other player's queue. It is now always sent for the selected player.
- **Tests:**
  - web: the start decision (setting, last used, reloads, unsupported browsers, hostile or blocked storage); transfer planning (both recipes, remote items, empty and long queues, hostile status values) and the runner against a scripted LMS (fallback, clean-up after failures); the output button, *Play on…*, the strip, app start-up, and the one-tab lock; the sync direction regression;
  - gateway: the whole transfer recipe passes between permitted players and is refused at every step for a player the caller may not control; over-long and malformed id lists are refused.
- **Screenshots** updated (the demo starts on Lounge, so they show the strip).

### Android: browsing on a par with the web app, rich pages, and player-first (A4 part 1)
- **Player-first (SPEC D20):**
  - This phone is the default player, and its player is on after sign-in.
  - An output button in Now playing and the mini player opens **"Play on…"**: this phone, the other players you may control (with what each is playing) and sync groups.
  - While another player is controlled, a strip says **"Playing on <name>"**, with **Back to this phone**.
  - **Settings → Default player:** this phone, last used, or a specific player.
  - The "Play on this phone" switch has left Home.
- **Library:**
  - New Music; Albums (by title, artist, year or date added); Artists with role chips; Genres (searchable, with an Albums/Artists switch); Years; Compilations; Random Albums ("Shuffle again");
  - Music Folder (play or add folders and tracks); Playlists; Favourites; Apps & Radio;
  - a virtual-library picker when LMS has libraries.
- **Home** puts the library first: recently added, random albums, favourites and playlists.
- **Album pages:** large art, details (year, genre, format such as "FLAC 24/96"), disc sections, and Play, Shuffle, Add and More.
- **Artist pages:** header art, albums grouped into *Albums*, *Appears on* and *Compilations*, and Play all, Shuffle and More.
- **Art-driven colour:**
  - the album, artist and Now playing pages take their colour from the cover;
  - an in-house quantiser picks it, and every text colour is adjusted to meet WCAG AA contrast.
- **"More…"** on tracks, albums, artists, genres, years, folders and playlists:
  - a bottom sheet with Play now, Play next and Add to queue;
  - then LMS's own menu, so plugin actions such as "more like this" appear.
- **Apps & Radio:**
  - apps, radio and plugins from LMS's home menu, with search boxes and paging;
  - apps that can't be used from here are hidden behind "Show all", and ones that need setting up say so.
- **`mobile/` (Rust):**
  - typed builders for all of the above, mirroring the web app's;
  - a SlimBrowse module (`Menus`) that replays only actions LMS offered, checks them again before sending, and lists plugin entries and their suitability;
  - sync groups; richer track details.
- **Tests:**
  - Rust: builders and parsers (hostile filters, ids, folder paths, arbitrary JSON).
  - Rust against the real gateway: every new command passes the allow-list. Forged, altered and never-offered menu actions are refused before reaching LMS, and file locations in context menus never arrive.
  - Android: view models for every browse view, the menu navigator, Apps & Radio, and the player-first rules.
  - Android Compose: the strip, the "Play on…" sheet, settings and pages; touch targets.
  - Android colour: contrast for 511 covers in both themes.

### Library: New Music and the rest of LMS's "My Music"
- **New Music:** recently added albums, newest first, in the sidebar and at the top of the Library page.
- **More ways to browse** (on the Library page):
  - *Genres:* a searchable list; a genre shows its albums or its artists, with Play all, Add to queue and More;
  - *Years:* a year shows its albums, with Play all, Add to queue and More;
  - *Compilations*;
  - *Random Albums*, with "Shuffle again";
  - *Music Folder:* browse folders and play or add folders and tracks, with More on each.
- **Artists by role:** album artists by default (as in LMS), all artists, and composers, conductors and bands where the library has them.
- **Album sorting** by title, artist, year or date added, on Albums, a genre's, a year's and Compilations.
- **Virtual libraries:** if LMS has any, the Library page offers a picker. Browse lists then show only that library, and the choice is remembered per user on this device. Without virtual libraries nothing changes.
- **Gateway allow-list:**
  - `role_id:` takes only LMS's role names (it took free text, and LMS fails outright on an unknown role);
  - `library_id:` takes only short hex;
  - `musicfolder` takes a single numeric `folder_id` (no lists);
  - `genres` takes a `genre_id`;
  - `libraries` is allowed with no arguments.
  - The music folder's file URLs, and any name holding a path, are stripped from its answers as before.
- **Tests:**
  - gateway: negative tests for every new parameter (wrong types, paths, `url:`, oversize values, unknown roles, injection attempts), and path-bearing music folder answers, filtered and end to end;
  - web: the builders (every new command pinned in the fixture the gateway checks), the pager, the remembered library choice (including storage that refuses access), music folder entries, and the pages with hostile names.
- **Demo:** genres, years, a music folder, recently added albums, compilations, composers and a virtual library, all invented.

### Browser player: seeks and volume take effect at once
- **Seek fix:** a seek could take about 8 s to be heard on iPhone, iPad and Android (media-element mode), and on MP3 it seemed not to work at all.
  - **Cause:** LMS sends the flush and the new stream back to back (about 2 ms apart for MP3, about 20 ms for FLAC). The player only dropped its queue when its 20 ms timer happened to see the stopped state in between, so the old ~8 s of queued audio played first.
  - **Fix:** the queue is now dropped on the flush itself, from the core's flush count, and the element's play head is moved to the new audio so the browser drops what it had already decoded. "Played" reports from before the flush no longer count against the new position (Web Audio mode played up to 0.3 s of old audio).
  - **Measured** against a real LMS (desktop Chromium on the LAN, command to new audio): MP3 seeks with the old code skipped the flush 4 times in 5 (the old queue played out first); now 25–80 ms in both modes, for MP3 (CBR and VBR) and FLAC. With a 100 ms timer the old code missed 3 flushes in 4; the new code misses none.
- **Quicker start after a seek:** playback starts once 0.5 s is decoded, rather than after all of LMS's 255 KiB threshold, if the stream arrives at least twice as fast as it plays (not live radio). On a throttled link, MP3 seeks are heard in about 0.3 s. FLAC is still bound by its headers (embedded artwork) and the decoder's 128 KiB read-ahead.
- **Seek timing in the connection log:** each seek logs when the new stream opened, when the first bytes arrived, and when the audio started, was queued and was heard.
- **Volume on this device:** LMS's volume is now heard on browser players, at once, from any controller.
  - **Where it's applied:** in the AudioWorklet (Web Audio), or with the element's `volume` (media element), after the queued audio, ramped over 30 ms. Before, it was applied when audio was decoded, up to 8 s ahead.
  - **iPhone and iPad:** a page can't set the element's volume, so the device plays at full scale and LMS's volume is held at 100%. Now playing and the player bar say "Use this device's volume buttons".
  - **Settings › This device › Volume on this device:** Automatic, Follow LMS volume, or Device buttons only.
- **Core:** `Player::set_host_volume` (the host applies volume) and `Player::set_quick_start`.
- **Tests:**
  - core: volume left to the host, quick start (fast stream, minimum, live-rate stream, after a seek), flushes counted when the new stream arrives with them, and MP3 (CBR and VBR) decoded from arbitrary byte offsets, including a full seek through the player;
  - web: the volume policy, gains and ramps, the note replacing the slider, the media element seek, the worklet's gain (including hostile values) and flush epochs, and the seek timer.

### A logo
- **The Lyrion Anywhere logo:** an original design in the family of the Lyrion Music Server logo, with pale-green waveform bars on a dark rounded square. The waveform turns into broadcast waves: your music, reaching you wherever you are.
- **Where it appears:**
  - *web app:* the favicon (a bolder version for small sizes), the app icons on home screens and Android (maskable), the Apple touch icon, the sidebar and sign-in, and a new About section in Settings with the version;
  - *plugin:* the settings page, and the release's `repo.xml` gives LMS's plugin manager an icon;
  - *Android:* the adaptive launcher icon (with a monochrome version for themed icons) and the About entry;
  - *README:* its header.

### Fixes
- **Android: "invalid peer certificate: revoked" for valid certificates.** Android's revocation check, as used by rustls-platform-verifier, reports any failure to check revocation as "revoked". Let's Encrypt certificates have had no OCSP responder since 2025, and Android can't fetch CRLs, so every such site failed. A "revoked" verdict from the platform is now checked again with webpki against the device's own system CA store, and accepted only if that passes; every other verdict stands. Network errors now also show their cause, so connection problems can be diagnosed.
- **Tapping the position bar of a paused or stopped track now plays from there** (web app and Android), instead of only moving the position.
- **Tracks stopping part-way:** when a player held enough audio not to need more for a minute (a long buffer, or paused), the gateway ended its stream cleanly, and the player took that for the end of the track and cut it short. The stream now ends with an error, so the player fetches the rest from where it was.
- **No other clean ends that aren't the end of the track:** a stream request resumed after LMS's stream had failed or stalled was served the rest and then ended cleanly, so the player reported a cut-off track as finished. Cancelled and superseded requests ended cleanly too. Only a stream LMS itself finished cleanly now ends cleanly.
- **Media element (iPhone, iPad, Android browsers):**
  - *"Played everything" only at the real end:* a stall that wasn't the end (the element paused, its play head stuck just before new audio, or audio still being appended) counted as having played all the audio handed over. The player then reported audio still to come as played, and near the end of a track told LMS the next track had started. A play head stuck just short of the new audio is now moved onto it.
  - *Paused by the system:* when the system paused the element (a call, another app's audio), LMS and the player carried on as if playing, in silence. The player is now paused in LMS too, so Play resumes it.
  - *A tap no longer plays old audio:* while the sound waited for a tap, a tap restarted the element even if LMS had paused the player meanwhile.
- **Dead connections noticed in time:** a player socket that died without closing (a phone or tablet changing networks) went unnoticed until the gateway's 60 s grace had run out; LMS had dropped the player after about 15 s, so the session ended and the player started a new one, which makes LMS restart the track, or stop it if paused.
  - *Gateway:* reports to LMS for an attached client that has been silent for 8 s, as it does for a detached one, until the client speaks again.
  - *Browser and Android players:* a socket that has brought nothing for 20 s (browser) or 25 s (Android) is given up and the session resumed (LMS polls every 5 s). On Android the limit was 60 s and was reset by the player's own messages, so it never fired while playing; sending to a dead socket is now limited to 10 s too.
- **Browser player: fewer new sessions.** A socket closed with 1001 started a new session straight away. Browsers use that code themselves, so the player now tries to resume first and starts afresh only if the gateway refuses (4404).
- **Browser player: an expired access token no longer fails the track.** A stream request refused with 401 failed the track (LMS then skipped or stopped). The player now renews the token and carries on; a second refusal is final.
- **Now playing showed the wrong track.** LMS's status leaves out a queued track it can't resolve, e.g. one a rescan gave a new id. The next track was then shown as the current one, with its title, artwork, length and waveform, while the real track played. When the status contradicts itself (the player's duration doesn't match the listed track), the gateway now looks up the track really at the current position and shows that.
- **Waveform and album downloads on a real LMS:** LMS sends `remote` as `"0"`, which the web app took for a stream.
- **Waveforms for everyone:** the waveform no longer needs the download permission.

### Audiophile output (A2)
- **Bit-perfect USB DAC output (Android 14+, detected at run time):** with a USB DAC that offers it, the app sets Android's bit-perfect mixer attributes for the track's rate and depth. The engine then plays to the DAC in that integer format (16, 24 or 32-bit), so the samples reach it unchanged.
  - **Fallback:** Android's mixer (32-bit float) whenever the phone, the DAC or a format refuses.
  - **Volume:** fixed at 100 % on a bit-perfect route; the DAC's own volume sets the level (SPEC D19).
  - **Changes:** the output switches format at track boundaries, and renegotiates when a DAC is plugged in or out or the setting changes.
- **Signal path:** "bit-perfect" end to end only when the mixer attributes were applied and the stream's format and rate match the source. It shows the DAC's name and accepted format, and every reason it falls short: the setting is off, there is no USB DAC, Android 13 or older, the DAC doesn't offer the mode or rate, DSP, resampling, a lossy source, and so on. Bluetooth shows the route and device name only; the codec isn't readable without system APIs.
- **Sound settings (Android):**
  - preamp;
  - loudness levelling: off, ReplayGain track or album, or the server's R128 analysis (A3's switch migrates to the last);
  - the true-peak limiter and its ceiling;
  - a parametric EQ from an AutoEq profile, pasted or picked as a file and checked by the engine;
  - crossfeed presets;
  - dither.

  Everything is off by default, with a "Bit-perfect" indicator that changes as soon as any stage is on. Settings are saved and apply live.
- **Opus:** advertised to LMS only by builds that decode it, and now tested end to end through the player.
- **Robustness fix:** the streaming decoder (the Android player and the web player) refuses WAV headers that overflowed Symphonia's reader, before the reader sees them. Native builds also catch any other decoder panic. A regression test fails without the fix.
- **Tests:**
  - Rust: 632 across the workspace, including:
    - integer bit-exactness at 16/24/32-bit for 44.1–192 kHz with the real decoder in the loop;
    - format negotiation as pure functions;
    - a fake DAC end to end: the files' integers arrive with LMS's volume at 25 %, a refused mixer, DSP on a DAC, a depth change, unplugging;
    - Opus through the player;
  - Android: 122 JVM/Robolectric tests (mixer selection, settings persistence, the Sound screen, signal-path reasons); lint clean;
  - web: unchanged (286 Vitest, 24 Node).
  - **Not verified on hardware:** bit-perfect USB output hasn't run on a phone or DAC yet. See the A2 status in `docs/plans/native-apps.md`, and the manual test procedure in `apps/android/README.md`.

### Analysis and waveform (A3)
- **Gateway track analysis:** `GET /v1/analysis/track/{id}` gives a track's waveform, EBU R128 loudness, loudness range, true peak, silence and fades. It answers in JSON or a compact binary form, with ETags.
  - **Access:** any signed-in user, for any library track; not tied to the download permission.
  - **How it runs:** the gateway decodes the track with the core, off the async runtime, and caches the result on disk by the stable track key.
  - **While it runs:** a request waits up to 10 s, then gets 202 with `Retry-After`.
  - **Bounded:** one analysis at a time, per-user limits (429), size and duration caps (413) and a deadline (504). Corrupt files are a clean 422.
  - **Switches:** `--no-analysis` and `--analysis-cache-mb`, and the plugin's *Track analysis* setting. `/v1/me` reports `features.analysis`.
- **Web app:** a waveform seek bar in Now playing. The played part is in the accent colour, and it is themed for every theme in light and dark. It is still the accessible native slider, with a plain-slider fallback.
- **Android:**
  - a waveform seek bar in Now Playing;
  - **Loudness levelling** in Settings, off by default (bit-perfect). It levels each track to −18 LUFS from its R128 analysis, falls back to ReplayGain tags, uses the true-peak limiter, and the signal path shows the source.
- **Core:** decoding of a whole seekable file, with decoder panics caught (a new robustness test found a Symphonia WAV header overflow). Binary analyses now decode exactly to the values encoded.
- **Demo LMS:** serves generated audio, so waveforms can be tried without a library.
- **Tests:**
  - Rust: 605 across the workspace, including 12 gateway integration tests for the endpoint's security and limits, and end-to-end levelling;
  - web: 286 Vitest, including decoder robustness;
  - Android: 92 JVM/Robolectric tests;
  - plugin: 825 Perl tests.
  - Not yet run on a phone: see the A3 status in `docs/plans/native-apps.md`.

### Android: the phone as a player (A1)
- **"Play on this phone":** the phone becomes an LMS player through the gateway, like the web app's player. LMS owns the queue.
  - Reconnects resume the same player, and LMS never sees it leave.
  - Broken downloads resume at their byte offset.
  - Expired sign-ins are renewed.
- **The audio engine is Rust:**
  - the core's decoders and a lock-free ring buffer;
  - AAudio output driven from Rust, opened at each track's sample rate;
  - bit-perfect by default, with optional preamp, ReplayGain levelling (from the file's tags) and a true-peak limiter.
- **Signal-path panel** in Now Playing: source, every stage, output, and an honest verdict ("bit-perfect up to Android's mixer" until A2's USB mode).
- **Media3 session:**
  - the notification, lock screen, Bluetooth and headset buttons;
  - audio focus (pause, duck, resume after a call), and "becoming noisy";
  - wake and Wi-Fi locks while playing.
- **Core:**
  - reports each stream's codec, bit depth and ReplayGain tags (parsed strictly);
  - can pull one track at a time;
  - counts the host's buffer in STAT and reports output underruns (`STMo`).
- **Tests:**
  - Rust: 573 across the workspace, including the player end to end against the real gateway and a fake LMS (gapless and bit-perfect FLAC, a rate change, a resumed disconnect, token expiry, refusals, hostile frames);
  - Android: 74 JVM/Robolectric tests; lint clean.
  - Not yet run on a phone: see the A1 status in `docs/plans/native-apps.md` for what is verified on hardware and what isn't.

### Native apps: foundations (A0) and the audio engine
- **Android app (`apps/android`), remote control:**
  - covers sign-in (with two-step verification), pairing links, home, albums, artists, search, now playing and settings;
  - Material You with adaptive navigation;
  - runs on Android 10 and later.
- **`mobile/` crate:** the network side of the native apps in Rust, exposed to Kotlin (and later Swift) through UniFFI. It covers sessions with single-flight token refresh, the command API, artwork, and an events stream that reconnects by itself. TLS uses the phone's own certificate store.
- **Security on the phone:** the refresh token is sealed with AES-256-GCM under a non-exportable Android Keystore key, and backups are off. Release builds allow no cleartext traffic and trust only system certificates.
- **Audio engine in the core:**
  - *Processing:* a 64-bit chain (levelling, 4× oversampled true-peak limiter, parametric EQ with AutoEq import, crossfeed, TPDF dither, resampling, crossfades), bit-perfect when bypassed.
  - *Analysis:* streaming EBU R128 loudness, loudness range, true peak, a 1,000-point waveform, and silence and fade detection.
  - *Formats:* Opus decoding behind the `opus` feature.
- **Tests:**
  - Rust: 526 tests across the workspace, including integration tests of the mobile crate against the real gateway, and fuzz tests on the engine;
  - Android: 35 JVM tests (view models, screens, sealing, network policy) and lint with no baseline;
  - CI: a new Android job.

## 2026-10-10

### Security testing of the internet-facing product: fixes
Live black-box testing of the deployment (110 checks) and an aggressive local penetration test (hostile fake LMS, fuzzing, floods) found no critical or high issues. Everything below is fixed, each with a regression test that failed before the fix.

**Medium**
- **Artwork relay:** images are streamed with a running 20 MiB cap, instead of being held whole in memory. A hostile response no longer drives memory up (measured 593 MiB before, 28 MiB after). Artwork requests are limited to 8 at a time per user and 32 overall.
- **Sign-in lockouts:**
  - the client address is taken from the right of `X-Forwarded-For`, skipping trusted proxies, so a client can't pick its own address behind an appending proxy;
  - an optional `--client-ip-header` (e.g. `CF-Connecting-IP`), also in the plugin settings, is honoured from trusted proxies only.
- **Account lockout by strangers:** someone else's wrong guesses can no longer lock you out. The lock now applies to the addresses that failed; other addresses share a small budget (2 attempts a minute), so guessing stays capped while the real user can still sign in. Pairing codes are limited the same way.
- **Event connections:** at most 4 per device and 16 per user; the oldest is closed.

**Low**
- **Response filtering:**
  - *Local URLs:* URLs of any scheme pointing at local hosts are caught, including `smb://`, `nfs://` and `ftp://`, numeric and short IP forms, single-label host names, CGNAT and IPv6 link-local addresses, and bare LAN addresses in text. The events stream is filtered the same way.
  - *Other servers:* their player lists are always removed, as are the server version and last scan time.
- **Artwork from third-party URLs:** names that point into your LAN (wildcard-DNS services, single-label names) are refused, and the name is also resolved and refused if it points at a local address.
- **Free text:** search terms and other text refuse local URLs, `file:` and file paths, while ordinary searches and public feed URLs still work.
- **Token refresh in two tabs:** two tabs or windows refreshing at once no longer revoke your own device. Theft (reuse of a token after its replacement was used) still does. Tabs also take turns refreshing.
- **Names:** device and player names are cleaned of markup, control characters and bidirectional-text tricks.
- **Devices:**
  - new devices are limited to 10 per user an hour;
  - a revoked device's LMS player is now removed from LMS;
  - revoked devices are deleted after 30 days.
- **Limits and timeouts:**
  - LMS responses are capped at 16 MiB;
  - request headers and bodies time out after 15 s, and idle connections close;
  - connections are capped with `--max-connections`;
  - streams and WebSockets are unaffected.
- **Downloads:** a failed LMS request is retried once, and every upstream failure is logged. LMS answers a request for just the first byte (`bytes=0-0`) with the whole file; the gateway used to refuse that (502) and now relays only the part asked for. Found by the live test.

**Info**
- **Headers:**
  - personal `/v1` responses are `no-store`;
  - signing out clears cached artwork (`Clear-Site-Data`);
  - a `Permissions-Policy` header is added;
  - the "TLS required" refusal now carries the security headers.
- **Origins:** only `https://` origins are accepted, apart from plain-HTTP loopback for development.

### iPad background playback: solved
- **Cause:** iPadOS Safari's Managed Media Source refuses FLAC frames that use the *variable* block-size strategy.
- **What plays:** the self-test (version 5) showed fixed-block FLAC playing at every sample rate, as 24-bit or 16-bit, and Apple Lossless (ALAC) too.
- **What the iPad uses now:** the iPad picks fixed-block 24-bit FLAC (variant 0x100), so it plays through the media element, which iOS keeps running with the screen locked. Confirmed on the device, and still bit-exact.

### Offline downloads, queue editing and sync groups
- **Offline downloads:**
  - **Downloading:** download albums (or the rest of one) to the device, as the original files or converted to MP3 (320 kb/s) or Opus (160 kb/s) when the server has `ffmpeg`. Default formats differ for Wi-Fi and mobile data. Downloads resume after interruptions, even after a reload.
  - **Downloads page:** shows storage use and lets you remove albums or everything.
  - **Offline playback:** downloads play through the browser's own audio element, with lock-screen controls, and also with no connection. When the server can't be reached, the app offers "Play your downloads".
  - **Privacy:** downloads are kept separately for each user. Signing out asks whether to remove them.
- **Gateway:**
  - **Endpoints:** `/v1/download/track/{id}` and `/info`. Only library files can be downloaded. Downloads support Range requests, and each track gets a stable key that never reveals file paths.
  - **Limits:** each user can download in at most 3 parallel streams, and a running download ends if its device is revoked. Each user has a download permission and a set of allowed formats, set on the plugin settings page.
  - **Conversion:** `ffmpeg` runs hardened (fixed arguments, no shell, input limited to files, temporary files only), into a size-capped cache (default 2 GiB).
  - **New options:** `--ffmpeg`, `--no-transcode` and `--download-cache-mb`.
- **Queue editing:**
  - drag to reorder (or Alt+Arrow keys);
  - swipe left to remove, with Undo;
  - changes show at once and are undone if LMS refuses them.
- **Sync groups:** in the player menu, group players to play in sync, or ungroup them. The player button shows the group, e.g. "Lounge + Study".
- **Fix:** the offline player's audio element is now part of the page, as mobile browsers require for background playback.
- **Tests:**
  - **Gateway:** 24 end-to-end download tests (garbage ids, Range, hostile file names, permissions, limits, timeouts, a fake ffmpeg, the cache), with unit and robustness tests for the parsers.
  - **Web app:** about 100 new tests for downloads, the offline player, queue gestures and sync groups.
  - **Plugin:** new tests, plus deployment checks.
  - **Checked end to end:** download and offline playback in Chromium against a real LMS, in all three formats.

### Now playing on phones: redesigned to fit without squashing
- **Why:** fitting the old layout to the screen made the cover small and crammed the details together.
- **New phone layout:**
  - the player picker, Queue and More sit in one row at the top (Queue and More as icon buttons), which frees the space the bottom button row used;
  - the cover takes the remaining height, up to the full width;
  - the title, artist, album and controls are centred under it, with comfortable spacing;
  - on short screens (up to 700 px tall), the quality line is left out first;
  - landscape phones still scroll.
- **Wide screens:** the same top row (player, Queue, More) moves above the details, next to the cover.
- **Checked:** no scrolling at 360×600, 360×680, 390×760, 412×780 and 412×915, and on desktop.

### Fixes: flickering menus; Now playing fits the phone screen
- **Menus flickered while open.** The page around them re-renders whenever the player's status refreshes (every 2 s). It handed the menu a new but identical source each time, so the menu started over: it flickered, and a submenu jumped back to the top. Menus now restart only when what they show really changes, such as a different track. Regression test: re-renders with an identical source load the menu once (the old code loaded it 6 times).
- **Now playing scrolled on phones, with a gap at the bottom.**
  - The layout used `100vh`, which on mobile browsers is the height with the address bar hidden, taller than what you see. It now uses the dynamic viewport height everywhere.
  - On phones, the page also kept room for the mini player, which is hidden there.
  - Now playing now fills exactly the screen above the tab bar, and the cover shrinks to fit. Checked at 360×640, 360×740, 390×844 and 412×915.
  - Very short screens (landscape) still scroll rather than squash.

### Admin-only plugins; Android reload fixes
- **Admin-only plugins:** the plugin settings page lists the plugins LMS offers (apps, radio, and My Music entries such as AudioMuse-AI). Tick a plugin to keep it for admins. Listeners then don't see it anywhere (app lists, plugins, context menus), and its commands are refused by the gateway, however they are reached. Changes apply at once.
- **Fix (Android): no sound after a reload.** The device restarted, but the sound waited for a tap, and the app listened for the wrong event. Browsers count a touch as a gesture only when the finger lifts (`pointerup`/`touchend`/`click`), not on `pointerdown`. The app now listens for those, and keeps listening until the sound really is on.
- **Fix (Android): media-element mode never chosen.** Firefox kept an old cached copy of the player's WebAssembly core, so the self-test failed ("WasmSegmenter is not a constructor") and the app fell back to Web Audio, which stops in the background. The core and the worker's helpers now load with the build version in their URLs, so a stale copy is never used. A self-test that fails on our side is no longer remembered, and the self-test runs again everywhere (version 5).
- **Tests:**
  - **Admin-only plugins:** storage and validation; hiding in app lists, the home menu and context menus; end to end for listeners vs admins; the admin endpoint; plugin operations, the token requirement and the contract against the real gateway; the settings page shows names as text.
  - **Android fixes:** tap-to-unmute with a browser that refuses touch-down, and versioned loading of the core.

### Security and reliability review: fixes
A full review covered the gateway's remote-access surface, the player and streaming path, the web app, the LMS plugin and the build. Every fix has tests; the security fixes' regression tests fail on the code before the fix.

**Remote access (the gateway's public port)**
- **Player connections:** only the Slimproto messages a software player needs reach LMS (HELO, STAT, RESP, META, DSCO, BYE!, and a name reply only when LMS asked). Everything else is refused, including IR/BUTN/KNOB remote-control input, which got round the command allow-list, and unsolicited renames. HELO must claim the software-player id; capabilities and names are sanitised.
- **Artwork SSRF:** image URLs are learned only from image fields of filtered responses, never from free text (e.g. a player name or track title holding an `imageproxy/` URL), and never for LAN hosts. Fetches are refused for them too.
- **Logins:**
  - at most 4 password checks run at once;
  - attempts count before the check, so parallel guesses can't beat the 5-try lockout;
  - IPv6 clients are limited per /64;
  - the lockout table is bounded.
- **Sign-in tokens:**
  - refresh rotation is one atomic transaction;
  - a token discarded by the 60-second lost-response rule is remembered, so a thief replaying within the window is caught when the owner returns;
  - two-step setup needs the password;
  - an admin password reset signs the user out everywhere;
  - two-step codes can't be reused by racing requests;
  - at most 20 devices per user (the least recently used is signed out).
- **Leaks:** track-info lines holding file paths or LAN URLs (e.g. "Location: /…") are dropped, and sync information shows only players you may control.
- **Streams:** ending or revoking a session stops its audio stream at once. A stalled stream times out, and can no longer hold up the player's control connection.

**Stability**
- A FLAC with very large embedded artwork no longer stalls playback.
- A slow client is closed cleanly and can resume.
- Overlong LMS event lines are dropped whole.
- A replaced connection can no longer feed the session.
- Fixed a race that could leave the events stream without a listener.

**Web app**
- **Audio:**
  - a sample-rate change creates one audio context, not dozens;
  - "Play on this device" works again after a disconnect or error;
  - Stop while starting really stops.
- **Signing out:**
  - sign-out holds even with a renewal in flight;
  - if offline, the app finishes signing out on the next load instead of signing back in.
- **Recovering after a failed renewal:** events and the player reconnect after a renewal that failed offline.
- **Artwork:** artwork paths can't be redirected to other API paths by encoded dots.
- **Resume after a reload:**
  - asks about this device's own player;
  - checks the track is still the same.
- **Smaller fixes:**
  - diagnostics retry after renewal;
  - the player picker supports Escape and focus;
  - no timers are left behind by a fast sign-out.

**LMS plugin and build (LAN side)**
- **Account commands:** commands that change accounts need a token that only the settings page has. A web page visited by someone on your network can no longer create a remote admin account through LMS's unauthenticated JSON-RPC.
- **Plugin settings:**
  - settings are validated however they are set, including LMS's `pref` command;
  - the admin port can't redirect the admin secret elsewhere;
  - a custom gateway binary is honoured only with `ANYWHERE_ALLOW_GATEWAY_PATH=1`.
- **Gateway supervision:**
  - crash restarts back off properly (capped at 60 s);
  - IP addresses are checked strictly;
  - the gateway's exit reason is shown.
- **Fix:** the sign-in lifetime could not be saved from the settings page.
- **CI:** actions are pinned to commit SHAs and base images to digests, with Dependabot keeping them current.

### Media element: Apple Lossless (ALAC) as the last resort
- **Why:** if Safari refuses every FLAC frame style, the next candidate is ALAC, which the iPad reports it supports through Media Source.
- **Core:** the segmenter can write ALAC (variant bit 10): uncompressed ("escape") frames, 16 or 24 bit, in an `alac` sample entry with its configuration. It's written from the published format description, and decodes bit-exactly (Symphonia), including a short final frame.
- **Self-test:** probes two ALAC styles after the four FLAC ones (test version 4), appended as `audio/mp4; codecs="alac"`. FLAC is preferred when it plays; when ALAC wins, the device uses the ALAC type.

### Apps and plugins judged by what they actually offer
- **Dynamic suitability:** each app and plugin is judged from its own top menu, with nothing hard-coded. Anything to open, play or search means it's usable. A lone message, such as "Log in to SoundCloud in Advanced settings", means it needs setting up in LMS: the tile says so, with LMS's message. Empty or broken ones (for example a streaming app with no account) are tucked away behind "Show all". Probes run three at a time, in the background, and the answers are kept for 10 minutes.
- **Menus:** entries that run something (e.g. AudioMuse-AI's "Similar to current track") now show a play mark, unlike those that open a submenu.
- **Tests:** classification from real LMS answers (SoundCloud's setup message, Spotty's empty menu), errors and junk, concurrency limits, the 10-minute reuse, and tampered stored answers.

### Installable app (iPhone, iPad, Android, desktop)
- **Install:** a web app manifest, with an app icon (standard and maskable versions, and an Apple touch icon). The app opens full screen from the home screen and keeps its settings. Settings shows "Install the app": an Install button where the browser offers one (Android, desktop Chromium), and Share › Add to Home Screen steps on iPhone and iPad.
- **Service worker:** the app opens quickly and offline. Offline, it says it can't reach the server instead of showing the sign-in form. It only stores the app's own files (hashed files kept, the rest refreshed on each load); API calls, audio, artwork and the player's files always go to the network and are never stored.
- **Tests:** the caching policy, which refuses the API (including under a sub-path), queries, other origins and non-GET requests, and stores only complete answers; install detection, including iPadOS presenting itself as a Mac. End to end in Chromium:
  - the worker runs under the strict CSP with no violations;
  - the manifest and icons are served;
  - the app opens offline;
  - the cache holds no API or player data;
  - playback and reload-resume still pass with the worker active.

### Themes, with Material Design as the default
- **Themes:**
  - **Material** (new default): Material Design 3 colour roles generated from an accent colour, with tonal surfaces instead of outlines, pill-shaped navigation indicators and large rounded shapes;
  - **Classic:** the original look;
  - **High contrast:** black and white, with strong outlines and focus rings.
- **Choices:** light, dark or automatic (follows the device, and switches live). For Material, eight accent colours or any colour. Previews of each theme in Settings › Appearance. The mobile browser bar takes the theme's colour.
- **Fix:** the light/dark choice was only applied after opening Settings; themes now apply at start-up, including on the sign-in page. The old setting carries over.
- **Colour:** palettes are computed in OKLCH and brought into the sRGB gamut keeping lightness, so every accent gives readable text.
- **Tests:**
  - colour conversion against reference values, gamut mapping, WCAG contrast;
  - AA contrast for text, muted text, errors and text on accent, for every theme, mode and accent, including extremes such as yellow, white and black;
  - saved choices are checked, including hostile values;
  - applying a theme.

### Staying signed in
- **Why devices were signed out:**
  - **Refresh reuse:** the live audit log showed a `refresh.reused` event that revoked a real device. When a phone suspends mid-refresh, the new token never arrives, and the next refresh presents the old one. That looked like theft, so the gateway revoked the device.
  - **Failed renewals:** the web app also signed out whenever a renewal failed for any reason (no network, the gateway restarting).
- **Gateway:** a used refresh token presented again within 60 s is treated as a lost response, if the token that replaced it is still unused. The unused replacement is discarded and a fresh one issued (audit `refresh.retried`). If the replacement has been used, two parties hold tokens for the device, so it is still revoked as before. Concurrent refreshes no longer race. Migration 4 adds token history and a settings table.
- **Sign-in lifetime:** configurable from 1 to 365 days (default 30) on the plugin settings page (Sign-in › Stay signed in), via `GET/POST /admin/v1/settings`. Every use renews it, so devices in regular use never sign in again.
- **Web app:** only a refusal signs out. If the server can't be reached, the app keeps the sign-in and retries (2 s, 5 s, 15 s, 30 s, then every minute). At page load it shows "Can't reach the music server" rather than the sign-in form.
- **Security fix (plugin):** every input check used Perl's `$`, which also matches before a trailing newline. So values such as `"alice\n"` passed validation and went into admin API paths and the gateway's command line. The checks now use `\z`, with regression tests that failed before the fix.
- **Tests:**
  - **Gateway:** the lost-response retry, replays outside the window or after the replacement was used (still revoked), a clock running backwards, the lifetime's bounds and sliding renewal, expiry, and the migration.
  - **End to end:** a lost response doesn't sign the device out (and its player session stays open), theft is still detected, and admin settings with hostile input and no secret.
  - **Plugin:** operation mapping and validation, and the contract test against the real gateway.
  - **Web app:** network failures keep the sign-in and retry, refusals sign out, and sign-out stops retries.

### Carry on playing after a reload
- **Before:** after a reload the app offered a "Resume playing here" button, and could switch to another player while this device's player was briefly gone from LMS.
- **Now:**
  - this device restarts by itself after a reload, on the same track and at about the same time. If LMS has stopped meanwhile, the app jumps back to where it was; the point is kept for 10 minutes;
  - the app holds on to this device as the selected player while it reconnects;
  - browsers only let sound start after a tap: if the sound is held back, a banner says so, and any tap or key press turns it on;
  - starting without a tap no longer waits for one (Web Audio).
- **Tests:** the resume point is checked, including hostile or stale saved values, and only the commands that are needed are sent; the selection is held while waiting (a regression test that fails without the fix). End to end in Chromium, in both playback modes: reload mid-track, carry on at the same track and time, still selected, a tap turns the sound on.

### Plugins that add themselves to My Music (AudioMuse-AI and others)
- **Bug:** AudioMuse-AI didn't appear. It adds itself to LMS's My Music menu, not to the apps list, as do Custom Browse, Library Views, Random Mix and many others; the app only read the apps and radio lists.
- **Fix:** Apps & Radio now also lists plugins from the player's home menu, with nothing hard-coded: whatever LMS offers there shows up. Groups such as Random Mix open as a list of their entries.
- **Navigation:** plugin menus follow LMS's own client convention: choosing an entry opens a submenu unless the entry says where to go next (`nextWindow`), in which case it runs. Previously only lists named `… items` opened as menus, so most of AudioMuse-AI's menu wouldn't have opened.
- **Gateway:** `menu` is allowed for permitted players, filtered: settings, player power and built-in library entries are dropped, so their actions can never be learned. Navigations may be paged; commands still run exactly as offered.
- **Tests:** home-menu filter (settings, power, orphans and LAN URLs refused), navigation paging rules, end to end through the gateway, the web model and the command contract fixture. Checked against a real LMS with AudioMuse-AI and Custom Browse.

### Media element: more FLAC frame styles for Safari
- **Why:** the self-test on the iPad showed Safari refusing all six header variants ("buffer error"), so the problem is likely in the FLAC frames themselves.
- **Core:** the segmenter can also write FLAC's fixed block-size strategy and 16-bit samples (variant bits 8 and 9). Both decode bit-exactly in the core tests.
- **Self-test:** now probes all four frame styles (24 tiny streams) and picks the best that plays (test version 3, so browsers test again).
- **Cache fix:** after an update, browsers could keep an old copy of the player's WebAssembly (Android: "WasmSegmenter is not a constructor"). App files now revalidate on every load; only hashed files are cached long-term.

### Media element: self-test, header variants and automatic fallback
- **Diagnostics from the devices showed:**
  - **iPad (Safari):** accepts the FLAC type, then rejects the first segment (media error 4). The player kept "playing" with no sound, and the progress bar jumped back and forth.
  - **Android (Firefox):** the media source never opened, so "Starting…" hung.
- **Self-test:** once per browser version, in the background, the app plays six tiny test streams through Media Source: 44.1 and 96 kHz, three header brands, and three ways of writing rates above 65,535 Hz. It caches the results and reports them in diagnostics. The media element is used only with a variant the browser really plays, and only if every sample rate works; otherwise Web Audio, with the reason shown.
- **Core:** init-segment variants (`Variant`: brand `iso6`/`iso5`/`mp42`; high rates written as 0, as a version-1 entry with an `srat` box, or as a rate divisor). Every variant still decodes bit-exactly.
- **Runtime fallback:** if the media element fails anyway, or its source doesn't open within 4 s, the device restarts on Web Audio by itself. If the browser then holds audio back, the next tap anywhere resumes it.
- **Tests:** variant field and decoding tests (core), variant choice and cache tests (web); end to end in Chromium (the self-test accepts every variant there, and playback passes).

### Fix: this device missing from the player list (Android)
- **Bug:** after "Play on this device", LMS can take a moment to list the new player. The app looked once, didn't find it, then picked another player or none, so the device never appeared selected.
- **Fix:** the app now keeps checking (up to 10 s) and selects the device the moment LMS lists it; later refreshes keep that choice. Regression tests added.

### Device diagnostics for troubleshooting
- **Web app:** reports its capabilities, playback choice (with reason and MIME type), player status and recent connection log. It sends once when you sign in, batched on changes (at most one every 15 s), and at once when the page goes into the background.
- **Gateway:** `POST /v1/diagnostics` takes reports of at most 32 KiB, keeps only a sanitised rebuild, stores the latest per device (migration 3), rate-limits to one stored update every 10 s, and deletes reports with the device. Admins read them via the admin API.
- **LMS plugin:** a Diagnostics button for each device on the settings page (shown as text), and the `anywhere diagnostics id:N` command.
- **Tests:**
  - **Sanitiser (gateway unit tests):** hostile reports (deep, wide, long, control characters, odd keys, non-objects), node caps, throttle.
  - **End to end:** authentication, size limits, sanitising, the admin read path, the rate limit, deletion on revoke.
  - **Plugin and web app:** plugin operation and command tests, settings-page text rendering, report building and batching.

### iPad media element fixed; plugin menus (first part)
- **iPad fix:** iPadOS Safari accepts FLAC through Media Source only as `codecs="fLaC"`, and the check is case-sensitive. The app asked for `flac`, Safari said no, and the iPad fell back to Web Audio (which iOS suspends in the background). The app now tries both spellings (regression test). Found from the capabilities table on the device.
- **Plugin and context menus (gateway):** "learned actions". A device may run a menu action only if LMS offered exactly that action in a menu relayed to that device; nothing is hard-coded per plugin. Server commands are never allowed this way, and shared-data changes need an admin. Context menus are added as entry points. Tests: 9 unit tests (matching, search text, never-list, admin-only, malformed, expiry and bounds, hostile menus); 4 end-to-end against a fake LMS (offered-then-allowed, tampering, per device and per player, server commands); mutation checks on exact matching and the never-list.
- **Web app:** Apps & Radio (every installed app and radio source, with icons); a generic menu view (folders, paging, search boxes, play/next/add, nested "More…", actions that return menus); "More…" on tracks, albums, artists and Now playing. Tests for action resolution, item classification and hostile menu data.

### Playback modes: Web Audio or a media element, chosen automatically
- **Why:** on the iPad, playback stopped whenever Safari went to the background, even with the silent keep-alive. iOS only keeps a page's audio running while a real media element plays. Android Firefox showed the same.
- **Core (`core/src/mux.rs`):** decoded PCM is re-wrapped losslessly as FLAC (24-bit verbatim frames) inside fragmented MP4 (`audio/mp4; codecs="flac"`). There's one init segment per sample rate, the timeline stays continuous across rate changes, and `set_time` re-bases after a flush.
- **Media-element output:** an `<audio>` element fed through Managed Media Source (iOS 17.1+) or classic Media Source. Played time comes from the element (handling rate changes); pause, resume, seek and stop are mapped onto it; played audio is trimmed so memory stays bounded.
- **Automatic choice, built in:** every browser's capabilities are checked when this device starts playing.
  - **iPhone/iPad and Android:** the media element, when the browser supports FLAC through Media Source; otherwise Web Audio.
  - **Desktops:** Web Audio (lowest latency, keeps running in the background).
- **Settings:** shows the mode in use and why, offers Automatic, Web Audio or Media element, and lists the browser's capabilities.
- **Gateway:** the CSP gains `media-src 'self' blob:` (Media Source object URLs only); its test pins it.
- **Core:** `Player::audio_exhausted()`. A pull that returns nothing while a track is still decoding is not the end of the audio.
- **Found while testing:**
  - Chromium rejects FLAC-in-MP4 when the sample entry's bit depth differs from STREAMINFO (regression test);
  - pause states are reported as "Paused { … }";
  - a stalled element won't play the last few milliseconds by itself, so stall detection re-arms when more audio arrives.
- **Tests:**
  - **Core:** FLAC and fragmented-MP4 output decodes bit-exactly through Symphonia at 22.05–192 kHz; CRC and UTF-8 coding checked against reference values; hostile PCM never panics; the end-of-audio signal is tested.
  - **Web app:** mode-choice, timeline and capability tests.
  - **End to end in Chromium,** in both modes: playback, real-time elapsed, pause, resume, seek, and LMS stopping at the end of the playlist (media element: three passing runs in a row).
- **Test hygiene:** the browser tests now sign out and remove their player from LMS afterwards. Six stale test players that had appeared for admin accounts were removed.

### Web app: favourites and playlists; playlist changes are admin-only
- **Favourites:** browse folders; play, play next or add stations and tracks. Icons come through the gateway's art relay (LMS icons, plugin icons, or image-proxy URLs only).
- **Playlists:** list with filter; playlist page with play/next/add, and play from any track. Admins can also create, rename, delete, reorder and remove tracks.
- **Policy change:** changing saved playlists (`playlists new/rename/delete/edit`) now needs the **admin** role. They're shared household data: a guest listener could previously delete them. Listing and playing stay open to listeners. Tests cover listeners refused and admins allowed, and argument validation is still checked as an admin. The app hides the editing controls from listeners.
- **Navigation:** the sidebar gains Playlists and Favourites; phone tabs become Playing, Library, Search, Queue and Settings, with a Library hub.
- **Fix:** a remembered player that no longer exists is forgotten. Before, the app kept polling it, LMS dropped every such request, and the page showed "music server unavailable". Regression tests added.
- **Tests:** 15 more commands in the gateway contract fixture (admin-only ones are checked both ways); id and image-path validation tests, including traversal and `javascript:`/`file:` inputs.

### Security fix: filesystem paths leaked in RPC responses
- **Found:** while starting on playlists in the web app. `playlists` responses carried `favorites_url: file:///…`, exposing the server's music folder layout to any signed-in user. The filter removed URL fields only by exact key name (`url`, `remote_url`, …), and LMS uses others.
- **Fixed:** responses are now filtered by **value** as well as by key. Any string that is a `file:` URL, or a URL whose host is private, loopback, link-local, IPv6 unique-local or a local name (`.local`, `.lan`, `.home.arpa`, …), is removed wherever it appears. `favorites_url` is also on the key list.
- **Tests:** a regression test fails without the fix; the value classifier is tested against 16 sensitive and 9 safe forms; and a live scan of ten real response types through the fixed gateway found no paths or LAN addresses.
- Deployed to the live installation the same day.

### iPad playback: background keep-alive, and a stuck end-of-playlist
- **Diagnosis:**
  - the live gateway log showed iPad Safari never attempting to resume after a drop: the whole page was suspended when backgrounded or locked, then restarted or reloaded. Android resumed within 10 s;
  - a 5-minute, 10-track soak at 96 kHz/24-bit ruled out memory: player memory was flat at 48 MB.
- **Fix:**
  - a silent looping `<audio>` element (served from the app, CSP unchanged) holds the media session while this device plays, so iOS keeps the page running;
  - the AudioContext is resumed when the page returns;
  - lifecycle events go into a connection log that survives reloads;
  - after a reload, the app offers **Resume playing here**, reconnecting as the same player.
- **Diagnostics:** Settings › This device shows buffered audio, downloaded-but-undecoded bytes and the player's memory.
- **Bug fixed (found by the soak):** at the end of a playlist, LMS kept showing the browser player as playing forever. The AudioWorklet reported played frames in batches of 1,024 and never reported the final partial batch, so the player never said "played out". It now reports everything once its queue runs dry. A regression test fails without the fix, and an end-to-end check confirmed LMS stops.
- **CI fix:** two art tests used jsdom's `Blob` as a fetch body, which Node 22 (used in CI) can't read; they now pass on Node 22 and 26.
- **Tests:** 9 new Vitest tests (keep-alive, persistent log, hostile stored data, blocked storage) and 2 worklet tests.

### Phase 2: the web app (first slice)
- **`web/app`:** SvelteKit + TypeScript, a static single-page app served by the gateway. It replaces the Phase 0 test page (`web/spike`, removed).
  - **Players:** sign-in with two-step verification; a player switcher with "this device" (the WebAssembly player, auto-selected once connected).
  - **Now playing:** art, track and format details, transport, seek, volume, shuffle and repeat.
  - **Queue:** jump, reorder, remove, clear.
  - **Library:** albums (five sort orders, infinite scroll), artists, album and artist pages, search; play now, play next and add to queue.
  - **Also:** Media Session lock-screen controls; light, dark or system theme; desktop and phone layouts. Artwork is fetched with the bearer token (never in URLs), at most 6 at a time so LMS isn't flooded.
- **Security:**
  - the build externalises SvelteKit's inline start-up script and fails on any inline script, event-handler attribute or `javascript:` URL;
  - a source-wide test bans `{@html}`, `innerHTML`, `eval` and similar;
  - components are rendered with hostile metadata (script/HTML payloads, bogus ids) and must show plain text with in-app links only (mutation-checked);
  - command builders validate every id and clamp paging and volume.
- **Contract test:** every command the app can send is pinned in `web/app/tests/fixtures/commands.json`; a Rust test runs each one through the real gateway policy. It immediately found that `albums` refused `album_id` (now allowed: numeric ids only, with negative tests).
- **Gateway:** CSP adds `style-src-attr 'unsafe-inline'` (style attributes only); the CSP test now checks every directive.
- **Tests:** 37 Vitest + 17 Node tests for the app; CI builds the app, type-checks with warnings as errors, and runs them; the Docker, deployment and release checks cover the new layout.
- **Live:** against a real library (read-only, as a listener whose only player is its own browser): the full album grid with art, search, album pages, and FLAC 96 kHz/24-bit playback on "this device", with live progress and queue.

### Plugin releases
- **`tools/build-release.sh`:** builds `LyrionAnywhere-<version>.zip` and `repo.xml`, containing:
  - static gateways for x86_64, aarch64 and armhf Linux (cargo-zigbuild, musl);
  - `Bin/SHA256SUMS`;
  - the browser player.
- **Integrity:** the plugin runs a bundled gateway only if it matches `Bin/SHA256SUMS`, and restores the executable bit that zip extraction can drop. A development "Gateway path" is not checked.
- **Browser player:** when bundled, the plugin passes it to the gateway (`--static-dir`), so the player page is served at `/`.
- **Tests:**
  - 20 new plugin tests: checksum match, tampered, truncated, wrong platform, missing list; a launch refused with the reason shown; the executable bit fixed. Mutation-checked.
  - `tools/test-release.sh` checks the zip layout, checksums, static binaries per architecture and `repo.xml`. It runs in a new CI job on main and tags.
- **First real installation:** the plugin running the gateway on the LMS host, behind nginx on a separate proxy host. All checks passed. It showed that the plugin should set its directory at load (fixed earlier today).

### Plugin fix from the first real install
- For 5 s after LMS started, the plugin's status showed a wrong binary path (`/Bin/…`), because the plugin directory was only set when the gateway started. It is now set at load. A regression test drives the real `initPlugin` and fails without the fix.

### Phase 1: nginx and Docker
- **Docker image:**
  - a static gateway binary plus the browser player;
  - distroless base, with no shell; runs as non-root; private `/data` volume.
- **`deploy/nginx/lyrion-anywhere.conf`:**
  - TLS, HSTS, WebSocket upgrades, unbuffered streams, a 64 KiB body cap;
  - `X-Forwarded-For` overwritten, never appended.
- **`docs/DEPLOY.md`:** the plugin and Docker set-ups, notes for running behind a CDN, and a checklist.
- **Gateway:** the events socket now sends a ping every 30 s, so proxies with idle timeouts (Cloudflare closes after 100 s) don't drop quiet sockets.
- **Tests:**
  - `tools/test-deploy.sh` runs the image behind nginx with the example config, as a new CI job. 24 checks cover: TLS redirect, a single CSP, LMS/admin/traversal paths unreachable, sign-in, cookie flags, origin checks, WebSocket upgrades, private data files, and lockouts that hold against spoofed `X-Forwarded-For`. A mutation check confirmed the spoofing check fails with the common `$proxy_add_x_forwarded_for`.
  - A keepalive test, mutation-checked.
- **Fixed:** a flaky events test that checked the LMS subscription before it was sent.

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

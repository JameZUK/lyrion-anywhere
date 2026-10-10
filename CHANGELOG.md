# Changelog

Dated entries, newest first.

## 2026-10-10

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

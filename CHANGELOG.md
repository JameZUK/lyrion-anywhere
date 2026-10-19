# Changelog

Dated entries, newest first.

## 2026-10-19: Mobile: a timed start armed while paused could be lost

### Mobile

- **A sync-group start armed while the phone was paused could leave it silent.** The ring between the engine and the audio output stored a timed start's frame before setting the hold. An output callback that read the new start between the two stores acted on it and released the hold, and the second store then held the ring again with no start pending: silence until the next pause or resume. The hold is now set first, and the output clears the start it acts on with a compare-and-swap, so a start armed or cancelled during the hand-over is kept for the next callback rather than wiped. Found when the ring's own concurrency test hung the v0.6.2 CI run for six hours (its drain wait spun for ever once the hold came back). A new test arms 20,000 starts while held and fails on the old code within a few hundred rounds; the output-thread tests now stop waiting when that thread has died, so a panic there fails the test instead of hanging it.

### CI

- **Every job has a timeout.** The jobs ran with GitHub's six-hour default, which is how the hang above held the v0.6.2 release for the night. Each job now has its own limit, three to four times its usual run (`test` and `release` 60 minutes, `android` and `android-release` 45, `web` and `deploy` 30, `plugin` and `supply-chain` 20, `docs` 10), so a hang fails the run within the hour and the job can be re-run.

## 2026-10-18: Plugin 0.6.2 released

Notes: `docs/release-notes/v0.6.2.md`. Diagnostics only, so issue #12 can be pinned down from the reporter's next Connection log; no playback behaviour changes.

### Web

- **The Connection log shows what LMS and the player said, and where the media element stood (#12).** Each `strm` from LMS but the status poll (`lms: strm s autostart=1 format=f threshold=255`, `lms: strm q`; only those letters and the threshold, never the request) and each STAT the core sends but the heartbeat, with a short gloss (`player: STMl (loaded, waiting for LMS to start it)`). These come from a new, bounded diagnostics queue in the core (`Player::set_diagnostics`, `take_diagnostics`; `WasmPlayer` likewise). Also: changes of the core's output state with their cause where known (`player: Playing → Paused (after strm p)`, `local pause`, `sync`), commands the page sends LMS by itself (`page: asked LMS to pause (the system paused the media element)`; also the volume hold, the hand-back after playing on without the server, and the resume after a reload), the media element's play, pause, playing, waiting, stalled, seeking, seeked, ended and error with the play head and the two buffered ranges nearest it (`media: waiting at 61.2 s, buffered 60.9–75.4 s`), an element that stops taking audio with a full lead (`media: element not consuming (lead 8.0 s, buffered 14.3 s)`), a download held back for 10 s by a full core buffer (`stream 5: reading paused, core holds 6.9 MB undecoded`), what the worker did with "drained" (`media: drained, counted as played out`), and how each clean end was settled (`stream 5: clean end at byte B, confirming`, then `end confirmed at byte B` or `not the end; continued from byte B`). Lines of one kind are logged at most once per 5 s, the next one saying how many were left out; state changes always go in. Reports now carry up to 120 lines (at most 16 KiB of them). Tests: `core/src/player.rs` and `wasm/tests/bindings.rs` (the queue: off by default, no status poll, bounded, letters and threshold only), `tests/node/diag.test.mjs` (the strm and STAT lines with junk input, output causes, the persisting-condition watch), `tests/node/reconnect.test.mjs` (the parked-reading and clean-end lines against the fake gateway), `tests/diagnostics.test.ts` (the rate limiter, the page-command lines never carrying free text, the report's size), `tests/media-output.test.ts` (element events with the play head and ranges), `tests/device.test.ts`, `tests/device-media-hold.test.ts` and `tests/local-playback.test.ts`.

## 2026-10-18: Plugin 0.6.1 released

The web player fix for issue #12 (below), on top of 0.6.0. Notes: `docs/release-notes/v0.6.1.md`. Deployed to the owner's server before the tag; the fix itself was reasoned from the reporter's diagnostics and gateway log, not reproduced locally.

### Web

- **Scrubbing at Original on iPhone and iPad no longer skips the track (#12).** In media-element mode a full media buffer ("media buffer full; waiting") waited for a trim that nothing scheduled, as no further segment comes while the worker waits. The element ran dry, the worker stopped acknowledging, and after 60 s the gateway ended the stream with an error; through a Cloudflare Tunnel that error could arrive as a clean end, which the player took as the end of the track and skipped. Now a full buffer removes the audio already played (all but 1 s behind the play head) at once and carries on ("media buffer trimmed; resuming"). With nothing worth removing, it tries again as the play head moves (`timeupdate`, `progress`, a managed buffer's `bufferedchange`), never in a loop. The worker also confirms every clean end that brought bytes with one more `?offset=` request. The gateway answers that at once with nothing at the true end, and otherwise sends the rest. The stream download loop moved into `static/player/reconnect.js` (`downloadStream`) so it can be tested. Tests: `tests/media-output.test.ts` (a full buffer after a scrub resumes without another append and removes only played audio; nothing to remove waits for the play head; a buffer that stays full doesn't spin; a flush forgets it) and `tests/node/reconnect.test.mjs` (against a fake gateway: a broken stream resumes from its offset; an early clean end is confirmed and the rest fetched; the true end costs one empty request; a refused confirmation ends as before; no loop without progress; acks across a resume).

## 2026-10-18: Plugin 0.6.0 released

The built-in Cloudflare Tunnel (below). Notes: `docs/release-notes/v0.6.0.md`. Checked live before the tag: the deployed plugin installed the pinned `cloudflared`, and a temporary tunnel opened from the settings page played on a phone over mobile data.

### Gateway

- **Built-in Cloudflare Tunnel.** On request from the plugin's settings page (`/admin/v1/tunnel/…`), the gateway installs the official `cloudflared` 2026.10.1 (pinned, SHA-256 checked) and supervises it: register a tunnel token, let the gateway create the tunnel through Cloudflare's API (the API token is used once and never stored), or try a temporary `trycloudflare.com` tunnel for an hour. While a tunnel runs the gateway accepts its origin and reads `CF-Connecting-IP` from it. New `--cloudflared <path>` and `--no-tunnel`; the gateway now also stops cleanly on SIGTERM. SPEC D27; `tools/update-cloudflared-pin.sh` moves the pin.
- **The tunnel has its own listener.** `cloudflared` now sends the tunnel's requests to a loopback listener of the tunnel's own (`--tunnel-listen`, default `127.0.0.1:9444`, not opened with `--no-tunnel`), and only requests on it have `CF-Connecting-IP` believed and the tunnel's origin accepted. Before, while a tunnel ran the header was believed on every loopback request, so a reverse proxy on the same host that passed it through let a client choose the address the lockouts and the audit log saw; and a non-loopback listen address was trusted as a proxy and exempt from the per-address connection cap. The public listener now behaves as configured, tunnel or not. A tunnel listener that can't be opened leaves the gateway running and the tunnel `unsupported` with a sentence saying why; the status gains `tunnel_listen`, and the settings page names it as the service URL to set in Cloudflare and shows the override notes only while a tunnel runs. Tests: `gateway/tests/cftunnel.rs` (the header and origin on each listener, with and without a running tunnel; lockouts through a same-host proxy; the ingress and `--url` naming the tunnel listener; a listener that couldn't be opened), `gateway/tests/tunnel_listener.rs` (the real binary: `--no-tunnel` opens nothing, a busy port, loopback only, and the public listener's per-address cap on a non-loopback address), unit tests in `lib.rs` and `cftunnel/mod.rs`, `plugin/t/js/tunnel.test.mjs` and `plugin/t/06-gateway-contract.t`.

### Docs

- **The Cloudflare Tunnel, for users.** INSTALL gains *Option C: built-in Cloudflare Tunnel* (both registration routes, the API token's three permissions, the temporary tunnel, the settings it overrides, outbound needs, Cloudflare's terms, and troubleshooting). SECURITY describes its trust boundary, DEPLOY a `cloudflared` sidecar for Docker, and FEATURES, PLATFORMS, ARCHITECTURE and the README list it. Release notes: `docs/release-notes/v0.6.0.md`. The plan records what was built differently (*As built*).

## 2026-10-18: Plugin 0.5.6 and Android 1.15.5 released

The remaining low findings of the October code review, from the five entries below (gateway and core, web, mobile, Android, plugin and CI). Plugin 0.5.6 and Android 1.15.5 (versionCode 34). Notes: `docs/release-notes/v0.5.6.md`, `docs/release-notes/android-1.15.5.md`. The first tag run failed Gradle's new dependency verification on five parent-POM and BOM files a warm cache had left out of the metadata; it was regenerated from a clean cache, and RELEASING.md now says to do it that way. The second tag run failed the release check's emulated armhf run: Docker refuses to pull a second platform of one multi-arch index digest ("cannot overwrite digest"), so the aarch64 pull of the newly pinned `alpine:3.22@sha256:5291449c…` blocked the armhf one; `tools/test-release.sh` now pins each platform's own manifest digest and prints the emulated run's output on failure. `tools/build-release.sh` no longer wipes all of `dist/` (it deleted the deploy binaries and Android builds kept there), only its own outputs.

## 2026-10-18: Plugin, CI and release: the review's remaining lows

The last low findings of the 15 October review for the plugin, CI and the supply chain:

- **`anywhere status` without the token says less.** Any LAN CLI or JSON-RPC caller could read the gateway's binary and data paths, pid, listen address and the last lines of its stderr. Without the settings page's token the answer is now only `enabled`, `running`, `binary_ok`, `uptime` and the gist of the last error (its first line up to the first ": ", so "gateway exited", never the paths or stderr after it). The settings page sends its token and still shows everything. Tests: the token-free status in `plugin/t/04-commands.t` (no token, a wrong one, a token as a list; no paths, pid, address or stderr; `public_error` for each kind of error), and the settings page sending its token with `status` in `plugin/t/js/settings.test.mjs`.
- **The admin secret goes only to 127.0.0.1.** With LMS's `webproxy` pref set, LMS (8.5 to 9.1, `Slim::Networking::Async::HTTP::use_proxy`) sends plain HTTP through the proxy unless the host matches `localhost` or `127.0.0.1`. The plugin always starts the admin API on 127.0.0.1, so the secret never went to the proxy. But `[::1]` or `127.0.0.2` would have been proxied, so the plugin now refuses to send the secret anywhere else. Test: `plugin/t/02-admin.t` (`[::1]`, `127.0.0.2`, `127.0.0.10`, look-alike host names, `localhost` and a LAN address are refused, with nothing sent).
- **Licences: no GPL or LGPL dependency.** `deny.toml` allowed GPL and LGPL for the whole workspace, so nothing enforced `LICENSE-EXCEPTION`'s condition that the app-store builds hold no third-party GPL code. GPL-2.0-or-later is now allowed only for the project's own six crates. GPL and LGPL are refused for every dependency, with all features and all targets checked, so the Android-only crates count too. Dual-licensed crates such as r-efi pass on their MIT or Apache-2.0 terms. Test: `tools/test-deny-policy.sh`, run in CI's supply-chain job. It refuses GPL-2.0/3.0, LGPL-2.1/3.0 and AGPL dependencies, allows MIT, MPL-2.0 and a permissive-or-LGPL choice, and fails against the old `deny.toml`. The Android app's Gradle dependencies are not licence-checked by a tool (SPEC D17).
- **`npm audit` in CI.** The builds run `npm ci --no-audit`. The web job now runs `npm audit --audit-level=high`, and a high or critical advisory fails it. Today it reports three lows, all `cookie` < 0.7.0 through `@sveltejs/kit`, whose fix is a major SvelteKit upgrade.
- **Dependabot** now covers npm (`/web/app`), cargo (`/`) and Gradle (`/apps/android`) as well as actions and images. Updates come weekly, with minor and patch updates grouped per tree.
- **Signed build provenance for releases.** LMS checks a plugin download only against the SHA1 in `repo.xml`, and the same release writes that file. The release jobs now attest the plugin zips, `repo.xml`, the APKs and the app bundle with `actions/attest-build-provenance` (pinned by commit). `id-token` and `attestations` are granted to those two jobs only. `docs/RELEASING.md` (*Verifying a release*) shows how to check a file with `gh attestation verify`.
- **Test images pinned by digest:** `tools/test-deploy.sh` (nginx, alpine) and `tools/test-release.sh` (alpine), like the Dockerfiles.
- **Docs:** PROTOCOL notes the admin listener's 404 fallback (`/healthz`, `/admin/v1/plugins` and the static fallback were already covered). SPEC, ARCHITECTURE and PLATFORMS are updated for all of the above. CHANGELOG gains the missing entry for the Android 1.15.0 test build. A repository-wide scan for addresses, host names, emails and names found only test fixtures.
## 2026-10-18: Web: the review's remaining lows

The last low findings of the 15 October review, in the web app.

- **Updates wait for you.** The service worker no longer takes over and purges the old caches the moment a new version installs, which left an open page unable to load its next lazily loaded piece (and a forced reload stopped playback; probably the "stale-looking app" a forum tester saw). A new version now installs and waits; open pages keep the version they loaded, and a previous version's cache stays until no open page runs it. The status line says *A new version is ready. Reload when convenient.* with *Reload* and *Later*. The handlers moved to `lib/pwa/sw-handlers.ts`, tested for the first time against fake caches and clients (`tests/sw-handlers.test.ts`); the page side is `lib/pwa/update.svelte.ts` (`tests/app-update.test.ts`).
- **Downloads are checked to be audio.** A 200 answer that is a web page or JSON (by its content type, or by its first bytes whatever the type claims) fails the download with *The server sent something that isn't audio.* and stores nothing (`offline-manager.test.ts`, with a robustness test of the checks).
- **A hung token refresh no longer blocks every tab.** The refresh request gives up after 15 s, a tab holds the cross-tab lock for at most 20 s, and a tab waiting behind another gives up after 45 s without running its refresh; a timeout keeps the sign-in and retries (`api.test.ts`, three hung cases).
- **Signing out forgets the remembered players** (and this device's resume point), so the next person to sign in on the browser doesn't start on them (`state-lifecycle.test.ts`).
- **The two-step QR code** is encoded as UTF-8, so a non-Latin-1 character in it can no longer throw; a missing one says to type the key (`settings-page.test.ts`).
- **Busy guard on New playlist › Create**: a double click makes one playlist.
- **Touch targets**: the Home shelf chips, menu rows without a thumbnail and the library note's *Show the whole library* are 44 px (`touch-targets.test.mjs`).
- **Nits**: diagnostics post on a tab hide only when there is something new; the player worker encodes the session id in the stream address (`streamPath`, `reconnect.test.mjs`); the settings pickers' radiogroup takes the arrow keys, Home and End, with one tab stop; a volume change not yet sent is dropped when another player is chosen (`volume-note.test.ts`); the HTML-injection check also bans `srcdoc`, `DOMParser`, `parseFromString` and `setAttribute` of links, sources and handlers; one shared `ago()` for the device lists; the password form's checkbox and Your devices' button have distinct hooks (`data-also-sign-out-others`, `data-sign-out-all-others`).
- New component tests (`review-components.test.ts`) for PlaylistsView, AppearancePicker, AdvancedSettings and the PickerRow keys. Already fixed on 15 October and left as they were: the two-step busy guards, the failed sign-out message, and the Your devices and settings-page test gaps.
## 2026-10-18: Mobile: the review's remaining lows

The last LOW findings of the 15 October code review in the mobile Rust layer (`mobile/`). Each fix has a test that failed before it, or (where the code was reshaped) a test of the new behaviour. No UniFFI API change; the Android app needs nothing.

- **Revocation on Android** (`tls.rs`): stated as a known limit in SPEC §9.3 as well as the module. Android's verifier can't fetch CRLs and reports any failure to check as "revoked", so that verdict is checked again against the device's CA store without online revocation; in effect revocation is not enforced, as in Chrome on Android.
- **Two ids, one file** (`offline/download.rs`, `offline/store.rs`): two queued ids that `/info` gives the same stable key (a rescan) downloaded at once into the same partial file, and the second's rename could leave an entry that was dropped at the next open. A download now claims its stable key in the library; the second waits, then finds the file on the device. Cancelling one keeps the partial file the other is still writing. Tests: `two_ids_for_one_file_fetch_it_once` (two fetches before) and `cancelling_one_id_for_a_shared_file_keeps_the_others_bytes` (the file was stored damaged before) in `mobile/tests/downloads.rs`.
- **Two libraries on one folder** (`offline/store.rs`, verified real): nothing stopped a second `OfflineLibrary` on the same user and server. Its open-time clean-up deleted a file the first had just moved into place but not yet listed, and each kept its own list, so whichever saved last dropped the other's downloads from the index. Opening a library that is already open in the process now returns the open one (an in-process registry by folder). Test: `a_second_open_shares_the_first_and_never_deletes_its_files` (the file was deleted before); the store tests that simulate a restart now close the library first, as the app does.
- **The download check never holds a stream up** (`player/engine.rs`): the check of a downloaded FLAC standing in for a stream (up to a 16 MiB header read and a full verify-decode) ran inside the stream task, which stopped reading the network until it was done. It now runs on the blocking pool beside the stream; the download takes over from wherever the stream has got to, and at the end of the first bytes (`?limit=`) the stream waits at most 3 s for it before asking for the rest. Test: `a_slow_download_check_never_holds_the_stream_up` (asking returns at once with an 800 ms check; the download is positioned past what arrived meanwhile; a late or empty answer leaves the network). The Phase C integration tests in `mobile/tests/player.rs` pass unchanged.
- **An overall deadline for reads** (`transport.rs`): `read_limited` had a time limit per piece but none for the whole body, so a gateway trickling a byte at a time could hold a 16 MiB read open indefinitely. The whole read now has 3 minutes. Test: `a_trickling_answer_is_cut_off_at_the_overall_deadline` (a fake that sends a byte every 20 ms).
- **TLS end to end** (`mobile/tests/tls.rs`, new): the real gateway behind a local TLS proxy whose certificate can be swapped. A self-signed certificate and a CA-issued one for the wrong name are refused on the HTTP client (sign-in and a signed-in request), the event socket and the player socket, with nothing passed on and the sign-in kept; a certificate from the CA the trust store holds (`SSL_CERT_FILE`, which the platform verifier reads on Linux) is accepted on all three. Fixtures: `mobile/tests/fixtures/tls` (a test CA and three certificates, valid until 2126). New dev-dependency: `tokio-rustls` (already in the tree through `hyper-rustls`).
- **The live sync check** (`mobile/tests/live_sync.rs`): `TEST_PLAYER_MAC` must be a `02:4c:41:` test address, as `SYNC_PLAYER_MAC` already had to be. Without `ANYWHERE_LIVE=1` it prints "skipped: no live environment"; `ANYWHERE_LIVE=1` without a usable env file now fails rather than passing; `ANYWHERE_REQUIRE_LIVE=1` turns a skip into a failure for a dedicated live run. Test (runs everywhere): `the_live_check_runs_only_as_asked_and_only_on_test_players`.
- **Test gaps the review named:** `a_redirect_to_another_host_is_never_followed` (`mobile/tests/auth.rs`: 301 to 308 on sign-in and on signed-in requests, nothing reaches the other host, the sign-in is kept); `a_socket_that_stops_taking_messages_is_given_up` (`SOCKET_SEND_TIMEOUT`, through the send helper the player socket now uses); `a_partial_discard_while_the_output_runs_never_plays_dropped_audio`, `a_skip_while_the_output_runs_drops_exactly_what_was_asked` and `a_timed_start_while_the_output_runs_lands_in_step` (`player/ring.rs`, consumer on its own thread). Already covered since the 15th, so not added again: a gateway ignoring `?limit=` and the stream-task watchdog (M10), greet-then-close pacing (M11), the 1008 renewal loop (M8), and `mobile/tests/devices.rs` with its 429 and devices-feature-absent cases.
- **Docs:** SPEC §8.3 (one fetch per file; one library per process), §9.3 (native apps' TLS and the Android revocation limit).
## 2026-10-18: Android: the review's remaining lows

The 15 October review's low findings for the Android app (`apps/android/`):

- **Dependencies are pinned.** `gradle/verification-metadata.xml` records the SHA-256 of every dependency and plugin, so Gradle refuses an artefact that changes. Above all the rustls-platform-verifier AAR, the JNI bridge every certificate check goes through, which comes from a GitHub branch. Only IDE sources and javadoc are trusted unchecked. How to refresh it is in `docs/RELEASING.md`. Test: `DependencyVerificationTest`.
- **Voice search asks first.** "Play … on Lyrion Anywhere" (`MEDIA_PLAY_FROM_SEARCH`, which any app can send) plays at once only while Play on this phone is on; otherwise a one-off "Play on this phone?" asks, and nothing changes unless the user agrees. The intent handling moved from `MainActivity` to `LaunchHandler`. Tests: `LaunchHandlerTest` (a Robolectric activity: the first-time prompt, yes and no, the already-on path, the query bound, Recents, recreation, pairing links).
- **Safe mode only for a crash loop.** A single crash, an ANR, or a failure that didn't stop the app now shows its report once over an app that starts normally. Only two crashes within five minutes open safe mode (the report alone, with Restart; Android Auto refused). Tests: `CrashLogTest` (the start decision) and `CrashLogStartTest` (Android's exit records through Robolectric).
- **Per-output EQ keeps its choice.** The output being bound is kept by kind and device name, not by its place in a list that starts with the current output. Tests: `theChosenOutputStaysChosenWhenTheCurrentOutputChanges`, `aChosenOutputThatIsGoneFallsBackToTheFirst` and `outputTargetsKeysAreStableAndSavedChoicesMigrate` in `Wave2Part4ScreenTests`.
- **Lists whose keys could repeat.** History rows get an occurrence tiebreak (no row id from the gateway), and Apps & Radio shows a command listed under both radio and apps once, keyed by section. Tests: `ReviewLowsScreenTests`, `ReviewLowsAppsTest`.
- **No seek to NaN.** A length that is infinite, NaN or over a week is treated as unknown, and the seek bar never seeks outside 0…length. Tests: `anAbsurdDurationNeverSeeksToNaN`, `aLongButRealDurationSeeksWithinIt`, `durationsThatArentNumbersAreShownAsUnknown`.
- **The app's own words.** The gateway's 404 and 409 are said in the app's wording (`AppError.Reason`), not the server's text. Your devices restored onto a server that doesn't offer it says so and asks for nothing. The app's own refusals (sign out first, choose a server, https only, no server for the code, the filter limit, a menu too deep to pin, the queue changed, waiting for Wi-Fi), the filter editor's problems, menu verbs, and the kHz, cushion and decade units moved into `strings.xml`. Tests: `aRestoredPageOnAServerWithoutItAsksForNothing`, `aNotFoundAnswerIsSaidInTheAppsOwnWords`, `MenuLabelTextTest`, `CushionFormatTest`, `AppErrorTest`, `FiltersTest`.
- **Volume popover.** It also follows the system's volume broadcast (receiver not exported), which comes at once, besides the settings observer, which Android updates about half a second late. Test: `PhoneVolumeObserveTest`.
- **Download service basics.** Test: `DownloadServiceLifecycleTest` (without a started app it stops at once; its Cancel action is explicit and immutable).
- Already done, so unchanged: the fake account's refusal for this device (fixed with M16), and the secure store's non-tamper failure paths (`EncryptedCredentialStoreTest`).
## 2026-10-18: Gateway and core: the review's remaining lows

The last LOW findings of the 15 October review in the gateway and the core. Each change has a test that failed before it.

- **History: quick seeks keep one listen** (`history.rs`). The race as reported (a seek's `STMs` before the play's row arrives) was already handled, and now has a test for each order (`a_row_arriving_before_the_seek_is_resolved_reaches_the_continued_listen`). The real gap was two seeks before LMS named the track: the lookup is coalesced, so the middle start was never named, the first listen ended at the second start, its row kept only the seconds heard by then, and the rest could be recorded as a second play. A start that is never named now leaves the listen before it waiting, and its time is credited only if the next named start continues it. Tests: `quick_seeks_before_the_track_is_named_continue_the_listen`, `an_unnamed_start_followed_by_another_track_ends_the_listen`.
- **Shelves are the user's own** (`history.rs`, SPEC D23). `/v1/rpc` doesn't allow `sort:recentlyplayed` or `sort:playcount`, so blending LMS's server-wide lists into Recent and Most played really did show each user albums others had played lately. Both shelves now use the user's own history only; stations still blend LMS's play counts. Tests: `recently_played_is_the_users_own_order`, `most_played_counts_only_the_users_plays`, and `shelves_return_album_ids_only` (LMS isn't asked; a user with no plays gets empty shelves).
- **Transcodes: the original counts against the cache, and the slot is for the encode** (`download.rs`, `transcode.rs`). The original is now fetched before a transcode slot is taken, so a slow fetch no longer holds up everyone's encodes, and it holds room in the cache while it is on disk (`Cache::reserve`: evicts to make room, waits while others' originals hold it, refuses one larger than the whole cache with 413). Tests: `work_in_progress_counts_against_the_cap`, `a_slow_original_does_not_hold_the_transcode_slot`, `an_original_bigger_than_the_cache_is_not_transcoded`; `the_transcode_cache_evicts_the_least_recently_used` sizes its cache for the original.
- **Admin-only Music & Artist Information** (`info.rs`, `lyrics.rs`). The `admin_only_plugins` setting now holds for artist and album information (403 `this plugin is for admins only`, as `/v1/rpc` answers) and for lyrics (the file's own lyrics only), checked before the shared caches so an admin's answer never reaches a listener. Tests: `an_admin_only_plugin_gives_listeners_nothing` (`gateway/tests/info.rs`), `an_admin_only_plugin_gives_listeners_only_the_tags` (`gateway/tests/lyrics.rs`).
- **Playlist names** (`rpc_policy.rs`): `playlists new|rename` names follow the queue save's rules (`queue_save::check_name`), so `a/../../b`, `..`, device names and invisible characters are refused there too. Test: `playlist_names_follow_the_queue_save_rules`.
- **Image proxy redirects: documented, not fixed** (SPEC §5.2, PROTOCOL, `art.rs`). LMS fetches image-proxy URLs itself and follows redirects, which the gateway can't see without fetching the image itself; only an `image/*` answer of at most 20 MiB is ever relayed.
- **Non-UTF-8 ids are unknown resources** (`lib.rs`). A `/v1/` path whose escapes don't decode to UTF-8 is 404 `not found` before any handler, instead of axum's 400 ahead of auth and CSRF. Tests: `ids_that_are_not_utf8_are_unknown_resources` (`gateway/tests/hardening.rs`), `paths_are_judged_by_their_decoded_bytes`.
- **Smaller fixes.** `/v1/rpc` bodies are read only up to 8 KiB (`limits::body_limit`; test `oversize_rpc_bodies_are_refused_without_reading_them`). Artwork answers 503 `busy` at once while LMS is blocked, as `/v1/rpc` does (`artwork_is_refused_at_once_while_lms_is_busy`; the RPC test of the 503 already existed). The gateway has no LMS CLI login: INSTALL and PROTOCOL say so, and a CLI that hangs up without answering is logged as such (`a_cli_that_refuses_the_connection_is_recognised`). An invalid refresh token clears the browser's cached data as reuse does (`an_invalid_refresh_token_clears_cached_data`). Arbitrary cookie and bearer headers never panic (`arbitrary_cookie_and_bearer_headers_never_panic`).
- **Core: the CLI streams only from its server** (`proto.rs`, `cli/`). In direct mode the CLI connected wherever a `strm` said; `proto::direct_stream_target` allows only the connected server (`server_ip` 0 or its own address) and this player's own stream, as the gateway enforces. Test: `direct_streams_go_only_to_the_server`.
- **Core: server renames are bounded** (`player.rs`, `names.rs`). A `setd` rename keeps at most 64 printable characters (control and invisible ones removed); one with nothing printable is ignored. Tests: `a_server_rename_is_bounded_and_printable`, `player_names_are_cleaned_and_bounded`.
- **Core: robustness tests** (`core/tests/robustness.rs`) for `Helo::parse`, `Stat::parse`, `validate_stream_request` with `direct_stream_target`, `parse_mac` and `OpusHead::parse`, and `a_caught_panic_is_reported_as_a_failure` (M13). The M12 tests (orphan segment, refused codec) already existed.

## 2026-10-18: Plugin 0.5.5 released

Internet radio plays: the gateway has LMS proxy streams for these players (the entry below). Everything else as in 0.5.4; Android 1.15.4 remains current. Notes: `docs/release-notes/v0.5.5.md`.

## 2026-10-18: Internet radio plays (LMS proxies streams for these players)

The owner found that Shoutcast and other internet stations would not play. The gateway log showed why: LMS told the player to fetch the station straight from the broadcaster's address (*direct streaming*, which LMS does for capable players), the gateway refused that `strm` as it must (these players only ever talk to the gateway), and the player reported the stream unsupported. The fix is in the gateway, so it covers plugin and Docker installs alike:

- **LMS is told to proxy.** The first time LMS sends a player anything after its HELO, the gateway sets that player's `mp3StreamingMethod` pref to 1 (*MP3 streaming method: proxied* under Player › Audio; despite the name it covers every HTTP stream). LMS keeps the pref, so it is a one-off per player.
- **A direct stream that still arrives is replayed.** If LMS nevertheless points the player elsewhere, the gateway sets the pref and asks LMS to replay the current entry (`playlist jump +0`), which it now serves itself. At most three times in a row, so a server that will not proxy is not asked for ever; an accepted stream resets the count.
- Tests: `lms_is_told_to_proxy_once_the_player_is_known` and `a_refused_direct_stream_is_replayed_through_lms_up_to_three_times` in `gateway/tests/gateway.rs`. Docs: INSTALL troubleshooting, SPEC player integration.

## 2026-10-18: A steadier grouped-rejoin test

`a_grouped_player_rejoins_without_pausing_the_group_or_moving_it_back` watched for the rejoin's seek for a fixed 6 s after the stream resumed; on a loaded CI runner (the v0.5.4 release run) the player's decision came later and the test saw no seek. The grouped-gap helper now watches for up to 20 s but stops 1.5 s after the first seek, so quick runs stay quick and a second seek in that time is still caught.

## 2026-10-18: Plugin 0.5.4 released

The web app's version display, done properly this time: the 0.5.3 zip still read "0.1.0" because the release build's container had only `web/app` and `vite.config.ts` fell back to the package version. Both Dockerfiles' web stages now copy `plugin/Plugins/LyrionAnywhere/install.xml`, the config fails rather than falling back (`ANYWHERE_WEB_VERSION` is the explicit way out for a standalone build), and `tools/test-release.sh` refuses a zip whose web app does not carry "Version <plugin version>". Notes: `docs/release-notes/v0.5.4.md`; the 0.5.3 notes carry a correction.

## 2026-10-18: Plugin 0.5.3 and Android 1.15.4 released

Plugin 0.5.3: the gateway's reads no longer wait for its writes (review M7, below), and Settings › About in the web app shows the plugin's version. Android 1.15.4 (versionCode 33): *Continue on* resolves queued downloads against the signed-in account's library only (review M14, below); nothing else changes in the app. Notes: `docs/release-notes/v0.5.3.md`, `docs/release-notes/android-1.15.4.md`.

## 2026-10-18: Mobile: Continue on resolves against the signed-in account's library (review M14)

The last of the three entry points the 15 October review found taking any downloads library without checking whose it was. `sync_plays`, `sync_ratings` and the downloader were bound to the signed-in account on 15 October; `resolve_offline_tracks`, which "Continue on <player>" uses to turn queued downloads back into LMS track ids, took no library at all. A stale handle across an account switch could then have looked another user's queue up under the new account.

- **Rust layer** (`mobile/src/offline/resolve.rs`): `resolve_offline_tracks(session, library, tracks)` takes the library and refuses one opened for another account or server with `ApiError::WrongAccount` (`Unauthorised` signed out) before anything is asked. Only tracks that library holds are looked up; any other queued track is unresolved without a request. Test: `continue_on_resolves_only_against_the_signed_in_accounts_library` (`mobile/tests/downloads.rs`): another account's and another server's library are refused and the gateway sees no request; with alice's own library her downloads resolve as before and a track she hasn't downloaded is never asked about. It failed before the check (it resolved all three tracks through bob's library). `offline_tracks_resolve_to_current_ids_through_the_gateway` passes the library.
- **Android** (`ContinueOn.kt`, `OfflineRepository.kt`, `AppContainer.kt`): `TrackResolver.resolve` takes the queue's owner, and `RustTrackResolver` passes that owner's library, the open one only if it is theirs (`RustOfflineRepository.storeOf`); with none it refuses as another account's downloads. Tests: `continueOnGetsOnlyTheOwnersOpenLibrary` (`OfflineRepositoryTest`), `bobsQueueIsLookedUpInBobsLibrary` and `theRealResolverNeedsASignInAndTheOwnersOpenLibrary` (`ContinueOnUseCaseTest`); the existing use-case and controller tests check each lookup was in the queue owner's library.
- **Docs:** SPEC §8.4 (Continue on, track ids).
## 2026-10-18: Gateway: reads no longer wait for writes (review M7)

The follow-up to M7 from the 15 October review. Writes on request paths already ran on the blocking pool, but every read shared the account database's one connection and its lock, so a write holding it (or stalled on the disk) still queued every request behind it, starting with the player-permission check each RPC makes.

- **A read connection** (`accounts.rs`): the database now has a second connection to the same file, opened read-only and with `PRAGMA query_only` (a stray write through it fails), after the migrations have run so it sees the current schema. Under WAL a reader never waits for the writer and sees every committed write. Pure reads use it: `player_scope`, `device_active`, account settings (two-step state, download permission, session lifetime, admin-only plugins), own and admin device lists, user and grant listings, the audit log, diagnostics, offline-play lookups, and the read halves of history, ratings, Home, stations and filters (`Accounts::read_db`). Anything that writes, or reads to decide a write (sign-in, pairing, renewal, rename, the "insert if missing" settings), stays on the writer (`with_db`). An in-memory database (unit tests, the demo) has the one connection, which reads use too (`has_read_connection` says which).
- **Admin API off the async workers** (`admin.rs`): every handler's database work, reads and writes, now runs through `Gateway::db` on the blocking pool, as the user-facing paths do.
- **Tests:** the integration harness now keeps its accounts database on disk (in a scratch folder removed with it), so every gateway integration test runs with the read connection, as in production. New in `tests/db_stall.rs`: `a_stalled_write_does_not_hold_up_requests_that_only_read` (an RPC status call, `/v1/account`, `/v1/devices`, history, ratings style, filters and Home), `a_stalled_write_does_not_hold_up_admin_reads` and `an_admin_write_waiting_for_the_database_does_not_hold_up_other_requests`; all three failed before the change. Unit tests in `accounts.rs`: `the_read_connection_sees_each_write_once_it_commits` (including a revocation seen by the next permission check), `reads_do_not_wait_for_a_write_in_progress`, `a_write_through_the_read_connection_is_refused` (also with `query_only` switched off), `in_memory_databases_read_what_they_wrote` and `the_read_connection_sees_the_migrated_schema`.
- **Docs:** SPEC (account data).

## 2026-10-18: The web app shows the plugin's version

Settings › About (and the Settings index) showed the web app's own package.json version, which never changed, so it read "0.1.0" through every release. The build now takes the version from `plugin/Plugins/LyrionAnywhere/install.xml`, the one release the web app ships in (`__PLUGIN_VERSION__`, defined in `vite.config.ts`; a build outside the full checkout falls back to the package version). Test: `tests/version.test.ts` checks the built-in version against install.xml and that About shows it.

## 2026-10-18: Plugin 0.5.2 released

The fixes for GitHub issues #14 and #11, the documentation for #13, the gateway's authentication lows from the October review (schema 13: the devices table rebuilt with `AUTOINCREMENT`, every row kept), and the quieter event log. Android 1.15.3 remains current. Notes: `docs/release-notes/v0.5.2.md`.

## 2026-10-18: Quieter event log, tidier tests, a steadier player test

- **Gateway:** an LMS CLI event line over 16 KiB was logged as a warning whenever it was dropped. A user's log showed three per menu change: LMS's `menustatus` and `displaynotify` lines for a controller, which the gateway never forwards. The warning is now kept for oversize lines of a kind the gateway does forward (`playlist`, `mixer`, `power` and the rest); other kinds are dropped at debug level. Test: `only_oversize_lines_we_would_forward_are_worth_a_warning`.
- **Tests leave the temp folder alone.** The test suites had left 16,500 `anywhere-*` directories (1.4 GB) in the system's temp folder on the development machine. The mobile harness now removes its analysis cache folder when it is dropped, and the gateway's download and live tests make their scratch folders through one helper that also removes its own leftovers from earlier runs. `tools/disk-gc.sh` gains a sixth step for the rest: this project's `anywhere-*` scratch directories, Robolectric's extracted runtimes, abandoned Playwright downloads and Vitest module caches made from this checkout, older than the usual N days; nothing else in the temp folder is touched.
- **A steadier test:** `a_real_underrun_after_a_grouped_rejoin_is_still_reported` asserted at once that half a second had been pulled after a stall; under CI load it had not always been. It now waits for it (the output pulls in the background). This was the failure on a Dependabot pull-request run on 17 October.
## 2026-10-18: Gateway: authentication lows from the October review

The open low findings about sign-in and sessions from the 15 October code review. Each fix has a regression test that failed before it.

- **Cross-username lockout reset** (`accounts.rs`): a successful sign-in removed the address's whole record, so an attacker with any account of their own could reset the address's cross-username lock every four guesses. The address's lock now remembers whose attempts its failures were, and a success clears only that user's own; failures for other usernames, or for names that can't exist, stand until the window ends. Test: `a_success_clears_only_the_users_own_failures_from_the_address`.
- **The right password without its code** (`accounts.rs`): with two-step on, it returned "code required" uncounted and unaudited, so whoever knew the password could have it checked (a full argon2 hash, holding a permit) without limit. It now counts against the address's lock, and at a password change against the account-change lock too, and is audited as `login.totp_required` or `password.change_totp_required`. It doesn't count against the username's lock, so the owner's usual two-step sign-in (ask, then give the code) still works while someone else's wrong guesses throttle the username, and the sign-in with the code clears it. Tests: `a_missing_code_counts_as_a_failure_and_is_audited`, `a_password_change_without_its_code_counts_and_is_audited`, `a_throttled_two_step_user_still_signs_in_from_elsewhere`.
- **Account-change lock per user and device** (`accounts.rs`, `auth.rs`): it was per user, so anyone holding one of the user's 15-minute access tokens could, with five wrong password changes, lock the owner out of password change, two-step setup and removal, and pairing codes on every device for up to a day. It is now keyed on the user and the device asking. Test: `one_device_cannot_lock_the_users_other_devices_out_of_account_changes`.
- **Device ids never reused** (`accounts.rs`, `lib.rs`): the devices table had no `AUTOINCREMENT`, so once the newest device was deleted (a deleted user, a pruned revocation) the next one got its id, and with it what was keyed by the id: the menu actions LMS had offered the old device, and its audit rows. Schema version 13 rebuilds the table with `AUTOINCREMENT`, keeping every device, id, refresh token and diagnostics report (foreign keys are off for the rebuild, so dropping the old table cascades nothing, and the references are checked after it). Ending a revoked device's sessions, which every revocation does, now also forgets its learned menu actions. Tests: `device_ids_are_never_reused`, `migration_to_13_keeps_devices_and_stops_id_reuse`, `a_revoked_device_forgets_its_learned_actions`.
- **A closed socket channel is held for resume** (`session.rs`): a frame for a socket whose writer had already gone detached it without the grace timer or the stand-in heartbeats, so LMS dropped the player after about 15 s and the client couldn't resume. It now takes the same path as a socket that couldn't keep up. Test: `a_closed_socket_channel_is_held_for_resume`.
- **Refresh rows bounded** (`accounts.rs`): each sign-in reusing a device added an unused refresh row that only expiry removed. A device signing in again now keeps at most its newest four unused refresh tokens; the older ones go, with their access tokens. Test: `signing_in_again_on_a_device_keeps_its_refresh_rows_bounded`.
- **Audit nits** (`accounts.rs`): signing out (`POST /v1/auth/logout`) records `device.revoked` with the device that did it (detail `self from <id>`, as for *Your devices*), and revoking a device that is already revoked, or doesn't exist, touches nothing on the admin path as on the owner's. Tests: `signing_out_records_the_device_that_did_it`, `revoking_a_revoked_device_again_touches_nothing`.
- `tests/account_password.rs` `two_step_users_need_a_code` now checks a wrong password after the successful change: asking without a code counts, so the old order reached the lock first.
- **Docs:** PROTOCOL (two-step sign-in and password-change rate limits and audit events), SPEC threat table.

## 2026-10-18: Web fixes from the first GitHub issues

The forum testers' first reports, filed as GitHub issues #9 to #14.

- **Ratings on phones (#14).** The one rating toggle beside the title on Now playing lit only for the user's own rating, so a track rated 4 or 5 stars in LMS showed a blank star on an iPhone while the wide layout's five stars showed it. It now lights for the rating shown: the user's own in the accent colour, or the rating LMS holds for the track, muted, with the label saying "Rated 4 of 5 on the server". A tap still loves the track or clears the user's own rating; the server's rating stays showing after a clear. The heart style behaves the same.
- **Downloads over the stream (#11).** Playing a downloaded album while this device streamed from LMS left both audible, with the transport on the downloads and no way to reach the stream. Now `controls.playDownloads` pauses whatever this device plays for the server first (through LMS while in reach, held here when out of reach, including a download playing from LMS's queue), and the server's music starting on this device stops the downloads, so what is heard is always what the screens show. Pausing the stream to start downloads is not a start, so it never stops them. Tests: `tests/downloads-exclusive.test.ts`.
- **Documentation (#13).** FEATURES and PLATFORMS now say that read-ahead, the replay cache and downloads standing in for streams are the Android app's; a browser holds only the current track. A multi-track cache for browsers is noted as a later improvement.
- The scrubbing report (#12) could not be reproduced on the owner's iPad; the reporter has been asked for details. Bit-perfect output from Safari (#9) is not possible from a web page and waits for the iOS app.

## 2026-10-18: Plugin 0.5.1 released

A web-app-only release carrying the entry below (the stream named in the quality badge and signal path, and *Automatic* explained per browser). The gateway and the plugin's behaviour are as in 0.5.0; Android 1.15.3 remains current. Notes: `docs/release-notes/v0.5.1.md`.

## 2026-10-18: The web app says what is actually streaming

Forum users on iPhones set *High*, saw the signal path describe their 96 kHz FLAC as 48 kHz "in MP4", and asked whether it was a display error. Half of it was. The gateway really had re-encoded the FLAC to Opus, which is always 48 kHz, and Safari plays through a media element fed losslessly as FLAC in MP4; but the panel never said "Opus", and its verdict came from the library track, so it could read "Lossless to the browser" while a lossy stream played. Nobody could tell from the app whether Opus was in use at all. This applies to every browser, not just Safari: the information comes from the player engine, not the browser.

- **Engine (`wasm`):** the status JSON now carries `source`: the decoded stream's codec, whether it is lossless, its bit depth and channels, from the core's `TrackAudio` (which the Android app already used). Cleared when stopped.
- **Signal path (web):** a *Stream to this device* row ("Opus 160 kb/s, re-encoded by the gateway for the High setting", "FLAC 24-bit, as stored (not re-encoded)", or "Not known yet" before the first audio is decoded); the first row renamed *Track in the library*; the media-element output described as "the decoded audio, re-wrapped losslessly as FLAC in MP4"; the rate row adds "(Opus is always 48 kHz)"; a verdict *Re-encoded to Opus 160 kb/s*, never "lossless", with a note pointing at *Settings › Playback*. With a lower level asked for and nothing decoded yet, the verdict is *Not known yet* rather than a guess.
- **Quality badge (web):** "Opus 160" instead of "FLAC 24/96 · Hi-Res" while this device plays a re-encoded stream (Now playing on phones, and the Info layout). Other players' badges are unchanged: only this device knows its own stream.
- **Settings › Playback (web):** *Automatic* explains itself per browser. Chrome on Android reports its connection; Firefox and Safari (every iPhone and iPad) have no Network Information API, so there the option reads "Always Original in this browser, which can't tell mobile data from Wi-Fi. Choose High or Data saver yourself when you're on mobile data", and the "Now: Original" line says why. The High and Data saver notes now give the data rate per hour, and say the gateway does the re-encoding.
- **Tests:** the engine's status before, during and after a track (`wasm`); the signal path's rows, headline and verdict for Opus, FLAC, lossy-as-stored and not-yet-decoded streams, with and without track details from LMS, on both output paths; the badge; the per-browser note; hostile `source` values (markup, long names, bad numbers) rejected before display; every row stays short plain text.
- **Docs:** FEATURES, PLATFORMS (iPhone and iPad), the streaming-quality plan, SPEC D25.

## 2026-10-15: Issue templates, and private vulnerability reporting switched on

For the forum announcement, which sends bug reports to GitHub Issues. `.github/ISSUE_TEMPLATE/` has a bug-report form (where it happens, versions, reverse proxy, what happened, steps, logs, and two required ticks: personal details replaced with placeholders, and not a security vulnerability), a feature-request form, and `config.yml` with contact links to private vulnerability reporting and the troubleshooting section. The repository's private vulnerability reporting was off, although `docs/SECURITY.md` already pointed at it; it is now on, so the *Report a vulnerability* button exists.

## 2026-10-15: README: the How-it-works diagram is a drawn SVG, and the docs are link-checked in CI

The owner asked whether *How it works* was meant to be a diagram. It was a Mermaid block, which GitHub renders in an iframe 180 px tall: the text was too small to read, GitHub's zoom buttons sat over the Lyrion Music Server box, and on a phone the whole thing was a smudge. Other viewers (the GitHub app, package pages, plain Markdown renderers) show Mermaid as source.

- **`docs/images/how-it-works.svg`:** a hand-drawn SVG, top to bottom so it reads at phone width, with the same content (the two apps on the internet, port 443 to the reverse proxy, the gateway's four jobs, LMS's three ports never forwarded, the plugin supervising the gateway). It follows the viewer's dark mode through a `prefers-color-scheme` media query inside the SVG, which browsers honour for an `<img>`. Checked in Chromium at 600 px in both schemes: every caption fits its box.
- **`tools/check-docs.py`:** every relative link, `#anchor` and image in README.md, SPEC.md, CHANGELOG.md and docs/ must resolve (GitHub's heading slug rules, duplicate headings numbered, fenced code skipped), SVG images must parse, and nothing may point outside the repository. Its `--self-test` plants each kind of breakage in a temporary tree and proves it is reported. A new CI job, `docs`, runs both on every push.

## 2026-10-15: Plugin 0.5.0 and Android 1.15.3 released

Plugin [0.5.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.5.0) (the full-review fixes, *Your devices*, the sync-group and stand-in heartbeat fixes, the web player's media-element sync corrections and start fix, *Sync with…* in Play on…) and Android [1.15.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/android-v1.15.3) (*Your devices*, playing with the screen off, the audio-focus and network-flap fixes, the output clock on the Signal path, *Sync with…*, Wi-Fi-only downloads by default). Notes: `docs/release-notes/v0.5.0.md` and `docs/release-notes/android-1.15.3.md`. The plugin release is *Latest*; the Android release stays a pre-release so the repository URL keeps serving the plugin.

## 2026-10-15: Docs: the README is a front page again; detail moved to docs/

Asked for by the owner: the README had grown to every list and table in the project. It is now a front page: a tagline (Lyrion Music Server brought into the mobile age: any device becomes a player, anywhere), a pitch that credits LMS and the Squeezebox world, *What it can do* (the strongest true capabilities first, sync with the players at home among them, each grounded in docs/FEATURES.md, SPEC D22 and D25 and the tests), *Under the bonnet* (the sans-I/O Rust core, its decoders, the 64-bit DSP that is bit-transparent when bypassed, track analysis and the signal path, the gateway and plugin, the resumable player tunnel and rolling cache), the screenshots, the How-it-works diagram, a three-step Quick start, a Documentation table linking every page, the licence and a one-line acknowledgement.

- **New pages:** `docs/SECURITY.md` (security and privacy in brief, and how to report a vulnerability; links both ways with `docs/PRIVACY.md`), `docs/PLATFORMS.md` (platform support, getting the Android app, project status and roadmap, brought up to plugin 0.5.0 and Android 1.15.3) and `docs/BUILDING.md` (the commands CI runs for the Rust workspace, the web app, the plugin and Android; the demo; live tests; contributing; the open-source acknowledgements).
- **docs/FEATURES.md:** opens with *Highlights* (what makes it different, then the browser and Android lists from the README), and its contents are the README's former *Features at a glance* table. *Synchronised playback* now describes media-element corrections and rejoining a group after a gap.
- **docs/README.md:** the index of every page, with a line on each; the Building row points at `docs/BUILDING.md`, and Wave 3.5's plan is listed.
- **Checked:** every relative link and anchor in README.md, SPEC.md, CHANGELOG.md, docs/*.md and the release notes resolves (231 links, none broken).

## 2026-10-15: Android player: a resume from another device no longer pauses the group

Seen live: the phone was in a sync group with a tablet, paused, screen off. Play on the tablet resumed the group, the phone played for under a second, then sent `pause` (70 ms after the resume, and again 300 ms later) and the whole group stopped. When LMS said play, the playback service asked for audio focus and paused LMS on a refusal. Since Android 15, an app that is neither on screen nor running a foreground service is refused focus outright, so every resume from another controller while the app was in the background was refused, and answered by pausing everyone. A skip worked because the engine never stopped.

- **Order:** when LMS (or the downloads) starts playing, the service is nudged into the foreground (`mediaPlayback`) before focus is asked for, and a refusal is asked again over about 2 s while the service gets there (`PlayFocusGate`).
- **Refusals told apart** (`FocusRefusals`, pure): refused while the app is on screen or a call is ringing or on is a real conflict and pauses at once, as before. Refused after the retries with the app wholly in the background (not on screen, the service not in the foreground, no call) is the background rule: playback carries on and the refusal is counted (`BackgroundPlayback.focusRefusals`, apart from the foreground refusals). Refused with the service in the foreground still pauses. Focus lost after it was granted (another app, a call) is handled as before.
- **Once per start:** one focus decision per start of playback (repeated reports before a stop make none), and a pause for a refusal is not sent again within 3 s, so a bouncing status can't pause twice.
- **Tests:** `PlayFocusGateTest` (Robolectric, the real `AudioFocusHandler` over a shadow `AudioManager` that refuses focus in the background and during a call) and `FocusRefusalsTest`. 10 of 12 failed against the old behaviour: the foreground first, a background refusal played through and counted, a conclusion only after about 2 s, a grant on a retry, a call on screen pausing once through a status bounce, one decision per start, a stop while retrying deciding nothing, and the pure rules. The two that passed guard what stays: a refusal in the foreground service pauses, and a later start after the cool-down is decided afresh. `BackgroundPlaybackTest` covers the new count.

## 2026-10-15: Web player: no silent "playing" after reopening the app on iPad

Seen live (Safari on iPadOS, the home-screen app reopened, media-element mode): the first track played silently while LMS, the core and the UI all treated it as playing, and a skip brought the sound back. The diagnostics log showed the chain. The start without a tap was refused (NotAllowedError) and the page asked for a tap. The tap's `play()` was then aborted (AbortError) by a stream restart at the same moment. Nothing tried again, and the "tap to resume" hold had already been cleared, because the element looked unpaused for an instant after `play()`.

- **The element's real state decides:** `MediaOutput.resume()` resolves with whether the element really plays. An aborted `play()` is tried again once the element is ready, on `loadedmetadata`, `canplay` or the next segment appended. It is tried again at each segment while the player wants it playing and it is paused. After 8 failed tries, the page asks for a tap instead. Each retry is logged ("media element: retrying play (1) after canplay"), and so is the play that follows.
- **A refused play goes back to "tap anywhere to resume":** this applies wherever the refusal comes from (the start, the worker's play, a synced start, coming back from the background). Until the tap, the worker's play and synced starts don't call `play()` again. The hold ends, and "audio resumed (media)" is logged, only when the element really plays (`onPlaying`). It no longer ends when the tap merely asks it to play. A tap whose play is aborted by a restart leaves the element to start by itself once the new audio is in.
- **No invented progress:** a paused element reports progress only up to where it stopped (`MediaOutput.pausedAt`). An element that never started reports none. A play head moved while paused (a seek, or Safari snapping it onto buffered audio) is skipped without counting, so the core never reports elapsed time for audio that was not heard.
- **Asking first where the browser needs a tap:** a refusal of an automatic start is remembered in this browser (`anywhere.playback.gesture`). The next automatic start opens the stream without calling `play()` and shows "tap anywhere to resume" at once. Firefox's `navigator.getAutoplayPolicy('mediaelement')` is used where present. A gesture in hand (`navigator.userActivation.isActive`) always lets the start try. A start that then plays without a tap clears the memory, so browsers that allow it keep starting by themselves.
- **Tests:** `tests/media-output.test.ts` adds 7 tests and `tests/device-media-resume.test.ts` adds 6. All but the `pausedAt` unit test failed before the fix. They cover:
  - an AbortError then `canplay` playing again;
  - retries at each segment until it plays, and none after a pause;
  - a NotAllowedError blocking until a tap;
  - endless aborts ending in the blocked state;
  - a deferred start;
  - the field report end to end (a tap aborted by a flush plays once the new audio is in, with the hold kept until then);
  - a refused play putting the hold back with no "audio resumed" claim and 0 frames reported;
  - a paused element whose play head moves reporting no progress;
  - the remembered refusal.

## 2026-10-15: Android player: the socket survives a network flap

Seen live: the phone (on Wi-Fi, screen on, playing in a sync group) lost its player socket about once a minute. The gateway logged "Connection reset without closing handshake". Android reports the default network lost, and the next one's capabilities a moment later, whenever it picks the default again (Wi-Fi scoring, a fallback to mobile data, a validation blip). The app passed that loss straight to the engine, and the engine dropped the socket at once by aborting its task. That left the gateway's unread keepalives in the receive buffer, so the kernel reset the connection.

- **Android:** a loss of the default network (or the default losing INTERNET) counts as no network only after none has been back for 1.5 s (`DefaultNetworkTracker` in `NetworkMonitor.kt`). A late loss of the network that was replaced is ignored. Wi-Fi and mobile data still change at once, for the downloads policy.
- **Engine:** a live socket outlasts a reported loss shorter than 2 s untouched: no close, no reconnect, the backoff unchanged. A socket that really died meanwhile is still caught by the idle and send timeouts, and its loss then counts as the network's.
- **Clean closes:** a loss that lasts closes the socket with a WebSocket close frame. Stopping sends `BYE!`, then a close frame. A protocol error closes the same way. A close from the gateway (sign-out, refusal) is answered before the socket is let go.
- **Reconnect now:** "gone, back" while already reconnecting still retries at once, and now always starts the backoff again (before, a quick pair kept the long delay it had reached).
- **Instrumentation:** the player status carries `network_flaps`, the losses reported in the last minute. The signal path shows "· 3 network losses a minute" on the output-clock line when there are any.
- **Tests:** `tests/player.rs` (each failed before the fix):
  - a loss and return 300 ms apart while playing: the gateway sees no disconnect and the audio plays on;
  - a 5 s loss: a close frame, not a reset, nothing tried meanwhile, and the session resumes when the network returns;
  - "gone, back" while reconnecting: a try at once and the first step of the backoff after it;
  - stopping: `BYE!` then a close frame;
  - signing out: the gateway's close is answered.

  The test proxy can now record what a client sends and how its side ends. The M11 flap test now expects no reconnect at all. Android: `NetworkTrackerTest` (5 of 8 failed against the old pass-through behaviour) covers a loss followed by a network within the grace never showing offline, a loss alone showing offline after the grace, Wi-Fi to mobile with nothing in between, a late loss of the replaced network, capability changes that keep INTERNET, and repeated losses. `LocalPlayerRepositoryTest` and `OutputClockTests` cover the new count.

## 2026-10-15: Web: join a sync group from Play on…

As on Android, the controlled player can join another player's group from *Play on…*.

- **Sync with… row:** an indented row sits directly under the controlled player's row (This device, or the remote player in use). It reads *Sync with…* when the player plays alone and "In sync with Study" when it is grouped. It opens a picker of every other player you may control. A group is one entry named after its members ("Lounge + Kitchen"), and the player's own group comes first, marked *This group*. Choosing an entry makes the controlled player join that group and play what it plays. *Leave the group* takes it out.
- **Each other player's ⋯:** gains *Join its group*. When that player is already in our group, the item is *Leave the group* instead.
- **The other direction is unchanged but renamed:** *Group players…* is now *Bring players into step with Lounge…*. Its switches bring other players into this player's group.
- **What is sent:** `<X> sync <Y>` makes Y join X's group (checked against LMS 9.1; see `lib/lms/sync.ts`). Joining therefore sends `<chosen> sync <this player>`, using the first member for a group, and leaving sends `<this player> sync -`. It goes through the gateway's existing RPC path, and only players you may control are listed or sent.
- **Needs the server:** the row, the picker and the menu items are disabled while the server is out of reach or busy, and they point at the connection message. A result (an error, or going back to the list) is applied only if the same player is still selected.
- **Tests:** `tests/player-switcher-join.test.ts` (new; all but one failed before the change). It covers:
  - the row in both states and where it sits;
  - the picker, with groups collapsed and the own group marked and disabled;
  - choosing a lone player or a group sends the right command for the right player;
  - *Leave the group*, and Back;
  - *Join its group* and *Leave the group* in the rows' ⋯;
  - disabled with a reason when out of reach and when busy, with nothing sent;
  - a failure is shown, but not after the selection changed;
  - hostile player names stay text;
  - the new label for the other direction.

  `tests/node/touch-targets.test.mjs`: the row, the picker entries and *Leave the group* are 44 px targets (this failed before the change). `player-switcher-sync.test.ts` and `rooms.test.ts` were updated for the new labels and menu item.

## 2026-10-15: Android: join another player's group from Play on…

Asked for by the owner, who couldn't find how to sync: the only ways were *Group players…*, which brings other players into step with the one you control (the inverse), and the Rooms page's ⋮.

- **Sync with…:** in *Play on…*, an indented row under the controlled player (or under *This phone*) reads "Sync with…", or "In sync with Lounge" while it is grouped. It opens a picker, as the web app's: "Sync Kitchen with…", the note "Kitchen joins that group and plays what it plays.", the player's own group first, marked "This group" and not a choice, then every other player you may control, a group as one entry ("Study + Hall", with "2 players"), and *Leave the group* while grouped. Phone and large windows share the one sheet.
- **Join its group:** each other player's ⋮ in *Play on…* has *Join its group*, or *Leave the group* when that player already plays with the controlled one.
- **Direction:** joining sends `<chosen> sync <this player>` (the argument joins the subject's group, as LMS takes it; a group is joined through its first member), never the other way round, which would pull the chosen player onto this one's queue. Leaving sends `<this player> sync -`. Both go through the existing `setSynced`, so the gateway's allow-list is unchanged.
- **Clearer inverse:** *Group players…* is now *Bring players into step with Kitchen…*, its switches "Bring Lounge into step".
- **Reliability:** the change shows at once and is put back, with a message, if refused; an answer is applied only while the same player is controlled and no later join or leave was made. Greyed out, saying why, while the music server is out of reach or busy.
- **Tests:** `JoinGroupTests` (written first; 19 of them failed against stubs): the view model sends `sync <chosen> <this> true` and nothing else, leaving takes only this player out, a refusal is put back and said, a late answer after switching player or after a later change is not applied, nothing is sent offline, busy, for itself, an unknown player or with no player; the picker's model (own group first, groups as one entry joined through their first member, unknown ids ignored); Compose tests of the row in both states and under *This phone*, the picker's entries and note, choosing an entry, *Leave the group*, the row's ⋮ (*Join its group* / *Leave the group*), offline and busy disabled with the reason, the renamed inverse, an accessibility audit, and a large window.

## 2026-10-15: Android player: the output clock, measured and shown

Seen live: an Android phone (its own speaker) and an iPad web player in a sync group. LMS saw their play points aligned, yet the iPad was heard about 100 ms ahead of the phone. The phone reports as heard what its audio device's timestamp says was presented. Without a usable timestamp it fell back on everything handed to the device, which is ahead of the speaker by the device's buffer and output latency (40 to 120 ms is typical on Android). Nothing on the phone said which was in use, so this couldn't be told apart.

- **Measured and shown:** the player's status now carries `output_clock` (device, estimated, or none while no output is open), `output_latency_ms` (from the device timestamp, the frames handed over but not yet heard, smoothed; without one, the device's buffer) and `output_clock_errors` (device timestamps in the last minute that were missing, stale, from the future or jumped by more than 20 ms; none are counted in the first second after an output opens). Worked out on the engine's tick, never in the audio callback. The signal path shows it: "Output clock: device · latency 38 ms" or "Output clock: estimated · latency about 90 ms", with "· 12 clock errors a minute" only when there are any, and a spoken description.
- **Better fallback:** without a usable timestamp, what is heard is what was handed over less the AAudio stream's buffer (its buffer size, or two bursts), never going backwards. A failed AAudio timestamp call is no longer stood in for by the frames read, for what is heard (it is still used, as before, to place a timed start).
- **Not changed:** how LMS's sync corrections are applied or reported.
- **Tests:** `audio.rs` (a fake output with a settable timestamp and buffer; these failed before the fix): with no usable timestamp and a 4800-frame buffer at 48 kHz, what is heard is 100 ms less than what was handed over and never decreases as the buffer grows; with a timestamp, the device clock and a 40 ms latency are reported; a closed output has no clock. `clock.rs`: the latency from a timestamp and from the buffer, smoothing, never going backwards over a mix of timestamps, unusable timestamps counted per minute (stale, future, negative, missing; ageing out after a minute; kept across a reopen), jumps counted, and wild input never panicking. `tests/player.rs`: a host that pulls PCM reports an estimated clock with no errors. Android: the status maps to the signal path's timing (`LocalPlayerRepositoryTest`), and a Compose test of both states, the error count and the spoken text (`OutputClockTests`).
- **What to read on the phone:** Signal path › "Output clock". "device" means the position LMS sees comes from the speaker's own timestamp; "estimated" means the device gave none and the latency shown is only the buffer. Clock errors mean the device's timestamp is unreliable.

## 2026-10-15: Web player: sync corrections in media-element mode

- **Web player:** sync corrections in media-element mode no longer seek or pause the element, which stalled on iPadOS and made LMS repeat the correction every two seconds. Seen with an iPad (the web app as a PWA, media-element mode) synced with an Android phone: LMS asked the iPad to skip ahead about 160 ms at every check for minutes, and the iPad dropped out briefly each time. A seek makes Safari drop the decoded audio and wait while it refills, which lost about as much time as the skip gained, so the play point LMS saw never improved.
  - The worker now makes LMS's skips and pauses in the audio before it is muxed (`static/player/media-sync.js`): a skip drops that much decoded audio, a pause inserts that much silence, and the element plays on without a break. Both are reported to the core at once, as the Web Audio worklet's are: a skip counts the dropped frames as presented straight away (they never reach the element, so they are never counted again); a pause holds back that many frames of the element's next played reports, and the silence pays them back when it plays. Once the element is past the correction, what is reported is exactly what is heard.
  - The correction is heard after the audio already appended to the element (up to 8 s, kept so the element doesn't run dry in the background on iOS); LMS sees it at once, so it doesn't ask again meanwhile. A skip of more than the core has decoded drops the rest as it arrives; there is no seek fallback.
  - `MediaOutput.pauseFor` and `skipAhead` are gone; the page ignores such messages. Web Audio mode is unchanged.
  - **Tests:** `tests/node/media-sync.test.mjs` (new; failed before the fix): a skip is reported at once and never counted again; a pause stalls the reported position at once and its silence is not counted as track audio; a model of LMS's 2 s check asks for one skip and then nothing; a skip beyond what is decoded; a flush; junk input; random corrections add up to what is heard. `tests/device-media-hold.test.ts`: a skip or pause message never seeks or pauses the element (failed before the fix).

## 2026-10-15: Android: the next track starts with the screen off

Reported on the 1.15.1 test build: with the screen off, a track changed from another device didn't play until the phone was woken. The gateway's log showed the phone's player socket dropping and resuming every 30 s to a few minutes meanwhile, while the music carried on from what had been read ahead.

- **Cause, from the code:**
  - Replacing the queue from elsewhere (LMS stops, clears the playlist, then loads the new one) read for a moment as no playlist. The session went idle with an empty timeline, and Media3 then removes the notification and leaves the foreground at once, with no grace. Android doesn't let a backgrounded app go back to the foreground (unless battery optimisation is off for it), so playback carried on as an ordinary background app, whose network Android cuts while the phone dozes.
  - The wake and Wi-Fi locks were let go the moment LMS's last read status stopped saying "playing", and taken only once the app had read LMS's status again, not when the engine itself started the next track.
  - Playback started while the app was already in the background is never made a foreground service, for the same reason. That Android rule can't be worked around without the battery optimisation exemption.
- **Fixes** (`playback/SessionState.kt`, `LmsSessionPlayer.kt`, `PlaybackService.kt`):
  - `SessionHoldover` keeps the last playlist for up to 90 s while LMS has none and the phone is still a player. It shows as buffering while LMS or the engine is starting a track, and as paused otherwise. It goes by itself once the time is up, and at once when the player is switched off or stopped.
  - `LockKeeper`: the locks are held while playing, while the engine buffers or plays (before LMS says so), and for 90 s after.
  - Android's refusals of the foreground are noted (`BackgroundPlayback`). When the app opens while playback carried on without the foreground, the service takes it back.
  - **Settings:** while this phone is a player and battery optimisation is on for the app, a one-time card explains that Android may cut the player's connection with the screen off and opens Android's battery optimisation list (*Open settings*, or *Not now*, which is remembered). Settings › Playback keeps the option until the app is unrestricted. No new permission: the list needs none, and the app doesn't ask for the exemption directly.
- **Not changed:** the engine's 25 s quiet-socket watchdog. With the network cut by Doze nothing arrives however long it waits, so a longer limit would only keep a dead socket; the fix is to keep the network.
- **Tests:**
  - The regression test (`LmsSessionPlayerTest.aQueueReplacedFromElsewhereKeepsTheSessionThroughTheGap`) failed before the fix: the timeline went empty mid-change.
  - JVM tests for the holdover (the gap, the clock from the start of the gap, a new playlist, no longer a player, nothing before any playlist, reconnecting) and the lock keeper (the grace, switched off, a clock going backwards), with seeded robustness runs on arbitrary sequences of each.
  - A Robolectric test that the kept playlist goes by itself after 90 s, and that a jump from it still names LMS's entry.
  - Compose tests for the card, its refusal wording and buttons, and the row in Settings › Playback. The data module's tests cover when it is offered and that *Not now* survives a restart.

## 2026-10-15: Held heartbeats no longer reach LMS after a gap; the gateway's own heartbeats never look stalled

Seen live: the phone's player socket dropped and resumed many times. After a four-minute gap, the Android player sent LMS about 250 `STMt` heartbeats it had held while away, all at once. Each had jiffies a second later than the one before but arrived in the same millisecond, so LMS re-based its clock for the player 257 times ("adjust jiffies epoch"), its sync play points went wrong, and it sent the group 178 `skipAhead` corrections in the next 15 minutes (49 to the phone, 129 to the other member).

- **Android player** (`mobile/src/player/protocol.rs`): `push_outbox` never holds an `STMt`, and `release_outbox` drops any it finds. A held heartbeat is worthless: the gateway reports for the player while it is away, and the core sends a fresh one within a second of the link returning. Every event (`STMs`, `STMd`, `STMu`, `STMo`, `STMr`, `STMf`, `STMp`, `BYE!`, …) is held and sent as before, the grouped-rejoin `STMo` rule included. A full outbox now simply drops its oldest frame.
- **Gateway, defence in depth** (`gateway/src/session.rs`): for 5 s after its last heartbeat for a session, a client `STMt` with jiffies behind that heartbeat's (wrapping-aware) is not forwarded, and doesn't become the report later heartbeats are built from; how many were dropped is logged at debug. Other frames are forwarded whatever their age. This covers older apps and the web player. The detached heartbeat's elapsed time already stops at the end of a known track.
- **The gateway's heartbeats never look stalled** (`gateway/src/session.rs`): while the phone was away, the heartbeats' elapsed time stopped at the end of the track LMS knew (and froze after an `STMu`) while their clock ran on. The phone was playing on from its cache, but to LMS it looked stalled: its apparent start time moved 31 s every 31 s, and it sent `skipAhead` corrections every 2 s for five minutes, correcting the other group member against that frozen reference too. Elapsed time now keeps pace with the clock while the client was playing, past the track's end and after an `STMu`; only `STMp`, `STMf` and `STMn` freeze it. The cap (added 2026-10-13 so LMS didn't show a position past the end of the track) is removed; the grace bounds how far it runs.
- **No sync corrections queued for an absent client** (`gateway/src/session.rs`): LMS's `strm a`, `strm p` with an interval and `strm u` with a start time, sent while the client is away, are dropped (counted in a debug log) instead of being replayed on resume, where the player would have jumped by their sum. Pause and unpause now, stop, flush, start, volume and the rest are kept.
- **Tests** (each failed before the fix):
  - gateway: `heartbeats_keep_advancing_past_the_end_of_the_track` (replaces `heartbeats_never_run_past_the_end_of_the_track`), `heartbeats_advance_after_a_track_end_while_away` (`STMs` then `STMu`, a known two-minute track, aged 30 s and 300 s: the apparent start is unchanged) and `sync_corrections_are_not_kept_for_a_detached_client` (three `strm a`, a `strm p` 500 and a timed `strm u` are not queued; `strm q`, `strm p`/`u` 0, `strm s` and `audg` are delivered on resume, in order; attached, everything goes through);
  - `held_heartbeats_are_never_released` (250 held `STMt` around an `STMs` and an `STMd` release only those two, in order, grouped or not), and the robustness test now also puts frames straight into the outbox and checks no heartbeat is released;
  - `mobile/tests/player.rs` `held_heartbeats_are_not_sent_after_a_gap_but_events_are`: a 30 s cut through the real gateway; before the fix LMS got 28 heartbeats older than it had already been told. Now none, a fresh one soon after the link returns, and the next track's `STMs` and `STMd` from the gap still arrive;
  - gateway: `stale_heartbeats_after_a_resume_never_reach_lms` (three stale `STMt` dropped, an old `STMs` and a newer `STMt` forwarded) and `the_stale_heartbeat_guard_expires_and_handles_the_wrap`; the negative `heartbeats_are_forwarded_when_the_gateway_never_spoke`.

## 2026-10-15: Web: review fixes for downloads, sign-in and Your devices

From the full code review of 15 October (M19, M20, M21's web part and four web lows).

- **Removing downloads asks first (M19):** *Remove download* (album and playlist pages), *Remove downloads* (artist page) and the remove buttons on the Downloads page (playlist, album, single track) open a confirm that names what goes and how many tracks. A removal runs once however often it is tapped: its control is off while it runs. A failed removal (IndexedDB refusing) says so rather than becoming an unhandled rejection; *Remove all* gets the same guard.
- **Sign-in says why it was refused (M20):** a 403 (the Origin or CSRF check, often a proxy that drops `Origin`), a 503 busy and a 5xx each have their own message, and other refusals name their status. *Could not reach the server* is kept for requests that never reached it (`lib/account/login.ts`).
- **Your devices (M21, web):** a failed first load offers *Try again*. The controls stay off until the refresh after a sign-out has finished, and a late answer to an older load is dropped, so a signed-out device can't reappear. A sign-out that worked takes the device off the list at once and keeps its message even if the refresh fails.
- **Web lows:** a 2xx answer that isn't JSON (a proxy's fallback page) is now a 502 rather than an empty answer. Artwork is fetched with `cache: 'default'`, no longer `force-cache`, which served even stale entries from a browser cache shared between accounts. The two-step forms run one request at a time. A failed sign-out is shown. When downloads were asked to go and can't be removed, the device stays signed in rather than leave them behind (`lib/account/sign-out.ts`).
- **Tests:** component tests that fail before each fix:
  - confirm, double activation and failure on all three download surfaces;
  - each sign-in status;
  - Retry, an out-of-order refresh, and a failed refresh after a sign-out;
  - the non-JSON success, the art cache mode, the double *Turn on* and the failed sign-out.
  - Coverage added for *Your devices*: a 409 and a 401 reaching the list, a double confirm and the empty list. The settings page test resets the gateway's features in `afterEach`.

## 2026-10-15: Plugin settings guarded against LMS's `pref` command; release and docs fixes from the full review

Fixes for findings M1, M17 and M18 of the full code review of 15 October 2026.

- **Gateway settings can only be changed on the settings page (M1, security)** (`plugin/Plugins/LyrionAnywhere/Gateway.pm`, `Settings.pm`): LMS's `pref` command could set the gateway's listen address, public URL, trusted proxies, client address header, ffmpeg path, download cache size and gateway path without the page's token, so any web page a LAN user opened could blind-POST to `/jsonrpc.js` and, at the next gateway start, have it listen on the LAN and trust the page's own forwarded headers. The settings page, the one authorised writer, now records these values in `settings.authorised` (0600, in the 0700 data folder), which `pref` cannot reach. The gateway always starts with the recorded values; a pref that differs is put back, logged, and shown in the page's status as "Settings changed outside the Lyrion Anywhere settings page were reverted (…)". On first run the current values are recorded.
  - **Tests:** `plugin/t/11-authorised-settings.t` (failed before the fix): `pref`-style changes after a page save are not used and are put back, with the notice; the gateway path and ffmpeg path likewise; a forged page save changes nothing; a later page save updates the record and clears the notice; first run; the file's mode; awkward characters round-trip; an unrecognised record is not trusted. The status row is tested in `plugin/t/js/settings.test.mjs`.
- **The plugin repository URL is guarded (M17)**: LMS installs fetch updates from `releases/latest/download/repo.xml`, and GitHub's *latest* is the newest full release, so promoting an Android pre-release (as `docs/RELEASING.md` used to say) would have made that a 404 and silently stopped plugin updates everywhere. Android releases now stay pre-releases (RELEASING.md explains why), and are published with `--latest=false`; plugin releases with `--latest`. New `tools/check-latest.sh` fails loudly unless that URL serves the plugin's `repo.xml` (optionally of a given version). CI runs it after each plugin and Android release, and the new *Plugin repository URL* workflow (`.github/workflows/repo-xml.yml`) runs it when any release is published, promoted, edited or deleted, and weekly.
  - **Tests:** `plugin/t/js/check-latest.test.mjs` (failed before: no script) runs the script against a local fake server: the plugin's `repo.xml` passes; a 404, an HTML page, another plugin's or a malformed `repo.xml`, a bad version, a server error and an unexpected version all fail.
- **Docs match what exists (M18):** SPEC.md no longer claims `cargo-fuzz` fuzzing (the parsers have deterministic, seeded robustness tests; coverage-guided fuzzing is listed as future work, and the `core/tests/robustness.rs` header says so), a plugin self-test of public LMS ports, signed releases or gateway signature verification (the plugin checks bundled gateways against `Bin/SHA256SUMS` in the same zip, an integrity check; signatures are future work). The plugin's version is single-sourced: `Plugin.pm` reads it from `install.xml` (it said 0.2.0 while `install.xml` said 0.4.0), and the release job checks `Plugin.pm` as well as the tag against it. README's stale note about plugin 0.3.0 is updated, and docs/PROTOCOL.md lists `GET /healthz`, the static fallback and `GET/POST /admin/v1/plugins`.
  - **Tests:** `plugin/t/12-version.t` (failed before: 0.2.0 against 0.4.0): `Plugin.pm`'s VERSION equals `install.xml`'s; a missing, empty or odd file gives `unknown`.

## 2026-10-15: Android review fixes: Keystore wipe, camera after pairing, Your devices messages, start-up initialiser check

From the full code review of 15 October (M6, M15, M21's Android part and a test-coverage gap).

- **Sign-in kept through a busy Keystore (M6)** (`data/secure/EncryptedCredentialStore.kt`, `AndroidSecureStorage.kt`, `AccountRepository.kt`): the sealed refresh token was deleted on any read or decrypt error, so a cold start while Android's keystore daemon restarted signed the user out and left the device registration on the server. It is now deleted only on a definite integrity failure: a bad tag (tampered with, or another key), a format the app never wrote, a permanently invalidated key or undecodable contents. A `KeyStoreException`, `ProviderException`, I/O error or anything else keeps it and reads as "unavailable".
  - **Start-up:** an unreadable store shows *Can't open secure storage* with *Try again*, never *signed out*. Nothing is cleared and the server isn't told. Offline mode's retries (and returning to the foreground) try again; once the store reads, the saved sign-in is resumed.
  - **Key provider:** no replacement key is made when the Keystore throws, or when the alias exists but won't yield its key. A new key is made only when there is none.
- **Camera off after leaving the pairing screen (M15)** (`ui/onboarding/QrScanner.kt`): leaving before CameraX's provider arrived let it bind the camera afterwards, running with no preview and still reading QR codes. A `CameraLease` now never binds once disposed and releases a camera bound while disposing. The analyser is switched off on disposal. Its on/off flag is now set after composition, not during it. The shared provider future isn't cancelled, as CameraX hands the same one to the next scanner.
- **Your devices messages (M21, Android)** (`ui/settings/Settings.kt`, `YourDevices.kt`): a list that loads now clears an earlier load failure. After a sign-out the page stays busy until the list has been read again, so a second sign-out can't start from a stale list or land out of order. If that reload fails (the shared limit of 30 a minute, say), the sign-out's message stays and the device just signed out leaves the list. A failed first load shows the error with *Try again*.
- **Merged manifest checked by the build** (`app/build.gradle.kts`): `verify<Variant>StartupInitialisers` fails the build unless the merged manifest's `androidx.startup.InitializationProvider` lists exactly EmojiCompat, ProcessLifecycle and ProfileInstaller. Every variant's assemble, bundle and lint depend on it. Until now only the source manifest was tested, which can't show what a library's merge adds.
- **Tests:**
  - Each fix's regression tests failed before the fix. M6: the store keeps the blob on `IOException`, `ProviderException`, `KeyStoreException`, `UnrecoverableKeyException` and other errors, and still deletes it on `AEADBadTagException`; the repository's start-up with an unreadable store neither clears it nor signs out, and resumes on retry; the key provider makes no replacement key. M15: the camera is never bound when the provider arrives after disposal, both directly and with the composable. M21: view-model and screen tests for each case above, plus the missing test that a failure is shown in the error colour.
  - The manifest check was confirmed to fail the build when WorkManager's initialiser is let back in.

## 2026-10-15: Review fixes: the vendored Opus decoder, and the player core

From the full code review of 15 October (H2, M22, M23, M12, M13).

- **Opus: one hostile packet no longer aborts the web player (H2).** A short Hybrid packet claiming more redundancy bytes than it holds panicked the vendored decoder (`third_party/opus-decoder/src/lib.rs`); the web player decodes Opus in WebAssembly, where a panic aborts it. The decoder now drops the bad redundancy, as libopus does. New hostile-packet tests in the vendored crate: every TOC configuration and frame-count code with random payloads, truncated and bit-flipped real SILK, Hybrid and CELT packets, and the two crafted packets that panicked it.
- **Opus: no more debug logging (M22).** Upstream tried to append to a log file at a fixed path on its author's machine on every CELT frame, and built JSON for no-op loggers. All of it is removed; output is bit-for-bit the same, and a test fails if file I/O comes back.
- **CI runs the vendored decoder's own tests (M23)**, including the check on the local FFT patch, the no-I/O check and the hostile packets (`third_party/opus-decoder/PATCHES.md`).
- **Player core: a replaced track no longer leaves the player stuck (M12).** A new stream replacing one still decoding left an unfinished segment behind, so a new track at another sample rate never played and `STMu` never went. A refused codec no longer leaves the player "decoding" for ever (a spurious `STMo` in place of `STMu`).
- **Robustness tests see panics (M13).** The decoder tests no longer count a caught panic as surviving. They found a real one: whole-file decoding and FLAC verification skipped the WAV header check, so a hostile channel count overflowed Symphonia's WAV reader in debug builds. Every whole-file entry point now checks first.

## 2026-10-15: Gateway review fixes: SSRF by DNS, two amplifiers, Your devices limit, blocking writes

Fixes for findings M2, M3, M4, M5 and M7 of the 15 October code review.

- **URLs in free text are resolved before they reach LMS (M2)** (`rpc.rs`, `art.rs`, `rpc_policy.rs`, `menu_actions.rs`, `stations.rs`): a search term, typed menu input or pinned-menu step holding a public-looking http(s) URL whose name resolved to a loopback or private address (an A record of 127.0.0.1, an IPv4-mapped private AAAA) could make an LMS plugin fetch from the server itself or the home network. Every http(s) URL in a command's text is now resolved, sharing the image proxy's check (2 s limit, at most four names per command), and the command refused like other sensitive text if any answer is local. Text the user typed may also give no port other than 80 or 443. Pinned menus are checked on every replay. Limits as for artwork: DNS rebinding between the gateway's lookup and LMS's is not caught, and a name that doesn't resolve is judged by its name.
- **Track-start lookups coalesced and capped (M3)** (`session.rs`, `history.rs`): each `STMs` from a client spawned an LMS `status` query, so a client looping them made the gateway query LMS once per frame. A session now has one lookup waiting at most, which asks only about the latest start, and makes at most 30 a minute.
- **One ratings batch at a time, shared writes bounded (M4)** (`ratings.rs`): an offline batch wrote LMS's shared rating once per track, serially, with the 30 s LMS limit each, and a user could run any number at once. Now one batch per user at a time (429 with `Retry-After`, as for `/v1/plays`), each shared write has a 3 s limit, and a batch spends at most 10 s on them, stopping at the first that LMS doesn't take. The user's own ratings are always kept.
- **Your devices allowance per device (M5)** (`auth.rs`, `accounts.rs`): the 30-a-minute limit was per user, so a stolen device's token could keep the owner from signing it out. It is now per device asking. `device.revoked` records which device did it (detail `self from <id>`).
- **SQLite writes off the async workers (M7)** (`lib.rs` `Gateway::db`, `auth.rs`, `tunnel.rs`, `session.rs`, `plays.rs` and the `/v1/me` stores): token renewal, sign-in and pairing bookkeeping, the last-seen write, player attach, Your devices, renames, listening history, ratings, Home, stations and filters wrote inline on tokio workers under the store's one lock, so a stalling disk could stop player sessions and their heartbeats. They now run on the blocking pool. Not changed: reads still share the lock, and the loopback admin API still writes inline.
- **Tests**, each regression test failing before its fix:
  - `gateway/tests/rpc.rs`: search terms, `term:` and typed menu input with URLs resolving to loopback or an IPv4-mapped private address refused with the usual refusal and never sent; ports other than 80 and 443 refused; plain text, public URLs and unresolvable names still pass. `me.rs`: a pinned menu replaying such a URL never reaches LMS. Unit and robustness tests for the URL scanning; the resolver takes fixed answers for tests (`Config::host_answers`).
  - `me.rs`: 200 `STMs` in 2 s make at most 3 lookups; spaced starts at most 30 a minute; ordinary track changes each still get theirs. Tracker unit tests for a row that arrives after its listen ended or continued.
  - `ratings.rs`: 500 ratings against a Ratings Light that hangs end within seconds with every rating kept; a second batch while one runs is 429 and another user is not held up.
  - `own_devices.rs`: a device that has used up its allowance can't stop another of the user's devices listing it and signing it out; the audit detail names the acting device; malformed ids are a strict 404.
  - `db_stall.rs`: with writes stalled through a test hook, an unrelated request answers at once during a stalled renewal, player attach and last-seen write.
- **Docs:** docs/PROTOCOL.md (free text, ratings, Your devices); SPEC.md (free text, listening history, the account database).

## 2026-10-15: Android/mobile fixes from the full code review (H1, M8, M9, M10, M11, M14, M16)

From the full code review of 15 October 2026. Each fix has a regression test that failed before it.

- **H1, replay cache emptied by Settings** (`mobile/src/player/cache.rs`): asking for the cache's totals (or clearing it) while no player held it open opened it with a cap of zero, which deleted every entry. A cap that isn't known now keeps whatever is there; only a player sets the real cap. Tests: a populated cache opened afresh with no cap keeps every file, through the cache and through `stream_cache_totals` with the player stopped; a smaller real cap still evicts.
- **M8, event socket renewal loop** (`mobile/src/events.rs`): a gateway that kept closing the event socket as "unauthorised" drove a tight loop of token refreshes and secure-store saves. Only a subscription now resets the renewal guard: one renewal is retried at once, then the normal backoff applies. Test: a fake gateway that always refuses sees at most three refreshes before the first wait.
- **M9, sync-group rejoin outliving the player** (`mobile/src/player/engine.rs`): a rejoin waiting on a slow LMS could still move the other speakers after the player was stopped, stopped by the server or took over playback on the phone. All three now drop it, before the goodbye's wait. Test: LMS holds its answer while the player stops; no `time` reaches LMS.
- **M10, stream hang on an oversize prefix** (`engine.rs`): a gateway that sent more than `?limit=` asked for, and then broke, made the retry's limit underflow; the stream task panicked and the track waited for ever. The limit is now worked out without underflow, and a watchdog reports a panicking stream task as a failed stream. Tests: a gateway that ignores `?limit=` and breaks; unit tests for the arithmetic and the watchdog.
- **M11, reconnect pacing** (`engine.rs`): a greeting reset the backoff, so a server that greeted and closed at once was reconnected every 0.5 s for ever. The backoff now starts again only after a connection has lived 5 s. The network coming back reconnects at once only if no connection was opened in the last 1.5 s, and only an outage that long resets the backoff. Tests: greet-then-close gets a growing delay; 20 network flaps in a second make at most three connections.
- **M14, offline library bound to its account** (`mobile/src/offline/`, `routine.rs`): `sync_plays`, `sync_ratings` and `Downloader::new` refuse a library opened for another account or server with the new `ApiError::WrongAccount`, before anything is sent. Android shows it as a refusal. Test: another account's library is refused by all three and the gateway sees no request. `resolve_offline_tracks` takes no library; the app already checks the queue's owner before "Continue on".
- **M16, "already signed out" by status** (`mobile/src/error.rs`, `session.rs`): 404 and 409 are now typed (`ApiError::NotFound`, `ApiError::Conflict`). Signing out a device answers "already gone" for any 404, and never for another status whose text happens to match. Android maps both to the same message as before, and the test fake answers for this device as the real repository does. Tests: 404 with any body, 400 and 403 with the old text, 409, 429 with `Retry-After`, and a gateway without the devices feature.

## 2026-10-15: Android 1.15.0 (versionCode 29)

A test build, not published: *Your devices* and the sync-group rejoin fix (the two entries below). The review fixes followed in 1.15.1 and 1.15.2, neither published either; 1.15.3 was the first of the series to be published (see *Plugin 0.5.0 and Android 1.15.3 released*, above).

## 2026-10-15: Android: a phone rejoining a sync group no longer pauses the others or moves them back

Found by the live check of A6 (`mobile/tests/live_sync.rs`). When the phone's link stalled while it was in a sync group, the other speakers went silent for 4 to 15 s once it came back. After a long stall, LMS also restarted the whole group about 14 s back.

- **Stale underrun report dropped** (`mobile/src/player/protocol.rs`, `engine.rs`): the `STMo` ("my output ran dry") sent mid-track while the link was down waited in the outbox and reached LMS on reconnect. LMS then paused the whole group to rebuffer. In a group, the phone now drops that held `STMo` (and the `STMr` for its recovery) when the link returns, since it handles the stall itself. The core sends no `STMr` for a report LMS never heard (`Player::withdraw_underrun_report`).
  - **Unchanged:** a phone on its own, the `STMo` sent in place of `STMu` at the end of the audio received, and any underrun while the link is up.
- **Restart at the group's position:** a phone with too little audio to skip the stall had LMS restart the group at `time ?`. With the phone as master, that lags at the phone's own stall point. It now restarts at where it stopped plus the time stalled, after checking with LMS that the same track is still playing.
  - **Nothing is sent** if LMS isn't playing (paused or stopped meanwhile), the track changed, the point is within 1 s of the track's end or past it, or the stall is longer than any session lasts.
  - **Fallback:** `time ?` is used only when the track's queue position was never learnt (a radio stream).
- **Tests:**
  - Regression tests that fail before the fix: the engine against the fake LMS (no `STMo` or `STMr` reaches LMS; the restart lands where the others are; no seek for a changed track, a paused group or past the end), and the core's withdrawn report.
  - Negatives: alone, the held `STMo` still reaches LMS; a real underrun after the rejoin is still reported.
  - Unit and robustness tests for the outbox release and the restart point, including arbitrary LMS answers and frames.
  - The live check now asserts what it only reported before: the others paused at most 1 s, and a restart moves them at most 1.5 s. It has yet to be run against the fix.
- **Docs:** docs/PROTOCOL.md, "Rejoining after a gap"; SPEC D22; the design doc's "A6 live check".

## 2026-10-15: Your devices (gateway, web and Android)

A signed-in user can now see every device signed in to their account and sign out the ones they don't want, without changing their password: *Settings › Account › Your devices* on the web and Android. Until now only an admin could (plugin settings), or the user by changing their password with *Also sign out my other devices*.

- **Gateway** (`gateway/src/auth.rs`, `accounts.rs`; docs/PROTOCOL.md *Your devices*): `GET /v1/devices` lists the caller's own signed-in devices: name, type and platform, when each first signed in and was last used (to the minute), and `current` for the one asking. No addresses, MACs, User-Agents or tokens. `DELETE /v1/devices/{id}` signs out one of the caller's other devices: its tokens stop working and its live player session ends at once, as for any revocation. `POST /v1/devices/sign-out-others` keeps only the caller. `/v1/me` lists `"devices": true` under `features`.
- **Security decisions:** only ever the caller's own devices, admins included (they manage others through the admin API). Another user's device, one that doesn't exist or has gone, and an id that isn't a plain positive number are all the same 404, so ids can't be probed; the ownership check is part of the revoking `UPDATE`, so nothing of another user's (tokens, diagnostics, player) is touched. The device asking is refused with 409 rather than signed out: that is *Sign out*, which also clears the browser's cookie and cached data. Signing out needs `X-Anywhere-Client` and an allowed `Origin`, like `/v1/auth/*`; a bearer token only, never a cookie. No password is asked for, as signing out only removes access and the caller can't sign itself out this way. The three calls share a limit of 30 requests a minute per user (429 with `Retry-After`). Audited as `device.revoked` with the user's name and detail `self`.
- **Web** (`lib/components/YourDevices.svelte`, `lib/account/devices.ts`): a *Your devices* row in Account, shown only when the gateway offers it, opens a dialog listing the devices (*This device* badge, platform, "Last used 3 hours ago"). Each other device has *Sign out*, and there is *Sign out all other devices*; both ask first. One announced message at a time. The gateway's list is checked before use: bad or repeated ids are dropped, at most one device counts as this one, names are cleaned and cut.
- **Android** (`ui/settings/YourDevices.kt`, `data/Devices.kt`; `mobile/src/session.rs`): a *Your devices* page under Account, with the same behaviour, through new Rust calls `Session::my_devices`, `sign_out_device` and `sign_out_other_devices`, and `AccountInfo::can_manage_devices`. A device already gone is said plainly, not as an error.
- **Tests:** gateway integration tests (`gateway/tests/own_devices.rs`): own devices only, for listeners and admins; cross-user delete is 404 with the other user's token, refresh token and live session untouched; unauthenticated, bogus, cookie-only and signed-out tokens refused; cross-site requests refused; malformed and arbitrary ids; the device asking refused; the revoked device's tokens dead and its session ended; pairing-code devices; the rate limit; racing sign-outs succeed once. Unit tests in `accounts.rs`, mobile integration and parser tests, web vitest and touch-target tests, and Android view-model and screen tests.

## 2026-10-15: App Store exception added to the licence

- **`LICENSE-EXCEPTION`:** the project's own code may now be distributed through app stores such as Apple's App Store or TestFlight, whose terms add restrictions the GPL alone doesn't allow, as long as the full source stays available under the GPL and the store's terms are the only added restrictions (SPEC D17). Third-party components keep their own licences; the mobile app's dependency tree has no other GPL code. The licence is otherwise unchanged: GPL-2.0-or-later.

## 2026-10-15: Plugin 0.4.0 released

The server side for Android 1.14.0: the gateway's busy-LMS handling and slow-request log, *Change password* (`POST /v1/account/password`), *Save the queue as a playlist* (`POST /v1/players/{player}/queue/save`) and the faster Opus decoder; and the web app's Wave 3.5 declutter, out-of-reach controls, busy notice and fuller downloads. The plugin's Perl is unchanged since 0.3.0. Notes: `docs/release-notes/v0.4.0.md`. The release also carries the Android 1.14.0 APKs, with their checksums in `SHA256SUMS-android.txt`.

- **Release check fixed:** `tools/test-release.sh` refused the zip because the web player's `web/player/local.js` matched its `local.*` pattern for local config. JavaScript files are now let through. A planted `dev/local.env`, `local.deploy.sh`, `local.json` or dotfile is still caught. A full local build (`tools/build-release.sh`) passes all 22 checks.

## 2026-10-14: Android: a track that ends just before the server goes away carries on

Found by CI, where `with_the_server_away_a_track_end_carries_on_into_a_download` failed in 3 of 4 runs. If the track LMS sent played out while LMS was still in reach (`STMu` sent, the next track not yet sent) and the link went down moments later, the phone stopped there, even with the next entry downloaded: it only carried on from a player still "playing". On a slow runner the 3-second test track ended before the test cut the link. On a phone, a track that ends in the seconds before a lost link is noticed would stop the same way.

- **Fix** (`mobile/src/player/engine.rs`): a track that played out with the link up counts as just ended for 15 seconds (`ENDED_GRACE`), so the next entry, if it is on the phone, follows as it would have. Later than that, LMS chose to stop there, and the phone doesn't start playing by itself. A stop from LMS (`strm q`) forgets the track, so it never counts.
- **Tests** (`mobile/tests/player.rs`): a regression test that fails before the fix (the track ends, then the link drops), and two negatives: a track that ended long before, and a player LMS stopped, both stay stopped.

## 2026-10-14: rustls 0.23.46 (RUSTSEC-2026-0346 to 0349)

Four rustls advisories published today stopped the Android 1.14.0 release in CI's supply-chain check. rustls is updated to 0.23.46. It is used by the Android app's Rust layer and the command-line tool; the gateway doesn't use it. Mobile and CLI tests pass (551), and the release APK was rebuilt and checked.

## 2026-10-14: README and screenshots brought up to date; Android 1.14.0 release notes

- **README:** the decluttered web and Android apps, controls that keep working while the server is out of reach, the busy-server notice, the rolling cache as built, *Play on…* in place of car mode among the Android highlights, and a folded *More screenshots* gallery of both apps. It notes that change password, save as playlist and the busy notice need a gateway newer than plugin 0.3.0.
- **Screenshots:** both sets regenerated (`tools/demo/shots.sh` and the Android `DocScreenshots` tests); only three web images changed.
- **Release notes:** `docs/release-notes/android-1.14.0.md`, covering everything since 1.10.0.

## 2026-10-14: Web: Wave 3.5 declutter, phase 3 (merges)

The third phase of [the declutter plan](docs/plans/wave3_5-declutter.md) for the web app, after its web design section (written there, beside Android's).

- **Rooms folds into Play on… (one player manager).** Play on… and Rooms now share one manager (`lib/rooms/manager.svelte.ts`). It reads every player's status only while one of them is open, and the actions (play/pause, volume, power, sync, moving playback) are the same in both.
  - **On phones, Play on… has everything:** a player's ⋯ holds *Move playback to <player>*, *Play/Pause <player>*, *Alarms* (that player's) and *Switch <player> on/off*.
  - **Rooms stays the expanded view** on tablets and desktops (the sidebar, and a link in Play on… there only).
  - **A Rooms card has three controls:** play/pause, volume and ⋯ (*Play on <player>*, *Move playback to <player>*, *Sync with…*, *Play on its own*, *Alarms*, *Switch <player> off*). A switched-off room shows one *Switch on* button instead.
  - **Gone:** the power button, the grip and the instructions paragraph. To sync, drag a card by its cover onto another, or use *Sync with…* from the keyboard and screen readers. While the server is out of reach, Rooms says it will update when your music server is back.
  - **Now playing's ⋯ no longer has Alarms:** a player's alarms are its row in Play on… and its card in Rooms.
- **Downloads play through the main mini player, Now playing and Queue (two engines, one face).** The downloads engine stays its own. An adapter (`lib/player/now.svelte.ts`) gives the main screens one view of what plays, and `controls` sends the transport to the downloads engine while its queue is active.
  - **The mini player** shows the download ("Downloads · artist"), with play/pause, next and previous (swipes too) and Play on….
  - **Now playing** shows its cover, title, a plain seek bar and the transport, with Up next from the downloads queue. ⋮/⋯ has *Stop playing downloads*. The rating, quality badge, sleep timer, This track, Car mode and the visualiser are left out for downloads.
  - **The Queue page and sheet** list the downloads in play order: a tap plays from that track, and the queue can't be edited there. The selected player's queue isn't read meanwhile.
  - **The separate downloads bar has gone** from the signed-in app. Offline mode's splash (nobody signed in on this device) keeps its own.
- **Tests:** `tests/downloads-one-face.test.ts` (the transport and the view while downloads play, the mini player, both Now playing layouts, the Queue page and sheet, hostile titles) and the Rooms and Play on… tests (three controls, Switch on, power and alarms in ⋯, drag by the cover, the shared menu, reading only while open).

## 2026-10-14: Web: Wave 3.5 declutter, phase 2 (structure)

The second phase of [the declutter plan](docs/plans/wave3_5-declutter.md) for the web app. Nothing was removed: every setting and section has a new home, each reached by a test.

- **Settings is an index.** It has seven rows (eight where the browser can install the app), each with a one-line summary of its value: Account, Appearance, Playback, Now playing, Downloads, Listening history, (Install the app,) About. Each opens a page of its own at `#/settings/<section>`.
  - **Advanced** is under About. It holds this device's player (start, stop, restart), the playback engine, the volume source and the diagnostics (folded away). It is the only place LMS is named.
  - **Account:** one row per action (Change password, Two-step verification, Add another device), each in its own dialog that asks for the password once, then Sign out. The code field is "Authenticator code" everywhere (sign-in too).
  - **Option lists open a picker** (one shared component, `PickerRow`) instead of being laid out inline: the start player, streaming quality, sleep fade, visualiser style, rating style, download formats, storage limit, playback engine and volume source.
  - **One switch component** (`Switch`) for every on/off setting: the lyric line, the visualiser, haptics, Wi-Fi only, listening history, an alarm and all of a player's alarms, and grouping players. Its state is in words ("On"/"Off") beside it, not colour alone.
  - Help is cut to one sentence per control. Download preferences are in Settings; the downloads themselves, and *Remove all*, are on the Downloads page, which now has a gear to its settings.
- **One status slot** at the top of every page, car mode included, replaces the layout's banners. Lasting states show in priority order: moving playback > out of reach > busy > reconnecting, then this device's offers (the sound held back until a tap, which is now a plain status rather than a placebo button; *Resume playing here* after a reload).
  - Results (`app.notice`, `app.error`) and the visualiser's style name on phones go to the one snackbar.
  - The "Moving playback…" strip above the mini player has gone: moving now shows in the slot.
- **Library is one model:** one row of sections that scrolls sideways (Albums, Artists, Playlists, Favourites, Smart playlists, Genres, Years, Compilations, Folders, Apps & radio, Downloads), each shown in place.
  - New music and Random albums are now orders of Albums ("Recently added", "Random", with *Shuffle again*).
  - `/albums`, `/artists`, `/new` and `/random` open Library at that section, without adding a history entry.
  - The virtual library choice is in the Library header's ⋯.
  - The second row of link pills, the library select and the duplicate album browser are gone.
- **Sidebar:** seven entries (Home, Library, Search, Queue, Now playing, Rooms, Settings). Library's seven repeat entries are gone. Now playing has its own icon (no longer the house), and an album lights up Library.
- **One trailing control per row**, continued: LMS menus (apps, radio, plugins, track info) put Play next, Add to queue, *Save as a station*, *Pin to Home* and the item's own *More…* in one ⋯ per row. Favourites rows do the same, and an alarm row is its switch (Delete is in the alarm's form, confirmed).
- **Naming:** "Smart playlists" everywhere the user reads it; "your music server" (not "your server", "the music server").
- **Accessibility:** a visible 2 px focus ring on every text input (sign-in included); filter boxes (Playlists, Genres, Search) show it around the whole pill.
- **Tests:** `tests/declutter-phase2.test.ts` (the switch, the picker, Library's sections in place, the orders, naming), `tests/status-slot.test.ts` (priority, one line, the connection id) and a rewritten `tests/settings-page.test.ts` (the index, every old setting still reachable, the pickers, Account's dialogs), plus the sidebar's rules in `tests/nav-tabs.test.ts`.

## 2026-10-14: Web: Wave 3.5 declutter, phase 1 (quick wins)

The first phase of [the declutter plan](docs/plans/wave3_5-declutter.md) for the web app. Almost every change is a move into a menu; nothing was removed. Each moved action has a test that reaches it in its new home.

- **No "Playing on …" strip.** The mini player now names the player in use ("on Lounge", first on its second line, in the accent colour). *Move playback to …* (formerly *Move here*, *Bring here*) and going back to this device are in **Play on…**. A strip shows only while playback is moving.
- **Play on…:** a tap on a row plays on that player. Moving the playback there is the row's ⋯ (*Move playback to Kitchen*, *Move playback to this device*). The sync switches are behind one *Group players…* row. The explanatory paragraph is gone, and *All rooms* is now *Rooms*.
- **Page headers are Play · Shuffle · Download · ⋯** on album, artist, playlist, genre, year and folder pages. *Shuffle* is new: it turns shuffle on and plays.
  - ⋯ holds *Play next*, *Add to queue*, *Download as…*, LMS's *More…*, and (for admins) a playlist's *Rename* and *Delete playlist*.
  - Destructive items come last, after a divider. *Remove download* is one of them.
  - The download format select is gone from the header. The usual format comes from Settings › Downloads; *Download as…* picks a one-off format.
  - Deleting a playlist asks in a dialog, as everywhere else, rather than "Tap again to delete".
  - A smart filter's page has Play, Shuffle and ⋯ (*Edit*, *Delete*).
- **Queue:** the header is the title and ⋯ (*Save as playlist…*, *Clear upcoming…*, *Clear the queue…*, each confirmed in a dialog). A row is its drag handle (44 px on touch), the track and ⋯ (*Play next*, *Move up*, *Move down*, *Remove from queue*). The ✕ and the move buttons are gone. The gapless link marks are gone from the queue; track info says "Gapless" instead. The queue peek keeps *Open the full queue* only.
- **One trailing control per row** elsewhere too: smart filter rows, music folder rows, a playlist's rows for admins, and Home's station cards each have one ⋯.
- **Now playing, wide windows:** the player bar is hidden under Now playing (no second transport, seek or volume). The action row is *Queue* and ⋯ (*Sleep timer*, *Turn the sleep timer off*, *This track*, *Signal path*, *Car mode*, *Alarms*), always the same, so it never shifts.
  - The Lyrics button and its dialog are gone. The lyric peek and the *L* key open the Lyrics layout.
  - The Play on button by the volume names the player.
  - A running sleep timer shows its time left by the track details.
- **Now playing, phones:**
  - The top row is close and ⋮. ⋮ holds *This track*, *Sleep timer*, *Visualiser*, *Car mode* and *Alarms*. *Signal path* is the quality badge's, *All rooms* is in Play on….
  - The bottom row is *Play on* (named for the player in use), *Lyrics* and *Queue*. Play on… carries the volume slider on phones.
  - The visualiser's style chooser and its on/off are in ⋮ › Visualiser (a swipe across it still changes the style).
- **One rating toggle by the title** (a heart, or a star for those who rate in stars): a tap loves the track (five stars), a second clears it. The full 1–5 rating is in ⋮ › *This track*, above the credits, which now come after its heading and close button.
- **Up next is one line** ("Up next · Title — Artist ›"). A tap opens the queue (the same sheet as Queue on phones). The ✕ is gone; a swipe or Delete still removes it, with Undo.
- **Home:** the station cards' bins are gone (*Delete station* is in each card's ⋯, confirmed). *Edit Home* is a quiet text button at the end of Home. The *Browse the library* card is gone (Library is a tab). A smart filter shelf's Play and Shuffle are disabled while the server is out of reach.
- **Continue on… is a snackbar with an action:** "Back online. Continue 3 tracks from your downloads on Lounge?" with *Continue* and *Other player…* (a list of the players you may control). Dismissing it is "Not now".
- **One mark per meaning:**
  - Downloaded tracks have one "available offline" glyph (an arrow in a circle) everywhere.
  - A downloading track shows a small ring. Waiting, paused and held-for-Wi-Fi tracks have no mark on the row (the header line says so).
  - The *On this device* mark shows only while the server is out of reach.
  - ✕ now means close or dismiss only: cancelling a download uses a stop glyph, removing a rule or a recent search a minus, ungrouping a broken link, deleting an alarm a bin.
- **One snackbar at a time:** every snackbar joins one list, and only the newest message shows. An older one shows again when the newer goes; screen readers still hear each. Snackbars wrap to two lines rather than cut off.
- **Empty states no longer lie:** while your music server is out of reach, a queue that couldn't be read says "The queue can’t be loaded until your music server is back", not "The queue is empty".
- **Naming:** "No player chosen. Choose a player with Play on." everywhere (it was four wordings). Now playing's empty state is "No player chosen" with a *Choose a player* button that opens Play on…. "Play all" is now "Play".
- **Tests:** `tests/declutter-wave35.test.ts` (header and row rules, the moved actions, one snackbar at a time, Continue on…, wide Now playing, honest empty states, the one offline mark) and the node layout checks (the player bar is hidden under Now playing at every width, and the new 44 px targets). Existing tests were updated to reach the actions in their new places.
## 2026-10-14: Android 1.14.0 (versionCode 28)

A test build of Wave 3.5 (declutter; see `docs/plans/wave3_5-declutter.md`): no "Playing on" strip (the player shows in the mini player), Play · Shuffle · Download · ⋮ headers, a tidier queue, a calmer Now playing (no tab bar, one rating control, one-line Up next, Play on · Lyrics · Queue), Settings as seven sections with pickers, one status line, one Library list, a simpler Play on… that now also holds what Rooms did, and downloads playing through the main player screens. Also the four layout fixes found by the review.

## 2026-10-14: Android declutter, phase 3 (Wave 3.5)

The merges of `docs/plans/wave3_5-declutter.md` for Android, built from the design section written there first.

- **Rooms folds into *Play on…*** (item 13): one player manager. Each player's row has, in its ⋮, *Move playback to …*, *Play/Pause …*, *Alarms* and *Switch … on/off*; the controlled player and any playing one have a volume slider. The sheet reads the players' status only while open. *All rooms* (the expanded view) is offered on large windows only. A Rooms card has three controls (play/pause, volume, ⋮): power moved into ⋮, the drag handle went (drag the cover instead; *Sync with…* and the TalkBack actions stay), and a switched-off room is one line. Now playing's ⋮ lost Alarms and Power (they are the player's row in *Play on…*). A move in progress is in the status slot on Rooms too.
- **Downloads use the main mini player and Now playing** (item 14): the downloads engine stays its own, and an adapter (`DownloadsPlayer`) shows its queue as a player called *Downloads*. The mini player says "artist · Downloads"; Now playing is the same screen as for any player (Lyrics and Info pages, the visualiser, the phone's volume), without the LMS-only rating and sleep timer, and with *Stop* in ⋮. **Downloads appear in the main Queue tab** (and the queue sheet), where a row plays from that track; the downloads queue can't be edited. Offline mode uses the same screens; the downloads-only Now playing and mini player are gone.
- **Docs screenshots** refreshed (`-PrecordDocScreenshots`).
- **Tests:** `Wave35Phase3Tests` (Play on rows do what Rooms' cards did, an idle player has no slider, a Rooms card has three controls with power in ⋮, the downloads adapter's player, status and queue, nothing loaded is no player, downloads on the main Now playing with Stop and without LMS controls, the mini player says Downloads, downloads in the Queue tab play from a row). Existing tests follow power, alarms, the visualiser and Stop to their new places.

## 2026-10-14: Android declutter, phase 2 (Wave 3.5)

The structural part of `docs/plans/wave3_5-declutter.md` for Android (item 11 is web-only). Nothing was removed: every setting and action has a page and a test.

- **Settings is an index** (item 8): seven rows, each with a one-line summary: Account ("alex · music.example.com"), Playback ("Starts on This phone"), Now playing ("Lyric line on · Visualiser: Bars"), Downloads ("1.18 GB · 34 tracks"), Listening history ("On"), This device (its name) and About (the version), with Advanced under About. Playback holds the default player, streaming quality on Wi-Fi and on mobile data, the sleep timer's fade-out and Sound; Now playing the lyric line, the visualiser and its style, and ratings; Downloads the Downloads page and *Download options* (was *Downloads and storage*); This device the phone's name, haptics, wallpaper colours and car mode; About the version, licences, the privacy policy and Advanced (read ahead, the replay cache).
- **One picker row** (`ChoiceRow`): every list of options is now one row showing the current choice, which opens a picker that closes as one is chosen (Cancel leaves it): the default player, streaming quality, sleep fade-out, visualiser style, ratings style, car mode, the replay cache's size, Download options (storage limit, formats on Wi-Fi and mobile data, recently added albums kept offline) and Sound (levelling, crossfeed, transitions, dither). Help is at most one sentence per control.
- **One status slot** (item 9): the line at the top shows one lasting state, by priority: moving playback, then out of reach, then busy, then reconnecting. The mini player and Now playing no longer have a transfer band of their own; a move that didn't work is a snackbar (Play on… still shows it with its detail while open).
- **One Library** (item 10): one list (Albums, Artists, Playlists, Favourites, Downloaded, Smart playlists), then Browse (Genres, Years, Compilations, Folders, Apps & Radio), each a page of its own. The chips and the second album browser are gone: Albums is the one browser (sort, grid or list, A–Z), with Recently added and Random as sorts (Random is one batch, with *Shuffle again*); Home's "See all" opens it in that order. Downloaded (the music) is in the Library; the Downloads page (the manager) is in Settings › Downloads.
- **Play on…** (item 12): a tap on a row controls that player; *Move playback to …* is in each row's ⋮ (This phone's too); grouping is one *Group players…* row with a dialog of players; a grouped player says who it plays with; the paragraph and the separate list of groups are gone.
- **Naming** (one word per concept): *your music server* (not "your server" or "LMS"), *Move playback to …*, *Downloaded* (the state) and *Downloads* (the page), *Smart playlists*, *No player chosen · Choose a player*, *Folders*, *Random albums*; unused strings removed.
- **Tests:** `Wave35Phase2Tests` (the index's seven rows and their summaries, no option inline, every setting on its page; the picker row; the slot's priority and only one line at a time; no transfer band on Now playing; the Library's one list; Play on's tap, ⋮ and Group players…), `Wave35RandomAlbumsTest` (one batch, shuffle again, a sort leaves random). Existing tests open the pickers and pages where their controls moved.

## 2026-10-14: Android declutter, phase 1 (Wave 3.5)

The quick wins of `docs/plans/wave3_5-declutter.md`, Android column. Each control that left a screen has a named new home, and every one is still reachable (tested).

- **No "Playing on" strip.** The mini player's second line says which other player it controls ("on Lounge", with a speaker); this phone's name isn't repeated. *Back to this phone* is the This phone row of *Play on…*, and *Bring to this phone* is its *Move playback here*. A move in progress still shows above the mini player.
- **Page headers** (album, playlist, artist, genre, year): Play · Shuffle · Download · ⋮ on one row. ⋮ opens the item's sheet, which has Play next, Add to queue, Download and Start radio (Start radio left the album page); where a header has no sheet, ⋮ offers Play next and Add to queue. Too narrow for Download, the header leaves it to the sheet rather than nest menus. "Play all" is "Play".
- **Queue:** the header is one line ("12 tracks on Lounge") with ⋮ (*Save as playlist*, *Clear upcoming*, *Clear queue*); rows keep one ⋮ and the drag handle; the gapless link marks are gone (Track info says *Gapless: plays straight on…*).
- **Now playing:** the tab bar is hidden (Now playing is a sheet over the app); one rating control by the title (the heart toggles; with stars, one star opens the full 1–5 rating; the full rating is also in ⋮ and on the heart's long press, with TalkBack's half steps kept); "Up next" is one line of text (still swiped away, or removed with TalkBack's action or Delete; hidden on short screens so the cover keeps its size: about 160 dp at 360×640, from 70); the bottom row is **Play on** (named after the player) · **Lyrics** · **Queue**. Sleep timer, Volume, the visualiser's switch and its style moved into ⋮; a running sleep timer shows in the top row (a tap changes it). The chip at the top went (Play on is in the bottom row); All rooms left ⋮ (it is in *Play on…*), and so did the signal path when the quality badge, which opens it, is shown.
- **Home:** station cards have no bin: a tap plays, ⋮ (or a long press) deletes, and TalkBack has *Delete station* on the card; a subtitle that only repeats the name is dropped. *Edit Home* is a quiet text button.
- **Continue on…** is a one-line banner: "Back online. Continue on Lounge?", ▾ to choose the player (it says what would move), Continue, and ✕ for Not now.
- **Marks:** one *Downloaded* mark (a filled ↓ in a circle) on track rows of the queue, album and playlist pages; the *On this device* mark only while your music server is out of reach. ✕ now only closes or dismisses: deleting an alarm shows a bin, removing a filter rule a ⊖, switching the visualiser off its own icon.
- **One trailing control per row:** download rows (pause/resume/retry and cancel), downloaded albums (play, remove), favourites (play next, add), folders, smart filters (play, shuffle, edit, delete), plugin menu rows and search results (album ▷, playlist ▷) now have one ⋮, with TalkBack actions where they had buttons.
- **Tests:** `Wave35Phase1Tests` (the mini player names the other player with no strip; Move playback here and Back to this phone in *Play on…*; header buttons on one row at 1.3× text; the queue header and its menu; at most one control per queue row and no gapless marks; Now playing's one rating control and three actions; the full rating from the star and from ⋮; the heart and its long press; every action taken off Now playing reachable from ⋮; Up next removable by TalkBack; gapless in Track info; All rooms only in *Play on…*; stations deleted from their menu and by TalkBack; the one-line Continue on banner; the offline mark on track and album rows; the cache mark only out of reach; favourites' Play next and Add from ⋮), `Wave35SmallPhoneTests` (the cover at least 160 dp at 360×640), `WholeScreenTest` (Now playing hides the tabs). Existing tests follow the actions to their new places.

## 2026-10-14: Android: four layout fixes

Found by a rendered review of every screen at phone, small-phone and tablet sizes:

- **Queue heading:** with *Save as playlist* offered, the heading was squeezed into one letter per line. It now stays on one line (ellipsised if it must), and the buttons that don't fit move into a ⋮ menu (*More queue actions*), with their labels. Clearing still asks first.
- **Connection line on Now playing:** the blurred cover behind Now playing spread past its edges and painted over *Can’t reach your server…* and the busy line. The background is now clipped to its bounds, and the line draws above the page.
- **Album and playlist headers:** the five header buttons wrapped, leaving ⋮ on a line of its own. The row no longer wraps: Play, Shuffle and ⋮ always show; Add to queue and Download move into the ⋮ when there isn't room (its last item, *More…*, opens the usual options).
- **Library chips:** a chosen chip off the edge (*Downloaded*, say) is now scrolled into view.
- **Tests:** `LayoutFixesTests` (the heading is one line, its buttons on one row, at 411, 360 and 240 dp with every action reachable, a menu action still asks first, the ⋮ stays on the Play row at 379, 328 and 300 dp, moved actions work from the menu, nothing moves on a wide screen, the connection line keeps its colour over the ambient cover, the cover doesn't paint outside its bounds, and the chosen chip is wholly on screen, also after choosing another). Each failed before the fix, bar the two wide-screen guards.

## 2026-10-14: Web: three fixes when the server is out of reach

Found by a rendered review of the web app with the server out of reach or busy.

- **No more bare "Can’t reach the music server" page after a reload.** The cause: SvelteKit's hash router reloads the whole page when the hash changes outside its router (a route typed into the address bar, a link from elsewhere, a script setting `location.hash`). Links inside the app never did this. A reload while the server was out of reach then couldn't sign in again, so the app fell back to offline mode's splash.
  - **A route typed into the address bar now opens in the app.** It steps back over the browser's own history entry and follows the route like a link, so Back and Forward work as usual. The page isn't reloaded, so this device's player doesn't stop.
  - **A page that does load while the server is away comes back signed in.** This applies when someone was signed in on this device, and only *provisionally*: there is no token, and no request is sent until the server confirms the sign-in. If the server refuses it, the user is signed out. The device keeps only who was signed in and as which device (never a token), checked field by field when read, and forgets it at sign-out. Offline mode's splash is now only for a device where nobody is signed in. The player, downloads and the "this device's player didn't start" fallback wait for the confirmation.
- **Now playing no longer says "Nothing to control yet" while the music server is busy.** This happened after a reload while LMS was busy. The selected player's last state is now kept on the device (per user, size-capped, checked when read, forgotten at sign-out). A page loaded while the server is busy or out of reach shows that state with the connection chip, its position held still, until the server answers.
- **Controls that need the server are greyed out consistently, and say why.** This covers:
  - Download and its format, Download now, Resume and Try again on album, playlist and artist pages;
  - the album and artist **More** (LMS's own menu);
  - the track menu's **Download** and **More…**;
  - Now playing's **More** and **This track**;
  - the "Playing on" strip's **Bring here** and **Back to this device**.

  Removing or pausing a download, and the other Now playing items that work without the server, stay available. Disabled controls, including Play and the transport, have a `title` and `aria-describedby` that point at the connection chip. They all go through one helper, `controls.needsServer()`, which is the same test as `app.canQueue`.
- **Tests:**
  - the address-bar routing (refusing anything that isn't an app route, plus a robustness sweep) and the listener's history handling;
  - the provisional sign-in: no empty token is ever sent, the server's refusal signs out, sign-out wins, and a malformed or tampered stored profile is no profile at all (with a robustness sweep);
  - the last-known state (per user and player, malformed and oversize input, a robustness sweep);
  - the real app state after a load while busy or out of reach;
  - the greyed controls and their descriptions on the album page, the track menu and the strip.

## 2026-10-14: Android 1.13.0 (versionCode 27)

A test build: with the server out of reach, controls that can't work are greyed out (Now playing, mini player, queue, notification, lock screen, Android Auto); on this phone's player, play and pause still work, and next, previous, a queue row and seeking work for tracks the phone has whole, playing on from its own copies; queue rows are marked *Downloaded* or *On this device*; the connection message is one line at the top that never covers the controls; back online, LMS picks up where the phone is if its queue hasn't changed. Includes the airplane-mode freeze fix from 1.12.1.

## 2026-10-14: Web: controls that work (or honestly don't) when the server is out of reach

The owner's report: with the server gone, skip and pause stayed tappable but failed, and the connection message got in the way. The web app now:

- **Knows whether the server is in reach:** one state for the whole app, *online*, *busy* or *unreachable*, from the events socket, requests that fail for want of a connection, the browser's online flag and this device's player socket. A blip under 3 s shows nothing.
- **Greys out what can't work:** on the player bar, both Now playing layouts, car mode, keyboard shortcuts, the queue peek and Queue page, and the lock screen (which now offers only the actions that work). A remote player: everything. This browser as the player: Play/Pause works here and keeps the position; Next, Previous and queue rows go to tracks on this device (downloads from storage, or the stream it holds, and back again); seeking works within a download. Shuffle, repeat, volume, queue edits and the sleep timer wait for the server, and so does every library play action (play, play next, add to queue, playlists, favourites, stations, apps and radio, search results, folders), disabled through one helper, `app.canQueue`. The Rooms page treats every room like a remote player. At a track's end it plays on to the next track here, or stops and says *Next track needs your server.*
- **Tells the server when it is back:** where this device got to (`playlist index`, `time`, pause), only if LMS still has the same track at that place; otherwise it follows LMS. Nothing done here, nothing told.
- **Marks the queue:** *Downloaded* or *On this device* on each row, with accessible labels.
- **Says it once, in its own room:** one compact chip at the top of the page, at every width (phones had nothing before): *Can’t reach your server. Playing from this device.*, *Can’t reach your server.*, or the busy wording. It replaces the sidebar's *Reconnecting…* and the busy banner. No error banner per failed command, and no note per command refused as busy.
- **Player worker:** a `local` message holds or lets go of the output (`static/player/local.js`); held, both outputs act as if paused while the core and what it tells LMS are untouched. Status reports `held`, `received` (the stream has all arrived) and the track handed to the output; `connected` says whether the session was fresh. With nothing done locally, behaviour is unchanged.
- **Media-element mode:** the page's own recovery (back from the background, the tap that unmutes, a synced start) never resumes an element held by a pause made here.
- **Tests:** reachability (threshold, each signal, busy, the first connection, network errors versus refusals), the gating table, Next/Previous/end-of-track decisions, marks, reconciliation (including hostile and mismatched track ids), the local player against a fake device and audio element, the components' disabled states, marks and lock-screen actions, the layout's chip, library play actions and Rooms disabled out of reach (with a sweep that fails if a new library play action skips `canQueue`), and a media-mode device test that recovery never resumes a held element (it fails without the guard). Node tests for the worker's hold (release conditions, deadline, message checking with hostile input, a random-sequence robustness test) and its wiring.

## 2026-10-14: Android: controls that work while the server is unreachable

With the server out of reach (airplane mode while playing on the phone, say), the controls stayed tappable and then failed, and the connection message sat over them. Now:

- **Greyed out when they can't work:** for another player, every control while the server can't be reached (or is busy). For this phone's player, only what it can do by itself works:
  - **Play and pause** act at once on the phone, keeping the position.
  - **Next, previous and a tap on a queue row** work when that entry is on the phone from start to end: downloaded, kept whole in the replay cache, or a next track already received whole. The phone then plays its own copies and carries on into the entries after it while they are on the phone too; at the first that isn't, it stops at the end with *Next track needs your server*. Previous past the first 3 s restarts the track if it is on the phone whole.
  - **Seek** works within a track the phone has whole.
  - Shuffle, repeat, queue edits, the sleep timer, LMS's volume and the library's play actions are greyed out. With the link up but the music server busy, play and pause hold the phone's output.
  - The notification, the lock screen and Android Auto offer only the actions that work.
- **Queue marks:** each row shows *Downloaded* (any player) or *On this device* (kept from streaming; while the phone is the player), with an icon and a TalkBack label. With the server away, rows that can't be played are dimmed.
- **One message, out of the way:** a single line at the top of the screen, which takes its own space instead of floating over the transport, the seek bar or the queue: *Can’t reach your server.*, *Can’t reach your server. Playing from this device.* while the phone plays on (a tap opens the downloads), *Reconnecting…*, or the calm busy note. Commands refused because the server is away no longer each raise a message.
- **Back online:** if you played, paused, skipped or sought on the phone meanwhile, LMS is told where the phone is (`playlist index`, `time`, `pause`), but only if its queue still has the same track at that position; otherwise LMS's state is followed. If you did nothing, everything is as before (the restart-where-left rule for long gaps is unchanged).
- **Rust layer** (`mobile`): `LocalPlayer.local_play`, `local_pause`, `local_next`, `local_previous`, `local_jump`, `local_seek`, `set_queue_snapshot` and `local_availability`, with `LocalPlayerStatus.offline_controls` saying what works. Skips play the phone's copies through the same audio path (decode, DSP, levelling, bit-perfect USB) and leave LMS's session; a new one starts when the server is back. Replay-cache entries now record whether the stream was the track's whole stream from its start, as LMS said when it began (`whole`); only such entries, checked against their hash before use, count as on the phone. `protocol::restart_commands` takes the phone's own point (`RestartPoint::local`, `paused`).
- **Tests:** Rust unit tests for the queue rules (next, previous, the end of a track, repeat all and one, runs of entries on the phone, arbitrary queues), the takeover (playing on, waiting at the end, a copy that can't be opened, pause, seek, wrap), the reconciliation (same queue, changed queue, malformed answers, nothing done) and whole-track cache entries; integration tests against the fake LMS and the real gateway for pause and play away, nothing done away, next to a download, a track's end carrying on into a download, and a changed queue on return. JVM and Robolectric tests for the gating table, the marks, the shown status, the routing, the session's commands and the line's placement.

## 2026-10-14: Android 1.12.1 (versionCode 26): no freeze in airplane mode

A fix for an "app not responding" freeze, seen when turning on airplane mode. UniFFI calls a Rust future's continuation while holding that future's lock, and on the main thread Kotlin resumed the waiting coroutine right there, inside the callback. When polling one future woke another (several requests failing at once as the network vanished), the resumed coroutine polled its own future again and waited for ever on the lock its own thread held. `anywhere-uniffi-bindgen` now patches the generated Kotlin so every Rust future is polled on `Dispatchers.Default`, which always dispatches a resumption rather than running it inline; this also keeps Rust work off the main thread. The build fails if UniFFI's generated glue ever changes shape. A regression test models UniFFI's locking and deadlocks against the unpatched driver.

## 2026-10-14: Android 1.12.0 (versionCode 25)

A test build: a busy music server (LMS frozen while its storage wakes) is shown as a calm "busy" note rather than "Can't reach your server" or offline mode, the player stays connected on the gateway's keepalives, polling backs off, and commands refused while busy say so instead of being replayed later.

## 2026-10-14: Android: a busy music server isn't "offline"

When LMS freezes for a while (a network music share waking its disks, say), the gateway now says so: `{"lms":"busy"}` / `{"lms":"ok"}` on the event stream, and an immediate 503 with `"busy": true` for commands (see `docs/PROTOCOL.md`). The phone used to take the silence for a lost server: its player socket gave up and resumed in a loop, requests piled up behind 30 s time-outs, and after about 25 s offline mode took over. Now, with a gateway that sends these:

- **Calm note, not an error:** a small *Your music server is busy. Waiting for it…* pill over the current screen. No *Can't reach your server* banner and no offline takeover, since the gateway is plainly reachable (Android Auto keeps browsing the server too). No network at all, or a sign-in that can't be checked, still counts as before.
- **Keeps playing:** the phone's player socket stays open on the gateway's 5 s WebSocket pings (answered by the WebSocket layer; no opt-in needed), so what is buffered plays on.
- **No pile-up:** status reads never overlap (one asked for meanwhile runs once after it). While busy with the event stream down, polling slows to every 15 s, or the gateway's Retry-After if longer. On `ok` the app refreshes at once.
- **Commands refused as busy:** a brief *Your music server is busy. Try again in a moment.*; nothing is queued and replayed later. Background reads refused as busy say nothing, and what is shown stays.
- **Rust layer** (`mobile`): `ServerEvent::LmsBusy { since_ms }` and `ServerEvent::LmsOk` from the event stream (anything malformed is ignored), and `ApiError::ServerBusy { retry_after_secs }` for a 503 whose body says `"busy": true` (Retry-After bounded to 1–300 s, 2 s if absent). A plain 503 or 429 maps as before. Plays sync-back treats busy as a rate limit.
- **Older gateways:** nothing is sent, so nothing changes.
- **Tests:** Rust unit tests for the messages (malformed and hostile ones ignored), the 503 mapping (with and without the busy body and Retry-After, garbage bodies, other statuses), and a fake socket proving Pings keep the player's socket alive and are answered while true silence still ends it. JVM tests for the busy tracker (including arbitrary sequences), the connection decision (busy never takes over, trouble counts afresh after it, no network still counts; the arbitrary-input test now varies busy too), the controller and Android Auto's measure, the player view model (poll back-off, Retry-After, `ok` refresh, no stacked reads, no replay, no message for background reads), and Robolectric tests for the pill and the error text.

## 2026-10-14: Web: a busy music server is shown calmly, and the app waits for it

LMS is a single process: when its music share stalls it can freeze for up to a minute. Before, the web app piled up status requests, showed a raw error banner, and its player abandoned and resumed its socket over and over. With a gateway listing the `lms_busy` feature (docs/PROTOCOL.md), the web app now:

- **Shows a calm status:** *Your music server is busy. Waiting for it…* (`role="status"`), from the events stream's `{"lms":"busy"}` or a `503` with `busy: true`. No error banner, no offline mode. Errors from other pages say *The music server is busy; try again in a moment.*
- **Doesn't stack requests:** status requests run one at a time (a poll while one is waiting is skipped; an event meanwhile asks for one more afterwards). While busy, the poll backs off from 2 s to 16 s, never sooner than `Retry-After`.
- **Recovers by itself:** on `{"lms":"ok"}` or the first command that succeeds, the status clears and the players and status are fetched at once. The events stream assumes LMS answers when it (re)subscribes.
- **Says when a command didn't happen:** a command refused as busy shows a short note. It is never queued and replayed later.
- **Keeps the player connected:** the player worker asks for text keepalives at sign-in (`"keepalive": true`), only when the gateway lists `lms_busy`, and learns the features before its first sign-in. A keepalive only refreshes when the socket was last heard: it is never taken for a greeting nor counted for resume. Text that is neither a valid greeting nor a keepalive is now ignored (before, any text was taken for a greeting).
- **Tests:** the busy 503 and `Retry-After` parsing (negatives: other 5xx, `busy` not `true`, HTTP dates, oversize values), `lms` events (unknown and mixed messages ignored), the backoff gate, the one-at-a-time runner, the app's busy, error, notice and recovery transitions against a fake gateway, and a regression test that status requests no longer overlap. For the worker: keepalive and greeting classification with fuzzed input, routing (keepalives never reach the greeting or the frame count), and the opt-in only with `lms_busy`.

## 2026-10-14: Riding out a blocked music server (gateway)

LMS is one process, and when its music folder sits on network storage that has to spin its disks up, opening a track can block it completely for a minute. Every app then saw "music server unavailable", requests piled up inside LMS, and players dropped and reconnected every 20 seconds. The gateway now notices when LMS has left a request unanswered for 4 seconds and tells the apps (`{"lms":"busy"}` / `{"lms":"ok"}` on `/v1/events`). While LMS is busy it refuses new `/v1/rpc` requests at once with 503 and `Retry-After` instead of queueing them, and keeps one cheap request waiting so it knows the moment LMS is back. Player sockets now get a ping every 5 seconds, and web players that ask get a text keepalive, so they stay connected through the stall. A pong no longer counts as the player speaking, so the gateway keeps reporting to LMS for a browser whose scripts are suspended. Refusing an unknown command no longer waits on a blocked LMS to re-read its app menus. `GET /v1/me` advertises `lms_busy`. Details in `docs/PROTOCOL.md` (*When LMS is busy*).

## 2026-10-14: Gateway logs slow LMS requests

When LMS is blocked (for example while a network music share waits for its disks to spin up), every request times out and the apps show "music server unavailable", but the log only said "LMS timed out". The gateway now logs any LMS request that takes 5 seconds or more, and every failed one, with the command (`playlist jump`, `albums`) and how long it took, so a stall can be traced to its cause. Labels are log-safe: the command word, plus the sub-command for groups such as `playlist`; never search terms, ids, paths or player addresses.

## 2026-10-14: Android 1.11.0 (versionCode 24)

A test build with the day's work: Opus about 80 times faster, changing your own password, saving the queue as a playlist, and "Continue on <player>" when back online.

## 2026-10-14: Performance measured

`docs/PERFORMANCE.md` now has measurements as well as budgets: the decoders' speed, the gateway's latency, throughput and memory (on the demo stack, and at rest on a live install), the web app's first load and WebAssembly size, and the Android release sizes against their budgets (within them). The decoder measurement found the slow Opus path fixed above. New `tools/demo/bench.py` repeats the gateway figures. Start-up, frames, memory and battery on a phone still need a device.

## 2026-10-14: Opus decodes about 80 times faster (Android, web player, gateway)

Found while measuring for `docs/PERFORMANCE.md`: the vendored Opus decoder (`third_party/opus-decoder`) computed its inverse MDCT's FFT as a direct O(n²) DFT, with a `sin_cos` per term. Opus therefore decoded at only about 2× real time on a desktop core: enough to play, but close to the limit on a phone, where it costs battery, and in a browser's WebAssembly. Opus is what the phone and the web player stream at *High* and *Data saver*, and what Opus downloads hold.

- **Fix:** a mixed-radix FFT plan (radix 4, 2, 3 and 5, generic otherwise), with its twiddles built once per MDCT size. See `third_party/opus-decoder/PATCHES.md`.
- **Speed (release, one desktop core, 160 kb/s stereo):** 2× → about 170× real time. FLAC, MP3 and Vorbis were already 850–2,000×.
- **Accuracy:** compared with libopus decoding the same files (160 and 64 kb/s), the output moved from 80.6 dB to about 100.5 dB signal-to-error. The old DFT's f32 π made its angles drift.
- **Tests:**
  - `plan_matches_exact_dft` (the plan against an exact f64 DFT at every CELT size and some awkward ones);
  - `opus_decodes_far_faster_than_real_time` in `core/tests/opus.rs` (asserts at least 20× in release builds; the old decoder fails it at 2.3×).
- **New:** `core/examples/decode_speed.rs` measures any codec's decoding speed and can dump the PCM.

## 2026-10-14: Web: no lone queue button on wide Now playing

In windows wide enough for the queue beside Now playing (1200 px and over), the button that pins or hides it sat on a line of its own under the Art, Lyrics and Info tabs. The Queue action in the row of actions now does that job there (pressed while the queue is shown); in narrower windows it still peeks at the queue.
## 2026-10-14: Continue on a player when back online (web and Android)

Back online with downloads queued on the device, the app now offers to hand the queue to an LMS player (SPEC §8.4). A non-blocking prompt, "Back online. Continue on <player>?", with a choice of player and *Not now*; it never happens by itself.

- **Players:** the ones this user may control, as *Play on…* lists them, plus this device's own player. The default is this device's player when it was in use, else the last one used.
- **Tracks:** each queued download is checked with `GET /v1/download/track/{id}/info`. Its id is used only when the gateway's stable key for it is the download's own. Tracks LMS no longer has, or whose id now names another file, are skipped, and the app says how many. Any other failure stops the hand-off before anything is sent.
- **Hand-off:** through the existing RPC policy, as Transfer's copy recipe: power on, shuffle off, the ids in order (batched to the gateway's id-list limit), the current track and position, paused if it was paused. Then the local player stops. If anything fails, it keeps playing and the prompt says why. A queue that changes while it is checked is checked again.
- **Sign-in ended while offline:** the queue is kept in memory and offered (paused) once the same user signs in again, never to another account. A deliberate sign-out forgets it.
- **Rust layer** (`mobile`): `resolve_offline_tracks` (the `/info` lookups, four at a time, each id once; malformed ids and keys are never sent) and `Players.load_track_list` (the queue in order, from a start index). No gateway changes: only endpoints and commands the policy already allows.
- **Tests:**
  - Rust: unit tests for the lookup (renumbered, gone, malformed, refused) and the list commands. Integration tests against the real gateway: lookups (with downloads turned off: refused as a whole; signed out), and a list load in order with a refused player and malformed input sending nothing.
  - Web (vitest): resolution with unresolvable keys, order, position and play state, batching, a failure leaving the downloads alone, a changed queue, another user's queue, and when the prompt appears.
  - Android: JVM tests for the hand-off use case, the offer and the controller, and a Robolectric test for the prompt.
## 2026-10-14: Change your own password, on the web and Android

Until now only an admin could set a password. A signed-in user can now change their own in *Settings › Account › Change password*.

- **Gateway:** `POST /v1/account/password` `{"current","new","code"?,"sign_out_others"?}` (docs/PROTOCOL.md).
  - It needs a bearer access token, `X-Anywhere-Client` and an allowed `Origin`, the current password, and a two-step code when that is on.
  - The new password must pass the usual policy and differ from the current one.
  - Wrong guesses count as failed sign-ins: the user's account-change lock, the address and the username's sign-in lock. Five lock them, and sign-in from that address too.
  - Wrong credentials are 403 rather than 401, so clients don't renew their token and resend the same guess.
  - Unused pairing codes are dropped. The caller stays signed in, and so do the user's other devices unless they ask for those to be signed out; the answer lists them.
  - Audited as `user.password_changed` (detail `self`) and `password.change_failed`, never with the password.
  - `GET /v1/me` lists `change_password` under `features`.
- **Web:** a *Change password* form in *Settings › Account* with a hidden username field and `current-password`/`new-password` autocomplete for password managers. A code field appears when two-step is on. Errors show inline and are announced: wrong password, the policy's reason, locked out (with the wait) and network failures. Afterwards it lists the devices still signed in.
- **Android:** the same form, opened from a *Change password* row in Settings' account section and shown only when the server offers it (the app now reads `/v1/me` features with the account). It asks for the two-step code when the account has two-step on.
- **Tests:**
  - gateway:
    - success, with the old password refused and the new one accepted;
    - the caller and other devices kept, or signed out on request;
    - pairing codes dropped;
    - wrong password, and a missing or wrong code;
    - a lockout reached here that blocks sign-in and two-step setup, and sign-in failures that lock this call;
    - weak and unchanged passwords costing no attempt;
    - missing, bogus, revoked and signed-out tokens;
    - cross-origin and header-less requests, and cookie-only requests;
    - malformed, unknown-field, duplicate-field, wrong-type and oversize bodies, and random bytes;
    - the audit entry without the password;
  - the web app's API call and form;
  - the mobile layer against the real gateway;
  - Android's repository mapping and a Compose test of the dialog.
## 2026-10-14: Web downloads: tracks, playlists and artists, Wi-Fi only, and a storage limit

The web app's offline downloads now cover what SPEC §8 asks for, as Android does.

- **What can be downloaded:** single tracks (the track menu on an album page: *Download* / *Remove download*), whole playlists (a *Download* bar on the playlist page) and all of an artist's albums (*Download all albums* on the artist page), as well as albums. Library tracks only, as before.
- **Groups, reference-counted:** each downloaded track lists what it was downloaded for (`album:<id>`, `playlist:<id>` or `track`). A playlist is its own group on the Downloads page, in its own order, with play, show tracks and remove. Removing a playlist, an album or a track takes only its own reference off; a track is deleted (audio, partial and cover) only when no download still needs it, and the page says how many stayed. Tracks downloaded before this change belong to their album. A playlist whose tracks are already here (for their albums) is "downloaded" at once, with nothing fetched.
- **Download on Wi-Fi only** (on by default, SPEC §8.2): while the browser reports mobile data (`navigator.connection.type`, Chromium on Android), the queue waits and says *Waiting for Wi-Fi*, with *Download now* for that album, playlist or artist, or for all on the Downloads page; downloads in progress stop and resume by Range on Wi-Fi. `effectiveType` (speed) never holds a download. Where the browser doesn't report the network (Safari, Firefox, desktop), nothing waits, and Settings says so.
- **Storage limit** (Settings › Downloads: no limit by default, or 1–100 GB, as on Android): a batch that would go past it is refused before anything is queued, saying how much it needs and how much is free under the limit (originals by LMS's file size, transcodes by duration). Each track is checked again by its real size as it starts, and a transcode of unknown size is stopped once it passes the limit; its bytes are thrown away. The browser's own free space (`navigator.storage.estimate`) is checked the same way. Settings and the Downloads page show the limit, the browser's quota and whether it keeps downloads.
- **Smaller fixes:** a queued download remembers its stable key, so cancelling it after a reload, before its details arrive, still deletes its stored bytes; clearing a failed download deletes its partial bytes.
- **Gateway:** unchanged. Album and playlist track lists ask LMS for two more tags (`f` file size, `x` remote), checked against the RPC allow-list by the existing fixture test.
- **Not done:** ratings made offline. The web app has no rating control in offline mode (ratings are only on Now playing for an LMS player), so there is nothing to queue yet.
- **Tests:** refs and size estimates (hostile values); request, groups and reference-counted removal (shared tracks, playlists removed mid-download, no orphaned audio, partials, covers or groups); the queue's new fields across a reload with hostile entries; Wi-Fi only (held, *Download now* per batch, a network change mid-download resuming by Range, the API absent or throwing); the storage limit (exactly full, one byte over, the queue counted, already-downloaded tracks free, nonsense limits, a real size larger than estimated, a transcode stopped mid-stream) and the browser's quota (refused, unknown or failing estimates never block); the album, playlist, Downloads and Settings pages.
## 2026-10-14: Save the queue as a playlist

Admins can save a player's queue as a saved playlist, on the web and on Android.

- **Gateway:** a typed endpoint, `POST /v1/players/{player}/queue/save` `{"name","replace"}` (docs/PROTOCOL.md "Save the queue as a playlist"). Raw `playlist save` stays refused by the RPC policy for everyone.
  - **Who:** admins only, as for the other playlist changes (listeners get 403 "changing playlists needs the admin role"), for a player they may control.
  - **The name:** trimmed, 1–100 characters; no control or invisible characters, no `/ \ : * ? " < > |`, no `..`, no leading or trailing dot, no Windows device names. The rules live in the core (`anywhere_core::names`), shared with the Android app's Rust layer; the web app and the Kotlin layer mirror them to say what's wrong as the name is typed.
  - **An existing name:** looked up ignoring case. Without `replace` the answer is 409 with the existing playlist; with it, that playlist is overwritten under its own spelling and keeps its id.
  - **Other refusals:** an empty queue is 409; unknown fields and bad types are 400, bodies over 1 KiB 413.
  - **The answer:** the new playlist's id and name. `GET /v1/me` lists `"save_queue": true` under `features`.
- **What's saved** is LMS's own `playlist save`: library tracks by file, radio and streaming entries by URL (never shown to clients).
- **Web:** "Save as playlist" on the Queue page and under the queue peek and side panel, for admins on a gateway that offers it. A dialog suggests the album (for a one-album queue) or "Queue" and the date, explains a bad name, and offers Replace for a taken one; then a message with Open.
- **Android:** the same in the queue header (Queue tab, queue peek and the queue beside Now playing), through `Players.save_queue` and `can_save_queue` in the Rust layer.
- **Tests:**
  - gateway: success, listener and stranger refused, every bad name, existing name with and without replace, empty queue, malformed, unknown-field and oversize bodies, raw `playlist save` still refused, and a fuzz run of the name check (never panics);
  - mobile Rust, against the real gateway: saved, taken, empty, listener refused, bad names and players never sent;
  - web: the name rules (with a fuzz run), the request and its answers, and the dialog;
  - Android: the name rules and repository checks (JVM), the view model (saved, 403, 409, empty, invalid) and Compose tests of the button and dialog.
## 2026-10-14: Web: a busy music server is shown calmly, and the app waits for it

LMS is a single process: when its music share stalls it can freeze for up to a minute. Before, the web app piled up status requests, showed a raw error banner, and its player abandoned and resumed its socket over and over. With a gateway listing the `lms_busy` feature (docs/PROTOCOL.md), the web app now:

- **Shows a calm status:** *Your music server is busy. Waiting for it…* (`role="status"`), from the events stream's `{"lms":"busy"}` or a `503` with `busy: true`. No error banner, no offline mode. Errors from other pages say *The music server is busy; try again in a moment.*
- **Doesn't stack requests:** status requests run one at a time (a poll while one is waiting is skipped; an event meanwhile asks for one more afterwards). While busy, the poll backs off from 2 s to 16 s, never sooner than `Retry-After`.
- **Recovers by itself:** on `{"lms":"ok"}` or the first command that succeeds, the status clears and the players and status are fetched at once. The events stream assumes LMS answers when it (re)subscribes.
- **Says when a command didn't happen:** a command refused as busy shows a short note. It is never queued and replayed later.
- **Keeps the player connected:** the player worker asks for text keepalives at sign-in (`"keepalive": true`), only when the gateway lists `lms_busy`, and learns the features before its first sign-in. A keepalive only refreshes when the socket was last heard: it is never taken for a greeting nor counted for resume. Text that is neither a valid greeting nor a keepalive is now ignored (before, any text was taken for a greeting).
- **Tests:** the busy 503 and `Retry-After` parsing (negatives: other 5xx, `busy` not `true`, HTTP dates, oversize values), `lms` events (unknown and mixed messages ignored), the backoff gate, the one-at-a-time runner, the app's busy, error, notice and recovery transitions against a fake gateway, and a regression test that status requests no longer overlap. For the worker: keepalive and greeting classification with fuzzed input, routing (keepalives never reach the greeting or the frame count), and the opt-in only with `lms_busy`.

## 2026-10-13: Rolling cache, Phase C: downloads stand in for Original FLAC streams

When the phone plays a track at Original quality that is downloaded as its original FLAC, it now fetches only the stream's first bytes and plays the rest from the download. The audio is bit-identical.

- **Matching** (`mobile/src/player/downloads.rs`): a FLAC stream (`strm` format `f`, as LMS sends it or with its artwork stripped by the gateway) at the Original level whose STREAMINFO equals a downloaded original's: the audio's MD5, total samples, sample rate, channels and bit depth. An MD5 of all zeros, or an unknown length, never matches. Opus, MP3 and non-FLAC downloads are never used.
- **Checked first; any doubt means the network:**
  - the offline store's own check: in the index as an Original FLAC, a plain file (never a symlink) of the recorded size;
  - the audio received so far equal to the download's bytes at the same place;
  - the whole file decoded with every frame intact to its MD5 and sample count (new `anywhere_core::decode::verify_flac`), once per version of the file.
- **Splice:** at the first audio frame, at the file's header length plus the bytes received less the stream's header length. Only the `?limit=` probe (96 KiB) is downloaded.
- **Counting:** `bytes_received` counts the whole stream; only what came from the network is acknowledged; such streams aren't kept in the replay cache.
- **Scope:** `LocalPlayerConfig.downloads_dir` is the offline library's folder, read only, under the signed-in user's scope. Removing downloads, signing out and revocation work as before.
- **Reporting:** `LocalPlayerStatus.from_downloads`, `SignalPathInfo.from_downloads`; Android shows "From downloads" in the signal path.
- **Tests:**
  - the header and STREAMINFO parser on truncated, oversize and bogus input, with a fuzz run (never panics);
  - the match rules (zero MD5, other samples, depth, rate, channels, another master);
  - folder, index and file validation (relative or `..` folders, symlinked index, tracks folder or file, another user's scope);
  - tampered, truncated and lying downloads falling back;
  - `verify_flac` on random and hostile files;
  - integration with the real gateway: a downloaded album fetches only the probe and plays bit-identical "from downloads"; a modified download falls back; a different master isn't matched;
  - a Compose test for the signal path's new stage.
- The test FLAC writer now sets the real MD5 in STREAMINFO.

## 2026-10-13: Android: offline mode no longer takes over after time in the background, and the app keeps your place

Before, after the app sat in the background without playing, switching back to it showed the offline screen for a few seconds and then came back at Home. Android cuts such an app's network, so the phone's player had dropped, and that counted as the server being away.

- **Background-aware decision** (`ConnectionJudge`, `OfflineModeController.ui`):
  - The phone's player failing counts only while the app is in use, or while it plays.
  - On return to the foreground there is a 10 s grace, and the player reconnects at once (`LocalPlayerRepository.reconnectNow`). A sign-in that couldn't be checked is retried.
  - No network at all is still noticed at once (after 3 s), in the background too.
  - The foreground comes from `ProcessLifecycleOwner`, whose start-up initialiser was already merged in. No initialiser is added: the merged manifest still has exactly EmojiCompat, ProcessLifecycle and ProfileInstaller.
- **The screen stays put:**
  - While reconnecting, a "Reconnecting…" pill shows over the current screen.
  - After 5 s of counted trouble, or with no network, a banner shows above the mini player: "Can’t reach your server · Downloads". It is one button that opens the downloads, and TalkBack announces it.
  - Offline mode takes the whole screen only with no network as the app opens, or when the server stays unreachable for 15 s while the app is in use.
  - Home's carousels and shelves, and the listening card, keep what they last loaded when a reload fails.
  - Android Auto's offline measure is unchanged.
- **Your place is kept:** `AppPlaces` holds the signed-in app's saved state (navigation back stack, each screen's state, scroll positions) in a `SaveableStateHolder` above the switch between the app, offline mode and "Signing in…". That state is saved with the activity.
  - When offline mode hands back, or after a sign-in check, the app returns to the screen you were on, scrolled where you left it.
  - After a configuration change, the same.
  - After the process is killed, the app reopens on the same screen with its back stack.
  - Another account or signing out starts at Home.
- **Tests:**
  - JVM tests for the decision: background ignored, the foreground grace, the network lost at once, the 15 s rule, playing in the background, the sign-in states, deadlines, and a seeded robustness run on arbitrary inputs and times.
  - Tests for the controller over virtual time, and for the player's reconnect kick.
  - Robolectric/Compose tests: on an album during a reconnect (pill, then banner); the banner's single labelled button, its 48 dp target and that it opens the downloads; offline mode taking over and handing back to the album; a sign-in check in between; another account; state restoration through "Signing in…"; the activity recreated; and the process killed with its state parcelled.

## 2026-10-13: Web player: `strm f` keeps the current track and drops the next

LMS sends `strm f` when the next track, already streamed, is deleted or moved in its queue, and keeps the current track playing. The core already kept the audible track. The web player only followed full flushes, so it could still play the audio of the dropped track that it had queued.

- **Web Audio:** the worklet drops the audio of the dropped track it has queued. The current track plays to its end.
- **Media element (`anywhere.playback.mode`):** the segments of the dropped track are removed from the element at the exact FLAC frame where the current track ends. The rest of that frame is queued again, so nothing of the current track is lost. If the dropped track was at another sample rate, the current track's rate is restored too.
- **WASM:** `WasmPlayer.take_trim()`, `trim()`, `trim_count()`, `pull_one()` and `pulled_track()`; `WasmSegmenter.push_track()` and `trim_after()`. The core's `mux::Segmenter` gains `pending_pcm()`, `truncate_pending()`, `decode_time()` and `set_decode_time()`.
- **Visualiser:** frames of the dropped audio are forgotten.
- **Tests:** regression tests for the WASM host (Rust and Node bindings), for the real core driving the real worklet, and for the media output's cut; robustness tests on arbitrary sequences of pushes, rate changes and trims, and on hostile trim messages.
## 2026-10-13: Rolling cache, Phase B: the replay cache

Phase B of `docs/plans/wave3-design.md` §2: tracks this phone streamed in full are kept, so a replay, a skip back or LMS sending a track again costs about 96 KiB instead of the whole track.

- **Gateway:**
  - **Live Opus is deterministic** (`-fflags +bitexact -flags:a +bitexact`): the same track at the same level is the same bytes every time.
  - **STAT `bytes_received` is clamped** to what the stream's relay sent, for every stream (it already was for live Opus). LMS is never told of bytes it didn't send.
  - **`/v1/stream/…?limit=m`** (1 to 16 MiB): at most `m` bytes, then a clean end; the rest follows with `?offset=`. Anything else is `400`.
- **Phone's player:**
  - **The replay cache** (`LocalPlayerConfig.stream_cache_bytes`; 0 is off): a stream that ended cleanly with every byte spooled is moved into `<cache_dir>/<scope>/stream-cache/`, named by its fingerprint, with a strictly checked, bounded index. Least recently used go first; entries in use are never evicted.
  - **Matching:** a stream's fingerprint is its level, format and first bytes (the header plus 64 KiB). On a hit the cached copy is checked against its length and SHA-256 before use, the download stops, and the rest plays from the phone, bit for bit. A copy that fails the check is deleted and the network used.
  - **Needs the download permission:** used only while `/v1/account` says downloads are allowed; emptied when they aren't, when the player is refused or revoked, and by `remove_stream_spools` (sign-out, revocation, another server or user).
  - **For the app:** `stream_cache_totals`, `clear_stream_cache`, `set_stream_cache_bytes`, and a `from_cache` flag in the status and the signal path.
- **Android:** Settings › Gaps in the signal › *Replay cache*: Off, 500 MB, 1 GB (default), 2 GB or 5 GB, what it holds, and *Clear*; disabled, with why, without the download permission. The signal path shows *From cache*.
- **Privacy:** the replay cache is a new copy of music on the phone; `docs/PRIVACY.md` lists it, and the streams being played.
## 2026-10-13: Fresh web screenshots from a fuller demo

- **Screenshots** (`docs/images/web/`): 19 shots of the current web app, phone and desktop, light and dark, all from the built-in demo. `tools/demo/shots.sh` takes them all in one go against a fresh demo; each waits for its screen's real content. The old top-level screenshots are gone (`docs/INSTALL.md` now uses the new ones).
- **Demo LMS** (`tools/demo/fake_lms.py`, invented throughout): five rooms with their own queues and the Lounge and Kitchen in sync; queue edits and Play next take effect; artist biographies and photos, album reviews and synced lyrics through the Music & Artist Information plugin's commands; roles, A–Z index and richer search; varied generated covers with their titles; and the plugin's settings page with invented users, devices and activity (`/demo/admin`).
- **Demo listening history:** `anywhere-gateway admin demo-seed-history <user>` seeds invented plays for the stats and Your year in sound. It exists only in debug builds, works only in a data directory holding the demo's marker file, only for a user with no history yet, and takes at most 20 000 bounded plays; `tools/demo/run.sh` uses it.
## 2026-10-13: Documentation rewritten

- **README:** a new introduction (who it is for, what makes it different), screenshot rows for the web and Android apps, an architecture diagram, features at a glance, quick start, security and privacy summaries, a platform support matrix, status and roadmap, and a documentation index.
- **New:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (components, trust boundary, request flows as sequence diagrams, the core and its hosts, the Android app, the plugin, storage, configuration and testing), [`docs/FEATURES.md`](docs/FEATURES.md) (every feature by area, with its platforms) and a docs index, [`docs/README.md`](docs/README.md).
- **INSTALL and DEPLOY:** `--live-transcode-jobs`, `--static-dir` and `--log-file` listed; ffmpeg's part in lower-data streams (and that *Transcoded downloads* off turns them off too); the *Track analysis* and *Allow http://* settings; first sign-in now describes playing on the device by default and the Android app; long-lived sessions (`--max-resume-grace`).

## 2026-10-13: Rolling cache, Phase A: Android, and restarting after a long gap

The Android part (A5) of `docs/plans/wave3-design.md` §2, and the owner's decision 3 in the engine. Phase A is now built; what remains is the manual testing on a real phone.

- **Restart after a gap longer than the grace** (engine): if the phone was playing when its link to LMS went, and the gap outlasted the resume grace, the new session restarts the track heard last (`playlist index`, then `time`) at the point last heard. It first checks with LMS that the same track is still at that queue position (or the phone had just played on into it), and that LMS isn't already playing or paused. Nothing is restarted if the user had paused or stopped, or held the output during the gap. The point follows playback through the gap, so a track the phone carried on into from its buffer is the one restarted.
- **Settings › Playback on this phone › Gaps in the signal:**
  - **Read ahead the next track** (on by default): the next track is fetched long before the current one ends.
  - **On mobile data too** (on by default; can be changed only while reading ahead is on).
  - Applied live: when either switch or the network changes (Wi-Fi or mobile data), the player is told at once.
- **The cushion:** while reading ahead, the signal path and the "Play on…" sheet show how much listening is on the phone ("45 s", "4 min", "1 h 5 min buffered ahead"; read out in full words by TalkBack), and nothing when there is none.
- **Network:** the player is told when the phone has no network (it stops trying to reconnect) and when it is back, from the app's existing network monitor.
- **Storage:** streams spool to an app-private `stream-cache` folder under `noBackupFilesDir`, beside downloads. Every spool is deleted at sign-out, on revocation and on a change of server or user, once the player has stopped.

## 2026-10-13: Rolling cache, Phase A: the phone's player engine

The mobile engine's part of A1, A3 and A6 in `docs/plans/wave3-design.md` §2, and `strm f`. The Android settings and cushion display come separately.

- **Disk spool:** with a cache folder (`LocalPlayerConfig.cache_dir`), each live stream is written to an app-private file as fast as the network allows, and fed to the decoder as it wants it. At most 256 MiB per stream and 512 MiB in all; beyond that, on a full disk or without a folder, streams are held in memory as before. Spools are deleted as soon as their track is fed, and on a flush, stop, sign-out, revocation, new session or start-up.
- **Read-ahead** (on by default; `LocalPlayer::set_read_ahead`): a track is reported ready once it is fully received, so LMS sends the next one long before the current one ends. A gap in the signal is then covered by the rest of the current track and the whole of the next.
- **Longer gaps:** the player asks for a 600 s resume grace where the gateway offers it, resumes until 5 s before the grant runs out (55 s with an older gateway), and retries stream downloads for as long as the grant lasts. Held reports drop heartbeats first, so track starts and ends reach LMS after a long gap.
- **No network** (`LocalPlayer::set_network_available`): no reconnect attempts until the network is back, then one at once.
- **`strm f`** (LMS dropped the already-received next track): the current track plays on to its end, and nothing of the dropped track is heard.
- **Sync groups:** after a stall, a grouped player skips what the others played meanwhile, or, with too little buffered, asks LMS to restart the group where it is.
- **For the app:** `LocalPlayerStatus` gains `cushion_ms` (listening held ahead), `resume_window_ms` and `read_ahead`; `remove_stream_spools(cache_dir)` clears spools when no player runs.

## 2026-10-13: Rolling cache, Phase A: the gateway's side

The gateway's part of A3 and A4 in `docs/plans/wave3-design.md` §2. The Android side comes separately.

- **Longer resume grace, on request:**
  - The player socket's sign-in takes an optional `"grace":S` (whole seconds, 0–3600). The gateway grants `min(S, --max-resume-grace)` and says what it granted in the greeting's new `"grace"` field.
  - **New flag `--max-resume-grace`:** 600 s by default, 60–3600. Players that don't ask still get 60 s; a resume without the field keeps the grace granted before.
  - A `grace` that isn't a whole number in range (a string, a fraction, a negative or larger number, `null`) is a policy violation: close 1008, nothing is opened, and a resumable session is ended.
  - `GET /v1/me` lists `"resume_grace": true` under `features`.
  - Unchanged by a longer grace: one LMS connection per device (a new session still ends a held one), revocation ends a held session at once, and the frame log keeps its 1 MiB bound (status polls while detached are answered, never queued).
- **Listening history judged by the player's clock:** an elapsed step counts if it is no bigger than the STAT's own jiffies step (plus 3 s), not the time between arrivals. So the reports a phone queues during a gap and sends together on resume count for what was really heard.
  - A clock that runs backwards (other than wrapping at 2^32) or jumps by more than 2 hours counts nothing.
  - **Security:** however the reports are timed, a session is never credited more than the wall time since its first report plus 10 s. Fuzz tests check this over random, racing, backwards and burst reports.
- **Heartbeats while detached** stop the elapsed time at the end of the track LMS says is playing, instead of running on past it.

## 2026-10-13: Your listening and Your year in sound (web)

The web app's side of `docs/plans/wave3-design.md` §1, built to the `GET /v1/me/stats` contract.

- **Your listening** (`#/stats`), opened from a card at the end of Home and from Settings › Listening history:
  - **Period picker:** this month, the last 30 days, this year, each past year the history reaches, and everything kept.
  - **Totals:** hours, plays, and different tracks, albums and artists.
  - **Most played:** artists, albums and tracks (top 10, "Show 20"), each linked to its page, with plays and hours. Albums and tracks are named by id from LMS, as for shelves. A track LMS no longer knows is still listed.
  - **New artists:** says "first played since your history began" for years and long periods.
  - **When you listen:** charts by hour, by weekday and (for a year or a long period) by month. Each chart has a summary sentence and a visually hidden table of every value.
  - **Streaks:** the longest run of days with music, and the busiest day.
  - **Empty states:** with history off, the page says so and links to the setting. With nothing played yet, it says that plays show within five minutes.
- **Home card:** "This month: 23 h · 412 plays", linking to the page. It is hidden when history is off, empty or unavailable, and while Home is being edited.
- **Your year in sound:**
  - **Offered:** from 1 December to 31 January on the Home card (for the year, or the year just ended). It is also there on demand from any year in the period picker.
  - **The story:** full-screen cards, one at a time: hours, top artist, top album, top track, top five artists, a new discovery, the busiest hour, the longest streak and a summary.
  - **Moving through it:** Previous and Next buttons, the arrow keys and sideways swipes. Nothing moves on its own.
  - **Closing and focus:** Escape, Close or Done closes it, and focus returns to what opened it. Focus starts on the first card and stays on a usable button. The card's entrance animation is off under reduced motion.
  - **Too little history:** it says how many more days with music are needed (a week of listening).
- **Share:** draws the current card onto a canvas on the device, as text only. Where the browser can share files, the PNG goes to the share sheet; otherwise it is saved as a download. Nothing is uploaded.
- **API:**
  - `MeApi.stats(period, tz)` checks the period and tz before sending: no request goes out for a bad value.
  - The device's offset is sent as minutes east of UTC. JavaScript's `getTimezoneOffset` counts west, so the sign is flipped and the value clamped to ±840.
  - The answer is parsed strictly:
    - With history off, everything shows as zero.
    - Bad or duplicate ids are dropped, and lists are capped at 20.
    - Counts are clamped, hour and weekday lists are padded or cut to 24 and 7, and dates must be real.
    - Names are plain text, with control and bidi characters removed and the length bounded.
- **Tests:** 48 new:
  - **API:** requests, refused periods and offsets (and a fuzz test of the period parser), hostile and malformed answers, and a fuzz test that any object parses safely.
  - **Period and tz maths:** fixed offsets, including +5:45 and ±14 h.
  - **The page's states:** loading, off, empty, error and data, and a slow old answer never replacing a newer one.
  - **Charts:** their text alternatives.
  - **Home card:** plus the Settings link.
  - **The story:** buttons, keys, swipes, Escape, focus and no autoplay, and its offer window.
  - **Share and export:** with a fake canvas and a fake `navigator.share` (shared, cancelled, refused, unsupported).

## 2026-10-13: Your listening and Your year in sound (Android)

The Android side of `docs/plans/wave3-design.md` §1, built to the `/v1/me/stats` contract. The Rust call (`Me.stats`) was added separately and wired into `RustStatsRepository` the same day.

- **Your listening:** opened from a card at the end of Home and from Settings › Listening history.
  - **Period picker:** this month, last 30 days, this year, each past year the history reaches, and everything kept.
  - **Totals:** hours, plays, tracks, albums and artists.
  - **Most played:** artists, albums and tracks, top 10 with "Show 20". Each shows its plays and time. Albums and tracks are loaded by id with their covers; a tap opens the artist or album (a track opens its album).
  - **New this period:** artists first played in it. Long periods say "First played since …", as the history only reaches back so far.
  - **When you listen:** hour, weekday and (for long periods) month charts, drawn on a Canvas. Each has a TalkBack summary with the key facts, such as the busiest hour and the next two, the most and fewest days with every day's plays, and the busiest month with every month.
  - **Streaks:** the longest streak and the busiest day.
  - **Other states:** history switched off (with a button to its setting); nothing played yet ("within 5 minutes of playing something"); an error with Try again; and a gateway without stats.
  - **Time zone:** the device's offset is sent as minutes east of UTC. It is read again for every load, so summer time is followed, and clamped to ±840.
- **Home card:** "This month: 23 h · 412 plays". It is hidden while loading, with history off, with nothing played, and on any error. From 1 December to 31 January it also offers "Your year in sound": in January it offers last year, even before this month's first play.
- **Your year in sound:**
  - **Cards:** full-screen cards for total hours, top artist, top album, top track, top five, a new discovery, the busiest hour, the longest streak and a summary. Cards with nothing to show are left out.
  - **Moving between cards:** swipe, use the Previous and Next buttons, or use TalkBack's actions on each card. Nothing moves on its own, and Back closes. The navigation bar and mini player give way while it is open.
  - **Opening it:** for any year with history, from the year's stats page.
- **Sharing a card:** the card on screen is drawn to a bitmap on the phone (a graphics layer) and saved as a PNG in the cache's `shared/` folder. It is offered to the system share sheet with a read-only grant for that one URI. Nothing is uploaded.
  - **The provider (`ShareImageProvider`):** a FileProvider that is not exported. It serves only `cache/shared/` and opens files read-only, whatever the caller asks for; it deletes nothing.
  - **Clean-up:** cards are deleted after an hour, on the next share or when the stats open.
- **Tests:**
  - **Data:** period strings, including clamping and ASCII digits in every locale; offsets east and west, half and quarter hours, ±840, summer time and clamping; the period choices against retention; the 1 December to 31 January window; and normalising answers, with a fuzz test that never throws and is idempotent.
  - **View models:** periods, retries, a late answer never replacing the chosen period, the offset sent, ids resolved, the unavailable state, the Home card's states and season, and the story's cards and states.
  - **Screens (Robolectric):** every state; the charts' descriptions; Show 20; the rows' links; the Home card and its place on Home; the Settings entry; the story's Previous/Next, TalkBack actions, no autoplay, Back and Close; and Share making an image the size of the card for a fake sharer.
  - **Sharing:** the file is in `cache/shared/`; the intent carries one read-only grant (no write, persistable or prefix grant); files elsewhere and `..` traversal are refused; every write mode is refused; old files are pruned; and the manifest and paths checks (not exported, grants only, one cache path).
  - **Accessibility audit:** every new screen and state, including each story card and a wide, short window.

## 2026-10-13: Now playing redesigned for phones (web)

Phones and narrow windows (up to 840 px wide) get a new Now playing in which nothing is squashed. Tablets and desktops (over 840 px) keep their layout.

- **One screen, no scrolling:** the page is exactly the visible height (100dvh) above the tab bar. Only the hero grows or shrinks; the cover stays square and shrinks to fit. At 360 × 640 the transport and every action stay fully on screen. Below 600 px tall, Up next makes room for the cover. A phone on its side (under 460 px tall) scrolls inside the column, never the page.
- **Top row:** Close (back to the previous page, or Home), a **Playing on …** chip that opens the player picker, and **⋮**. The ⋮ sheet holds this track's menu, Car mode, Alarms, All rooms, Signal path and Visualiser style.
- **Hero pager:** Cover, Lyrics and Track info replace the Art/Lyrics/Info segmented control. Move between them by swiping (touch, pen or mouse), with ← and → on the pager, or with the dots underneath (a tab list with the usual tab keys). The 1, 2 and 3 keys still work, and a swipe up over the cover still peeks at the queue. Lyrics and track info scroll inside their own page. A page's content is made when it is first shown, and hidden pages are inert. Under reduced motion the pager doesn't slide.
- **Under the hero:** the title on one line (ellipsis) with the rating beside it. Under that, artist · album and the quality badge, which opens the signal path; the signal path is never shown inline on phones. Then the waveform seek bar with times, the transport (shuffle Off → Tracks → Albums, previous, a large play/pause, next, repeat) and Up next on one compact line (✕ to remove, with Undo).
- **Four actions:** Sleep (shows the time left when set), Queue, Visualiser and Volume.
  - **Visualiser** is one button. A tap switches it on or off (going to the cover). A long press, a right-click or Shift+F10 opens the style chooser. A swipe across the visualiser changes the style (over Glow, or with it off, a swipe moves the pager). The style's name shows briefly and is read out. It is hidden for a remote player, and disabled with an explanation under reduced motion.
  - **Volume** opens a small popover with a vertical slider, labelled "Volume on <player>". It sets the volume of the player in use: another player's LMS volume, or this browser's own when it is the player, as the slider did. When the device's own buttons set the level (iPhone, iPad), the popover says so instead. Escape or a tap outside closes it, and focus returns to the button.
- **Removed on phones:** the separate visualiser style button, the full-width volume row, the car, alarms, lyrics and More buttons (now in ⋮ or the pager), and the lyric peek line.
- **Tests:**
  - Component tests for the layout's order (and what is gone), the top row, the ⋮ sheet (and focus returning to it), the pager (dots as tabs tied to their panels, arrow keys on tabs and pager, swipes by touch and mouse, a right-button drag ignored, a swipe over a visualiser versus Glow, content made on first view, no sliding under reduced motion), the Visualiser button (tap, the read-out name and its timeout, long press without a toggle, a short press, right-click, Shift+F10, Switch off, a remote player, reduced motion, a stale long press not swallowing a later tap) and the Volume popover (a remote player versus this browser, Escape, a tap outside, clamped values).
  - Unit tests for the pager gestures and keys, including junk input.
  - Node tests on the CSS: no page scroll, 100dvh, only the hero flexible, a square cover, a height budget at 360 × 640 that leaves at least 120 px of cover, and 44 px targets.
  - Updated the existing Now playing, lyrics, queue and volume tests for both layouts.

## 2026-10-13: Now playing redesigned (Android)

Built to the owner-approved mock-up. The old screen was cluttered, its play/pause could start partly off screen, and it had a segmented Art/Lyrics/Info row, two visualiser buttons and a whole row for the volume.

- **Nothing scrolls.** The transport is always fully on screen, on a 360 × 640 dp phone and up. The cover is square and shrinks to fit the height that is left.
- **Top row:** close (with its legible disc), "Playing on <player> ▾" in the middle (opens *Play on…*), and ⋮ at the end. ⋮ holds car mode, alarms, all rooms, the signal path, the visualiser's style, the track's options and the player's power.
- **The hero is a pager:** Cover, Lyrics and Track info, with small page dots under it.
  - The dots are tabs for TalkBack ("Cover", "Lyrics", "Track info"). Each page also has actions to show the others.
  - Lyrics and Track info scroll inside their page only.
  - Track info shows the album and year, artist, genre, format, length and the credits (links). The signal path is no longer shown there.
  - A swipe up on the cover shows the queue; a swipe down closes.
- **Under the hero:**
  - the title on one line, with the stars or heart at its end;
  - the artist and album, with the quality badge (a tap opens the signal path);
  - the waveform seek bar, showing elapsed and remaining time;
  - shuffle, previous, a 72 dp play/pause, next and repeat.
  - The lyric line under the title stays, on screens with room for it.
- **Up next** is one compact line with a ✕ to remove the track (Undo as before).
- **Four actions** along the bottom:
  - **Sleep:** shows the time left while a timer runs.
  - **Queue:** opens the queue sheet. On wide windows it shows or hides the queue panel instead.
  - **Visualiser:** one button.
    - A tap switches it on or off. It draws over the cover; Glow is the halo around the cover.
    - A long press chooses the style. So does a sideways swipe across it (the pager takes swipes when the visualiser is off or on Glow).
    - The style's name shows for a moment.
    - It is hidden for a remote player. It stays off, saying why, in battery saver or with animations removed.
  - **Volume:** a small vertical slider. For a remote player it sets that player's volume in LMS. When this phone is the player it sets Android's media volume, as the hardware keys do. The app's volume keys now always set the media volume.
- **Downloads** use the same layout, with Queue, Visualiser, Volume and Stop along the bottom.
- **Player level:** LMS's own level for this phone moves to Settings › Sound › *Player level*. It is normally left at 100%, and LMS can still change it. Now playing's volume no longer touches it while this phone plays.
- **Loudness follows the phone's volume:**
  - While this phone plays, loudness-compensated volume follows Android's media volume as well as the player level. The app reads the attenuation for the current output (`getStreamVolumeDb`, or the default media curve) and passes it to the engine through a new `set_loudness_reference_db`.
  - A fixed-volume output, or a Bluetooth device with absolute volume, falls back to the player level.
  - Volume and output changes are followed while playing. The signal path says "(following the phone's volume)".
- **Tests:**
  - Robolectric layout tests at 360 × 640 and 360 × 800 dp: play/pause fully in the window at full size, and nothing scrolling.
  - Screen tests for the pager, dots and actions, the top row, ⋮, the visualiser's tap, long press and swipe, both volume targets and a fixed volume, Up next and the seek times.
  - Accessibility audits of every state: cover, lyrics and info pages, visualiser on and off, Glow, the popover (remote and phone), ⋮, the style, signal path, sleep and queue sheets, downloads and the player level.
  - View-model tests for the player level.
  - Playback tests for the volume curve, the Bluetooth fallback and the follower.
  - Data tests for passing the attenuation to the engine.
  - Rust tests in the pipeline, plus engine tests for the streaming and offline players.

## 2026-10-13: Now playing tidy-up, four tabs and a new Glow (web)

- **Signal path on demand:** Now playing no longer shows the signal path all the time. The quality badge (e.g. "FLAC 16/44.1") still opens it in a sheet, and the Info layout keeps it folded behind a "Signal path" disclosure button. Nothing is lost.
- **The same four tabs as Android:** the phone tab bar is now **Home · Library · Search · Queue**. Now playing opens from the mini player (tap, or swipe up), as before; the "Playing" tab is gone. **Settings** moves to a gear button at the top right of Home (labelled "Settings"), and stays in the desktop sidebar.
  - Tabs light up for the pages under them: albums, artists, genres, playlists, favourites, apps and downloads under Library; stations, Settings and Rooms under Home; Now playing, car mode and alarms under none.
  - The desktop sidebar starts with the same four (Home, Library, Search, Queue), then Now playing, the library sections, Rooms, Downloads and Settings.
  - "Edit Home" now has a grip icon, so the only gear on Home is Settings. Targets stay 44 px, with labels.
- **A new Glow** (the same design and numbers as the Android app): a soft halo behind the cover in the artwork's own colours, instead of the background swelling with the level.
  - The colours: the cover drawn small (32 × 32) and grouped by k-means into up to three dominant, saturated colours, worked out once per album; grey or black-and-white covers use the theme's accent.
  - The halo: three overlapping radial lobes, turning once every 24 seconds while music plays and holding still when paused.
  - Beat-reactive, not level-following: a beat is bass energy (bands 0–4) above 1.3 × its mean over the last 1.5 s and rising, at most one every 150 ms. Each beat kicks a pulse (30 ms attack, 350 ms exponential decay) that takes the halo from 1.0 to 1.25 × its size and 0.35 to 0.85 opacity, and the cover from 1.00 to 1.02 × its size. Between beats the halo breathes a little with the smoothed bass.
  - Under "reduce motion" the halo is still, at rest, with no pulse or turning. It is hidden in the high-contrast theme. The scrim is unchanged, so text keeps its contrast. As before, only while this browser is the player and Glow is chosen; a remote player keeps the plain ambient background.
- **Tests:** unit and robustness tests for the beat detector (a steady tone gives no repeated beats, a periodic kick one per kick, the refractory period, silence and NaN or garbage frames give none), the pulse envelope's timing, the halo's ranges, the colour clustering on synthetic pixels (and a grey image falling back to the accent) and the per-album cache; tab matching (with look-alike routes); component tests for the Home gear, the four-tab bar and sidebar order, the signal path disclosure, and the halo (turning only while playing, kicked by a beat in the frames, still under reduced motion, absent for a remote player).

## 2026-10-13: Artist biographies, photos and album reviews (gateway, web, Android)

Following `docs/plans/ux-research.md` X5, part 1 (booklets are part 2, later).

- **Where they come from:** the *Music & Artist Information* plugin on the server, if it is installed. The gateway asks it with an id only: `musicartistinfo biography|artistphoto artist_id:<id>` or `musicartistinfo albumreview album_id:<id>`. The plugin may look these up online, under its owner's settings (docs/PRIVACY.md). Lyrion Anywhere calls no such service itself.
- **Gateway:** `GET /v1/info/artist/{id}` answers `{"biography", "photo", "credits"}` and `GET /v1/info/album/{id}` answers `{"review"}`, each `null` when missing (docs/PROTOCOL.md, SPEC D26).
  - Bearer token, the per-user allowance, and strict ids: a positive integer, otherwise 404; any query string is 400.
  - **Text is cleaned to plain text:**
    - HTML tags removed, with the content of scripts, styles and comments;
    - entities decoded once;
    - control and invisible formatting characters removed;
    - whitespace collapsed, keeping paragraphs;
    - lines naming a server path or a LAN address dropped;
    - at most 20,000 characters.
  - **Photos only through the image proxy:** the plugin's external image URL is never handed out. If it isn't on the home network, the gateway adds it to the image proxy's allow-list and answers with a `/v1/art/imageproxy/…` path. LMS fetches it only after the proxy's usual checks, DNS check included.
  - At most 4 lookups at once (429 with `Retry-After`), each with a time limit (503 on a timeout). Answers are cached for 24 hours, and for 1 hour when nothing was found. Failures and half answers are never cached.
- **Web:** the artist page shows the artist's photo (round, with its credit) and a Biography section. The album page shows a Review section.
  - Both load after the page and are simply left out when missing or failing.
  - Long text shows three lines and a "Read more" disclosure button (`aria-expanded`, `aria-controls`) under a proper heading.
  - Text is always rendered as text.
- **Android:** the same on the artist and album screens.
  - The photo replaces the header art, and falls back to the album art if it can't be loaded.
  - The text is expandable, with headings for TalkBack and a "Read more" button that says whether the text is collapsed or expanded.
  - Loaded lazily, so a slow lookup never holds the page up; hidden on failure.
- **Mobile core:** `MusicLibrary.artist_info(id)` and `album_info(id)`. Ids are checked on the device, and answers are checked again as untrusted input (the photo only as an image-proxy artwork key).
- **Tests:**
  - gateway: unit and fuzz-style tests for the text cleaning (HTML, entities, scripts and styles, control characters, huge and pathological input, never panicking) and the photo rules. Integration tests against the fake LMS cover success, missing data, local photo URLs, auth refusal, hostile ids (negative, zero, non-numeric, overflow, traversal, injection), that only the three plugin commands with an id reach LMS, that the photo is only a proxied path and is fetchable through it, caching, timeouts, the lookup limit and the allowance;
  - mobile: unit and robustness tests, and an integration test through the gateway harness;
  - web: the checks (hostile and random input), the API call, the disclosure, and both pages;
  - Android: view models, screens and an accessibility audit.

## 2026-10-13: Now playing tidy-up, four tabs and a new Glow (Android)

- **Now playing without a top bar:** the "← Now playing" bar (hard to read over the cover's colours, and 64 dp lost) is gone. A small close chevron ("Close Now playing" for TalkBack) sits at the top left over the art area, on a disc of the page colour that is as opaque as it must be for 4.5:1 over anything beneath, in light and dark themes and over any cover. System back and the swipe down still close it; the page now runs under the status bar. The downloads' Now playing has the same chevron. The action icons (queue, alarms, sleep, car, Play on…, more) stay where they were, on the ambient scrim that already keeps them legible.
- **Signal path on demand:** the always-visible signal path panel under the transport is gone. The quality badge (such as "FLAC 16/44.1") and a *Show the signal path* button in the Info layout open the full path as a sheet, with the EQ profile chip in it.
- **Four tabs, as on the web:** Home · Library · Search · Queue. Queue is a top-level destination showing the selected player's queue with its sections (Now playing, Up next, Then), swipes, drag to reorder and Clear, as under Now playing. Now playing stays the mini player above the tabs. Settings remains the gear at the top right of Home (its only entry point) and Downloads a Library chip. The tabs keep and restore their state as before; rails and drawers on tablets and foldables show the same four.
- **A new Glow:** instead of the blurred background swelling with the bass level, Glow puts a soft halo behind the cover, made from the cover's own colours (up to three dominant, saturated colours by a small k-means over the downsampled cover; the theme accent for a grey cover). It is drawn as three overlapping radial lobes, 120° apart, turning once every 24 s while playing and frozen when paused. It reacts to beats, not to the level: an onset detector on the bass bands (the mean of bands 0–4 against its running mean over the previous 1.5 s; an onset needs three earlier frames, energy above 0.02, 1.3× the mean and rising, 150 ms after the last) kicks a pulse (30 ms attack, then exponential decay with a 350 ms time constant) that takes the halo's radius from 1.0 to 1.25×, its opacity from 0.35 to 0.85 and the cover's scale from 1.00 to 1.02; between beats it breathes with 0.15 of the smoothed bass. The parameters match the web app's exactly. In battery saver and with animations removed the halo holds still with no pulse; under the system's high contrast there is none. The halo spreads sideways but fades out within a few dp above and below the cover, so it never sits under text; the ambient scrim for text is unchanged.
- **Tests:** the onset detector and envelope (a steady tone gives no repeated onsets, a periodic kick one onset per kick, the refractory period, silence and whispers, the first frames, garbage and NaN frames, time going backwards, the envelope's timing), the halo's ranges, the colour extraction (prominence order, grey, transparent and speck covers, saturation, determinism), the close button's contrast over black, white and random art for the default and cover-tinted schemes, screen tests for the chevron (also before a player is chosen, on wide windows and offline), the signal path sheet from the badge and the Info layout, the halo, the Queue tab and its sections, the four tabs and the Settings gear, and accessibility audits of the changed Now playing, the Queue tab, the tabs and Home's Settings gear.

## 2026-10-13: Loudness-compensated volume (Android)

- **Loudness-compensated volume** for this phone's player: a switch in Settings › Sound, next to per-output EQ, off by default and remembered on the device. At low volume the ear hears less bass and treble (the ISO 226 equal-loudness contours), so the engine adds a low shelf at 100 Hz and a high shelf at 10 kHz that grow as the player's volume is turned down:
  - 0 dB at full volume; 1 dB of bass and 0.4 dB of treble for every 4 dB of attenuation, up to +10 dB and +4 dB at −40 dB;
  - driven by the player's own volume (LMS's `audg` as the engine applies it), not Android's system volume;
  - the shelves follow volume changes smoothly (ramped every 32 frames), so there is no zipper noise;
  - they sit after the EQ and before the true-peak limiter, which always runs with them, so the boost can never clip;
  - on a bit-perfect USB DAC the DAC's volume rules: loudness is bypassed and the signal path says so. Otherwise the signal path shows a *Loudness* stage with the current boost.
- **Core:** a new pure DSP stage, `core::dsp::loudness`, in the chain between EQ and crossfeed.
- **Tests:** the curve (0 dB at full volume, monotonic, bounded, finite for NaN, infinite, negative and tiny volumes), the shelves' responses, smoothness across volume steps, no clipping at full scale through the chain and the pipeline (with and without an EQ boost), and the bypass on a bit-perfect route (unit tests, and end to end with a fake LMS and a fake DAC); on Android, the setting and its persistence, the engine settings it produces, the switch, the signal-path stages and an accessibility audit of the new control.

## 2026-10-13: Search on plugins (web, Android)

Following `docs/plans/wave2-design.md` §3.3 (U7), which had left plugin searches for later.

- **Search on…:** below the library results (shown once something is typed, even when the library has nothing), one row per app or radio service that offers a search: "Search <app> for “<text>”". A row opens that app's own results in the plugin menu browser, where play, add and the rest work as for any app item.
- **Found generically:** an app qualifies when its own top menu lists a search box whose action opens a menu with the typed text as its `search:` parameter, and nowhere else. Nothing is hard-coded per plugin.
  - The apps come from LMS's `apps` and `radios` lists, as the gateway filters them, so plugins reserved for admins stay hidden for listeners.
  - What was found is kept for the session and looked at again when the apps listed change (and on the web, when the user changes).
- **Only what LMS offered:** before searching, the app's top menu is read again. The text then goes only through that search box's action, as its `search:` value (control characters blanked, at most 200 characters, empty text refused). The gateway runs it because LMS offered it to this device and checks the text as it checks any search text, so paths and local addresses never reach LMS. No gateway change was needed.
- **Recent searches** stay on the device; choosing a row keeps the search as a recent one.
- **Accessibility:** rows are links on the web (keyboard reachable) and labelled rows with an "Open results" action for TalkBack on Android.
- **Tests:**
  - web: unit, negative and robustness tests for finding search boxes and building the command (a hostile query stays one `search:` value; lookalike actions are refused), the per-session directory, and component tests for the rows and the results page;
  - mobile: unit tests for the detection, and an integration test through the gateway harness (a hostile query sent only as the search value; paths and LAN addresses refused before LMS; empty text and bad players refused on the device; an admin-only app neither listed nor searchable for a listener);
  - Android: view model tests for the directory, Search and the results menu, screen tests and an accessibility audit.

## 2026-10-13: Car, system surfaces, haptics and smart filters (Wave 2 part 4, web)

Following `docs/plans/wave2-design.md` §5.1, §5.4, §5.5 and §5.6.

- **Car mode:** a *Car* button on Now playing opens a driving screen:
  - huge art, a large elapsed time, and giant previous, play/pause and next (hold play/pause to stop; the context-menu key or Shift+F10 also stops);
  - *Start radio* from the current track: AudioMuse-AI's mix where the server offers one, run as offered, with Don't Stop The Music switched on only when it is off; the app says so when there is no radio;
  - one level of Favourites (those that play at once) and Stations;
  - white on black in every theme, targets of at least 76 px, no visualiser, and an obvious *Exit* (or Escape);
  - the screen stays on while playing (Screen Wake Lock API where available), released on pause, when hidden and on leaving.
- **Haptics:** light ticks through `navigator.vibrate` where the browser has it (Android), off by default, with one *Haptics* switch in Settings. They tick on a skip by swipe, scrub detents (every tenth), volume steps, drag pick-up and drop, ratings and the sleep timer.
- **System surfaces:**
  - manifest shortcuts: *Resume*, *Radio* and *Search*;
  - Media Session: play, pause, stop, previous, next and seeking (to a point, back and forward), with the position for the system's progress bar;
  - keyboard shortcuts: Space, the arrow keys on Now playing and in car mode (seek, Shift for previous/next, volume), `/` for search, `L` for lyrics and `?` for a help sheet (also in Settings). They never act while typing, in a dialog or menu, or on a control that uses the key.
- **Smart filters:** a rule builder (genre, decade, artist in an optional role, compilation; order and a limit of up to 500), saved per user through `/v1/me/filters` exactly as the contract has it, and checked against its limits before anything is sent:
  - a *Filters* chip in Library, with play, shuffle, edit and delete (with a confirmation), and a page per filter with its albums;
  - a `filter` Home shelf with Play and Shuffle;
  - play and shuffle load the first 50 albums through the album play path.
- **Tests:** unit, negative and robustness tests for the filter contract checks and the API wrapper, Start radio's parsing and the shortcut rules; component tests for car mode (controls, long press, wake lock with a fake), haptics (switch, events, no API), shortcuts (each key, ignored while typing or in dialogs, the help sheet), the Media Session (a fake `navigator.mediaSession`) and the filters UI; node tests for the manifest shortcuts and the 44 px and 76 px targets.

## 2026-10-13: Wave 2 part 4 on Android: car, platform, smart filters and the offline visualiser

Following `docs/plans/wave2-design.md` §5, smart filters (§5.6) included.

- **Car mode (U8):**
  - a driving screen: huge art, giant previous / play-pause / next (a long press on play/pause stops), "Start radio" from the playing track, a large elapsed time, voice search, and one level of Favourites and Stations;
  - white and amber on black (every pair over 7:1), every target at least 76 dp, no visualiser, and the screen stays on while playing; Back leaves it;
  - opened from Now playing's car button, or by itself (Settings › This device): *Off*, *Any car audio* or *This device* (chosen from the paired devices);
  - it opens while the app is in front, or when it is next opened, with a matching car connected, and leaves when the car that opened it disconnects; left by hand, it stays closed until that car reconnects;
  - Android's `BLUETOOTH_CONNECT` permission is asked for only when an automatic option is first chosen; refused (or later withdrawn), the setting goes back to Off with a note;
  - Bluetooth is watched only while the app is in front, by a receiver registered at run time (not in the manifest, not exported). The car's name and address stay on the phone (`docs/PRIVACY.md`).
- **Android Auto artwork (U9):** album lists in the car carry their covers. A new content provider serves them, **not exported**: each cover's URI is granted to the verified Android Auto app alone, read-only, as its list is sent. It serves only covers already fetched through the gateway (or the downloads), kept under random 128-bit keys in the app's cache (at most 300, emptied each run and at sign-out, grants revoked). Paths, other files, write modes and callers without a grant are refused.
- **Per-output EQ (H2):**
  - an EQ profile can be bound to the built-in speaker, wired headphones, a USB DAC (by product name) or a Bluetooth device (by name), or "no EQ" there; other outputs use the main profile;
  - it switches by itself as the output changes (streaming and downloads alike), and an "EQ: <profile>" chip on the signal path says which is in use;
  - the editor shows the profile's curve (20 Hz–20 kHz) and has an A/B button that bypasses the EQ while held, or toggles it with TalkBack; the bypass never outlives the screen;
  - device names stay on the phone; no change to the Rust engine was needed.
- **Haptics (D6):** light ticks on a skip by swipe, every 10 % while scrubbing, volume steps (5 %), picking up and dropping in the queue and Rooms, setting a rating and setting a sleep timer. One *Haptics* switch (on by default) covers every tick the app makes, and Android's own touch-feedback setting still applies.
- **System surfaces (U11):**
  - a second, larger resizable widget: art, title and artist, a progress line and previous / play-pause / next (drawn only when what it shows changes, the progress in coarse steps);
  - app shortcuts *Start radio* and *Sleep 30 min*, through the same player controller as the app's buttons (they land on a private, non-exported activity);
  - *Wallpaper colours* as a setting (on by default, Android 12+).
- **Offline visualiser:** the visualiser on the downloads' Now playing, from the offline engine's own analysis, with the same styles, switch and limits as streaming.
- **Smart filters (X7, §5.6):** a *Filters* chip in Library (play, shuffle, edit, delete after asking), a rule builder (name, 1–20 genres, decade, artist with an optional role, compilations, order and up to 500 albums; checked by the Rust layer's own check before anything is sent, problems announced), each filter's albums paged from the gateway, and a Home `filter` shelf with Play and Shuffle (addable in edit mode; a deleted filter's shelf is left out). Play and shuffle load the first 50 albums through the album play path, shuffle setting track shuffle first, as on the web. Tested with fakes (repository refusals, play order and bounds, view models, screens, Home shelves) and in the accessibility audit in every state.
- **Tests:**
  - the Auto art provider's refusals (unknown key, traversal, wrong caller, write modes, safe mode), the per-URI read-only grant to Android Auto only, the opaque-key cache (bounds, images only, timeouts) and the manifest (not exported, grants only);
  - the Bluetooth side under Robolectric: only the two connection broadcasts with a device are read, nothing is registered or read without the permission, Android 11 and earlier need none, and the automatic opening and leaving end to end;
  - car mode's state machine and permission flow, its screen (long-press stop, TalkBack's Stop action, one-level lists, keep-screen-on, Back) and a check that every target is at least 76 dp;
  - per-output EQ: binding rules, storage (round trip and fuzzed stored strings), the engine getting the right profile as the output changes, and the bypass; the curve reader fuzzed;
  - haptics' steps and switch, the shortcuts' strict actions, the widget's progress and previous button;
  - the accessibility audit extended to car mode (each view, wide), its setting (refusal note, device picker), the Sound screen with per-output EQ, Now playing with the EQ chip and car button, the offline visualiser in each style and the device settings.

## 2026-10-13: Now playing and players (Wave 2 part 3, web)

Following `docs/plans/wave2-design.md` §4.

- **Visualiser:**
  - the player worker feeds the core's analyser (`core::visual`, now exposed through WASM) with the PCM it sends out, in both playback modes;
  - each frame reaches the page only once that audio is being heard, at most 30 a second;
  - styles: Bars, Scope, VU meters (needle ballistics of about 300 ms, with peak hold) and Glow (the blurred background and a halo swelling with the bass);
  - change the style with a swipe across it or its style button;
  - off by default (Now playing's visualiser button, or Settings), and remembered on the device;
  - shown only while this browser is the player, Now playing is on screen and the art is visible; with reduced motion it stays off;
  - the worker analyses nothing unless it is shown;
  - decorative: hidden from screen readers, with labelled on/off and style controls.
- **Now playing layouts:**
  - *Art*, *Lyrics* (synced lyrics, large) and *Info* (quality badge and signal path, credits as links, rating, Up next);
  - chosen with a segmented control, the keys 1, 2 and 3, or a sideways swipe on the art area, and remembered on the device;
  - wide windows (over 840 px): the art stays on the left with lyrics or info beside it;
  - windows with room for it (1200 px and over, as the web's sidebar takes space): the queue pinned on the right, which can be hidden.
- **Rooms:** a page in the menu, and "All rooms" in the player picker. Each permitted player (up to 20) has a card with:
  - art, title and artist;
  - play/pause, a volume slider and power;
  - sleep remaining and the next alarm.

  Each card is one group, with its controls labelled with the room's name ("Kitchen volume"). Tap a card to control it. Drag it onto another to sync; the grip works by pointer or keyboard, and "Sync with…" is in the card's menu. "Play on its own" unsyncs; "Move playback here" transfers, also from a long press. Statuses are read every 5 s and at once on a room's events, only while the page is open. Alarms are read on opening and after a change.
- **Tests:**
  - WASM binding tests for the frame layout and bad input;
  - node tests for the worker's tap (timing against the output, flushes, bounds, odd input);
  - unit and fuzz tests for frames, VU ballistics, drawing and the remembered choices;
  - component tests for the layouts (keys and swipe alternatives), the visualiser's controls (reduced motion, remote player) and Rooms (labelled controls, sync by menu, keyboard and drag, polling that stops when closed);
  - 44 px target checks for the new controls.

## 2026-10-13: Wave 2 part 3 on Android: Now playing and players

Following `docs/plans/wave2-design.md` §4.

- **Visualiser (D4):** Bars, Scope, Glow (the blurred-art background swells with the bass) and VU (two needle meters, about 300 ms rise and fall, with peak hold).
  - **Where:** in the art's place on Now playing, only while this phone is the player. A remote player keeps the ambient background.
  - **Analysis:** the engine taps the decoded audio into the core's analyser only while a visualiser is on screen. Nothing exists while it is off. Frames are released as their audio is heard, through a single latest-frame slot the audio thread never waits on, at most 30 a second.
  - **Choosing:** off by default. Switch it on with its button on Now playing or in Settings; change the style with its button or a swipe across it. Both are remembered on the device.
  - **When it stops:** in battery saver and when animations are removed (followed as they change), and whenever Now playing isn't showing.
  - **Accessibility:** decorative and hidden from TalkBack. Its switch says why it is off.
- **Now playing layouts (D2):** Art, Lyrics (the synced lyrics large) and Info (quality badge and signal path, credits as links, the rating, an Up next card).
  - **Choosing:** layout buttons, or a sideways swipe across the art area, which now changes the layout. The track's next and previous stay on the buttons, the mini player's swipes and TalkBack actions. TalkBack also gets the other layouts as actions. The choice is remembered on the device.
  - **Wide windows (over 840 dp):** the art (or visualiser) and controls on the left, lyrics or info on the right, and the queue as a side panel that can be hidden.
- **Rooms (U10):** every room at a glance, from the player chip on Now playing ("All rooms") and from "Play on…".
  - **Each card:** art, title and artist, play/pause, volume, power, sleep remaining and the next alarm.
  - **Actions:** tap a card to control that room; long-press to move playback there (Transfer). Drag a card by its handle onto another to sync them. "Sync with…" in the card's menu and a TalkBack action per room do the same. Leave the group from the menu.
  - **Kept fresh:** statuses are read every 5 s (at once when an event arrives) for at most 20 players, only while the screen shows. Alarms are read once, then again when this app changes them.
  - **Layout:** a list on phones, a grid on wider windows. Each card is one group for TalkBack, its controls named with the room ("Kitchen volume").
  - **Mobile layer:** `Players::statuses` reads up to 20 statuses together. Every id is checked before anything is sent.
- **Tests:**
  - the analysis tap: timing, keeping step with what is heard, flushes, rate and track changes, hostile blocks and its cost; the offline player end to end;
  - Rooms reads and controls through the real gateway, with refusals;
  - the frame parser, including random bytes;
  - VU ballistics, the Rooms view model's polling, sync and alarms;
  - UI tests for every new control, and the accessibility audit over each new screen and state.

## 2026-10-13: Library and queue (Wave 2 part 2, web)

- **Credits:** a track's "More…" lists every role LMS reports (album artist, artist, track artist, composer, conductor, band). Each name links to that contributor's albums in that role ("Name · Composer"). Artist pages show "Appears as" chips with album counts.
- **Library:**
  - chips for Albums, Artists, Playlists and Favourites;
  - album orders: Recently added, A–Z, Artist and Year;
  - a grid or list view;
  - an A–Z fast scroller from LMS's index, which jumps without loading the list up to the letter and works from the keyboard and screen readers.

  The choice is remembered on the device. *Recently played* waits for the gateway to allow that sort.
- **Search:**
  - recent searches, kept on the device only;
  - results grouped under a Top result, with type chips that page through one type;
  - play, play next or add from any result.
- **Queue:**
  - sections: Now playing, Up next (this device's Play next, best effort) and Then;
  - swipe right to play next and left to remove, with menu items and keys (N, Delete) as alternatives;
  - Clear upcoming, from the end with each removal checked, or Clear all for a long queue;
  - a queue peek on Now playing;
  - browse lists swipe too: right plays next, left adds.
- **Shuffle:** cycles Off, Tracks and Albums, named for its state. A long press offers the three. Gapless markers link consecutive tracks of one album in the queue.
- **Tests:** unit and robustness tests for every new parser (credits, the A–Z index, search results, recent searches, the Up next tracker), and component tests for each new control and its keyboard and screen-reader use.

## 2026-10-13: Wave 2 part 2 on Android: library and queue

Following `docs/plans/wave2-design.md` §3.

- **Credits (X4):** track information (the "More…" sheet) lists everyone LMS credits by role. Each name opens that person's albums in that role ("Name · Composer"). Artist pages show "Appears as" chips with album counts.
- **Library (U6):** type chips (Albums, Artists, Playlists, Favourites, Downloaded); tap the chosen one again for the overview.
  - **Albums:** sorted Recently added, A–Z, Artist or Year, as a grid or a list.
  - **A–Z scroller:** for albums in A–Z order and for artists. Built from LMS's index, so a letter reads only its own page. It is adjustable with TalkBack and works from the keyboard.
  - **Remembered:** the chip, order and view are kept on the device.
  - **Recently played:** not offered. LMS's sort is server-wide, so the gateway refuses it.
- **Search (U7):**
  - **Recents:** up to 20 recent searches, kept on the device only, each removable, with Clear all.
  - **Results:** grouped, with a Top result. Type chips page through one type. Tracks play from their rows; albums and playlists have a play button.
- **Queue (U4):**
  - **Sections:** Now playing, Up next (this device's Play next, tracked best effort) and Then. Shown flat when the queue changes elsewhere.
  - **Swipes:** queue rows swipe right to play next and left to remove (with Undo). Browse rows swipe right to play next and left to add to the queue. Each swipe is also a menu item and a TalkBack action.
  - **Clear upcoming:** asks first, deletes from the end and checks each entry. A long queue gets Clear all instead.
  - **Queue peek:** a swipe up on the artwork, or the Queue button, opens the queue in a sheet.
- **Shuffle and gapless (H4):** the shuffle button cycles Off → Tracks → Albums, says which, and offers all three on a long press. Consecutive tracks of an album get a labelled link mark.

## 2026-10-13: Review fixes (gateway, native apps, web)

- **Gateway:**
  - **Connection caps:** WebSockets (events and the player socket) now count against the connection caps for as long as they are open. Before, an upgraded connection left the caps, even one that never authenticated.
  - **Renewals:** limited to 30 a minute per device (429, the token kept). A device keeps at most 8 live access tokens, and old refresh-token rows are pruned.
  - **Skipped tracks:** a stream superseded by the next track is cancelled at once, with its LMS connection and encoder.
  - **Lyrics:** a hostile lyrics tag can no longer stall a worker. Parsing is linear; before, 256 KiB of `<` took about 11 s.
  - **Information hiding:** a refused `strm` is passed on without its address and request. Events blank the ids of players the user may not see.
  - **Ratings:** clearing your own rating no longer clears LMS's shared rating.
  - **Revocation:** a device revoked while connecting is caught. Deleting a user disables it first.
  - **Housekeeping:** migrations are atomic. Station and menu replays refresh LMS's app list at most once a minute.
- **Native apps (mobile layer, Android):**
  - **Sign-in kept:** only the gateway's own 401 ends a sign-in. A proxy's or CDN's error page during renewal no longer signs the app out and deletes every download.
  - **Saved sign-in:** a refresh token that can't be saved no longer leaves a stale one behind, which would have got the device revoked on the next start.
  - **Downloads index:** an unreadable index no longer deletes the downloaded files. A copy is kept aside and read back when it can be.
  - **Reconnecting:** repeated bad frames back off instead of reconnecting in a tight loop.
  - **Paused downloads:** a skipped paused entry no longer pauses another.
  - **Background (Android):** stations and "end of …" sleep timers keep going with the app in the background.
- **Web:**
  - **Navigation:** in-app navigation no longer reloads the page, which used to stop this browser's audio.
  - **Signing out:** ratings, lyrics and search are reset when the user changes.
  - **Stale answers:** late answers no longer overwrite newer ones in search, ratings and alarms.
  - **Queue edits:** checked against the entry before removing or moving it.
  - **Lyrics:** retried after a failure.
  - **Artwork:** images still on screen keep their artwork.
  - **Accessibility and layout:**
    - every page has its own title;
    - the track "More options" menu works from the keyboard;
    - detail pages have Back buttons;
    - "Getting this device ready" shows progress and a way out;
    - Now playing fits a 360×640 phone;
    - touch targets are 44 px;
    - search keeps its state;
    - Settings puts diagnostics last;
    - confirmations are in-app dialogs.
- **Tests:** a regression test for each fix, and the hostile and timing cases above.

## 2026-10-13: Plugin review fixes (security, supervision, settings page)

- **Security:**
  - **Settings form:** saving needs the settings page's own token. LMS checks its anti-CSRF token only when its *CSRF protection* setting is raised (it is off by default), so any web page could post the form, for example to point the gateway at a different binary. Without the token nothing is saved and the page says to reload.
  - **Admin secret:** sent only to the gateway the plugin started, on the port it started it with. `admin_port` is an ordinary pref that LMS's `pref` command can change; a later change no longer redirects the secret. On Linux the port must also be held by the gateway process itself.
  - **Restart gateway** no longer starts a gateway that is turned off.
  - **http:// Public URL:** saved only with *Allow http://* ticked as well; the page warns as it is typed.
- **Supervision:**
  - **Orphaned gateways:** the gateway's pid is kept in the data directory. A gateway left running by an LMS that crashed is stopped before a new one starts, but only once `/proc/<pid>/cmdline` shows it really is our gateway serving our data directory.
  - **Stopping never blocks LMS:** SIGTERM, then SIGKILL after 5 s, from a timer. At LMS shutdown the plugin waits at most 3 s.
  - **Backoff:** restarts double from 5 s up to 5 minutes; a gateway that can't start at all backs off too (it was retried every 5 s after a long good run).
  - **Log:** rotated at 5 MiB, keeping one old file.
  - **Errors:** the gateway's own reason (e.g. "username already exists") is shown instead of a bare status code.
- **Settings page:**
  - **Add a device** says the codes are for the Android app; in a browser people sign in with their password.
  - **Enter** in the page's fields runs that field's action instead of submitting LMS's form (which lost what was typed).
  - **Gateway stopped:** the controls that need it are turned off, and the page says why once.
  - **Messages** appear beside the section just used, in a live region.
  - **Accessibility:** focus returns to the same control (or the section heading) after a list is rebuilt; repeated buttons have names saying what they act on ("Delete: alice"); choosing a player to give someone needs an explicit **Add**.
  - **Dark skins:** errors and diagnostics no longer rely on fixed text colours.
  - **Gateway path** is under *Advanced (for developers)*; audit events are shown in words; **Restart gateway** asks for confirmation and warns that it disconnects remote listeners.
- **Tests:** settings saves without, with a wrong or a malformed token (nothing changes); the secret never sent after a pref change, to a stopped or exited gateway, or to a port another program holds; restart while turned off; orphans stopped only when verifiably ours (other programs, other data directories, junk pid files left alone); non-blocking stop, SIGKILL after the grace, bounded shutdown; backoff after failed launches; log rotation; gateway error messages; passwords never logged; and, on the page, Enter handling, the stopped state, restart confirmation, explicit grants, unique button names, focus return and audit wording.

## 2026-10-12: Wave 2 (part 1): alarms and ratings (gateway, web, Android)

- **Alarms:** LMS's own alarms for any player you may control, from an Alarms page (web) or screen (Android, from Now playing).
  - **Settings per alarm:** time, days, once or weekly, what to play (the current queue, or the server's favourites, playlists, random mixes and sounds), shuffle and volume.
  - **Managing:** an on/off switch for each alarm and one for all of a player's alarms. Deleting asks first, in the page.
  - **Where they run:** alarms ring on hardware players too, with no app open.
  - **Gateway:** typed endpoints with a strict grammar. Sounds are opaque ids resolved to LMS's own URLs on the server; no URL is ever taken from a client or sent to one. Other players are refused.
- **Ratings:** stars (half steps from the keyboard or TalkBack) or a heart, as each user chooses (Settings; the same on every device), on Now playing.
  - **Storage:** kept per user on the gateway by stable track key, so a rating survives a rescan.
  - **Blending:** your own rating counts three times LMS's shared rating (set by Ratings Light or other apps).
  - **Ratings Light:** where installed, it receives your rating too.
  - **Offline (Android):** a downloaded track rated offline is queued and sent with the plays.
  - **Settings:** "Clear my ratings".
- **Privacy:** ratings are listed in `docs/PRIVACY.md`.
- **Tests:**
  - **gateway:** the alarm and rating grammars; sounds without URLs; other players and hostile bodies refused before LMS; raw RPC still refused; storage, blending, migration 11 and offline batches checked against stable keys;
  - **mobile:** the checks, the offline queue, and both features through the real gateway;
  - **web:** the checks, the values sent, the rating control and the Alarms page;
  - **Android:** the cache, the offline fallback, the view model, the rating control (including the accessibility audit) and the Alarms screen.

## 2026-10-12: Streaming quality on mobile data (gateway, web, Android)

- **Lower-data streams:** a player can ask for **High** (Opus 160 kbps, about 70 MB an hour) or **Data saver** (Opus 96 kbps, about 45 MB an hour) instead of **Original** (CD-quality FLAC is about 400 MB an hour). The gateway re-encodes FLAC streams on the way through with the `ffmpeg` it already uses for downloads; other formats are sent as they are.
- **Android:** Settings › *Streaming quality* has a level for Wi-Fi (Original by default) and one for mobile data (High by default), following the network as it changes. The signal path says when the stream is re-encoded.
- **Web:** Settings › *Streaming quality in this browser*: Automatic (High on a mobile connection, Data saver when the browser is asked to save data, where the browser says; Safari doesn't), or a fixed level. The browser player now decodes Opus.
- **Gateway:**
  - the level comes at sign-in or in a control message, and the `strm` is rewritten to Ogg Opus;
  - resume, acknowledgements and back-pressure work on the Opus stream, and LMS hears its own byte counts;
  - `ffmpeg` is hardened as for downloads and killed with its stream;
  - at most `--live-transcode-jobs` encodes at once (default 4), the original beyond that;
  - `/v1/me` lists `stream_quality`.
- **Fixed:** the Opus decoder we use (`opus-decoder` 0.1.1) overflowed on some transients: a panic in debug builds, and wrongly decoded frames in release builds, including Android's offline Opus downloads. A patched copy is kept in `third_party/opus-decoder` until a fixed release.
- **Tests:**
  - **gateway:** the level grammar and hostile control messages; the `strm` rewrite (FLAC only); resume, LMS's byte counts, a failing or missing encoder, the slot limit, and the encoder being killed when its stream is abandoned (with a stand-in encoder); and the real `ffmpeg`, now required in CI, with its output decoded by the players' core, also after a seek;
  - **core:** a regression fixture for the decoder fix;
  - **mobile:** a phone plays Opus at Data saver, then FLAC after switching back mid-session, through the real gateway;
  - **web:** the automatic choice, the preference, the worker's messages, and Opus decoding in WebAssembly;
  - **Android:** the level following the network and the setting, the settings dialog and the signal path.

## 2026-10-12: Wave 1: lyrics (web and Android)

- **Lyrics on Now playing**, from the file's tags (plain or synced LRC) or, when it is installed on the server, the **Music & Artist Information** plugin. Lyrion Anywhere itself never contacts a lyrics service; that plugin may, depending on its own settings.
- **Full-screen view:** synced lyrics highlight the playing line and keep it in view (unless you have just scrolled), and tapping a line seeks there; plain lyrics scroll. A *Lyrics* button shows whenever the track has any.
- **Peek:** the current line, small, under the track details, when a track has synced lyrics. It is on by default, can be switched off in Settings, and tapping it opens the full view.
- **Gateway:** `GET /v1/lyrics/track/{id}` takes only the track id and answers with lines already parsed by the Rust core (`core::lyrics`):
  - times checked, `[offset:]` applied, enhanced-LRC word times and metadata removed;
  - at most 4,000 lines of 500 characters from 256 KiB;
  - lines that look like a server path or LAN address dropped;
  - answers cached for 15 minutes;
  - at most 4 plugin lookups at once, each limited to 12 s.
- **Tests:**
  - **parser:** unit tests, and robustness runs over random, LRC-shaped, mutated and oversize input that never panic and always keep the limits;
  - **gateway:** unit tests (answer shape, path and address redaction, bounded cache), and integration tests against the fake LMS:
    - tags first, then the plugin with only the id;
    - "no lyrics" remembered;
    - unknown tracks;
    - no token;
    - hostile ids and parameters refused before LMS;
    - the per-user allowance;
  - **mobile layer:** checking of the answer, and a round trip through the real gateway;
  - **web (Vitest):** checking of the answer, the current line, fetching once per track, races, bounded memory, the peek, the view, tap to seek, the setting, and hostile text shown as text;
  - **Android (Robolectric):** the same on Now playing and in Settings, the view model following the track, and the cache.

## 2026-10-12: Wave 1 (part A): Now playing and the sleep timer

- **Sleep timer** on every player, from a moon button on Now playing (the mini player shows a small moon while one runs; one tap turns it off):
  - 15, 30, 45, 60 or 90 minutes, a custom length (1 minute to a day), or the **end of this track, album or queue**;
  - for this player, its **sync group**, or **all players** you may control;
  - set as LMS's own `sleep <seconds>`, so it also works on hardware players and with the app closed. "End of …" is worked out from the queue (time left) and set again whenever a seek, skip, pause or queue edit moves that end; an end more than a day away is not offered;
  - **fade-out:** this device's own player fades out before the end over a length you choose in Settings (off to 2 minutes, 30 seconds by default), in its engine (the Rust engine on Android, so it carries on in the background; the AudioWorklet or media element in the browser). Other players use LMS's own sleep behaviour, which fades the volume over the last minute (at most) and then stops the player and turns it off. The fade setting is kept per user on each device for now.
- **Ambient Now playing:** the cover, blurred, drifting slowly behind Now playing (still under reduce motion, battery saver, low-power devices, and while the page is hidden). A colour filter bounds the art's brightness and a scrim worked out for each theme keeps every text colour at WCAG AA over any cover; the contrast tests cover it. Web: a small image with CSS blur; Android: RenderEffect blur on Android 12+ and a tiny scaled-up image below. Off in the high-contrast theme. The mini player takes a faint wash of the cover's colour.
- **Up next:** a row under the transport with the next track and its cover. Tap for the queue; swipe it away (or Delete, or the Remove action) to take it out of the queue, with Undo. Art and title cross-fade between tracks (not under reduce motion).
- **One mini-player gesture model** (web and Android): swipe left/right for next/previous (with a haptic tick on phones), swipe up or tap to open Now playing, long-press (or right-click) for *Play on…*. Every gesture is also a button, a TalkBack or screen-reader action, and on the web a key (Alt+→/←, Shift+F10 or the context-menu key).
- **Quality badge** on Now playing, e.g. **FLAC 24/96 · Hi-Res** or **MP3 320** (Hi-Res: more than 16-bit or more than 48 kHz, lossless only). Tapping it opens the signal path: on Android the existing panel; on the web a new one showing the source, decoding, volume and processing, the output mode (Web Audio or media element) and rate, and whether the browser resamples.
- **Status:** players are now asked for the current and the next track (`status - 2`); the gateway's allow-list already took that and `sleep`.
- **Tests:** sleep timing (end of track, album and queue, re-arming, "armed and over", sleep all and group targets, the fade curve and setting) with the same vectors in TypeScript, Rust and Kotlin; the badge and Hi-Res rule (Rust and TypeScript, hostile input); Up next derivation; the gesture state machine (with a robustness run); the engine fade end to end (Rust integration test); the gateway's `sleep` allow-list with negative tests; Vitest component tests (badge and signal path, sleep sheet, Up next swipe/remove/Undo, ambient and reduce motion, mini-player gestures and keys); Robolectric tests for every gesture and its accessibility action, the sleep sheet, Up next and the fade setting; and contrast tests for the ambient scrim and the mini player's tint in every theme.
- **Demo LMS** (`tools/demo`): `status - n` starts at the current track, `sleep` works, and the playing album is 24/96.

## 2026-10-12: Wave 1 (part B): listening history, stations and a customisable Home

Agreed design: `docs/plans/wave1-design.md` §6–8; decision D23 in `SPEC.md`.

### Listening history (gateway)
- **What it records:** each signed-in user's plays through Lyrion Anywhere that count (the `/v1/plays` rule: half the track or 240 s), by stable track key, with the track, album and artist ids, when it started and the seconds heard.
- **Live plays** are followed from each player session's own status reports. A session belongs to one device, so to one user: hardware players and other users' devices are never attributed. The track is asked of LMS as it starts. Time heard is elapsed time while playing, never more than wall time allows, and a seek that restarts the stream continues the same play.
- **Offline plays** are added when `POST /v1/plays` records them.
- **Kept 13 months**, at most 50,000 plays per user, pruned hourly.
- **New endpoints:**
  - `GET/DELETE/PATCH /v1/me/history`: view (paged), clear, switch off;
  - `GET /v1/me/shelves/{recent|most|on_this_day}`: album ids, the user's plays weighted 3 against LMS's play counts and last played (LMS 9.1 `albums sort:playcount|recentlyplayed`).

### Customisable Home
- **Layout per user on the gateway** (`GET/PUT/DELETE /v1/me/home`): versioned, at most 24 shelves and 4 KiB, a fixed set of shelf types plus the user's own stations, pinned menus and genres.
- **Default shelves:** recently played, stations, new music, most played, on this day.
- **Edit mode** on the web and Android: drag by a handle or Move up/down (also accessibility actions), hide, add, remove, reset.
- **Web app:** a Home page (the phone's Home tab; the sidebar's second entry) and a Stations page.
- **Android:** Home follows the layout. Favourites and playlists became shelves that can be added.

### Stations
- **Built-in, computed on the gateway** as batches of track ids that never repeat the user's last 200 plays or what the station served lately: forgotten favourites, fresh, deep cuts, several genres, a decade (with "favour what I've played").
- **A single genre** plays as LMS's Random Mix, through a new fixed `randomplay` grammar in the allow-list.
- **Clients refill** when fewer than about 5 tracks are left, one batch at a time, and stop when something else plays. The rules are shared: `web/app/src/lib/stations/refill.ts` on the web, and `mobile/src/me.rs` (`StationRunner`) for Android.
- **Save as station and Pin to Home** on plugin menu items (e.g. a mix):
  - what is stored is a reference to the action LMS offered (the menu requests to it, and its play/add actions), never a free command;
  - every use replays it under the user's own role and players, with the learned-action check;
  - if LMS no longer offers it, the station is marked broken with the reason, and nothing runs.

### Security
- **New endpoints** need a token, are rate-limited per user (120 a minute; 20 station batches a minute), size-capped, and take strictly typed bodies with unknown fields refused.
- **Other users' ids** are answered 404.
- **Random Mix:** `randomplay` takes a fixed mode word and at most one numeric `genre_id` (LMS reads the mode as a regular expression). Random Mix's server-wide genre and library choices became admin-only as learned menu actions.
- **Titles by track id** (`titles … track_id:1,2,3`) are allowed, for history and stations.

### Storage
Database migration 10 adds `history`, `user_prefs` and `user_pins`, all deleted with the user.

### Tests
- **Gateway:** unit and fuzz tests for the layout and station-spec parsers, the play tracker, scoring and pruning; a migration test; integration tests against the fake LMS (live and offline history, switching off, shelves, Home, stations, Random Mix, pinned-station security, pinned menus, rate limits, other users' data and hostile ids).
- **Mobile:** end-to-end tests through the real gateway.
- **Web:** Vitest for the API client, refill rules, layout editing, the Home page, menu saving and history settings.
- **Android:** layout, view-model, menu-saving and history tests, and accessibility audits.
- **Demo:** the demo LMS has invented play counts, last-played order and a mix menu for screenshots.

## 2026-10-12: plugin 0.2.0 and Android 1.1.0

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

### Synchronised playback: our players now stay in step with hardware players
- **Fixed: a phone or browser in a sync group sounded late** by its output latency: tens of ms through a speaker or Web Audio, 150–300 ms over Bluetooth. Elapsed time counted frames as they were handed to the output, not as they were heard. LMS kept the reported positions together, so it never saw the gap. Now:
  - **position as heard:** each player reports what is audible:
    - Android: from AAudio's presentation timestamp on `CLOCK_MONOTONIC`, which includes Bluetooth latency where Android reports it;
    - Web Audio: from `getOutputTimestamp()`, or `outputLatency` and `baseLatency`;
    - iOS media-element mode: from the element's play head, smoothed.

    Each STAT's elapsed is extrapolated to its own jiffies (core: `Player::presented`), on the same monotonic clock as the audio timing.
  - **exact timed start:** when LMS starts a group (`strm u` with a start time), the first frame is placed so it is heard at that time, to the sample. Audio is pre-rolled while waiting. A start the output can't make (Bluetooth against LMS's 200 ms) drops what it missed, and so starts in step.
  - **corrections at the output:** LMS's pause-for-N-ms and skip-N-ms take effect at once, in the audio callback or worklet (or on the media element). Before, they took effect behind the 0.3–8 s of audio already queued. Pauses shorter than that were lost entirely.
- **Measured** in a new harness (`mobile/tests/sync.rs`): two players against a fake LMS that learns clocks, starts and corrects as LMS does. It uses the real core, ring and timestamp maths, with simulated latency, clock drift and network jitter. Against a hardware player:
  - phone speaker and Bluetooth: within 7 ms (mean 0.5–6 ms);
  - before this change, the Bluetooth player was 256 ms late.

  Real devices still need checking.
- **LMS settings:** the plugin changes none of LMS's per-player sync settings; the defaults are right now. `docs/PROTOCOL.md` › *Synchronised playback* lists what users can tune: *Player audio delay* for latency a platform doesn't report, and the minimum adjustment.
- **Tests:**
  - core: extrapolation, timed starts across the jiffies wrap, corrections handed to the host, and hostile start times and intervals (refused or capped);
  - ring: the device clock and its history, exact and late timed starts, inserted silence, skips, and consistency while the audio thread runs;
  - timestamp maths: drift, jitter, Bluetooth latency and nonsense values;
  - worklet: runs, hold, the timed start, silence, skip and hostile messages;
  - the worker's presentation tracker, the output clock and play-head smoother, the media element's sync pause and skip, and the WASM bindings.

### Faster start and seeks for FLAC with embedded artwork
- **What:** the gateway now drops embedded artwork (PICTURE) and PADDING metadata from the start of FLAC streams, so the browser, iOS and Android players no longer download the cover before the first audio. The artwork is still shown: the apps get it from `/v1/art`, as before. It also saves that much mobile data per track start, and per seek when LMS sends the metadata again.
- **Measured** (tests, a simulated 80 ms / 8 Mbit/s link, a FLAC with 480 KiB of artwork): bytes received before 0.5 s of audio is decoded (when quick start plays) fell from 704 KiB to 224 KiB, about 800 ms to about 310 ms, in line with MP3.
- **Safety:**
  - only streams LMS announces as FLAC; only the metadata chain changes, and the last-block flag is moved correctly; STREAMINFO, SEEKTABLE, VORBIS_COMMENT and the audio frames are forwarded byte for byte, so the audio decodes bit-identically;
  - bounded (16 MiB of metadata, 1024 blocks), and anything unexpected (no `fLaC` marker, a bad or second STREAMINFO, reserved or invalid block types, no frame sync after the last block, a stream that ends early) passes the stream through unchanged;
  - `?offset=` resume, acks and the replay window all count the stripped stream, which is made once per stream, so resuming is unchanged. LMS's byte count in STAT reports is converted back to what LMS sent.
  - `--keep-flac-pictures` turns it off.
- **Tests:** the metadata parser fuzzed (random, truncated, mutated, spliced and oversize streams, lying last-block flags, nested garbage; never panics, and the output is always either the input or a clean strip of it, whatever the chunking); stripped streams decode to the same PCM, bit for bit, over randomised layouts; through the gateway: artwork dropped with the audio identical, resume twice (inside the metadata and in the audio) giving the same bytes as one fetch, STAT counts converted, MP3, unusual FLAC and the switch passing through unchanged; and the before/after measurement.

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

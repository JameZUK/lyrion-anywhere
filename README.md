<p align="center">
  <img src="docs/images/logo.svg" alt="Lyrion Anywhere logo: a waveform turning into broadcast waves" width="128" height="128">
</p>

<h1 align="center">Lyrion Anywhere</h1>

<p align="center"><strong>Your <a href="https://lyrion.org">Lyrion Music Server</a>, anywhere: securely, losslessly, in any modern browser and in a native Android app.</strong></p>

<p align="center">
  <a href="https://github.com/JameZUK/lyrion-anywhere/releases"><strong>Releases</strong> (Android app and LMS plugin)</a> ·
  <a href="docs/INSTALL.md">Installation guide</a> ·
  <a href="docs/FEATURES.md">All features</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a>
</p>

Lyrion Anywhere lets you listen to the music on your home Lyrion Music Server (LMS) from anywhere: on your phone on the train, on a laptop at work, in the car. Every phone, tablet and browser you sign in on becomes a real LMS player, streaming your library losslessly, and can also control your home players and move playback between them. When the signal drops, it keeps playing from what it already holds.

LMS itself never touches the internet. One small, hardened gateway sits behind your own HTTPS reverse proxy, and it is the only thing anyone outside your home can reach.

<p align="center">
  <img src="docs/images/web/now-playing-desktop.png" alt="The web app's Now playing page on a desktop browser: a large album cover beside the track title, artist and album, a quality badge reading FLAC 24/96 Hi-Res, the current lyric line, a waveform seek bar, transport controls, an Up next line and the volume, with the queue pinned on the right" width="820">
</p>

> Screenshots come from the built-in demo ([`tools/demo`](tools/demo)) and the Android app's screenshot tests. The library, players, people and artwork in them are invented.

## Who it is for

- **LMS owners who want their music away from home** without a VPN and without exposing LMS to the internet.
- **Households:** give each person their own account, with their own devices, history and settings. Guests can be limited to the players you choose.
- **Listeners who care about sound:** lossless and gapless by default, bit-perfect to a USB DAC on Android, and a signal path that shows exactly what happens to every sample.

## What makes it different

- **Every device is a real player.** Your phone or browser appears in LMS as a Squeezebox-style player, so gapless playback, sync groups, alarms, plugins and play counts all work as they do on any LMS player. It is not a remote control that needs another speaker.
- **Secure by design.** LMS's own ports stay on your home network. The gateway allows only an explicit list of commands, checks every argument, strips file paths and LAN addresses from every answer, and has its own accounts with optional two-step verification.
- **Lossless where it matters, frugal where it counts.** FLAC plays as FLAC. On mobile data the gateway can re-encode to Opus on the fly, and offline downloads can be the original files or smaller copies.
- **Honest when the server is away.** If your server can't be reached, controls that can't work are greyed out, while the ones that can keep working on the device: pause, resume, and skipping to tracks it already holds. A server that is merely busy (say, its music drive spinning up) is shown calmly as busy, not offline.
- **A native Android app**, sharing a Rust audio engine and network layer with the planned iOS app: bit-perfect USB output, optional DSP, Android Auto, offline downloads, and read-ahead that rides out tunnels and dead spots.
- **Made for LMS.** Apps, radio and plugins (AudioMuse-AI, Custom Browse, podcasts, streaming services and more) appear by themselves, with nothing hard-coded.

## Highlights

### In the browser (installable web app)

<p align="center">
  <img src="docs/images/web/now-playing-phone.png" alt="Now playing on a phone, dark theme: the cover fills the top of the screen above the title with a rating star, a quality badge, a waveform seek bar, large transport controls, a one-line Up next and three actions: Lounge (the player in use), Lyrics and Queue" width="250">
  &nbsp;
  <img src="docs/images/web/home-phone.png" alt="Home on a phone: shelves of recently played albums, stations and new music, with the mini player naming the players in use, on Lounge and Kitchen" width="250">
  &nbsp;
  <img src="docs/images/web/play-on-phone.png" alt="The Play on sheet on a phone: a volume slider, then every player with what it is doing, this device first, Lounge ticked as the one in use, and Group players at the bottom" width="250">
</p>

- **Plays on the device straight away:** sign in and press Play. Lossless decoding runs in WebAssembly; on iPhone, iPad and Android it keeps playing with the screen locked.
- **Uncluttered:** one row of Library sections, Play · Shuffle · Download · ⋯ on every album and playlist, one ⋯ per row, one status line at the top, and Settings in seven short sections.
- **Your whole library:** albums (including recently added and random), artists by role, playlists, favourites, smart playlists, genres, years, compilations, folders, downloads, and every app and radio service LMS offers.
- **Now playing that earns its space:** waveform seek bar, synced lyrics, artist biographies, a visualiser, a quality badge that opens the signal path, and a sleep timer.
- **Your own Home, stations and listening stats,** including a "Your year in sound" story you can share as an image made on the device.
- **Play on… and Rooms:** every player and what it is playing; move playback, group players, switch them on or off and set their alarms. On a wide screen Rooms shows them as cards: drag one cover onto another to sync them.
- **Downloads:** albums, playlists, artists and tracks for offline listening, with a storage limit and an optional Wi-Fi only setting; they play through the normal player screens.

### On Android (native app)

<p align="center">
  <img src="docs/images/android/now-playing.png" alt="The Android app's Now playing: the album cover, the title with one rating star, artist and album with a quality badge, the current lyric line, a waveform seek bar, transport controls, an Up next line and Lounge, Lyrics and Queue along the bottom" width="250">
  &nbsp;
  <img src="docs/images/android/play-on.png" alt="The Android Play on sheet: this phone, ready and with four minutes buffered ahead, then Lounge ticked as the player in use, Study, Garden Room switched off, and Group players" width="250">
  &nbsp;
  <img src="docs/images/android/signal-path.png" alt="The Android signal path sheet: source format, each processing stage, the output route to a USB DAC and a bit-perfect verdict" width="250">
</p>

- **Bit-perfect audio:** a Rust engine with AAudio output, bit-perfect by default, and bit-perfect to a USB DAC on Android 14 and later where the DAC supports it.
- **Optional DSP, all off by default:** loudness levelling (ReplayGain or the server's EBU R128 analysis), parametric EQ with AutoEq import and per-output profiles, loudness-compensated volume, crossfeed, dither, and smart crossfades that never cross into a gapless album.
- **Android Auto, car mode, widgets, a Quick Settings tile and app shortcuts.**
- **Offline:** downloads (including smart downloads of recent albums and favourites), an offline mode, plays counted in LMS when you are back online, and *Continue on…* any player where you left off.
- **Gaps in the signal:** the next track is fetched long before the current one ends, and a session survives up to ten minutes without a connection. A replay cache plays repeats and skips back from the phone, and an album downloaded as FLAC stands in for Original-quality streams.
- **Out of reach, not out of action:** while the server can't be reached, the phone keeps playing, pausing and skipping between tracks it holds; the queue marks which tracks are on the phone, and what you did is passed back when the server returns.
- **Layouts for phones, tablets and foldables.**

**Get it:** signed APKs are on the [releases page](https://github.com/JameZUK/lyrion-anywhere/releases) (tags `android-v*`; the latest is 1.14.0); `…-arm64-v8a.apk` suits most phones and the universal `.apk` works on any. Check the files against `SHA256SUMS`. Google Play and F-Droid listings are being prepared. The source is in [`apps/android`](apps/android).

<details>
<summary><strong>More screenshots</strong></summary>

**Web app**

<p align="center">
  <img src="docs/images/web/queue-phone.png" alt="The queue on a phone, in sections: Played, Now playing, Up next and Then, each row with a drag handle and a ⋯ menu" width="200">
  <img src="docs/images/web/library-phone.png" alt="Library on a phone: one row of sections with Albums selected, a sort choice and a grid of album covers with an A to Z scroller" width="200">
  <img src="docs/images/web/lyrics-phone.png" alt="Synced lyrics on a phone, the current line highlighted above the title, seek bar and controls" width="200">
  <img src="docs/images/web/settings-phone.png" alt="Settings on a phone: seven sections, each with a one-line summary of its current setting" width="200">
</p>
<p align="center">
  <img src="docs/images/web/album-desktop.png" alt="An album page on a desktop browser: the cover, title and artist, Play, Shuffle, Download and ⋯, a review and the track list" width="400">
  <img src="docs/images/web/rooms-desktop.png" alt="Rooms on a desktop browser: a card for each player with what it is playing, play/pause, volume and ⋯, and Switch on for one that is off" width="400">
</p>
<p align="center">
  <img src="docs/images/web/stats-desktop.png" alt="Your listening on a desktop browser: listening totals, a button to play Your year in sound, and the most played artists, albums and tracks" width="400">
  <img src="docs/images/web/now-playing-desktop-dark.png" alt="Now playing on a desktop browser in the dark theme, with the queue pinned on the right" width="400">
</p>

**Android app**

<p align="center">
  <img src="docs/images/android/home.png" alt="Home on Android: recently played, stations and recently added shelves" width="200">
  <img src="docs/images/android/queue.png" alt="The queue on Android: the player and track count, then rows with cover, title, length, ⋮ and a drag handle" width="200">
  <img src="docs/images/android/downloads.png" alt="Downloads on Android: storage used, downloads in progress with Pause all and Cancel all, and downloaded albums" width="200">
  <img src="docs/images/android/car-mode.png" alt="Car mode on Android: huge artwork and giant previous, play/pause and next buttons in white and amber on black" width="200">
</p>
<p align="center">
  <img src="docs/images/android/now-playing-light.png" alt="Now playing on Android in the light theme" width="200">
  <img src="docs/images/android/sound-settings.png" alt="Sound settings on Android: a bit-perfect verdict, bit-perfect USB output, the player level, loudness levelling, preamp and a true-peak limiter" width="200">
  <img src="docs/images/android/gaps-settings.png" alt="Gaps in the signal and replay cache settings on Android: read ahead the next track, on mobile data too, and the cache size" width="200">
  <img src="docs/images/android/stats.png" alt="Your listening on Android: totals for the month and the most played artists" width="200">
</p>
<p align="center">
  <img src="docs/images/android/tablet-now-playing.png" alt="Now playing on an Android tablet: the cover and controls on the left, synced lyrics in the middle and the queue on the right" width="820">
</p>

</details>

## How it works

```mermaid
flowchart LR
  subgraph Internet
    WEB["Web app (PWA)<br/>WebAssembly player"]
    AND["Android app<br/>Rust engine"]
  end
  subgraph Home["Your home network"]
    RP["Reverse proxy<br/>TLS on :443"]
    subgraph GW["Lyrion Anywhere gateway (Rust)"]
      AUTH["Accounts, sign-in,<br/>two-step"]
      RPC["JSON-RPC allow-list<br/>and response scrubbing"]
      TUN["Slimproto tunnel<br/>with resume"]
      STR["Stream relay,<br/>live Opus, downloads"]
    end
    subgraph LMS["Lyrion Music Server"]
      PLG["Lyrion Anywhere plugin<br/>(runs the gateway)"]
      P3483[":3483 Slimproto"]
      P9000[":9000 HTTP / JSON-RPC"]
      P9090[":9090 CLI events"]
    end
  end
  WEB -- "HTTPS + WSS" --> RP
  AND -- "HTTPS + WSS" --> RP
  RP -- "HTTP, loopback or LAN" --> GW
  TUN --> P3483
  RPC --> P9000
  STR --> P9000
  GW --> P9090
  PLG -. "starts, supervises,<br/>admin API on loopback" .-> GW
```

- **Clients** speak only HTTPS and secure WebSockets, to your reverse proxy (nginx, Caddy, Traefik or a Cloudflare Tunnel).
- **The gateway** is the only process the proxy forwards to. It signs devices in, passes allow-listed LMS commands through and scrubs the answers, tunnels each device's Slimproto player connection over a WebSocket (resuming it after a dropout), and relays audio with a replay window so a broken stream carries on from where it stopped.
- **The LMS plugin** starts the gateway, restarts it if it stops, and adds a settings page for users, devices and options. Accounts live in the gateway's own database, not in LMS.
- **LMS's ports (3483, 9000, 9090) are never forwarded.**

The detailed design, with request flows and sequence diagrams, is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Features at a glance

Each group has a full breakdown, with the platforms each feature runs on, in [`docs/FEATURES.md`](docs/FEATURES.md).

| Area | In short |
|---|---|
| [Playback](docs/FEATURES.md#playback) | The device is a real LMS player; lossless and gapless; background playback; resume after dropouts and reloads; sleep timer; lower-data streams on mobile data |
| [Players, rooms and sync](docs/FEATURES.md#players-rooms-and-sync) | *Play on…*, move playback between players, sync groups, Rooms, alarms |
| [Library and browsing](docs/FEATURES.md#library-and-browsing) | All of LMS's "My Music", virtual libraries, credits, A–Z scroller, smart filters, apps, radio and plugins |
| [Search](docs/FEATURES.md#search) | Grouped results, recent searches, search inside apps |
| [Queue](docs/FEATURES.md#queue) | Sections, drag, swipe, Undo, checked edits, album-aware shuffle, save as a playlist, marks for tracks on this device |
| [Now playing](docs/FEATURES.md#now-playing-lyrics-visualiser-and-signal-path) | Waveform seek bar, lyrics, info, visualiser, quality badge and signal path, ratings |
| [Sound](docs/FEATURES.md#sound-and-dsp) | Bit-perfect output, levelling, EQ, loudness, crossfeed, dither, transitions (Android) |
| [Discovery](docs/FEATURES.md#discovery-and-radio) | Home shelves, stations, Start radio, More like this |
| [Offline](docs/FEATURES.md#offline-and-downloads) | Downloads in three formats, offline mode, plays synced back, controls that keep working while the server is out of reach |
| [Your listening](docs/FEATURES.md#listening-history-stats-and-your-year) | Private listening history, stats, "Your year in sound" |
| [Car and system](docs/FEATURES.md#car-android-auto-widgets-and-system-surfaces) | Car mode, Android Auto, widgets, tile, shortcuts, haptics, Media Session |
| [Appearance and accessibility](docs/FEATURES.md#appearance-and-accessibility) | Material, Classic and High contrast themes; tested contrast; screen readers and keyboards |
| [Accounts and admin](docs/FEATURES.md#accounts-devices-roles-and-admin) | Per-user accounts, roles, player grants, QR pairing, change your own password, device list, diagnostics |
| [Security](docs/FEATURES.md#security-features) and [privacy](docs/FEATURES.md#privacy-controls) | See below |

## Quick start

**Step by step: [`docs/INSTALL.md`](docs/INSTALL.md).** The reference for proxies and Docker is [`docs/DEPLOY.md`](docs/DEPLOY.md).

1. **Install the plugin** in LMS: *Settings › Plugins*, add the repository `https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml`, tick **Lyrion Anywhere** and restart LMS. (No plugin binary for your platform? Run the gateway with Docker instead.)
2. **Put an HTTPS reverse proxy in front of it,** for a name such as `music.example.com`. A tested nginx configuration is in [`deploy/nginx`](deploy/nginx); with Caddy it is two lines.
3. **Set the Public URL and create users** on the plugin's settings page (*Settings › Plugins › Lyrion Anywhere*).
4. **Sign in:** open `https://music.example.com` on your phone and sign in, then optionally *Add to Home Screen*. Or install the Android app and scan the QR code from *Add a device*.

**Requirements**
- Lyrion Music Server 9.0 or later. The plugin bundles the gateway for Linux on x86_64, aarch64 and armhf; elsewhere use Docker.
- A domain name, port 443 forwarded, and a reverse proxy with a certificate.
- Optional: `ffmpeg` on the gateway's machine, for MP3 and Opus downloads and lower-data streams.

## Security in brief

- **Nothing of LMS is exposed.** The gateway forwards only allow-listed commands with validated arguments; everything else is refused. Answers are scrubbed of file paths, LAN addresses, server identifiers and other servers' players.
- **Each device is held to its own player.** The Slimproto tunnel accepts only what a software player sends, pins the device's assigned player address, and never lets LMS redirect a device to another server.
- **Accounts of its own:** argon2id passwords, optional TOTP two-step verification, sign-in lockouts that strangers can't use to lock you out, 15-minute access tokens, rotating refresh tokens with reuse detection, and per-device revocation that takes effect at once.
- **Least privilege:** listeners control only their own devices and the home players an admin grants them. Plugins can be reserved for admins.
- **Hardened web app:** a strict Content Security Policy with no inline script, no third-party code at runtime, and metadata always rendered as text.
- **Tested like it matters:** negative tests for every endpoint, robustness tests for every parser, a regression test for every security fix, and reviews recorded in [`docs/security/`](docs/security).

More in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#4-trust-boundary-and-threat-model) and [`SPEC.md`](SPEC.md) §9. Please report vulnerabilities privately through GitHub's *Report a vulnerability* (Security tab), not in a public issue.

## Privacy in brief

- **Nothing goes to the project.** No analytics, advertising, tracking or third-party SDKs. The apps talk only to your own gateway.
- **Your server keeps what it needs:** accounts, devices (including each device's last address, for admins), an audit log for a year, and, per user, a listening history kept for 13 months that you can view, clear or switch off. No one else, admins included, can see your history through the apps or the API.
- **Your phone keeps your sign-in sealed by the Android Keystore,** your settings and your downloads. Signing out can remove the downloads; a revoked device deletes them.
- **Lyrics, biographies and reviews** come from your server's own plugins; Lyrion Anywhere never calls those services itself.

The full policy, including Google Play's Data safety answers, is [`docs/PRIVACY.md`](docs/PRIVACY.md).

## Platform support

| Platform | Status | Notes |
|---|---|---|
| **Chrome, Edge (desktop)** | Supported | Web Audio playback; installable as an app |
| **Firefox (desktop)** | Supported | Web Audio playback |
| **Safari (macOS)** | Supported | Web Audio playback |
| **Android browsers** (Chrome, Firefox) | Supported | Media-element playback, which keeps going with the screen locked, after a self-test of what the browser can play; Web Audio otherwise. Installable |
| **iPhone and iPad (Safari, iOS/iPadOS 17 or later)** | Supported | Media-element playback through Managed Media Source (17.1 or later), so music carries on with the screen locked. The volume is set with the device's own buttons (a page can't set it). Install with *Share › Add to Home Screen*; offline downloads are reliable only in the installed app. No haptics, and *Automatic* streaming quality can't tell mobile data from Wi-Fi |
| **Android app** | Android 10 (API 29) or later | Bit-perfect USB DAC output on Android 14 or later where the phone and DAC offer it; wallpaper colours and the sharper ambient blur on Android 12 or later; Android Auto |
| **iOS app** | Planned | Designed in: the shared Rust layer ([`mobile/`](mobile)) already covers networking, the player engine and downloads |
| **Gateway** | Linux x86_64, aarch64, armhf (via the plugin); any Docker host | LMS 9.0 or later |

## Project status and roadmap

- **Built and in use:** the gateway and LMS plugin, the web app, and the Android app (1.14.0, releases tagged `android-v*`). See [`CHANGELOG.md`](CHANGELOG.md) for what changed and when.
- **Recently finished:** a declutter of every screen on the web and Android ([`docs/plans/wave3_5-declutter.md`](docs/plans/wave3_5-declutter.md)); controls that work while the server is out of reach; riding out a busy server; and the rolling cache for patchy mobile data (read-ahead to disk, a ten-minute resume grace, a replay cache and downloads standing in for streams; design in [`docs/plans/wave3-design.md`](docs/plans/wave3-design.md) §2). Change password, save the queue as a playlist and the busy notice need a gateway newer than plugin 0.3.0; they arrive with the next plugin release.
- **iOS app:** planned, on the same Rust layer and protocol, with CarPlay. It will be licensed GPL with an App Store exception for the project's own code (SPEC D17).
- **Later:** passkeys, a cryptographic release signature for the plugin's bundled gateways, smart filters with track-level rules, a guest queue, album booklets. Store listings for the Android app (Google Play, F-Droid) are being prepared.

## Documentation

| Document | What it covers |
|---|---|
| [`docs/INSTALL.md`](docs/INSTALL.md) | Step-by-step installation, users, first sign-in, troubleshooting |
| [`docs/DEPLOY.md`](docs/DEPLOY.md) | Deployment reference: plugin, Docker, reverse proxy, CDN |
| [`docs/FEATURES.md`](docs/FEATURES.md) | Every feature, by area, with the platforms it runs on |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Components, trust boundary, request flows, storage, testing |
| [`docs/PROTOCOL.md`](docs/PROTOCOL.md) | The public API and player protocol, the command policy and security rules |
| [`docs/PRIVACY.md`](docs/PRIVACY.md) | What each part stores, where and for how long |
| [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) | Android performance budgets and how to measure them |
| [`docs/RELEASING.md`](docs/RELEASING.md) | How plugin and Android releases are made |
| [`docs/security/`](docs/security) | Security reviews and penetration tests |
| [`SPEC.md`](SPEC.md) | The design and every decision behind it |
| [`docs/README.md`](docs/README.md) | Index of all documentation, including design plans |

## Building from source

The whole project builds from this repository: the Rust workspace (`core`, `gateway`, `wasm`, `mobile`, `cli`), the web app, the Perl plugin and the Android app. In short:

```sh
cargo test --workspace                     # core, gateway (unit and end-to-end against a fake LMS), mobile
tools/build-web.sh                         # WebAssembly core and the web app (needs wasm-pack and Node 22+)
cd web/app && npm run check && npm test    # strict type check; Vitest and Node tests
cd plugin && PERL5LIB=t/stubs:. prove -r t/ && node --test t/js/*.test.mjs
cd apps/android && ./gradlew assembleDebug testDebugUnitTest lint
```

- **Try it without an LMS:** `tools/demo/run.sh` runs the real gateway and web app against an invented library (see [`tools/demo`](tools/demo)); sign in at `http://127.0.0.1:19443` as `demo` / `demo-password-123`.
- **Plugin releases:** `tools/build-release.sh` (needs Docker). **Android:** see [`apps/android/README.md`](apps/android/README.md) for the SDK, NDK and `cargo-ndk` set-up.
- **Contributing:** read [`CLAUDE.md`](CLAUDE.md) first. Every change ships with tests (security first); no personal or environment details go in the repository (use `music.example.com`, `10.0.0.10`, `/srv/music`); British English in docs and UI.

## Licence

GPL-2.0-or-later. See [`LICENSE`](LICENSE). Binaries that include Apache-2.0-licensed dependencies are distributed under the terms of GPLv3.

## Acknowledgements

- **[Lyrion Music Server](https://lyrion.org)** (formerly Logitech Media Server) and its community, without which none of this would exist. Lyrion Anywhere is an independent project, not an official Lyrion one.
- **[squeezelite](https://github.com/ralph-irving/squeezelite)** and **[Material Skin](https://github.com/CDrummond/lms-material)** were references for behaviour only. This is a clean-room implementation: no code was copied from either.
- **Open-source building blocks,** among them Symphonia (decoding), tokio and axum (the gateway), rustls, SQLite, UniFFI, SvelteKit, Jetpack Compose and Media3. A patched copy of the `opus-decoder` crate is kept in [`third_party/`](third_party) until a fixed release.
- **The logo** is an original design in the spirit of the Lyrion Music Server logo: a waveform turning into broadcast waves, for your music reaching you wherever you are.

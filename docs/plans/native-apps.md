# Native apps plan: Android first, iOS designed in

**Status:** agreed plan (2026-10-11). This plans SPEC §10 and milestones 4–5. Decisions N1–N4 are confirmed (SPEC D14–D17); N5 and N6 follow the recommendations unless changed.

**Goal: the best way to listen to your own LMS library on a phone.**
- **Audiophile-grade:** bit-perfect where the hardware allows, and a signal path you can see and trust.
- **As fluid and delightful as Plexamp:** waveform seeking, loudness levelling, smart transitions and radio-style discovery.
- **Secure by design,** like the rest of the project.

---

## 1. What "killer" means here

Users choose a music app for how it sounds, how it feels, and what it lets them discover. We aim to win on all three:

| Pillar | What it means | Why we can win |
|---|---|---|
| **Sound** | Bit-perfect playback to USB DACs (Android 14+); automatic sample-rate switching; a 64-bit DSP path; a visible signal path from file to DAC; true gapless; loudness levelling that never clips | Our Rust core already decodes losslessly and bit-exactly. The app owns the whole audio path, without depending on a platform player's resampling |
| **Feel** | A waveform seek bar ("seekprint"); album-art-driven colour; 120 Hz animations; gestures; instant start; widgets, lock screen, Android Auto | Native Compose UI over a tested core. The web app's UX work (queue editing, sync, themes) carries over |
| **Discovery** | Track, artist and album radio; mixes; "more like this"; smart transitions between tracks | LMS already has the brains (AudioMuse-AI, Don't Stop The Music, Random Mix, LastMix); we make them first-class, not buried menus |
| **Trust** | Your server, your data, end-to-end security, no third-party tracking | Already the project's core design: default-deny gateway, audited auth, pentested |

## 2. Product scope

### 2.1 Must-have for 1.0

**Library and control**
- Library: home (recently played, recently added, favourites, mixes), albums, artists, playlists, search, genres, years, folders.
- Remote control of every permitted LMS player: transport, volume (hardware keys too), queue editing, sync groups, power.
- This phone as a player:
  - it appears in LMS like any other player, with gapless playback and sync;
  - background playback, lock screen, Bluetooth and headset controls;
  - audio focus and ducking.
- Dynamic plugin support (apps, radio, AudioMuse-AI and others, context menus), like the web app.

**Sound and signal path**
- Signal-path panel: Source → Decode → DSP → Output, with each stage marked lossless, resampled or lossy. Example: "FLAC 96/24 → decode (lossless) → levelling −3.2 dB → USB DAC 96/24 bit-perfect".
- Bit-perfect USB DAC output on Android 14+ where the device supports it. Elsewhere, the output rate follows the source (no needless resampling), using a 32-bit float path.
- Loudness levelling (EBU R128 and ReplayGain, track or album mode, with a true-peak limiter) and a preamp.
- Waveform seek bar from server-side analysis (§4).

**Offline and car**
- Offline downloads (original, MP3 or Opus), smart downloads, an offline mode, and play-count sync-back (D6).
- Android Auto.
- Sign-in with password or by scanning a QR pairing code from the web app (already in the protocol).

### 2.2 Shortly after 1.0
- **Smart transitions** ("sweet fades"): crossfade only between unrelated tracks. Never inside an album or a gapless sequence. The fade shape comes from analysing the end and start of each track.
- **Radio and discovery:**
  - track, artist or album radio and "more like this", via AudioMuse-AI or LMS's Don't Stop The Music, whichever is installed (found dynamically);
  - mix builders.
- **EQ:** a parametric equaliser with per-output-device profiles (headphones, car, speaker) and headphone correction presets.
- **More surfaces:**
  - **Visualiser:** an optional, battery-aware spectrum or oscilloscope view drawn from the decoded audio.
  - **Lyrics:** shown where LMS plugins provide them.
- **Home-screen widgets, a Quick Settings tile, and a Wear OS remote.**

### 2.3 Later or optional
- **DSD:** DSF/DFF files sent to USB DACs as DSD over PCM (DoP), in bit-perfect mode only.
- **More DSP:**
  - *crossfeed* for headphones;
  - *high-quality upsampling* (optional; off by default, because bit-perfect is the point).
- **Accounts and servers:** several servers and accounts in one app; passkeys.
- **Tablets, foldables and Chromebooks** with adaptive layouts (designed in from the start; polished here).
- **Not planned:** MQA (proprietary and obsolete), and Chromecast as an output (LMS players already cover casting-like use).

## 3. Architecture

### 3.1 Shape

Thin native UIs over a shared Rust layer, as decided in SPEC D3 and §10:

```
┌──────────────── Android app (Kotlin) ────────────────┐   ┌─────────── iOS app (Swift) ───────────┐
│ Jetpack Compose UI · Media3 MediaLibraryService      │   │ SwiftUI · MPNowPlaying · CarPlay      │
│ Android Auto · widgets · WorkManager downloads       │   │ AVAudioSession · background audio      │
│ AudioOutput: AAudio / AudioTrack (+ USB mixer attrs) │   │ AudioOutput: AVAudioEngine source node │
└───────────────┬──────────────────────────────────────┘   └───────────────┬───────────────────────┘
                │ UniFFI (Kotlin)                                          │ UniFFI (Swift)
┌───────────────┴──────────────────────────────────────────────────────────┴───────────────────────┐
│ mobile/  (new Rust crate, does I/O): API client, auth + token refresh, Slimproto tunnel,          │
│          event stream, download manager, offline store index, play-event queue, queue model,      │
│          audio engine (decode → DSP → ring buffer), analysis cache                                │
├─────────────────────────────────────────────────────────────────────────────────────────────────┤
│ core/  (existing, sans-IO): Slimproto, player state machine, decoders (Symphonia), FLAC/ALAC mux  │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **`core/` stays sans-IO** (the project rule). A new **`mobile/`** crate is the native *host*: it does the I/O (tokio, rustls, WebSocket) and is shared by Android and iOS. This puts about 70% of the app's logic in one tested place, which makes the iOS app mostly UI and platform glue.
- **The audio engine lives in Rust:** decode, DSP and a lock-free ring buffer. Only the last step (handing PCM to the device) is platform code. That keeps the signal path identical, testable and bit-exact on both platforms.
- **Media3 `SimpleBasePlayer`** wraps our engine. Media3 then provides the notification, lock screen, Bluetooth and headset buttons, Android Auto's browse tree and the session tokens, while the sound stays ours. The iOS equivalent is `MPRemoteCommandCenter` plus `MPNowPlayingInfoCenter`.

### 3.2 Two playback sources, one engine

| Mode | When | How |
|---|---|---|
| **LMS player** (default when online) | Streaming from your server | As in the web app, the phone is a Slimproto player through `/v1/player`, so LMS drives the queue, gapless playback, sync and plugins. The app declares lossless capabilities (`flc`, `alc`, `pcm`, `mp3`, `ops` …), so LMS sends original files where possible |
| **Local queue** | Offline, or for downloaded content | The app owns the queue and plays from its download store. Plays are queued and sent to LMS on reconnect (D6) |

Both feed the same engine, so DSP, levelling, the waveform, the signal path and bit-perfect output behave identically. Switching between them (e.g. losing signal mid-album when the album is downloaded) is seamless where the content is available offline.

### 3.3 Audio output, per platform

**Android**
- **Normal output:** AAudio (via Rust's `ndk` crate) in low-latency or power-saving mode as appropriate. The stream is opened at the *source's* sample rate, so the platform only resamples when the hardware can't follow.
- **USB DAC, bit-perfect (Android 14+, API 34):**
  - the app queries `AudioManager.getSupportedMixerAttributes(usbDevice)` and sets `MIXER_BEHAVIOR_BIT_PERFECT` with the source's format and rate;
  - in that mode Android skips mixing, volume and effects, so volume control is the DAC's own (the UI says so) and DSP that changes samples is bypassed or flagged;
  - BIT_PERFECT is optional for device makers, so it's detected at runtime and the signal path shows what was actually achieved.
- **Bluetooth:** shows the codec in use (LDAC, aptX HD, AAC, SBC) in the signal path, where the platform reports it.

**iOS**
- **Output:** AVAudioEngine with a source node fed from the Rust ring buffer.
- **Sample rate:** `AVAudioSession.setPreferredSampleRate` follows the source on external DACs (USB-C or Lightning). iOS has no exclusive mode; with an external DAC at the matching rate and unity volume, the output is bit-transparent, which the signal path reports honestly.
- **System integration:** background audio mode, CarPlay, AirPlay as an output (marked lossy or resampled in the signal path).

### 3.4 Engine details (Rust)
- **Decode:**
  - Symphonia (FLAC, ALAC, WAV/AIFF, MP3, AAC, Vorbis).
  - **Opus** needs adding (libopus via the `opus` crate, or Symphonia's Opus support if it has landed). Opus downloads need it.
  - **DSD:** DSF/DFF parsing and DoP packing later (§2.3).
- **DSP chain:** 64-bit float, applied in this order, with each stage able to be bypassed:
  1. preamp;
  2. loudness levelling with a true-peak limiter;
  3. EQ;
  4. crossfeed;
  5. dither when reducing bit depth.

  Any active stage marks the path "processed" in the signal path panel.
- **Resampling:** only when the output can't take the source rate, with a high-quality sinc resampler (e.g. `rubato`), shown in the signal path.
- **Gapless:** sample-accurate, already proven in the core and gateway.
- **Look-ahead:** the next track is decoded early, for gapless playback and smart transitions.

## 4. Waveform, loudness and track analysis

Plexamp's waveform seek bar ("seekprint"), loudness levelling and sweet fades all come from per-track loudness analysis. We do the same, on the server side, so every client (web, Android, iOS) benefits and phones don't burn battery.

**Gateway:** a new analysis service.

| What | Detail |
|---|---|
| Output per track | Waveform peaks (about 1,000 buckets of peak plus RMS, 2–4 KiB); EBU R128 integrated loudness, loudness range and true peak; leading and trailing silence; fade-in and fade-out shape (for smart transitions). Album loudness is aggregated from its tracks |
| Endpoint | `GET /v1/analysis/track/{id}` → compact JSON (or CBOR), cached with `ETag`; keyed by the stable track key (survives rescans) |
| When | On demand the first time a track is played or shown, plus an optional low-priority background pass. It is CPU-capped (one job, idle priority) because many LMS hosts are Raspberry Pis |
| How | Fetch the original through the existing download path, decode with the core (Symphonia), analyse in streaming fashion (constant memory). Results go in the gateway's database (a few KiB per track) |
| Security | Same rules as downloads: permission checks, library tracks only, no paths in output, limits per user |

**Clients:**
- **Waveform seek bar:** drawn from the peaks, coloured from the album art, with the played portion highlighted and drag-to-seek. While no analysis exists, it falls back to a plain bar and fills in progressively.
- **Loudness levelling:** uses the integrated loudness and true peak (track or album mode; the target is configurable, e.g. −16 LUFS on phones). Without analysis, ReplayGain tags are used.
- **Smart transitions:** use the end and start analysis to choose fade length and shape. They never apply within an album, a gapless sequence or a classical work.

**Phasing:** loudness and peaks first (they're cheap). Tempo and key are later, only if a feature needs them. Similarity ("sonic analysis") isn't ours to build: AudioMuse-AI does it in LMS, and we integrate it.

## 5. Android app structure

- **Language and UI:**
  - Kotlin, with Jetpack Compose and Material 3 (expressive);
  - dynamic colour from album art (palette extraction), with themes aligned to the web app (Material, Classic, High contrast);
  - adaptive layouts (compact, medium and expanded) from day one.
- **Modules:**

| Module | Contents |
|---|---|
| `:app` | Navigation, DI |
| `:ui` | Design system and screens |
| `:playback` | `MediaLibraryService`, the `SimpleBasePlayer` adapter, audio focus, the becoming-noisy handler |
| `:audio` | AAudio/AudioTrack output, USB detection and mixer attributes, signal-path reporting |
| `:downloads` | WorkManager jobs; storage on internal storage or SD card |
| `:auto` | Android Auto browse tree, through Media3 |
| `:widgets` | Glance widgets |
| `:rust` | Generated UniFFI bindings plus `cargo-ndk` builds for `arm64-v8a`, `armeabi-v7a` and `x86_64` |

- **Minimum Android version: 10 (API 29) (D15).** About 95% of active devices. Bit-perfect needs 14+ and is enabled where available.
- **Security:**
  - refresh token in Android Keystore-backed storage; access token in memory only;
  - no cleartext traffic (network security config); an optional biometric app lock;
  - no analytics or third-party SDKs;
  - certificate pinning as an option (trust on first use), since servers use their own certificates;
  - pairing by QR code (protocol exists);
  - remote wipe of downloads on revocation (SPEC §8.5).
- **Performance:**
  - baseline profiles, a startup budget (under 500 ms to first frame on mid-range phones), and art pre-fetching;
  - the audio thread never waits on the UI;
  - a battery budget for the visualiser and analysis.

## 6. iOS: designed in now, built after Android

- **Shared:** the `mobile/` crate (UniFFI Swift bindings, built as an XCFramework). It already covers the API client, Slimproto, the engine, downloads and the analysis client.
- **iOS-specific:**
  - SwiftUI UI;
  - the AVAudioEngine output node;
  - `MPNowPlayingInfoCenter` and `MPRemoteCommandCenter`;
  - CarPlay (templates);
  - background audio mode;
  - the Keychain;
  - `URLSession` background transfers, which could replace the Rust download I/O for reliability.
- **Licensing: GPL plus an App Store exception (D17).**
  - **The problem:** Apple's App Store terms conflict with the GPL, per the FSF. VLC's iOS app was removed over this in 2011.
  - **Option A:** the copyright holder adds an "App Store exception" to the GPL for this project's own code. Every contributor must agree, so it's easiest now, while there is one author. Dependency licences (MIT, Apache-2.0, MPL-2.0) also need checking.
  - **Option B:** licence the mobile apps under a permissive or dual licence.
  - **Option C:** distribute outside the App Store, through TestFlight or EU alternative marketplaces.
  - **Recommendation:** A, decided before the iOS milestone and recorded as a new decision in SPEC.
- **App Store review:** a bring-your-own-server app needs a demo account. The `tools/demo` stand-in LMS (an invented library) can be hosted read-only for reviewers.

## 7. Gateway and protocol additions

| Addition | For |
|---|---|
| `/v1/analysis/track/{id}` (§4) | Waveform, levelling, transitions |
| `POST /v1/plays` (batched, idempotent, signed by device) | Offline play-count sync-back (D6), shared with the web app |
| Capability flags in `/v1/me` | Which optional features this server offers: analysis, transcoding, AudioMuse-AI, Don't Stop The Music |
| "Home" data (recently played and added, per user) | The home screen. Built from LMS queries already allowed by the policy where possible; a small cached endpoint if not |

Every addition follows the standing rules: default-deny, negative tests, fuzzing for any parser, and a security review before release.

## 8. Milestones

Each milestone ends with: tests green in CI, a security review of new surfaces, a release to a closed test track, and a short demo.

| # | Milestone | Outcome |
|---|---|---|
| **A0** | Foundations | `mobile/` crate with UniFFI; Android project skeleton, CI (build, unit tests, lint, Rust cross-compile), signing; sign-in (password, QR pairing), token storage; API client; browse the library; **remote-control** LMS players. *First internal build.* |
| **A1** | The player | On-device LMS player through the Rust engine, with AAudio output and source-rate matching, plus the Media3 session (notification, lock screen, Bluetooth, focus). Background-reliable, gapless, ReplayGain. **Signal-path panel.** |
| **A2** | Audiophile output | USB DAC detection; **bit-perfect** on Android 14+ (runtime-detected); a 64-bit DSP chain with preamp and true-peak limiter; Bluetooth codec display; Opus decoding. Tested against real DACs (measurement plan in §9) |
| **A3** | Analysis and waveform | Gateway analysis service and endpoint; **waveform seek bar**; R128 loudness levelling. The web app also gets the waveform |
| **A4** | Killer UX | Home, search, artist and album pages, art-driven colour, gestures, plugin menus and context menus, queue editing, sync groups, widgets, Quick Settings tile, tablet layouts |
| **A5** | Offline | Download manager, smart downloads, storage management, offline mode, `/v1/plays` sync-back |
| **A6** | Car and discovery | Android Auto; radio and "more like this" through AudioMuse-AI or Don't Stop The Music; smart transitions |
| **A7** | Release 1.0 | Accessibility audit, performance budgets met, privacy policy, a full pentest of app and protocol, Play Store production, an F-Droid build and GitHub APKs (D16) |
| **I0–I3** | iOS | After A5: SwiftUI shell over `mobile/`; AVAudioEngine output; CarPlay; App Store (subject to the §6 licensing decision) |
| **A8+** | After 1.0 | Parametric EQ with device profiles, crossfeed, visualiser, lyrics, Wear OS, DSD (DoP) |

### A0 status (2026-10-11)

Built, in CI (job `android`):
- **`mobile/` crate** (UniFFI 0.31): `Server`, `Session` (password, two-step, pairing, body refresh token, sign-out), `MusicLibrary`, `Players`, `Art`, `Events`; rustls with the platform verifier; integration tests against the real gateway and a fake LMS.
- **Android app** (`apps/android`): modules `:app`, `:ui`, `:data`, `:rust`; screens for server, sign-in, QR pairing, home, albums, album, artists, artist, search, now playing, settings and licences. Remote control only. See `apps/android/README.md`.
- compileSdk is 37 (current AndroidX requires it); targetSdk stays 36 for now.

Not yet: the gateway/web app showing a pairing QR code (the app reads `lyrion-anywhere://pair?…` links, documented in `docs/PROTOCOL.md`; the plugin's pairing code can be typed meanwhile), signed release builds in CI, instrumented tests, Paging-library lists, a disk cache for artwork, and opening pairing links from outside the app.

## 9. Quality: testing audio as seriously as security

**Bit-perfect verification**
- An automated test plays known test signals (e.g. a pseudo-random 24-bit sequence) through the engine. It checks the PCM handed to the output is bit-identical when no DSP is active, at 44.1, 48, 88.2, 96, 176.4 and 192 kHz in 16- and 24-bit.
- On real hardware: a USB DAC with digital loopback, or a capture device, checks bit-perfect delivery end to end on Android 14+.

**Engine tests**
- **Gapless:** sample-accurate track boundaries, the core's existing tests extended.
- **DSP correctness:** the limiter never exceeds its ceiling (true peak); EQ responses match their targets; dither is applied only when bit depth is reduced.

**Behaviour and robustness**
- **Platform lifecycle:** process death, audio focus loss, headphones unplugged, Bluetooth switching, Doze, network handover (Wi-Fi to cellular), and resuming the Slimproto session.
- **Security:** the standing rule applies, including:
  - Keystore use;
  - no tokens in logs or backups (`android:allowBackup` set correctly, backup rules);
  - TLS-only;
  - QR pairing abuse;
  - downloaded files removed on revocation.

**UI and devices**
- **UI tests:** Compose UI tests, screenshot tests (Roborazzi) for themes and sizes, and macrobenchmarks for startup and scrolling.
- **Device matrix:** a few reference phones (Pixel, Samsung, one budget device) plus emulators in CI. One phone with known BIT_PERFECT support is used for A2.

## 10. Decisions

| # | Question | Decision |
|---|---|---|
| N1 | Playback model | **The phone is an LMS player when streaming; local queue offline (D14).** Confirmed. |
| N2 | Minimum Android version | **Android 10 (API 29); bit-perfect on 14+ (D15).** Confirmed. |
| N3 | Distribution | **Free: Google Play, F-Droid, GitHub release APKs (D16).** Confirmed. |
| N4 | iOS licensing | **GPL plus an App Store exception (D17).** Confirmed; exception text added before I0. |
| N5 | Where analysis runs | Recommended: **on the gateway**, CPU-capped and off-switchable; the app falls back to tags. |
| N6 | App name and branding | A working title until a naming and icon pass in A4. |

## 11. Risks

| Risk | Mitigation |
|---|---|
| BIT_PERFECT is optional for device makers; behaviour varies by phone and DAC | Runtime detection, an honest signal path, a curated compatibility list, AAudio fallback |
| Background killing by aggressive OEMs (battery optimisers) | Foreground media service (Media3), in-app guidance on battery-optimisation exemptions, resumable Slimproto sessions (already in the gateway) |
| Analysis load on small LMS hosts (Raspberry Pi) | One job at a time, idle priority, on-demand first, off-switch; results cached forever by stable key |
| UniFFI is pre-1.0 (API churn) | Pin versions; keep the FFI surface small and coarse-grained; generated-code tests in CI |
| App Store and GPL | Decide N4 before any iOS investment |
| Scope creep (a "killer app" invites everything) | 1.0 is §2.1 only; everything else waits behind it |
| Playing via LMS limits features (e.g. crossfade with look-ahead) | Smart transitions run in the engine using LMS's next-track stream (already sent early for gapless). Anything needing more look-ahead uses local-queue mode for downloaded content |

## 12. Sources

- **Android bit-perfect USB:**
  - [Improve audio playback](https://developer.android.com/media/platform/improve-audio-playback) (Android Developers);
  - [Preferred mixer attributes on USB devices](https://source.android.com/docs/core/audio/preferred-mixer-attr) (AOSP).
- **Plexamp reference:**
  - [Plexamp settings (Plex forums)](https://forums.plex.tv/t/how-to-plexamp-4-6-1-settings/724870);
  - [Seekprint/waveform discussion](https://forums.plex.tv/t/disabling-seekprint-waveform-display/884124);
  - [Plexamp 4.11 feature list](https://www.apkmirror.com/?p=7199771).
- **UniFFI:** [UniFFI](https://docs.rs/crate/uniffi/latest), used in production by Mozilla for Firefox on Android and iOS.
- **GPL and the App Store:** [GPLv2 blocks VLC from Apple's App Store](https://www.computerworld.com/article/1347653/gplv2-blocks-vlc-from-apple-s-app-store.html).

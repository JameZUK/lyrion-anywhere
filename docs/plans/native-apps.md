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
│ Android Auto · widgets · dataSync-service downloads  │   │ AVAudioSession · background audio      │
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
| **LMS player** (default when online) | Streaming from your server | As in the web app, the phone is a Slimproto player through `/v1/player`, so LMS drives the queue, gapless playback, sync and plugins. The app declares the codecs it can decode from a stream (`flc`, `ops`, `ogg`, `mp3`), so LMS sends those files as they are. LMS converts the rest, losslessly where the source is lossless: WAV and AIFF (LMS may send `pcm` headerless), and ALAC and AAC in MP4 (a stream with the index at the end can't be decoded without seeking) |
| **Local queue** | Offline, or for downloaded content | The app owns the queue and plays from its download store. Plays are queued and sent to LMS on reconnect (D6) |

Both feed the same engine, so DSP, levelling, the waveform, the signal path and bit-perfect output behave identically. Switching between them (e.g. losing signal mid-album when the album is downloaded) is seamless where the content is available offline.

### 3.3 Audio output, per platform

**Android**
- **Normal output:** AAudio (via Rust's `ndk` crate) in low-latency or power-saving mode as appropriate. The stream is opened at the *source's* sample rate, so the platform only resamples when the hardware can't follow.
- **USB DAC, bit-perfect (Android 14+, API 34):**
  - the app queries `AudioManager.getSupportedMixerAttributes(usbDevice)` and sets `MIXER_BEHAVIOR_BIT_PERFECT` with the source's format and rate;
  - in that mode Android skips mixing, volume and effects, so volume control is the DAC's own (the UI says so) and DSP that changes samples is bypassed or flagged;
  - BIT_PERFECT is optional for device makers, so it's detected at runtime and the signal path shows what was actually achieved.
- **Bluetooth:** the signal path shows the route and the device's name. The codec in use (LDAC, aptX HD, AAC, SBC) can't be shown: `BluetoothA2dp.getCodecStatus` is a system API, and the codec-change broadcast isn't public (checked against the API 37 SDK in A2).

**iOS**
- **Output:** AVAudioEngine with a source node fed from the Rust ring buffer.
- **Sample rate:** `AVAudioSession.setPreferredSampleRate` follows the source on external DACs (USB-C or Lightning). iOS has no exclusive mode; with an external DAC at the matching rate and unity volume, the output is bit-transparent, which the signal path reports honestly.
- **System integration:** background audio mode, CarPlay, AirPlay as an output (marked lossy or resampled in the signal path).

### 3.4 Engine details (Rust)
- **Decode:**
  - Symphonia (FLAC, ALAC, WAV/AIFF, MP3, AAC, Vorbis).
  - **Opus:** done (see §3.5). Symphonia 0.6 demuxes Ogg Opus but still has no decoder, so the core registers the pure-Rust `opus-decoder` crate with Symphonia, behind the `opus` cargo feature.
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

### 3.5 Engine mathematics: implemented in the core

The signal processing and analysis are in `core/` (sans-IO, deterministic, `f64` throughout), so the `mobile/` host only moves buffers. Module docs carry the details; this is the summary.

**Internal format and bit-perfect playback (`core::dsp::pcm`, `core::dsp::chain`)**
- Samples are `f64` scaled to ±1.0. Integer PCM of up to 32 bits converts in and out exactly: an `f64` holds every `i32`, and the scale is a power of two.
- A bypassed stage doesn't touch the buffer, so a chain with nothing active is bit-transparent by construction.
- A harness proves 16-, 24- and 32-bit pseudo-random PCM (with full-scale extremes) comes back identical at 44.1, 48, 88.2, 96, 176.4 and 192 kHz. Widening (16 → 24-bit) and float output are exact too.

**The chain, in order (`core::dsp`)**

| Stage | What it does |
|---|---|
| Gain | Preamp plus levelling. Exactly 0 dB is a true bypass |
| Levelling (`levelling`) | Gain to a target (e.g. −16 or −18 LUFS) from R128 analysis or ReplayGain tags, track or album mode. Attenuation is always applied; a boost is capped so the peak stays under the limiter's ceiling (plus optional limiter headroom) |
| EQ (`eq`, `biquad`) | RBJ-cookbook peaking, shelves, low/high pass and notch, transposed direct form II, recomputed on a rate change. Parses AutoEq `ParametricEQ.txt`, with bounded size, lines, filter count and values |
| Crossfeed (`crossfeed`) | Bauer-style: the opposite channel, delayed, low-passed and attenuated, with unity gain for centred bass. Light, moderate and natural presets |
| True-peak limiter (`limiter`) | Look-ahead brickwall with linked channels. 4× oversampled detection per BS.1770-4 Annex 2, hold and release, and a moving-average attack that reaches each target on time. Latency is reported |
| Output (`dither`) | TPDF dither, optionally noise-shaped, only when the integer depth is reduced (a narrower output, or after processing). Never for float output or untouched audio |

**Outside the chain**
- **Resampler (`resample`, `resample` feature):** wraps `rubato`'s 256-tap sinc in `f64`, used only when the output can't take the source rate.
- **Transitions (`transition`):** sample-accurate equal-power or linear crossfades, and gapless joins.
- **Signal path (`signal_path`):** stage list plus a verdict: bit-perfect, lossless processed, resampled, or lossy source.

**Verified**
- **Limiter:** the output's true peak never exceeds the ceiling. Tested with full-scale square waves, fs/4 sines at 45°, impulses, bursts, steps, rail noise and sweeps, at boosts up to +20 dB, at 44.1–192 kHz, and with randomised signals and settings. It aims 0.05 dB under the ceiling; the worst case measured lands 0.0001 dB above that aim.
- **Hostile samples:** NaN, infinities and huge values are sanitised on the processing path, and a stage that still goes non-finite is reset.
- **Speed (release build, one thread, 44.1 kHz stereo):** the full chain (gain, 10-band EQ, crossfeed, limiter, dither) runs at about 170× real time, and analysis at about 400×.

**Opus (`core::opus`, `opus` feature)**
- **Options considered:**
  - libopus via `opus`/`audiopus`: needs a C toolchain per target and can't build for `wasm32-unknown-unknown`;
  - Symphonia: no Opus decoder in 0.6;
  - pure-Rust decoders.
- **Chosen:** `opus-decoder`, which is MIT/Apache-2.0, has no `unsafe` code, and claims RFC 8251 conformance.
- **What it supports:**
  - mono and stereo;
  - the header's output gain;
  - pre-skip and end trimming, so decoding is sample-accurate.
- **Format code:** Slimproto's `u` is accepted only in builds that can decode it.
- **Web:** first off for WebAssembly (browsers decode Opus natively); later turned on, so the web player decodes Opus in its core too (`wasm/Cargo.toml`). There a panic aborts the player, so the vendored decoder has its own hostile-packet tests (`third_party/opus-decoder/tests/robustness.rs`).

## 4. Waveform, loudness and track analysis

Plexamp's waveform seek bar ("seekprint"), loudness levelling and sweet fades all come from per-track loudness analysis. We do the same, on the server side, so every client (web, Android, iOS) benefits and phones don't burn battery.

**Gateway:** a new analysis service.

| What | Detail |
|---|---|
| Output per track | Waveform peaks (about 1,000 buckets of peak plus RMS, 2–4 KiB); EBU R128 integrated loudness, loudness range and true peak; leading and trailing silence; fade-in and fade-out shape (for smart transitions). Album loudness is aggregated from its tracks |
| Endpoint | `GET /v1/analysis/track/{id}` → compact JSON or a binary form (about 2.1 KiB), with `ETag`; keyed by the stable track key (survives rescans) |
| When | On demand the first time a track is played or shown, plus an optional low-priority background pass. It is CPU-capped (one job, idle priority) because many LMS hosts are Raspberry Pis |
| How | Fetch the original through the existing download path, decode with the core (Symphonia), analyse in streaming fashion (constant memory). Results are cached on disk under the data directory (about 2 KiB per track, LRU), as built in A3 |
| Security | Same rules as downloads: permission checks, library tracks only, no paths in output, limits per user |

**Clients:**
- **Waveform seek bar:** drawn from the peaks, coloured from the album art, with the played portion highlighted and drag-to-seek. While no analysis exists, it falls back to a plain bar and fills in progressively.
- **Loudness levelling:** uses the integrated loudness and true peak (track or album mode; the target is configurable, e.g. −16 LUFS on phones). Without analysis, ReplayGain tags are used.
- **Smart transitions:** use the end and start analysis to choose fade length and shape. They never apply within an album, a gapless sequence or a classical work.

**Implemented in the core (`core::analysis`, for the gateway service):**
- **One pass, constant memory:**
  - a streaming `Analyser` takes decoded PCM at any rate from 8 to 768 kHz, with up to 16 channels;
  - `analyse_encoded` decodes and analyses a file in one call.
- **Loudness:**
  - **K-weighting at any sample rate:** BS.1770-4's 48 kHz biquads are mapped to their analogue prototypes and re-discretised, pre-warped at each filter's natural frequency; the result is exact at 48 kHz.
  - **Integrated loudness:** 400 ms blocks with 75% overlap, gated at −70 LUFS absolute and −10 LU relative. Gating uses a 0.01 LU histogram that keeps the exact power summed in each bin.
  - **Loudness range:** per EBU Tech 3342 (3 s blocks, −20 LU relative gate, 10th–95th percentile).
  - **Also reported:** maximum momentary and short-term loudness.
- **Peaks:** true peak (the limiter's 4× detector) and sample peak.
- **Waveform:**
  - N buckets (1,000 by default) of peak and RMS, quantised to `u8`;
  - built by doubling bucket widths: at most 4N buckets are kept, so the track length needn't be known.
- **Silence and fades:**
  - leading and trailing silence (threshold and minimum length);
  - fade-in and fade-out from the least-squares slope of momentary loudness over the first and last seconds of audio.
- **Output:** a versioned struct (`ANALYSIS_VERSION`), with two encodings:
  - JSON, with the waveform as base64: about 3 KiB for 1,000 buckets;
  - a binary form: about 2.1 KiB.

  Both decoders refuse hostile input (fuzz-tested). Album loudness is the duration-weighted power mean of the tracks.
- **Verified against signals with known loudness:**
  - EBU Tech 3341 steady tones (−18, −23 and −33 dBFS give the same LUFS within ±0.1 at 44.1, 48, 96 and 192 kHz);
  - the gating sequences of Tech 3341 cases 3 and 4;
  - the Tech 3342 loudness-range cases (within ±1 LU);
  - fs/4 inter-sample peaks;
  - BS.1770 channel weighting for mono and 5.1.

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
| `:playback` | A `MediaLibraryService` (since A6, for Android Auto), the `SimpleBasePlayer` adapter, audio focus, the becoming-noisy handler, wake and Wi-Fi locks |
| `:audio` | USB detection and mixer attributes (A2). The AAudio output itself lives in Rust (`mobile/`), so no JNI runs per buffer; the output route reaches the signal path from `:playback` |
| `:downloads` | Built in A5 without its own module: the download manager is Rust (`mobile/src/offline`), its Android side is in `:data` (repositories) and `:app` (a `dataSync` foreground service; no WorkManager, see A5). Storage is app-private internal storage |
| `:auto` | Built in A6 without its own module: the browse tree's grammar and layout are Rust (`mobile/src/auto.rs`), its Media3 side and caller checks are in `:playback` (`playback/auto`) |
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

### A1 status (2026-10-11)

Built, in CI. **Verified on the host and the JVM only: nothing has run on a phone yet** (no device or emulator was available). The list below separates the two.

**`mobile/`: the player engine (`LocalPlayer`, UniFFI)**
- **Engine:** its own thread and single-threaded runtime drive the core's Slimproto player.
  - *Tunnel and streams:* the authenticated `/v1/player` tunnel and `/v1/stream` downloads, with a private HTTP pool.
  - *Reconnect, as the web worker does:* resume the same session (frames counted, a bounded outbox); after the grace period, or 4404/1001, a new session with the same MAC; renew the token on 1008 "unauthorised"; stop on other refusals and takeovers (4000). A broken download resumes at its byte offset, acknowledged every 256 KiB. `BYE!` on stop.
- **Pipeline:**
  - *Bit-perfect by default:* the core's PCM goes straight to the ring. LMS volume at 100 % is exactly 1.0.
  - *Optional DSP from `core::dsp`:* preamp, ReplayGain levelling (track or album, from the file's tags, which the core now reads) and the true-peak limiter, decided per track (`pull_track`).
  - *Resampling:* only if the output refuses the source rate.
- **Ring and STAT:**
  - *Ring:* lock-free SPSC, safe Rust, f32. It supports discard on flush, hold (instant pause), and ducking where 1.0 is a true bypass.
  - *Played frames:* reported as the output consumes them, so elapsed time, `STMs` and `STMu` are right. The ring counts in STAT's output buffer, and underruns are reported once each (`STMo`).
- **Output:** AAudio from Rust on Android (float, stereo, at the source rate, power-saving mode; the callback neither allocates nor locks; a disconnect reopens the stream), or a pull API (`read_pcm`) for hosts and tests.
- **Capabilities:** `flc`, `ops`, `ogg`, `mp3`, and `MaxSampleRate=192000`. Deliberately not advertised, so LMS converts them (to FLAC where lossless):
  - `pcm` and `aif`: LMS can send headerless PCM, which the decoder can't probe;
  - `alc` and `aac`: MP4 files with the index (`moov`) after the audio, a common layout, can't be decoded from a stream (checked against ffmpeg-made files: "fast start" MP4 and ADTS decode, the default layout fails).
- **Signal path:** source, every stage (mono/downmix, server ReplayGain, volume, DSP, resampler, ducking, the system mixer's rate), output, and two verdicts. *Engine bit-perfect* means our path is untouched. *Bit-perfect* means end to end, claimed only for a direct route (A2's USB mixer mode), so on A1 the panel says "bit-perfect up to Android's mixer".

**Android**
- **`:playback` module:** a `MediaSessionService` (not exported) with a `SimpleBasePlayer` mirroring LMS's queue and status for this phone's player (D14). Its buttons send LMS commands. It also handles:
  - audio focus (pause, duck, resume after a transient loss);
  - "becoming noisy" (an instant local hold, then pause through LMS; the hold lifts when LMS next resumes);
  - wake and Wi-Fi locks while playing;
  - the output route for the signal path.
- **App:**
  - a "Play on this phone" switch (Home), remembered; the phone is selected as the player once LMS lists it;
  - a signal-path panel in Now Playing when the phone is the selected player;
  - foreground service type `mediaPlayback`.

**Tests**
- **Rust:**
  - ring: model-based random operations, a two-thread transfer, bit exactness;
  - pipeline: bit-perfect at 44.1–192 kHz, levelling at track boundaries, played-frame accounting with and without resampling;
  - the greeting parser: hostile and random input;
  - ReplayGain tag parsing: negative and random input;
  - signal-path verdicts;
  - integration against the real gateway and a fake LMS: gapless bit-perfect FLAC to the end of the playlist, a 44.1 → 96 kHz change, a mid-stream disconnect resumed with no audio lost or repeated (LMS never sees the player leave), token expiry, refusals (unsupported format, a stream the gateway refuses, `serv`), a barrage of hostile frames, stop and sign-out.
- **Kotlin (JVM, Robolectric):** the LMS → Media3 state mapping, the session player's buttons, the controller, the focus policy, the repository's lifecycle, the signal-path panel and the switch, and the manifest's service, permissions and export rules.

**Verified only on the host and JVM; unverified on hardware**
- the AAudio stream itself: opening at each rate, the deep-buffer path, the callback timing, disconnect and reopen (the code compiles and passes clippy for all three ABIs);
- the Media3 notification, lock screen, Bluetooth and headset buttons;
- real audio focus behaviour, "becoming noisy", the locks under Doze;
- background survival on aggressive OEMs;
- gapless playback through a real DAC.

**Known gaps (later milestones or follow-ups)**
- Elapsed time doesn't yet subtract the device's output latency (AAudio timestamps): a few tens of ms early.
- The service starts while the app is in front. If LMS starts playback on an idle phone whose app is in the background, the media session appears only once the app is opened.
- No settings screen for DSP yet: it is off (bit-perfect), with the engine and API ready. (A3 adds the loudness levelling switch; A2 adds the Sound screen.)
- Volume keys control Android's stream volume, not LMS's, while this phone is the player. (Since 2026-10-13: with a remote player selected and the app in front, they step that player's volume by 5, as other LMS controllers do.)

### A2 status (2026-10-11)

Built, in CI. **Verified on the host and the JVM only. Bit-perfect USB output has not run on a phone or a DAC yet** (no device or emulator was available). The lists below keep the two apart.

**Bit-perfect USB output (`mobile/src/player/negotiate.rs`, `engine.rs`; Android `:playback` `UsbDac.kt`)**
- **Negotiation:** the engine asks the app's `OutputNegotiator` (Android's `AudioManager`) what the output offers before opening its output for a new format:
  - the USB DAC (`TYPE_USB_DEVICE`, else `TYPE_USB_HEADSET`);
  - its `getSupportedMixerAttributes`;
  - or why there is nothing to ask: the setting is off, there's no USB DAC, or Android is older than 14.
- **Choosing a format:** a pure function picks a bit-perfect, stereo offer at the source's rate, in the shallowest integer format that holds the source's depth:
  - a 16-bit source in a 24- or 32-bit container is lossless padding, and still bit-perfect;
  - lossy sources get 24 bits;
  - a DAC shallower than the source is used with dither, and said so.
- **Applying it:** the app sets it with `setPreferredMixerAttributes` (`USAGE_MEDIA`). The engine opens AAudio in that format (`I16`, `I24_PACKED`, `I32`; no performance hint) at the source's rate, and checks it got exactly that.
- **When anything refuses** (the attributes or the stream format), Android's normal mixing is restored and the output opens as float, as before.
- **When the format changes:**
  - a new depth at the same rate switches format at the track boundary;
  - plugging a DAC in or out, or turning the setting off, renegotiates at once;
  - the attributes are cleared when the output closes or the player stops, through the negotiator that set them.
- **Exactness:** the core's decoded `f32` carries 24 significant bits exactly (power-of-two scales), so the integer output is exact for sources of up to 24 bits; 32-bit sources are never claimed bit-perfect. Processed or lossy audio is dithered onto the DAC's grid in the pipeline, so the output only ever converts exact values.
- **Volume:** fixed at 100 % on a bit-perfect route (SPEC D19): LMS's `audg` is ignored there, and the signal path says "use your DAC's volume".
- **Signal path:** `bit_perfect` needs all of these:
  - the attributes applied;
  - the stream opened in their format at the source's rate;
  - nothing in the engine changing a sample.

  It shows the DAC's name, the accepted depth and the fixed volume, and lists every reason it falls short: setting off, no USB DAC, Android 13 or older, not offered, not at this rate, DAC too shallow, source deeper than 24 bits, refused by Android, format refused, DSP, resampling, lossy source, other processing (LMS volume or ReplayGain, ducking, downmix).
- **Bluetooth:** the route and the device's name only (see §3.3).

**Sound settings (Settings → Sound)**
- **The stages:**
  - preamp (±12 dB);
  - loudness levelling: off, ReplayGain track or album, or the server's R128 analysis (A3's switch migrates to R128);
  - the true-peak limiter and its ceiling (−6 to 0 dBTP);
  - a parametric EQ from an AutoEq `ParametricEQ.txt`, pasted or picked as a file (64 KiB cap), checked by the engine's bounded parser;
  - crossfeed (light, moderate, natural);
  - dither: TPDF, noise-shaped, or off. It applies only when depth is reduced, e.g. processed audio to a USB DAC.
- **Defaults:** everything off (bit-perfect), with a "Bit-perfect" indicator that becomes "Processing: …" when any stage is on.
- **Saving:** DataStore, applied live through `DspSettings`. The engine clamps numbers again and drops an invalid or flat profile.

**Opus:** `ops` is advertised only when the `opus` feature is built in (Android builds have it), checked both ways; an Ogg Opus stream plays through the whole player in an integration test.

**Robustness fix:** the streaming decoder (the Android and web players) walks every `RIFF…WAVE` header the prober can see. Before Symphonia parses it, it refuses:
- channel counts that overflow `u16` arithmetic;
- channel masks that need a 32-bit shift;
- RIFF lengths under 4.

This holds in WebAssembly too, where panics abort. Native builds also catch any other decoder panic.

**Tests**
- **Rust:** 632 across the workspace (`--all-features`), including:
  - integer bit-exactness at 16/24/32-bit for 44.1–192 kHz, through the pipeline, ring and integer reads, and with the real decoder (WAV files of known integers);
  - the limit of `f32` for 32-bit sources;
  - negotiation as pure functions, including random offer lists;
  - DSP settings clamping and refusal;
  - EQ text fuzz through the settings path (on top of the core's AutoEq fuzz);
  - the WAV regression: it fails without the fix, and asserts the caught-panic safety net never fires;
  - **end to end against the real gateway with a fake DAC:**
    - the files' integers arrive with LMS's volume at 25 %;
    - 16-bit then 24-bit at the same rate switches format;
    - a refused mixer falls back to float with its reason;
    - DSP on a DAC stays direct, dithered, not claimed bit-perfect;
    - unplugging and replugging mid-track;
    - dropping the negotiator clears the attributes;
  - Ogg Opus through the player.
- **Android:** 122 JVM/Robolectric tests (was 92), including:
  - mixer-attribute mapping and selection as pure functions, including random platform values;
  - the negotiator's refusals under Robolectric;
  - settings persistence across a restart, migration and refusal of stored nonsense;
  - Sound settings to `DspSettings`;
  - the Sound screen and view model;
  - the signal path's reasons and the volume note;
  - the playback module's one permission.
- **Clean:** lint with no baseline; clippy for the host and for Android (arm64); `cargo fmt`; the web app's tests and checks.

**Verified on the host and JVM; unverified on hardware**
- Any real device's `getSupportedMixerAttributes` answer, and whether `setPreferredMixerAttributes` engages for our AAudio stream (AAudio over the bit-perfect output, no performance hint). AOSP documents it for `AudioTrack`; AAudio's legacy path uses one, but this is untested.
- The integer AAudio formats on a device (`I24_PACKED` and `I32` need API 31; the format is checked after opening).
- Bit-perfect delivery itself (§9: a DAC with digital loopback, or a capture device).
- How Android reroutes or disconnects our stream when a DAC is plugged in while playing (the engine renegotiates either way).
- The Sound screen's file picker (Robolectric covers paste, not the system picker).

**Known gaps**
- The bit-perfect attributes are re-applied at each output open, even when unchanged. That costs one binder call per track format.
- A format switch at a track boundary waits for the old track to play out: a short gap where a 16-bit track meets a 24-bit one on a bit-perfect DAC (as with rate changes).
- No hardware volume control: on a bit-perfect route the level is the DAC's own (or its hardware volume, if Android exposes one for it).
- Bluetooth codecs aren't shown (system API only).
- 32-bit integer sources are played at 24 significant bits (dithered), and said so.

**Manual test with a USB DAC:** see `apps/android/README.md`.

### A3 status (2026-10-11)

Built, in CI. **Verified on the host, in a browser and on the JVM; nothing has run on a phone yet.**

**Gateway: `GET /v1/analysis/track/{id}`** (`gateway/src/analysis.rs`; contract in `docs/PROTOCOL.md`, decision SPEC D18)
- Any signed-in user, library files only. Not tied to the download permission: the waveform is for anything you can play (owner decision, 2026-10-11). The original is fetched as for downloads into a temporary file, decoded with the core on a blocking thread and analysed in one pass. A new core entry point (`decode_seekable`, `analyse_source`) reads the whole file seekably, so MP4 with the index at the end works; it has a stop check, a duration limit and a cap on consecutive corrupt packets, and it catches decoder panics.
- **Pending behaviour (decided):** a request waits up to 10 s; after that it gets 202 with `Retry-After: 3` and the job carries on. Requests for the same file join one job.
- **Limits:** one analysis at a time, a queue of 16, 2 jobs and 8 open requests per user (429), 1 GiB and 2 hours (413, before fetching when LMS knows), a 5-minute deadline (504). 413, 422 (corrupt) and 504 are remembered for 10 minutes.
- **Cache:** the binary form on disk (`<data-dir>/analysis-cache`, 64 MiB, LRU), named by the salted stable key, LMS's size and mtime, and the analysis version; ETags and 304s.
- `--no-analysis`, `--analysis-cache-mb`; the plugin's *Track analysis* setting; `/v1/me` reports `features.analysis`.
- FLAC, MP3, AAC/ALAC (MP4), Vorbis, Opus and WAV. Others (AIFF, WMA, APE, WavPack, DSD) are 415.

**Web app:** Now playing draws the waveform under a transparent native range input (keyboard, screen readers and touch unchanged), played part in the accent colour; the plain slider while there is no analysis. Per-session cache, cleared at sign-out.

**`mobile/` and Android**
- `Analysis` (UniFFI) returns ready, pending or unavailable; the core's decoder checks the binary form.
- `DspSettings.analysis_levelling` (off by default). When LMS starts a stream, the engine asks LMS (`status - 2`) which track it is: the current one for a fresh start, the next for a gapless follow-on, the same one under repeat-one, checked against the playlist position. It then fetches that track's analysis. The gain is −18 LUFS minus the integrated loudness, capped by the true peak, with the limiter; ReplayGain tags are the fallback. A fresh start waits up to 1.5 s for the analysis, and that silence isn't reported as an underrun. A gapless track has until the current one ends. The signal path names the source ("(R128 analysis)" or "(ReplayGain)").
- Android: the waveform as the Material slider's track in Now Playing; Settings → "Playback on this phone" → "Loudness levelling".

**Tests**
- **Gateway integration** (fake LMS serving generated WAV and hostile files): success, both encodings, caching and restarts, a damaged cache entry, ETags; 401 (missing, bad, oversized, revoked); users without download permission still get analyses; bad ids, traversal and parameters (404/400, never reaching LMS); streams 404, unsupported types 415; oversize and overlong 413 (with LMS understating both); corrupt and fuzzed audio 422 without a panic; a stalled fetch 504; 202 then 200; per-user and queue 429; analysis off.
- **Robustness:** `analyse_source` on truncated, bit-flipped, header-mangled and random files. This found a Symphonia WAV header overflow, now caught. Also the TypeScript binary decoder (20,000 cases) and the stream-track parser on random JSON.
- **Mobile, end to end against the real gateway:** fetching (ready, pending, unavailable, refused ids, signed out). A −10 dBFS track is levelled by −8 dB from its first sample; without an analysis it plays untouched.
- **Totals:** 605 Rust tests across the workspace (`--all-features`); 286 Vitest plus 24 Node tests for the web app; 92 Android JVM/Robolectric tests; 825 plugin Perl tests plus 11 JS.

**Verified:**
- The web waveform in a real browser against the demo LMS (`tools/demo` now serves generated audio): wide and phone layouts, light and dark.
- Levelling maths and the engine's gain on the host.
- Android code compiled, lint clean, and JVM/Robolectric tests passing (built for x86_64 only locally; CI builds all ABIs).

**Unverified:**
- Levelling heard through AAudio on a phone.
- The Compose waveform on a device (Robolectric only).
- Analysis speed on a Raspberry Pi class host.
- Real LMS answers to `status - 2` at gapless boundaries (shuffle, repeat-all wrap). The fake LMS in the tests returns a fixed status, so only fresh starts are covered end to end. Gapless offsets are covered by unit tests only.

**Known gaps**
- Album-mode levelling uses album ReplayGain tags where present. Album loudness from analyses (`core::analysis::album_loudness`) would need every track of the album and isn't wired in.
- No background analysis pass yet: a track is analysed the first time it is asked for, so the first play of a long track shows the plain bar for a few seconds.
- The analysis thread doesn't lower its own CPU priority (the one-job limit is the protection).
- The web mini player (bottom bar) keeps the plain slider.
- ~~In debug builds, the player's streaming decoder can still panic on the WAV header overflow the fuzz test found.~~ Fixed in A2: the streaming decoder refuses those headers before Symphonia parses them.

### A4 part 1 status (2026-10-11): browse parity, rich pages, player-first

Built, in CI. **Verified on the host and on the JVM (Robolectric); nothing has run on a phone yet.**

**Player-first (decision D20, agreed 2026-10-11)**
- **This phone is the default player.** After sign-in its player is on unless the user turned it off, and Now playing, the mini player and the queue follow it. The A1 "Play on this phone" switch on Home is gone; it became the setting below.
- **No prominent picker.** An output button (next to the volume in Now playing, and in the mini player) opens a **"Play on…" sheet**:
  - this phone, ticked when it is the player;
  - the other players the user may control, each with what it is playing;
  - sync groups.
- **Controlling another player is always visible:** a coloured strip on Now playing and the mini player says "Playing on <name>", with one tap for **Back to this phone**.
- **Default player, per device** (Settings): this phone (default), last used, or a specific player; this phone if that player isn't available.
- The rules live in one small place (`ui/player/PlayerChoice.kt`); screens only ask the player controller to play things.
- **Transfer ("Move playback here", "Bring to this phone") is agreed but not built:** its LMS recipe is still being designed on the web side. Each player row in the sheet already has a slot for that action next to *Control* (`PlayOnActions.rowActions`).

**Browse parity with the web app** (using the same commands, which the gateway already allowed)
- **Library page:**
  - **New Music** first;
  - Albums, Artists, Genres, Years, Compilations, Random Albums and Music Folder;
  - Playlists, Favourites, and Apps & Radio;
  - a virtual-library picker, shown only when LMS has libraries; the choice is remembered on the device.
- **Albums** sort by title, artist, year or date added. **Artists** have role chips (album artists, all, and composers, conductors and bands where the library has them).
- **Genres** (searchable) open with an Albums/Artists switch. **Years** open a year. **Random Albums** come as one batch with "Shuffle again". **Music Folder** uses numeric `folder_id` only; folders and tracks can be played and added.
- **Rich pages:**
  - Home: the library first, with carousels for recently added, random albums, favourites and playlists, plus a compact player card while something is playing.
  - Album: large art, details (year, genre, track count, length, format such as "FLAC 24/96"), numbered tracks with durations, disc sections, and Play, Shuffle, Add and More.
  - Artist: header art, albums grouped as *Albums* (album artist), *Appears on* and *Compilations*, and Play all, Shuffle and More.
  - Playlists and favourites pages.
- **Art-driven colour:**
  - an in-house quantiser (no new dependency) picks the cover's colour;
  - the album, artist and Now playing pages are tinted with it;
  - every foreground is adjusted until it meets WCAG AA contrast on its background.
- **"More…" everywhere** (tracks, albums, artists, genres, years, folders and playlists, from a button or a long press):
  - a bottom sheet with Play now, Play next and Add to queue;
  - then LMS's own `trackinfo`/`albuminfo`/`artistinfo`/… menu, so plugin entries such as "more like this" appear.
  - Its actions run through the gateway's learned-action check, as the web app's do.
- **Apps & Radio:**
  - apps, radio services and plugins from the player's home menu;
  - SlimBrowse menus with search boxes, paging, Play next and Add;
  - suitability judged from each top menu (3 at a time, remembered for 10 minutes): unusable apps are hidden behind "Show all", and apps needing setup say so.

**`mobile/`**
- Typed builders mirroring the web app's, with a `LibraryFilter` (genre, year, artist, compilation, role, virtual library) and new `Selection`s (playlist, genre, year, folder).
- `Menus` resolves SlimBrowse actions exactly as `slimbrowse.ts` does (items' own and base actions with `itemsParams`). It checks command words, keys and values again before sending, and refuses `url` parameters. It also lists plugin entries and judges suitability.
- Also new: `Players::sync_groups` and `play_favourite`. Tracks now carry disc, genre, format and year.

**Tests**
- **Rust unit:** every new builder's shape (matching the web app); hostile filters, item ids and app names refused; parsers for folders (paths cut to their last part), libraries, favourites, playlists, years and sync groups; SlimBrowse resolution, hostile menus and robustness on arbitrary JSON.
- **Rust integration against the real gateway:**
  - every new command passes the allow-list;
  - apps, radio, plugins and app search boxes open;
  - context menus offer the plugin action and never the file's location;
  - forged, altered and never-offered actions, and server administration, are refused by the gateway and never reach LMS;
  - malformed actions never leave the device.
- **Android:**
  - view models: albums with filters and the virtual library, random batches, duplicate pages, role chips, genre search, album discs and details, artist grouping (and a failing role query), Home's carousels, the menu navigator, Apps & Radio suitability and its cache;
  - D20: the start-up rules, the phone waited for (never another player meanwhile), "Back to this phone" (including with "Last used"), and the default-player setting and its persistence (malformed values ignored);
  - Compose: the "Playing on" strip, the "Play on…" sheet, the default-player setting, the Library picker, the album page, the "More…" sheet and Apps & Radio, plus 48 dp touch targets with labels;
  - art colour: contrast for 511 covers in both themes, and the quantiser.
- **Totals:** 150 `anywhere-mobile` tests; 174 Android JVM/Robolectric tests.

**Unverified:** everything on a device. That includes:
- the art tint as rendered;
- bottom sheets with TalkBack;
- real LMS context menus and plugins through the app: AudioMuse-AI, Spotty and so on are covered only by the fake LMS and the web app's experience;
- the phone being chosen at start-up against a real LMS, which lists the player a moment after it connects.

**Known gaps (A4 part 2 and later)**
- ~~Transfer of playback (above).~~ Built in part 2.
- ~~Queue editing; syncing and unsyncing players from the sheet (groups are listed only); widgets, Quick Settings tile, gestures and tablet layouts.~~ Built in part 2.
- Playlist editing (admins).
- Search results don't yet carry art or "More…" for albums and artists.
- "Shuffle" leaves LMS's shuffle mode on, as LMS's own clients do.

### A4 part 2 status (2026-10-11): transfer, queue editing, sync, tablets, widget, tile, gestures

Built. **Verified on the host and on the JVM (Robolectric); nothing has run on a phone or tablet yet.**

**Transfer (SPEC D20; docs/PROTOCOL.md › Transfer)**
- The same recipe as the web app: group then leave, with "copy the list" as the fallback, and every refusal it documents (same player, empty queue, radio or app items, over 500 entries or a short read, LMS won't group).
- **Planning is pure Rust** (`mobile/src/transfer.rs`): the source's status in, command steps out (`plan_group`, `plan_copy`, `grouped`).
  - The runner is in Rust too, behind a small `Rpc` trait, so it is unit-tested against a scripted LMS.
  - It reports its stage (reading, grouping, starting, copying), takes the target out of the group again after any failure once it has joined, and checks player ids before anything is sent.
  - `Players::transfer` exposes it to Kotlin.
- **Android:**
  - every other player in *Play on…* has **Move playback here**;
  - the phone's row, and the "Playing on …" strip, have **Bring to this phone**;
  - progress shows as a line with a progress bar; refusals and errors stay, with the `AppError` detail, until dismissed;
  - a move that works controls the target and closes the sheet;
  - "Bring to this phone" before the phone is an LMS player starts it and says so.

**Queue editing** (as the web app's `queue-edit.ts`):
- swipe a row left to remove it, with **Undo** in the snackbar (library tracks only; Undo re-adds then moves it back, and says so if the queue changed meanwhile);
- long-press the handle and drag to reorder;
- **Clear queue** after a confirmation dialog;
- tap to jump to a track;
- TalkBack users get Move up, Move down and Remove as actions on each row.
- Edits show at once, run one at a time, and are put back with the error's detail if LMS refuses. A queue read that lands mid-edit doesn't undo what is shown.
- `Players` gained `remove_from_queue`, `move_in_queue`, `clear_queue` and `restore_to_queue`.

**Sync groups:** each player in *Play on…* has a link toggle, "Play in sync with <controlled player>". `Players::set_synced` sends `<selected> sync <other>` to join, and `<other> sync -` to leave, the direction fixed on the web today. The sheet shows the change at once and puts it back if refused.

**Tablet layouts** (`ui/components/Adaptive.kt`, Material's 600/840 dp breakpoints):
- **Expanded:** Albums, Artists and Playlists show the chosen item beside the list; Now playing puts the queue beside the controls.
- **Medium:** one pane with the navigation rail, except Now playing in a window under 480 dp tall (a phone on its side), which also gets two panes.

**Home-screen widget:** artwork, title, artist and player, with play/pause and next.
- It shows this phone's player while it plays, otherwise the player the app last controlled.
- It is fed only from state the app already has (`NowPlayingHub`: the playback service for the phone, the UI for the controlled player).
- It is composed once per change with `GlanceRemoteViews` and handed to the launcher: no Glance session, WorkManager job, alarm or periodic update (`updatePeriodMillis` 0), so it never wakes the phone.
- Buttons go through the same repository (and the gateway's policy); player ids are checked first.

**Quick Settings tile:** play/pause for this phone's player; unavailable when the phone isn't a player. It updates only while the panel is open.

**Gestures:** swipe the mini player up to open Now playing; pull Now playing down from the top of its list to close it; swipe the artwork left or right for next or previous, with a haptic tick (also offered as TalkBack actions).

**Tests**
- **Rust unit:**
  - both transfer plans, step by step;
  - every refusal, including hostile queue entries (negative, string and missing ids, `remote`);
  - chunking at the gateway's 200-character limit;
  - lenient status parsing;
  - the runner: order, fallback, clean-up after a failure, and a refused target stopping at the first step;
  - queue-edit and sync builders and the sync direction;
  - Undo's add-then-move, with a lagging or meddled queue.
- **Rust integration (real gateway, and a fake LMS that now models sync groups):**
  - a full transfer each way by grouping, and by copying when LMS won't group;
  - a target or source the account may not control is refused (403), and nothing naming it reaches LMS;
  - every step of both recipes passes between permitted players and is refused for another account's player (mirroring the gateway's `transfer_recipe_needs_both_players`);
  - queue edits and sync pass the allow-list, and are refused for others' players.
- **Android:**
  - transfer view-model states (stages, no overlap, refusals, errors, the phone not ready);
  - queue editing (optimistic, failure put back, Undo, an old snackbar's Undo ignored);
  - sync (direction, optimistic, put back);
  - the widget's and tile's models and buttons (malformed player ids never sent);
  - layout selection by size;
  - gesture thresholds and pull-to-close;
  - Compose: the sheet's actions, progress and error detail, swipe to remove, drag handles' accessibility actions, the clear dialog, swipes on the artwork and mini player, pull-down to close, and the two-pane layouts;
  - the manifest test now allows exactly the widget receiver (update broadcast only) and the tile (system-bound) beside the launcher activity.

**Unverified:** everything on a device. That includes:
- the gestures' feel and their thresholds, and haptics;
- the widget on real launchers (sizes, dynamic colour), and the tile's refresh;
- tablets and foldables;
- transfer against a real LMS from the phone (the web app's recipe was measured against LMS 9.1).

**Known gaps**
- The widget shows what the app last knew: with the phone idle and the app closed, a remote player's state isn't followed (by design, nothing polls).
- Drag to reorder moves one row at a time visually (the others don't make room while dragging); the drop is right.
- Album and artist pages opened from a detail pane open full screen.
- Playlist editing (admins); search results without art or "More…" for albums and artists.

### A5 status (2026-10-11): offline

Built. **Verified on the host and on the JVM (Robolectric); nothing has run on a phone yet.**

**`mobile/`: downloads, the offline index and plays (`mobile/src/offline`)**
- **`OfflineLibrary`** (no network): what is on the device for one user on one server.
  - Folder: `<app-private root>/<hash of server and user>/`; files are `tracks/<stable key>.<ext>` (a fixed list of extensions), partial downloads `partial/<key>.<format>`, covers `art/<album>.img`. Nothing names a user, server, path or token.
  - The index (`index.json`) lists key, track id, title, artist, album, album id, number, duration, format, codec, extension, size, date and origin (manual or smart). It is bounded (100,000 tracks, 96 MiB), every entry is checked (malformed ones dropped, text cleaned and cut), and it is written atomically with the previous version kept as a backup.
  - On opening, entries whose file is missing or the wrong size, and files no entry names, are removed (crash clean-up).
- **`Downloader`** (its own thread): the web app's manager, natively.
  - Two at a time (the gateway allows three per user); `GET /v1/download/track/{id}/info` then the audio, resumed with `Range` from the partial file's length.
  - Integrity: the stable key names the file; `/info`'s size, `Content-Length` and `Content-Range` must agree; a body may not run past them; a short body resumes rather than being kept; a 416 or a `Content-Range` starting elsewhere starts again.
  - 401 renews the token once and retries; 403 fails for good and is reported (`forbidden`) so the app says so; 404 is "not downloadable"; 429 waits as `Retry-After` says (not counted as a failure); other failures back off (2 s … 60 s, five attempts).
  - Pause, resume and cancel (one or all); the queue survives a restart (`queue.json`); progress reaches the app through `DownloadObserver`.
  - Policy: the format by network ("auto": Original on Wi-Fi, Opus on mobile data by default), *Wi-Fi only*, and a storage cap: the user's own downloads remove smart downloads (oldest first) to make room, never the reverse.
  - The album's cover is cached once per album (`music/<track>/cover`, 600 px; only real images are kept).
- **Plays** (SPEC D6): each play heard offline of at least 5 s (track id, the download's stable key, start time, seconds heard, a random UUID as idempotency key) goes into `plays.json` (bounded, durable). `sync_plays` sends them in batches of 100 to `POST /v1/plays` (`docs/PROTOCOL.md` › Offline plays) and reads each play's `recorded`/`duplicate`/`refused`/`retry` and the server's `retry_after`; a 404 (an older server) keeps them. The server applies the counting rule and, with the stable key, refuses a play whose id a rescan gave to another file. The sender is behind a trait (tested with a fake) and is also tested against the gateway's real endpoint.
- **Smart downloads:** `plan_smart` (pure) compares what the lists hold with what is on the device; `MusicLibrary::favourite_library_items` finds library tracks and albums among the LMS favourites (SlimBrowse form, one folder deep).

**`mobile/`: local-queue playback through the same engine (`mobile/src/player`)**
- The engine's audio half (pipeline, ring, output, bit-perfect negotiation, signal path) is now `AudioPath`, fed through an **`AudioSource`** trait by either the core's Slimproto player (LMS stream) or **`LocalSource`** (downloaded files). DSP, levelling, bit-perfect USB output and the signal path are therefore identical.
- `LocalSource` decodes files with the core's new **`FileDecoder`** (seekable, sample-accurate, the WAV header check first, panics caught), in per-track segments; the next track is opened and decoded ahead, so tracks of the same format play gaplessly (a change of rate or format reopens the output at the boundary, as when streaming). Missing or broken files are skipped and reported.
- `PlayQueue` (pure): play order, shuffle (the current track stays first) and repeat (off, all, one, looped gaplessly).
- **`OfflinePlayer`** (UniFFI): `play_tracks(keys, start, shuffle)`, play, pause, next, previous (restart within 3 s), jump, seek, shuffle, repeat, stop; the same hold, duck, route, negotiator and pull-PCM calls as `LocalPlayer`. Plays are recorded in the library as tracks end, skip or stop (a seek doesn't start a new play).

**Android**
- `:data`:
  - download settings (formats, Wi-Fi only, cap, smart) and the offline owner in DataStore;
  - `NetworkMonitor` (Wi-Fi means unmetered);
  - **`RustOfflineRepository`**:
    - opens the signed-in user's library, or offline the last user signed in on this server;
    - runs the `Downloader` while signed in;
    - removes everything when the server revokes the device;
    - at sign-out sends plays first, then keeps or removes the downloads and forgets the owner;
    - refreshes smart downloads on Wi-Fi (at most six-hourly, or when their settings change; all or nothing);
    - sends plays when online (`PlaysSender`: at once, then after the server's `retry_after`, or 1, 5, 15 and 60 minutes; hourly for an older server).
  - **`RustOfflinePlayerRepository`** (the engine starts on the first play, stops when the library closes);
  - **`OfflineModeController`** (offline when the saved sign-in can't be checked, or, signed in, after 3 s without a network or 20 s of this phone's player failing to reach the server; retries every 30 s and at once when the network returns);
  - **`PlaybackArbiter`** (downloads and this phone's LMS player never play at once).
- `:playback`: **`OfflineSessionPlayer`**, a `SimpleBasePlayer` over the local queue (play order, cover from the download store, shuffle and repeat commands). `PlaybackService` hands the session to it while a local queue is loaded, so the notification, lock screen and Bluetooth buttons work offline; focus, becoming noisy, the route and the USB DAC negotiator apply to both engines.
- `:ui`:
  - *Download* on album and playlist pages (marked when all or some tracks are here) and in "More…" for tracks, albums and playlists;
  - the **Downloads** screen (in progress with progress bars, pause, resume or retry and cancel, each or all; downloaded albums; storage used against the cap; remove and remove all, both confirmed);
  - offline album, mini player and Now playing;
  - **Settings → Downloads and storage** (cap, Wi-Fi and mobile formats, Wi-Fi only, smart downloads, refresh now);
  - the sign-out question (keep or remove downloads);
  - covers from the download store through the image loader (`offline:` keys).
- `:app`:
  - **offline mode**: the downloaded library, played on the phone, under an *Offline* banner with *Try again* (and *Sign out* before sign-in); back to the app by itself when the server returns;
  - **`DownloadService`**: a foreground service of type `dataSync`, running only while downloads are busy, with a progress notification and *Cancel*. On Android 15's daily limit (`onTimeout`) downloads pause and the service stops. Started only while the app is in front (Android refuses otherwise; downloads then carry on while the process lives).
  - Notification permission is asked for at the first download (Android 13+).
- **Manifest:**
  - new permissions: `FOREGROUND_SERVICE_DATA_SYNC` and `POST_NOTIFICATIONS` (app), `ACCESS_NETWORK_STATE` (`:data`, normal);
  - new component: `DownloadService` (not exported);
  - no new providers, start-up initialisers or exported components (A5 adds no Android dependency). The release APK, checked with `aapt2 dump xmltree`, has one provider, `androidx.startup.InitializationProvider` (not exported), with three initialisers: `EmojiCompatInitializer`, `ProcessLifecycleInitializer` and `ProfileInstallerInitializer`. WorkManager's initialiser stays removed.
- **Storage and backup:** downloads live under `noBackupFilesDir/offline` (app-private, never backed up or transferred; the app also disables backup).

**Tests**
- **Rust unit:**
  - `FileDecoder`: exact frames, sample-accurate seeks, hostile, truncated and random files;
  - the index: round trip, malformed entries, cleaning, unreadable files, a fuzz test (3,000 cases);
  - the store: scopes, persistence, crash clean-up, the backup index, removal, covers, durable plays;
  - plays: keys, bounds, file damage, verdicts, batches, an older server, `retry` not looping;
  - `/info` parsing, `Content-Range`, refusal classification, backoff, the saved queue, stray partial files, smart planning;
  - `PlayQueue` (including random operations) and `LocalSource` (gapless, a rate change, missing and broken files, seek, next, previous and jump, repeat-one, shuffle, pause and stop).
- **Rust against a scripted gateway:** resume after a cut (`Range`), a 416, a misplaced `Content-Range`, a short `Content-Length`, a chunked body longer than `/info` says, 401 renewal, 403 (reported, never retried), 404, 429 with `Retry-After`, pause and cancel, the storage cap, Wi-Fi only with the format following the network, the queue across a restart, plays sent once and kept by an older server, and no token in any file name or the index.
- **Rust against the real gateway:**
  - a download through `/v1/download/track/{id}`, and `Range` resume through it (a marked partial file proves only the rest was fetched);
  - an unknown id (404), downloads turned off (403, shown) and on again;
  - a revoked device stops downloading;
  - favourites read through the allow-list;
  - plays through `POST /v1/plays` (with the fake LMS plugin's `play_record`): one recorded, one refused because its stable key names another file, one too short; a resent key is a duplicate and never counted twice.
- **The offline player end to end** with real FLAC files: bit-exact gapless output across tracks with one output run, the signal path and DSP, seek, next, previous, jump, pause, shuffle, repeat, stop, a vanished file skipped and reported, unknown keys refused, a gapless repeat-one loop, and plays recorded.
- **Android:**
  - settings persistence and refusal of stored nonsense;
  - the offline repository (sign-in, offline owner, revocation, sign-out, downloads, smart downloads, plays sending, another user);
  - the offline player repository;
  - the plays sender's timing;
  - offline-mode decisions and timing;
  - the arbiter;
  - the Media3 mapping and session player;
  - the view models;
  - Compose: the Downloads screen, offline library, album and Now playing, storage settings, the sign-out question, the Download button;
  - the notification's progress;
  - the manifest (the download service is private and `dataSync`; WorkManager's initialiser stays removed; the expected permissions only).
- **Totals:** 243 `anywhere-mobile` tests (was 180); 116 `anywhere-core` library tests (was 112); 295 Android JVM/Robolectric tests.

**Unverified:** everything on a device. That includes:
- the `dataSync` service and its notification;
- Android 15's time limit;
- the offline Media3 session with real Bluetooth and lock-screen controls;
- AAudio playing downloads;
- offline-mode switching on real network changes;
- plays reaching a real LMS through the plugin (tested only with the fake plugin).

**Known gaps**
- Plays are sent with the track id they were recorded with: there is no key-to-id lookup yet, so after a rescan that renumbered the track the server refuses the play (`track_changed`) rather than counting it.
- *Keep favourites offline* finds only favourites LMS identifies by library id. Favourites saved by file location can't be resolved through the gateway (it strips URLs, rightly), so many may be skipped; a gateway lookup would fix it.
- Offline playback uses ReplayGain tags for levelling: R128 analyses aren't cached for offline use.
- The widget and the Quick Settings tile don't show downloads playing.
- "Continue on <player>" when back online: built 2026-10-14 (SPEC §8.4), verified on the JVM and against the real gateway in the Rust tests only. A track whose id a rescan renumbered is skipped, as there is no lookup by key.
- Not built: ratings sync-back, refreshing a download when its library file changes, and a free-space check before downloading (a full disk fails the download cleanly).
- While signed in with a network but an unreachable server, offline mode relies on this phone's player failing to reconnect, so it needs *Play on this phone* on.

### A6 status (2026-10-11): car and discovery

Built. **Verified on the host and on the JVM (Robolectric) only. Android Auto has not been tried (no car, head unit, emulator or Desktop Head Unit was available), and nothing has run on a phone.**

**Android Auto (`:playback` `playback/auto`; the grammar in `mobile/src/auto.rs`)**
- `PlaybackService` is now a Media3 `MediaLibraryService` and **exported** (Android Auto binds to it from outside the app), with the `MediaLibraryService` and `MediaBrowserService` actions only.
- **Who may connect** (`CallerPolicy`, decided in `onConnect` per uid and package, and checked again on every library call):
  - *browse and play:* Android Auto and the Google assistants **only with their release certificates** (SHA-256, both of Android Auto's release keys, as Android's media sample lists them; debug keys excluded); anything platform-signed, on the system uid or holding `MEDIA_CONTENT_CONTROL` (system UI, Bluetooth, Android Automotive's media centre); this app;
  - *transport only* (no library commands, no setting media items): other controllers Media3 trusts, such as a notification listener the user enabled;
  - *refused:* everyone else; a package that doesn't belong to the calling uid; a known package name with the wrong certificate. In safe mode (after a crash) the service builds nothing and accepts no one.
- **The tree:** tabs *Home* (carry on, new music, favourites, playlists), *Library* (new music, albums, artists, playlists, favourites) and *Downloads*; offline, *Downloads* only. Albums and playlists open to their tracks; favourites to their folders and stations. Search covers the library and the downloads; a spoken query (no id) plays the best match, and none at all carries on.
- **Ids:** the Rust grammar (`al/7`, `at/7/3`, `fv/b99447ab.1`, `da/a42`, …) with numeric ids in the gateway's range, favourite ids in its item-id alphabet and download references in their own form; it is fuzzed, and whatever parses is exactly what the app writes. Every id coming back from the car is parsed before anything is looked up. Items carry an id and text only: no URI, artwork URI or extras. Play requests are rebuilt from the id (any URI the caller sent is dropped).
- **Playing** goes to this phone (D20): its LMS player is switched on if it was off (the setting the app's switch uses), and the request waits up to 15 s for it to connect. Downloads play from the app's own queue, online or not. Both session players take the request (`PlayRequests`), and Media3's following "play" is absorbed so LMS isn't told to play the old queue first.
- **Cold start from the car:** the app's start-up runs as usual (no new initialiser); the tree waits up to 8 s for the saved sign-in before deciding online or offline.
- **Artwork:** the now-playing cover is sent as bytes, as before. Browse items have none: the car loads artwork by URI, which for an app-private, authenticated image needs a content provider (exported, or granted per URI to the car's package). That isn't strictly needed for a working tree, so none was added; it is the main gap below.
- `res/xml/automotive_app_desc.xml` (`<uses name="media"/>`) and the `com.google.android.gms.car.application` metadata. The launcher activity also takes `MEDIA_PLAY_FROM_SEARCH` (voice "play … on Lyrion Anywhere", which lint asks for with a media library service); it reads only the query text, bounded and cleaned, and uses the same search.

**Manifest, checked on the release APK** (`aapt2 dump xmltree`, against the alpha.8 release APK built from `main` before this work):
- `PlaybackService`: `exported` false → **true**; its action `androidx.media3.session.MediaSessionService` → `androidx.media3.session.MediaLibraryService` and `android.media.browse.MediaBrowserService` (what Android Auto binds with). No permission: Android Auto must be able to bind, so callers are checked in code (above). Lint's `ExportedService` warning is suppressed on this one service for that reason.
- `MainActivity`: a second intent filter, `android.media.action.MEDIA_PLAY_FROM_SEARCH` with `DEFAULT` (lint requires it of a media library app; it reads only the query).
- Application metadata: `com.google.android.gms.car.application` → `@xml/automotive_app_desc`.
- Unchanged: permissions (none added), providers (still only `androidx.startup.InitializationProvider`, not exported, with the same three initialisers: `EmojiCompatInitializer`, `ProcessLifecycleInitializer`, `ProfileInstallerInitializer`; WorkManager's stays removed), every other component and its export flag. No dependency was added (Media3's library service is in `media3-session`, already used).

**Discovery (`mobile/src/discovery.rs`; Android `data/Discovery.kt`, `ui/discovery`)**
- **AudioMuse-AI:** its entries are picked out of the item's own context menu (`trackinfo`/`albuminfo`/`artistinfo`), by command word (`audiomuse…`) and kind: an entry that runs (or a list that can also be played) is a *mix*; a list is *similar music*; a lone AudioMuse-AI submenu is looked inside once. They run exactly as offered, through the gateway's learned-action check.
- **Don't Stop The Music:** its settings menu (`dontstopthemusicsetting`) is read and parsed strictly (only entries setting exactly the provider preference to a well-formed token). Radio keeps the provider the player already uses; if off, it switches it on, preferring AudioMuse-AI's provider, then a similarity one, then anything but random.
- **"Start radio"** (in "More…" for tracks, albums and artists, and on album pages) plays the AudioMuse-AI mix, else the item; with Don't Stop The Music it never ends. With neither, the action is hidden. **"More like this"** is AudioMuse-AI's similar list, as a shelf on album pages and in a track's "More…". Answers are remembered for ten minutes per connection, player and item.
- **Gateway:** two fixed allow-list entries, for permitted players only: `dontstopthemusicsetting` (exactly), and `playerpref plugin.dontstopthemusic:provider <token>|0|?` (no other `playerpref`). Providers named after an admin-only plugin are removed from the menu and refused.

**Smart transitions (`mobile/src/player/transitions.rs`; Settings › Sound › Transitions)**
- **Off by default:** nothing passes through the new stage and playback stays gapless and bit-perfect (the existing bit-exact tests are unchanged and pass).
- **How:** the end of the current track (the crossfade length, plus 5 s for smart trims) is held back between the source and the pipeline, so a stream's end is in hand when the next track's audio appears, without knowing lengths. At the boundary a pure planner decides:
  - *gapless* for the same album (consecutive tracks of an album, repeat-one, shuffled tracks of one album), an unknown album or track, a change of sample rate, or tracks too short;
  - *smart* (with both tracks analysed): trailing and leading silence trimmed (up to 5 s each), and the overlap follows the outgoing track's fade-out, else the incoming fade-in, else a 2 s blend, never longer than the setting;
  - *fixed* (the setting, 1–10 s, default 6) for *Crossfade*, or *Smart* without analysis.
  - Never more than half of either track; hostile analysis values are tamed.
- **Mixing:** the core's sample-accurate equal-power mixer, in `f64`. The outgoing tail is corrected for its own levelling gain (the pipeline applies the incoming track's). Every source frame is reported as played exactly once (a mixed block carries both tracks' frames, trimmed silence rides with the next block), so LMS's elapsed time, track starts and the offline play log stay right.
- **Never at the cost of a drop-out:** while the hold waits for the next track or its facts, the ring keeps playing; if it runs low the boundary goes gapless.
- **Sources:** the LMS stream (the engine now asks LMS for the streamed track's album and duration with `status - 2 tags:xed`, and for smart transitions fetches the analysis' silence and fades, which `TrackAnalysisInfo` now carries) and the offline queue (album from the download's index; no analysis offline, so smart uses the fixed crossfade).
- **Signal path:** a "Crossfade 6 s (not within albums)" or "Smart transitions, up to 6 s (not within albums)" stage, and a new reason, `Crossfade`, why it isn't bit-perfect.

**Tests**
- **Rust:**
  - transition planning as pure functions (off, album rule, unknown, rate change, fixed, smart trims and natural lengths, hostile values, 20,000 random cases);
  - the crossfader against scripted sources: bit-exact pass-through when off, the mix equal to the core's whole-track join, the equal-power curve, album-gapless, trims counted as played, levelling correction, waiting for facts without starving, a late next track, reset, turning off mid-play, a short incoming track, and 60 random scripts never losing or inventing frames;
  - the offline player end to end with real FLAC: a 1 s crossfade between albums (exact length, untouched either side, the mix in the overlap, both plays recorded) and bit-exact gapless within an album with smart transitions on;
  - `stream_track` (album and duration, hostile values), the analysis record's edges, the signal path's crossfade stage and reason;
  - discovery: AudioMuse-AI offers, other plugins ignored, URL-carrying actions never offered, Don't Stop The Music parsing (hostile tokens, fuzzed), provider choice, the commands; through the real gateway: options, the shelf, starting radio (exact commands reaching LMS), forged plans refused, another account's player refused;
  - the browse tree: ids round-trip, nodes outside the grammar get no id, over 40 hostile ids refused, 50,000 random ids, layout, paging;
  - gateway: the Don't Stop The Music entries (allowed shapes, permitted players only, over 20 refused variants), admin-only providers hidden and refused, and both end to end through `/v1/rpc` against the fake LMS.
- **Android:**
  - `:playback`: the caller policy (8 cases including impostors and a package claimed by another uid), its package-manager reading under Robolectric (4), the library callback (5: connection results per caller, library calls and play requests refused for others, ids stripped), the browse tree with fakes (12), and play requests on the session player (2);
  - `:app`: the car's source (5) and the manifest (the media library service is the only new exported component, with only its two actions; the car metadata; voice search on the activity only);
  - `:ui`: radio availability and starting, the shelf, their Compose views (11), and the transitions setting (3);
  - `:data`: transitions' persistence and mapping (2).
- **Totals:** 289 `anywhere-mobile` tests (was 243); 116 `anywhere-core` library tests (unchanged: the core didn't change); gateway 216 library and 21 `/v1/rpc` integration tests (2 new each); 348 Android JVM/Robolectric tests (was 295).

**Unverified:** everything on a device or in a car. That includes:
- Android Auto itself: the tree, search, voice, playing from the car, and whether Android Auto and the assistants are recognised on real phones (their certificates are pinned from Android's published list, so a key rotation would need an update);
- Android Automotive OS (the app isn't built for it; the policy would admit its media centre);
- AudioMuse-AI's real menu shapes (only the fake LMS's and the gateway's learned-action rules were exercised) and Don't Stop The Music against a real LMS;
- crossfades heard through AAudio, and crossfades on the LMS stream at real boundaries (the planner and mixer are tested on the host; the LMS path's facts lookup is not exercised end to end against a fake LMS).

**Known gaps**
- No artwork in the car's browse lists (needs a content provider; deliberately not added).
- No "Apps & Radio" or LMS plugin menus in the car.
- Transitions can't see a "gapless sequence" or classical work across albums (the album is the unit), and a boundary whose album is unknown stays gapless.
- Album-mode levelling across a crossfade uses each track's own gain, not the album's.
- A crossfade waits for the next track's facts only while the output has audio in hand (about 0.2 s); a slow lookup makes that boundary gapless.
- Radio from Don't Stop The Music leaves its provider switched on for the player afterwards (as choosing it in LMS would).
- "Start radio" is offered for tracks, albums and artists only; the car has no radio action.

### A7 status (2026-10-11): release readiness

Built: everything for a 1.0 except choosing the version, which is the owner's call. **Verified on the host and the JVM (Robolectric) only; nothing has run on a phone, and no measurement against the performance budgets has been taken yet.**

**Accessibility (audit and fixes)**
- **TalkBack:**
  - The queue rows' Move up, Move down and Remove actions, and the artwork's Next and Previous, now sit on the node TalkBack focuses. Before, they were on unfocusable wrappers, which the old tests didn't notice.
  - Home cards are buttons, not "not selected" items.
  - Sign-in buttons keep their name while busy, and errors are announced and tied to their fields.
  - The current track is said ("Now playing") and marked by an icon, not by colour only.
  - Toggles have constant names with a state: power, offline shuffle and repeat, sync ("Play X in sync with Y").
  - The limiter ceiling has its own name, and "See all" says what it opens.
  - Live regions on errors, headings on top bars and dialogs, merged signal-path lines, a pane title for tablet details, and a stated state on the Quick Settings tile.
- **Contrast:**
  - the waveform's unplayed bars are at least 3:1 against the background;
  - the art-tinted accent is at least 4.5:1 on its container, and the artist link is underlined;
  - the art tint gives way to the system's high-contrast setting (Android 14+).
- **Font scale 200 %:**
  - top bars ellipsise;
  - the "More…" sheet's buttons, the "Playing on" strip and the offline banner wrap instead of squeezing their text;
  - track numbers grow;
  - dialogs scroll;
  - the widget drops its second line at large font sizes.
- **Keyboard and D-pad:**
  - queue rows: Alt+Up and Alt+Down move a row, Delete removes it;
  - albums: the Menu key (or Shift+F10) opens "More…";
  - Search closes the keyboard;
  - sign-in fields take focus first, and renaming submits with Done;
  - Space plays and pauses in Now playing.
- **Automated checks (Robolectric):**
  - a semantics walker (`A11yAudit.kt`) applied to the main screens: every clickable has a label and a 48 dp target; custom actions are on focusable nodes; no button claims to be selectable; toggles and sliders are labelled;
  - font-scale-200 % tests;
  - contrast tests for the brand schemes, the art-tinted schemes over random covers, and the waveform.

**Performance** (`docs/PERFORMANCE.md`)
- **Budgets defined:**
  - cold start ≤ 500 ms to first frame;
  - scrolling P90 ≤ 12 ms with under 1 % jank;
  - memory: ≤ 160 MB browsing, ≤ 200 MB playing;
  - no underruns while browsing;
  - size: ≤ 20 MB per-ABI APK, ≤ 40 MB universal.
- **Build:**
  - R8 and resource shrinking (already on, checked);
  - a hand-written baseline profile covering the app's own code;
  - the Rust library in its own `android-release` profile (LTO, one codegen unit), with build paths remapped;
  - per-ABI APKs beside the universal one (`-Panywhere.splitApks=true`), with the universal APK limited to the built ABIs.
- **Still to do:** generating a measured profile needs a device; the procedure is documented.

**Privacy:** `docs/PRIVACY.md`, linked from Settings › About › Privacy policy. It covers:
- what the phone keeps;
- what the gateway keeps, and for how long;
- that nothing goes to the project;
- deletion;
- Google Play's Data safety answers.

**Security review and pentest** (`docs/security/pentest-2026-10-android-1.0.md`). Seven findings and two hardening changes, every fix with a regression test:
- **F1:** crash reports redacted. They had included memory from Android's native-crash record.
- **F2:** self-renames rate-limited and unchanged names not written; renames audited with their actor; the audit log bounded (one year, 100,000 entries).
- **F3:** plays of one track closer together than the counting time are refused as `overlap`. Before, fresh keys inflated play counts.
- **F4:** every plugin admin command needs the settings page's token, reads too. They expose remote users' addresses.
- **F5:** accepted and documented: Don't Stop The Music providers of admin-only plugins are recognised by name only.
- **F6:** analysis time-outs are remembered only briefly.
- **F7:** no build paths in the native library.
- **Hardening:**
  - stored credentials never print their token;
  - `sync` to the same player in another case is refused.
- **Accepted by design:** the refresh-token reuse grace (SPEC D21), among others listed in the report.

**Distribution** (`docs/RELEASING.md`)
- **Google Play:** `bundleRelease` (signing from the environment), and store metadata under `apps/android/fastlane/metadata/android/en-GB/` (title, descriptions, changelog, icon). The screenshot procedure is documented.
- **F-Droid:**
  - every dependency checked: free software, with no Play Services;
  - the one prebuilt artefact, rustls-platform-verifier's Kotlin half, can be built from source with `-Panywhere.verifier.source` (new `:verifier-source` module, checked against the published one);
  - `-Panywhere.cargo` for the build server's Rust;
  - a draft recipe and srclib in `apps/android/fdroid/`;
  - reproducible-build notes.
- **GitHub:** the `android-release` CI job runs on `android-v*` tags only (the plugin's job takes `v*`). It:
  - builds from scratch;
  - signs with the repository's secrets;
  - checks signatures and that no build paths are in the native libraries;
  - publishes a pre-release with the universal and per-ABI APKs, the app bundle, `BUILDINFO.txt` and `SHA256SUMS`.

**Manifest:** no change to providers, initialisers, permissions or exported components. Checked on the release APK against alpha.10 with `aapt2 dump xmltree`: the only differences are two resource ids (the widget's and the tile's labels moved when strings were added). The release APK went from 39.0 MB to 31.8 MB universal; per-ABI APKs are 16.2 MB (arm64-v8a), 13.8 MB (armeabi-v7a) and 16.2 MB (x86_64); the app bundle is 19.2 MB.

**Tests:**
- 742 Rust tests in `anywhere-mobile` and `anywhere-gateway` (new ones for every pentest fix, the per-user analysis request limit and abandoned play claims);
- 398 Android JVM/Robolectric tests (was 348), including the accessibility walker over 15 screens, font scale, contrast and the crash-report redaction;
- 1,039 plugin Perl tests and 15 JS tests;
- lint with no issues; clippy for the host and Android (arm64).

**For the owner:**
- choose the 1.0 version;
- add the signing secrets;
- set up the Play Console (developer account, own signing key, Data safety, demo server for review);
- submit to F-Droid;
- take screenshots and performance measurements on a device.

### Sync status (2026-10-12): in step with hardware players

The owner found a phone in a sync group with a hardware player audibly late. The cause was elapsed time counted as frames handed to the output, not frames heard, plus a start and corrections applied behind the output's queue. Fixed in the core, Android and the web player (`docs/PROTOCOL.md` › *Synchronised playback*):
- **position as heard**, extrapolated to each STAT's jiffies;
- **a sample-exact timed start**, allowing for output latency;
- **pause and skip corrections applied at the output.**

**Verified on the host only.** The harness (`mobile/tests/sync.rs`) measures within 7 ms of a hardware player for a speaker (30 ms) and Bluetooth (250 ms), and 3 ms for a jittery Web Audio-like output. Its "Web Audio-like" output is the Android path with Web Audio's numbers; the worklet and worker are tested on their own. Before the fix it measured 256 ms late on Bluetooth.

**Needs a device:**
- a phone (speaker, wired, Bluetooth) synced with a Squeezebox or squeezelite, by ear or with a two-microphone recording;
- whether this phone's Bluetooth stack reports its latency in AAudio's timestamp;
- Chrome, Firefox and Safari in Web Audio mode;
- an iPhone in media-element mode (best effort; the limits are in PROTOCOL.md).

**Decision:** the plugin sets no LMS sync preferences for our players (D22).

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
| N5 | Where analysis runs | **On the gateway**, CPU-capped and off-switchable; the apps fall back to a plain seek bar and tags. Confirmed in A3 (SPEC D18). |
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

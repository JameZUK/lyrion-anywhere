# Lyrion Anywhere for Android

The native Android app: sign-in, browsing, remote control of LMS players (milestone A0), the phone as an LMS player with native audio output (A1), bit-perfect USB DAC output and the Sound settings (A2), the waveform seek bar with loudness levelling (A3), and browsing on a par with the web app, with rich album and artist pages, LMS's own menus and "player-first" control (A4 part 1). The plan is in [`docs/plans/native-apps.md`](../../docs/plans/native-apps.md); the protocol it speaks is [`docs/PROTOCOL.md`](../../docs/PROTOCOL.md).

## Building

You need:

- JDK 21;
- the Android SDK with `platforms;android-37.0` and `ndk;29.0.14206865` (`ANDROID_HOME` set, or `sdk.dir` in `local.properties`);
- Rust (stable) with the Android targets, and `cargo-ndk` 4:

  ```sh
  rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
  cargo install --locked cargo-ndk --version 4.1.2
  ```

Then, from this directory:

```sh
./gradlew assembleDebug testDebugUnitTest lint
```

The first build compiles the Rust library for every ABI. To save time and disk locally, build only the ABIs you need:

```sh
./gradlew assembleDebug -Panywhere.rust.abis=arm64-v8a,x86_64
```

The debug APK is `app/build/outputs/apk/debug/app-debug.apk`. Release builds are minified; they are signed only when `ANYWHERE_KEYSTORE`, `ANYWHERE_KEYSTORE_PASSWORD`, `ANYWHERE_KEY_ALIAS` and `ANYWHERE_KEY_PASSWORD` are set in the environment. Signing material never goes in the repository.

To try a debug build against a gateway on your computer, forward the port with `adb reverse tcp:9444 tcp:9444` and use `http://127.0.0.1:9444` as the server (plain HTTP is accepted only for loopback, and only debug builds' network policy allows it).

## Architecture

```
:app       Application, manual DI (AppContainer), navigation, Coil set-up
:ui        Design system, screens and view models (Compose, Material 3)
:playback  Media3 session service for "Play on this phone": SimpleBasePlayer over LMS,
           audio focus, becoming noisy, wake/Wi-Fi locks, output route
:data      Repositories over the Rust layer, typed errors, settings, secure storage
:rust      libanywhere_mobile.so (cargo-ndk) + generated UniFFI Kotlin bindings
```

- **The Rust layer** (`mobile/` in the repository root) does all networking: HTTPS, sign-in, token rotation, the allow-listed LMS commands, artwork and the event stream. Kotlin calls it through UniFFI as `suspend` functions. The `:rust` module builds it (`cargoBuild<Variant>`) and generates the bindings (`uniffiBindgen<Variant>`) as generated sources of each variant, so a clean checkout builds with one Gradle command.
- **TLS:** rustls with [`rustls-platform-verifier`](https://github.com/rustls/rustls-platform-verifier): certificates are checked by Android's own TrustManager, so the platform trust store (and the network security configuration) applies and no CA list is bundled. Its small Kotlin half comes from the project's Maven repository, restricted to that one artefact; the build checks its version matches `Cargo.lock`.
- **DI:** manual (`AppContainer`). The graph is small with one scope; constructor injection keeps view models testable with fakes, with no annotation processing or reflection.
- **Artwork:** a Coil fetcher calls the Rust Art API, which adds the bearer token in a header (never in a URL), validates the path and caps the size. Images are cached in memory only.
- **State:** view models expose `StateFlow`s; one `PlayerViewModel` per sign-in controls the selected player. It listens to `/v1/events` while the UI is visible (refreshing on changes, coalesced) and polls slowly as a fallback.

### Browsing and LMS menus

- **Library:** New Music; albums (by title, artist, year or date added); artists by role (only the roles the library has); genres (searchable) and years; compilations; random albums; the music folder; playlists; favourites; Apps & Radio. A virtual-library picker appears only when LMS has libraries.
- **Pages:**
  - Home puts the library first: recently added, random albums, favourites and playlists.
  - Album pages show large art, details (year, genre, format) and disc sections.
  - Artist pages group albums into *Albums*, *Appears on* and *Compilations*.
  - Both offer Play, Shuffle, Add and More.
- **Art-driven colour:**
  - `ui/theme/ArtColours.kt` picks a cover's colour with a small quantiser (no dependency) and derives a tinted scheme;
  - every foreground is pushed until it meets WCAG AA contrast;
  - `ArtTinted` applies it to the album, artist and Now playing pages.
- **"More…"** (a button, or a long press on an album) opens a bottom sheet:
  - Play now, Play next and Add to queue;
  - then LMS's own context menu (`trackinfo`, `albuminfo`, …), where plugins add entries such as "more like this".
- **Apps & Radio:**
  - apps, radio services, and plugins from the player's home menu;
  - each judged from its own top menu, so unusable ones are hidden behind "Show all".
- **The Rust layer resolves menu actions** (`mobile/src/menus.rs`) exactly as the web app does, and checks them again before sending. The gateway runs only actions LMS offered this device. Text from LMS is only ever displayed.

### Player-first (SPEC D20)

- **This phone is the default player:** Now playing, the mini player and the queue follow it.
- **"Play on…":** an output button (in Now playing and the mini player) opens it. It lists this phone, the other players you may control (with what each is playing) and sync groups.
- **Another player is always named:** while you control one, a coloured strip says "Playing on <name>", with **Back to this phone**.
- **Settings → Default player:** this phone (default), last used, or a specific player.
- The choice rules are in `ui/player/PlayerChoice.kt`. Screens never pick a player themselves.

### Play on this phone

The phone becomes an LMS player, as the web app's player does. LMS owns the queue (SPEC D14); the app only mirrors it.

```
LMS ⇄ gateway ⇄ /v1/player + /v1/stream ⇄ Rust engine (own thread) ⇄ core decode ⇄ [DSP] ⇄ ring ⇄ AAudio callback
                                                     ▲ status, signal path
Kotlin: RustLocalPlayerRepository ─► PlaybackService (Media3 session) ─► notification, lock screen, buttons
                                  └► PlayerViewModel ─► "Play on this phone" switch, signal-path panel
```

- **The engine is Rust** (`mobile/src/player`): the Slimproto tunnel with resume, stream downloads with byte-offset resume, the core's decoders, optional DSP, and a lock-free ring buffer. **AAudio is driven from Rust** (the `ndk` crate): its real-time callback reads the ring directly, so no JNI runs per buffer, and the callback neither allocates nor locks.
- **Bit-perfect by default.** DSP is off, and LMS volume at 100 % leaves samples untouched. The output opens at the source's rate and reopens when it changes. The signal-path panel shows exactly what happens. It claims "bit-perfect" end to end only for a direct route to a USB DAC (below), and otherwise lists why not.
- **Media3:** `PlaybackService` (not exported) wraps LMS's state for this player in a `SimpleBasePlayer`. Its buttons become LMS commands. It also handles:
  - audio focus (pause, duck, resume after a call);
  - "becoming noisy" (instant silence, then pause through LMS);
  - wake and Wi-Fi locks while playing.

  Media3 runs it as a foreground service (type `mediaPlayback`) while playing.
- **Starting:** the phone's player is on by default (D20). The service starts while the app is in front; if LMS starts playback on an idle phone whose app is in the background, the session appears when the app is next opened.

### Bit-perfect USB DAC output (Android 14+)

```
engine: new track format ─► OutputNegotiator.offers() ─► plan() (Rust, pure) ─► apply(offer)
                            UsbDacNegotiator: getSupportedMixerAttributes      setPreferredMixerAttributes
        ─► AAudio opened in that integer format at the source's rate ─► DAC
```

- `UsbDacNegotiator` (`:playback`, `UsbDac.kt`) is Android's side. It finds the USB DAC and lists its mixer attributes (API 34), then applies or clears the one the engine chose, for `USAGE_MEDIA`. The mapping between platform values and the engine's offers is pure Kotlin, tested on the JVM.
- The engine (`mobile/src/player/negotiate.rs`) chooses the format: bit-perfect, stereo, at the source's rate, in the shallowest integer format holding the source's depth. It then opens AAudio in that format, and falls back to float through Android's mixer if anything refuses.
- **Volume is fixed at 100 % on a bit-perfect route** (SPEC D19): use the DAC's own volume. The signal path says so.
- `PlaybackService` registers the negotiator and renegotiates when a USB DAC comes or goes. Settings → Sound → *Bit-perfect USB output* turns it off.
- `MODIFY_AUDIO_SETTINGS` (a normal, install-time permission) is needed for the mixer attributes; the `:playback` module declares it.

### Sound settings

Settings → Sound:
- preamp;
- loudness levelling (off, ReplayGain track or album, the server's R128 analysis);
- the true-peak limiter and its ceiling;
- a parametric EQ from an AutoEq `ParametricEQ.txt` (pasted, or picked with the system file picker; checked by the engine before it is saved);
- crossfeed;
- dither.

Everything is off by default (bit-perfect), with an indicator saying so or naming the stages that change the samples. Settings are stored with DataStore and applied to the engine at once.

### Manual test with a USB DAC (not yet done)

Nothing here has run on hardware yet. With an Android 14+ phone and a USB DAC:

1. Install a debug build, sign in, and turn on *Play on this phone*. Plug in the DAC. Check Settings → Sound: *Bit-perfect USB output* is on and the indicator says *Bit-perfect*.
2. Play a 16-bit/44.1 kHz FLAC to this phone from LMS. Open Now Playing → Signal path. Expect:
   - the verdict *Bit-perfect*;
   - output "<DAC name> · 44.1 kHz · 16-bit integer";
   - the line "Volume: use your DAC's".

   If it shows reasons instead, note them (e.g. "this DAC doesn't offer a bit-perfect mode"): that is the device's answer.
3. Confirm the phone's mixing is bypassed:
   - `adb shell dumpsys media.audio_flinger`: an output thread with the `BIT_PERFECT` flag at 44100 Hz, our track on it;
   - `adb shell dumpsys audio`: our preferred mixer attributes.

   Change LMS's volume: the level must not change (the DAC's volume controls it). A notification sound shouldn't mix into the music (Android mutes other apps in this mode).
4. Play a 24-bit/96 kHz track next. The output switches to 24-bit at 96 kHz at the track boundary.
5. Unplug the DAC while playing. Expected:
   - playback pauses (becoming noisy);
   - after you resume on the speaker, the signal path shows "the output isn't a USB DAC", and LMS's volume applies again.

   Plug it back in and resume: bit-perfect again.
6. Turn on a Sound stage (e.g. preamp −3 dB):
   - the route stays direct;
   - the signal path shows *Lossless, processed*, a dither stage and "a Sound setting changes the samples".

   Turn it off again: *Bit-perfect*.
7. **Bit-exactness, if you have the equipment:** play a known test file to a DAC with a digital output (S/PDIF) into a capture device, record it, and compare the samples with the file. They should be identical, apart from any leading or trailing silence.

Record the phone, Android version, DAC and results (in the A2 status of `docs/plans/native-apps.md`, without serial numbers or other personal details).

## Security

- The refresh token and device id are the only secrets stored: sealed with AES-256-GCM under a key generated in the AndroidKeyStore (not exportable), in the app's no-backup directory. A blob that fails to open is deleted and the user signs in again. Access tokens live in memory only.
- No backups or device transfers (`allowBackup="false"`, data extraction rules exclude everything).
- No cleartext traffic and only system CAs (`network_security_config.xml`); debug builds also allow plain HTTP to loopback. The Rust layer independently refuses `http://` except to loopback.
- Tokens and passwords are never logged or put in error messages; the password is dropped from memory after sign-in.
- Pairing QR codes are parsed and validated in Rust (`lyrion-anywhere://pair?server=…&code=…`, see `docs/PROTOCOL.md`); the server address in one obeys the same rules as a typed one.
- No Google Play Services, analytics or proprietary libraries (F-Droid compatible).

## Tests

- `:data`: the secure-store sealing (tampering, wrong keys, strict encoding), error mapping.
- `:ui`: view models with fakes (sign-in and two-step, pairing, paging, search debounce, player selection, events, optimistic transport), and Compose screens under Robolectric (labels for accessibility, error messages).
- `:ui` (A4): every browse view model (filters, virtual library, role chips, genre search, random batches, duplicate pages), album discs and details, artist grouping, Home's carousels, the menu navigator and Apps & Radio suitability.
- `:ui` (A4): player-first rules and "Back to this phone".
- `:ui` (A4): Compose tests for the "Playing on" strip, the "Play on…" sheet, the default-player setting, the Library page, the album page, the "More…" sheet and Apps & Radio, with 48 dp touch targets.
- `:ui` (A4): WCAG contrast of the art-tinted colours for 511 covers in both themes.
- `:data` (A4): the default-player and library settings (malformed stored values ignored), the virtual-library choice, and the suitability cache.
- `:app`: regression tests for the manifest (no exported service, the expected permissions only), backup rules and network policy.
- `:playback`:
  - the LMS → Media3 state mapping and the focus policy (plain JVM);
  - the session player's state and buttons (Robolectric);
  - the controller's refreshes and commands;
- `:data`: the local player's lifecycle (sign-in, switch, replaced players); the Sound settings' persistence (across a restart, migration, refusal of stored nonsense), clamping and mapping to the engine's `DspSettings`.
- `:ui`: the signal-path panel (with the reasons it isn't bit-perfect and the DAC volume note), the "Play on this phone" switch, and the Sound screen and its view model.
- `:playback`: bit-perfect USB output's platform side (mixer-attribute mapping and selection, USB DAC choice, refusals).
- The Rust layer has its own unit and integration tests against the real gateway (`cargo test -p anywhere-mobile`). These include the player end to end against a fake LMS:
  - gapless, bit-perfect, rate changes;
  - resume after a dropped connection, refusals and hostile frames;
  - a fake USB DAC receiving the files' integers;
  - Ogg Opus.

Instrumented tests (the AndroidKeyStore key itself, the camera, AAudio output, bit-perfect USB output, the media notification and buttons) need a device and are not run in CI yet. The A1 and A2 statuses in `docs/plans/native-apps.md` list what has and hasn't been verified on hardware.

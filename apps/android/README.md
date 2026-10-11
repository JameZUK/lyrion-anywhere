# Lyrion Anywhere for Android

The native Android app: sign-in, browsing, remote control of LMS players (milestone A0), and the phone as an LMS player with native audio output (A1). The plan is in [`docs/plans/native-apps.md`](../../docs/plans/native-apps.md); the protocol it speaks is [`docs/PROTOCOL.md`](../../docs/PROTOCOL.md).

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

### Play on this phone

The phone becomes an LMS player, as the web app's player does. LMS owns the queue (SPEC D14); the app only mirrors it.

```
LMS ⇄ gateway ⇄ /v1/player + /v1/stream ⇄ Rust engine (own thread) ⇄ core decode ⇄ [DSP] ⇄ ring ⇄ AAudio callback
                                                     ▲ status, signal path
Kotlin: RustLocalPlayerRepository ─► PlaybackService (Media3 session) ─► notification, lock screen, buttons
                                  └► PlayerViewModel ─► "Play on this phone" switch, signal-path panel
```

- **The engine is Rust** (`mobile/src/player`): the Slimproto tunnel with resume, stream downloads with byte-offset resume, the core's decoders, optional DSP, and a lock-free ring buffer. **AAudio is driven from Rust** (the `ndk` crate): its real-time callback reads the ring directly, so no JNI runs per buffer, and the callback neither allocates nor locks.
- **Bit-perfect by default.** DSP is off, and LMS volume at 100 % leaves samples untouched. The output opens at the source's rate and reopens when it changes. The signal-path panel shows exactly what happens; it claims "bit-perfect" end to end only for a direct route, which needs A2's USB mixer mode.
- **Media3:** `PlaybackService` (not exported) wraps LMS's state for this player in a `SimpleBasePlayer`. Its buttons become LMS commands. It also handles:
  - audio focus (pause, duck, resume after a call);
  - "becoming noisy" (instant silence, then pause through LMS);
  - wake and Wi-Fi locks while playing.

  Media3 runs it as a foreground service (type `mediaPlayback`) while playing.
- **Starting:** the switch is remembered. The service starts while the app is in front; if LMS starts playback on an idle phone whose app is in the background, the session appears when the app is next opened.

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
- `:app`: regression tests for the manifest (no exported service, the expected permissions only), backup rules and network policy.
- `:playback`:
  - the LMS → Media3 state mapping and the focus policy (plain JVM);
  - the session player's state and buttons (Robolectric);
  - the controller's refreshes and commands;
- `:data`: the local player's lifecycle (sign-in, switch, replaced players).
- `:ui`: the signal-path panel and the "Play on this phone" switch.
- The Rust layer has its own unit and integration tests against the real gateway (`cargo test -p anywhere-mobile`). These include the player end to end against a fake LMS: gapless, bit-perfect, rate changes, resume after a dropped connection, refusals and hostile frames.

Instrumented tests (the AndroidKeyStore key itself, the camera, AAudio output, the media notification and buttons) need a device and are not run in CI yet. The A1 status in `docs/plans/native-apps.md` lists what has and hasn't been verified on hardware.

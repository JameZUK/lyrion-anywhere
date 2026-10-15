# iOS app: parked (2026-10-15)

The iOS app (milestones I0–I3 in [`native-apps.md`](native-apps.md)) is parked. This page records what exists, what was decided, and where to pick up, so the work can resume without re-deriving it.

## Why parked

- No Apple Developer account (needed for TestFlight and the App Store; without it a build can only run on a developer's own devices for a few days).
- No Mac in the development environment. The chosen route when work resumes is GitHub's macOS CI runners for building and signing.
- The web app already gives iPhone and iPad users a working player today (see below), so an iOS app is an improvement, not a gap.

## What already exists for it

| Piece | State | Where |
|---|---|---|
| Shared Rust player core (sans-I/O: Slimproto, player state machine, decoders, DSP, analysis) | Done, used by the web player (WebAssembly) and Android | [`core/`](../../core) |
| Shared native host layer (networking, sign-in, player engine, rolling cache, downloads, offline library) | Done for Android; designed to be shared with iOS | [`mobile/`](../../mobile) |
| UniFFI bindings generator | Generates Kotlin; the Swift language path is wired but unused | [`mobile/uniffi-bindgen/src/main.rs`](../../mobile/uniffi-bindgen/src/main.rs) |
| Audio output per platform | Android done (AAudio, bit-perfect USB); iOS designed (AVAudioEngine source node, `setPreferredSampleRate`) | [`native-apps.md` §3.3](native-apps.md#33-audio-output-per-platform) |
| Licensing for the App Store | Decided and added: GPL-2.0-or-later plus an additional permission for app-store distribution of the project's own code; the mobile dependency tree has no other GPL code | [`LICENSE-EXCEPTION`](../../LICENSE-EXCEPTION), SPEC D17 |
| iOS and iPadOS in the web app | Supported: media-element playback through Managed Media Source so music carries on with the screen locked, home-screen install, sync groups, offline downloads (installed app only) | [`PLATFORMS.md`](../PLATFORMS.md) |
| Protocol and gateway | Nothing iOS-specific is needed; the app would use the same documented API and player protocol as Android | [`PROTOCOL.md`](../PROTOCOL.md) |

Not started: an `apps/ios` project, generated Swift bindings, the AVAudioEngine output host, `MPNowPlayingInfoCenter`/`MPRemoteCommandCenter` integration, background audio session handling, CarPlay, downloads on iOS, and the App Store listing (which needs a demo server for review).

## Decisions already taken

- **Thin native UI over the shared layer** (`mobile/`), the same shape as Android: SwiftUI over UniFFI, with the engine and networking in Rust ([`native-apps.md` §3](native-apps.md#3-architecture)).
- **Output:** AVAudioEngine with a source node fed from the Rust ring buffer; sample rate follows the source on an external DAC; AirPlay marked in the signal path as lossy or resampled.
- **Licence:** option A from the plan's §6, recorded as SPEC D17; contributions are accepted under the GPL together with the exception.
- **Distribution:** free; the App Store review needs a demo server (SPEC §10 open item).

## How to resume

1. Get an Apple Developer account and add its signing material to the repository's secrets, mirroring the Android release key set-up in [`RELEASING.md`](../RELEASING.md).
2. **I0:** generate Swift bindings from `mobile/` (`uniffi-bindgen generate --language swift`), add `apps/ios` with a SwiftUI shell that signs in, lists players and controls them (remote control only), and a GitHub macOS CI job that builds it.
3. **I1:** the AVAudioEngine output host for the engine, background audio session, `MPNowPlayingInfoCenter`, lock-screen and headset controls; the screen-locked and sync-group checks that Android went through (see the 2026-10-15 entries in [`CHANGELOG.md`](../../CHANGELOG.md): network flaps, audio focus and media-element start behaviour are the kinds of platform rules to expect).
4. **I2:** downloads and offline, Siri and CarPlay.
5. **I3:** TestFlight, then the App Store listing with the demo server.

Milestone details and the product scope stay in [`native-apps.md` §8](native-apps.md#8-milestones).

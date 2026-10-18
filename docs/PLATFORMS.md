# Platforms and project status

Where Lyrion Anywhere runs, how to get the Android app, and what is built, in progress and planned.

## Platform support

| Platform | Status | Notes |
|---|---|---|
| **Chrome, Edge (desktop)** | Supported | Web Audio playback; installable as an app |
| **Firefox (desktop)** | Supported | Web Audio playback |
| **Safari (macOS)** | Supported | Web Audio playback |
| **Android browsers** (Chrome, Firefox) | Supported | Media-element playback, which keeps going with the screen locked, after a self-test of what the browser can play; Web Audio otherwise. Installable |
| **iPhone and iPad (Safari, iOS/iPadOS 17 or later)** | Supported | Media-element playback through Managed Media Source (17.1 or later), so music carries on with the screen locked. The volume is set with the device's own buttons (a page can't set it). Install with *Share › Add to Home Screen*; offline downloads are reliable only in the installed app. No haptics. *Automatic* streaming quality means Original here (Safari can't tell mobile data from Wi-Fi), so choose *High* or *Data saver* in *Settings › Playback* when on mobile data; the quality badge then shows "Opus 160" |
| **Android app** | Android 10 (API 29) or later | Bit-perfect USB DAC output on Android 14 or later where the phone and DAC offer it; wallpaper colours and the sharper ambient blur on Android 12 or later; Android Auto |
| **iOS app** | Planned | Designed in: the shared Rust layer ([`mobile/`](../mobile)) already covers networking, the player engine and downloads |
| **Gateway** | Linux x86_64, aarch64, armhf (via the plugin); any Docker host | LMS 9.0 or later |

Every client plays in LMS sync groups; how each one keeps in step is in [`FEATURES.md`](FEATURES.md#playback) (*Synchronised playback*) and [`PROTOCOL.md`](PROTOCOL.md).

## Getting the Android app

- **Signed APKs** are on the [releases page](https://github.com/JameZUK/lyrion-anywhere/releases), tagged `android-v*` (the latest is 1.15.4). They are marked pre-release on GitHub so that *Latest* stays the LMS plugin, whose repository URL depends on it.
- **Which file:** `…-arm64-v8a.apk` suits most phones; the universal `.apk` works on any. Check the files against `SHA256SUMS`.
- **Stores:** Google Play and F-Droid listings are being prepared ([`RELEASING.md`](RELEASING.md)).
- **Source:** [`apps/android`](../apps/android).

Signing in for the first time is covered in [`INSTALL.md`](INSTALL.md#6-first-sign-in-and-installing-the-app).

## Project status and roadmap

- **Built and in use:** the gateway and LMS plugin (0.5.5), the web app, and the Android app (1.15.4, releases tagged `android-v*`). See [`CHANGELOG.md`](../CHANGELOG.md) for what changed and when.
- **Recently finished:**
  - joining a sync group from *Play on…* (*Sync with…*) on the web and Android, sync corrections made in the audio in the browser's media-element mode, and a phone rejoining a group in step after a dropped connection;
  - *Your devices*, to see and sign out your own signed-in devices (plugin 0.5.0 and Android 1.15.3);
  - a declutter of every screen on the web and Android ([`plans/wave3_5-declutter.md`](plans/wave3_5-declutter.md));
  - controls that work while the server is out of reach, and riding out a busy server;
  - the rolling cache for patchy mobile data: read-ahead to disk, a ten-minute resume grace, a replay cache and downloads standing in for streams (design in [`plans/wave3-design.md`](plans/wave3-design.md) §2).
- **iOS app:** planned, on the same Rust layer and protocol, with CarPlay. Its code is covered by the project's App Store exception ([`LICENSE-EXCEPTION`](../LICENSE-EXCEPTION), SPEC D17).
- **Later:** passkeys, a cryptographic release signature for the plugin's bundled gateways, smart filters with track-level rules, a guest queue, album booklets, and a multi-track cache-ahead for browsers (today only the Android app reads ahead; GitHub issue #13). Store listings for the Android app (Google Play, F-Droid) are being prepared.

The milestones and decisions behind all of this are in [`SPEC.md`](../SPEC.md) and the design plans listed in the [documentation index](README.md#design-plans-and-history).

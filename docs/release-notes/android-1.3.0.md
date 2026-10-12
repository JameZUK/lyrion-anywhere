# Lyrion Anywhere for Android 1.3.0

Installs over 1.2.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

## What's new
- **Streaming quality on mobile data:**
  - Settings › *Streaming quality* has one level for Wi-Fi and one for mobile data: **Original**, **High** (Opus 160 kbps, about 70 MB an hour) or **Data saver** (Opus 96 kbps, about 45 MB an hour).
  - By default: Original on Wi-Fi, High on mobile data. The level follows the network as it changes, from the next track.
  - Your server re-encodes lossless tracks when it offers this (the gateway from this release on, with `ffmpeg`); other tracks are sent as they are.
  - The signal path says when the stream is re-encoded.
- **Fixed:** some Opus audio, including offline downloads in Opus, could be decoded wrongly on sharp transients (a fault in the Opus decoder library, now patched).

## Downloads
| File | For |
|---|---|
| `LyrionAnywhere-android-1.3.0.apk` | Any phone |
| `…-arm64-v8a.apk` | Most phones (smaller) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`.

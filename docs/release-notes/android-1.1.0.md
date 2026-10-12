# Lyrion Anywhere for Android 1.1.0

Installs over 1.0.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Works best with the Lyrion Anywhere plugin **0.2.0** or later on your server.

## What's new
- **In sync with your Squeezeboxes:**
  - when this phone is in a sync group, it now reports the audio you actually hear, Bluetooth delay included, to LMS;
  - it starts at exactly the moment LMS asks;
  - LMS's corrections act at once, keeping it in step with hardware players.
- **Add this phone by QR code:**
  - *Scan QR code* on the welcome and sign-in screens reads the code from the plugin's *Add a device* or the web app's *Add another device*;
  - `lyrion-anywhere://pair` links open the app with the details filled in;
  - the app always shows the server and asks before signing in.
- **Faster FLAC starts** with plugin 0.2.0 (embedded artwork is no longer streamed).
- **Safety:** plain `http://` addresses are refused in release builds.

## Downloads
| File | For |
|---|---|
| `LyrionAnywhere-android-1.1.0.apk` | Any phone |
| `…-arm64-v8a.apk` | Most phones (smaller) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`.

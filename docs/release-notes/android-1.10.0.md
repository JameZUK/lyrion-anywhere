# Lyrion Anywhere for Android 1.10.0

Installs over 1.3.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

The longer gap protection needs the plugin [0.3.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.3.0) or later on your server. With an older one, the app keeps playing through gaps of about a minute.

## What's new
- **Playing through gaps in the signal:**
  - The phone reads ahead and keeps the next track on disk, so a tunnel or a dead spot doesn't stop the music.
  - Settings › *Gaps in the signal* sets how long to keep the player through a gap (up to 10 minutes). After a longer gap, playback restarts where you were.
  - Now playing shows how much music is buffered.
  - In a group of synced players, the phone rejoins the others when the signal comes back.
- **Replay cache:** replaying a track or skipping back plays from the phone instead of downloading it again (Off, 500 MB, 1 GB, 2 GB or 5 GB; 1 GB by default). The signal path says *From cache*.
- **Streaming from your downloads:** at Original quality, an album you've downloaded as FLAC plays from the download, checked bit for bit first. The signal path says *From downloads*.
- **Quiet reconnects:** back from the background, the app stays on the screen you left. A small "Reconnecting…" shows instead of the offline screen.
- **Now playing redesigned**, with four tabs and a new Glow; **Your listening** and **Your year in sound**; artist biographies, photos and album reviews.
- **Alarms, ratings, lyrics, the sleep timer, stations and a customisable Home.**
- **Library and queue:** more ways to browse, sort and edit; search on plugins.
- **Car mode, Android Auto and system surfaces, haptics, smart filters**, loudness-compensated volume, and the visualiser offline.
- **Fixed:** car mode drew under the system bars; Up next showed its red remove background all the time.

## Downloads
| File | For |
|---|---|
| `LyrionAnywhere-android-1.10.0.apk` | Any phone |
| `…-arm64-v8a.apk` | Most phones (smaller) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`.

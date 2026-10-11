# Lyrion Anywhere for Android 1.0.0

The first full release of the native Android app for [Lyrion Anywhere](https://github.com/JameZUK/lyrion-anywhere): your Lyrion Music Server, securely, from anywhere. It needs Android 10 or later and a Lyrion Anywhere gateway (plugin 0.1.0 or later) reached over HTTPS.

Installs over the alpha builds (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

## What's in it

**A player in its own right**
- This phone is the player by default. A small output button sends playback elsewhere: *Play on…*, *Move playback here*, *Bring to this phone*, and sync groups.
- Lock-screen, notification, Bluetooth and Android Auto controls; a home-screen widget and a Quick Settings tile.

**Sound**
- Native audio with gapless playback. It is bit-perfect by default, and bit-perfect to USB DACs on Android 14+ where the DAC supports it.
- A signal-path panel shows exactly what happens to the sound.
- Optional loudness levelling (ReplayGain or the server's R128 analysis), parametric EQ with AutoEq import, crossfeed, dither and smart crossfades (never within an album).
- A waveform seek bar.

**Your library**
- Home, New Music, Albums, Artists by role, Genres, Years, Compilations, Random albums, Music Folder, Playlists, Favourites, and Apps & Radio including plugins.
- Every LMS menu action, so AudioMuse-AI's *Start radio* and *More like this* are there too.
- Queue editing with undo.
- Layouts for phones, tablets and foldables.

**Offline**
- Download albums, playlists and tracks; smart downloads; storage controls.
- An offline mode that plays your downloads through the same engine.
- Plays are synced back to LMS when you're online again.

**Security and privacy**
- Your sign-in is sealed in the phone's Keystore; no backups; TLS only, checked against the phone's own certificate store.
- No analytics or third-party services. See [PRIVACY.md](https://github.com/JameZUK/lyrion-anywhere/blob/main/docs/PRIVACY.md).

## Downloads

| File | For |
|---|---|
| `LyrionAnywhere-android-1.0.0.apk` | Any phone (all CPU types) |
| `…-arm64-v8a.apk` | Most phones (smaller download) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`; `BUILDINFO.txt` records the toolchain for reproducible builds.

Crash reports are built in. If the app ever stops, open it again and use *Share*, then send the report.

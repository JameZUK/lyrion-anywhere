# Lyrion Anywhere for Android 1.15.3

Installs over 1.14.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

This release brings together everything since 1.14.0 (the test builds 1.15.0 to 1.15.2 were not published).

*Your devices* needs plugin [0.5.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.5.0) or later on the server. Everything else works with 0.4.0.

## What's new
- **Your devices** (*Settings › Account*): see every device signed in to your account, when each was last used, and sign out the ones you don't want, one at a time or all the others at once, without changing your password.
- **Sync groups:** a phone that rejoins a group after a dropped connection no longer pauses the other players or moves them back. It restarts where the others are, after checking with the server that the same track is still playing.
- **Playing with the screen off:** a track change from another device no longer waits until the phone is woken. The app keeps its playback service, wake lock and Wi-Fi lock through a track change and for 90 s after a stop, and takes the foreground back when you open it. If Android's battery optimisation is still on for the app, Settings now says so and offers to open the system page, because without that exemption Android cuts the phone's connection with the screen off.
- **Sync groups:** heartbeats queued while the connection was down are no longer replayed on reconnect, which made the server re-base the phone's clock and skip it ahead. The Signal path sheet now shows the output clock in use ("device" or "estimated") and the measured output latency, so sync offsets can be diagnosed; when the device gives no usable timestamp, the device buffer is taken off the reported position instead of being counted as heard.
- **Download on Wi-Fi only is now on by default.** Turn it off in *Settings › Downloads › Download options* if you want downloads on mobile data.
- **Fixed:**
  - Opening Settings while the phone was not playing could empty the replay cache.
  - A temporary problem with the phone's secure storage signed you out. The app now keeps your sign-in and offers to try again.
  - The camera could stay on after leaving the QR pairing screen early.
  - A server that keeps dropping the connection, or network flaps during a Wi-Fi to mobile handover, no longer cause rapid reconnect loops.
  - A stream that delivered more data than the phone asked for could hang the player.
  - The event connection no longer loops on token renewal.
  - Offline downloads and play history are bound to the account that made them.
  - *Your devices*: messages no longer go stale, and a failed load offers *Try again*.

## Downloads
| File | For |
|---|---|
| `LyrionAnywhere-android-1.15.3.apk` | Any phone |
| `…-arm64-v8a.apk` | Most phones (smaller) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`.

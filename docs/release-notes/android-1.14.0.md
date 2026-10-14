# Lyrion Anywhere for Android 1.14.0

Installs over 1.10.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

This release brings together everything since 1.10.0 (the test builds 1.11.0 to 1.13.0 were not published).

Three features need a gateway newer than the one in plugin [0.3.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.3.0): *Change password*, *Save as playlist* and the "music server is busy" notice. With 0.3.0 everything else works as described.

## What's new
- **A calmer, tidier app.** Every screen was reviewed for clutter:
  - No "Playing on" strip. The mini player names the player in use ("on Lounge"), and moving playback is in *Play on…*.
  - Album, artist and playlist pages have Play · Shuffle · Download · ⋮. The format choice and the rest are in ⋮.
  - Queue rows are the drag handle, the track and ⋮.
  - Now playing has no tab bar. It has one rating control (the full stars are in ⋮), a one-line Up next and Play on · Lyrics · Queue.
  - Settings is seven sections, each with its own page, and options open a picker.
  - One status line at the top for the connection, and one message at a time.
  - Library is one list of sections.
  - *Play on…* now also does what Rooms did: each player's ⋮ moves playback, plays or pauses it, opens its alarms and switches it on or off.
  - Downloads play through the normal mini player, Now playing and Queue.
- **Controls that work when your server is out of reach:**
  - Controls that can't work are greyed out instead of failing.
  - When the phone is the player, Play/Pause still works, and so do Next, Previous and tapping a track in the queue, if that track is on the phone (downloaded or in the replay cache).
  - The queue marks tracks that are *Downloaded* or *On this device*.
  - The connection message no longer covers anything.
  - When the server is back, what you did meanwhile (paused, skipped, sought) is passed on to it, as long as nobody else changed the queue.
- **A busy music server isn't "offline".** When LMS stops answering for a while (for example while its music drive spins up), the app says the server is busy and waits for it, instead of going offline.
- **Continue on a player:** back online after playing downloads, carry on where you were on any player.
- **Change your password** in Settings › Account.
- **Save the queue as a playlist** from the queue's ⋮.
- **Opus decodes about 80 times faster,** which saves battery on lower-data streams.
- **Security:** the TLS library is updated to rustls 0.23.46, for four advisories published today (RUSTSEC-2026-0346 to 0349).
- **Fixed:**
  - The app froze when airplane mode was turned on while playing.
  - The queue heading wrapped on narrow phones.
  - The ambient background drew outside its card.
  - The header ⋮ wrapped onto a second line.
  - The selected Library section scrolled out of view.

## Downloads
| File | For |
|---|---|
| `LyrionAnywhere-android-1.14.0.apk` | Any phone |
| `…-arm64-v8a.apk` | Most phones (smaller) |
| `…-armeabi-v7a.apk` | Older 32-bit phones |
| `…-x86_64.apk` | Chromebooks and emulators |
| `…aab` | Google Play |

Check downloads against `SHA256SUMS`.

# Lyrion Anywhere for Android 1.17.0

Installs over 1.16.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

**Update both.** Released alongside plugin [0.7.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.3), which this version needs to play on the phone: against an older plugin the app says so and still controls your other players. Plugin 0.7.3 in turn needs this app or later for the phone's player; an older app is told to update.

## What changed

- **The quality badge says what you hear.** *Hi-Res · FLAC 24/96*, *Lossless · FLAC 16/44.1*, *Transcoded · Opus 160* or *Lossy · MP3 320*: the weakest link first, then the stream actually heard, never just the file's format (the old badge said *FLAC 24/96 · Hi-Res* while the stream was Opus). Transcoded and Lossy are tinted amber. Also in the mini player.
- **The signal path is a block diagram:** *File* → *LMS* (only when LMS changed something) → *Lyrion gateway* (passed through, or re-encoded and why) → *This phone*, stacked on a phone and in a row on a tablet, the block that lowered the quality tinted, with the old detail list under *Details*. Downloads show *Download → This phone*; another player shows *File → Player*.
- **Change the streaming quality from the diagram:** *Original · High · Data saver* under the gateway block override your Wi-Fi or mobile-data setting for this session (until the app restarts, the network type changes or you sign in again) and take effect at once, restarting the stream where it is once the server has confirmed the level.
- **Fix output level at 100%** (*Settings › Sound*, on by default): the phone sets its own player's *Volume Control* in Lyrion to "Output level is fixed at 100%", so the phone's volume is the only one that matters and no controller can turn it down. *Player level* is disabled while it is on; untick it to give LMS the volume back. Any player whose output level is fixed shows "Output level is fixed at 100%" instead of a slider, and the volume keys leave it alone.
- **A new player starts at full volume** (from the plugin): the first play on a fresh install is no longer quiet.
- **Over `http://` the app needs the server's private IP address**, such as `http://10.8.0.1:9443`, not a name or a public address. The message now says so, and where to set the Public URL so the QR code carries it. The rule itself is unchanged.
- **Protocol 2.** The app states its protocol version when it signs in and reads the server's; a plugin below 0.7.3 keeps the phone's player off with a plain message, and *Settings › About* shows the server's protocol. The engine confirms a clean stream end before treating it as the track's end (a follow-up to issue #12).
- **Manifest changes:** the version only; no new permissions.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

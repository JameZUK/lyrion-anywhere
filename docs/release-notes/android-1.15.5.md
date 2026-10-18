# Lyrion Anywhere for Android 1.15.5

Installs over 1.15.4 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

The remaining low findings of the October code review, in the app and in its Rust layer. Works with plugin 0.5.0 or later; goes with plugin [0.5.6](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.5.6).

## What changed

**App**
- **Voice search asks first.** A "play" request from another app (the Assistant contract) used to turn on *Play on this phone* and start playing with no confirmation. It now plays at once only when *Play on this phone* is already on; otherwise a one-off dialog asks, and nothing changes unless you agree.
- **Safe mode is harder to trip.** It needs two crashes within five minutes; a single crash, or an ANR, shows its report once over the normally started app. Android Auto is no longer refused after a lone crash.
- **Per-output EQ settings stay put** when outputs come and go: the chosen output is saved by a stable key, not a position in a list.
- **Steadier screens:** duplicate rows in listening history or in Apps and Radio no longer crash their screens; a track with an unusable length gets a sensible seek bar and never sends a bad seek; *Your devices* explains itself on a server that lacks the feature instead of showing the server's error text.
- **The volume popover follows the hardware keys** promptly, through the system's volume-change broadcast.
- Wording that was hard-coded in English now comes from resources, ready for translation.
- **Every dependency is checksum-verified at build time** (Gradle verification metadata), including the certificate-verifier bridge that was fetched from a mutable branch before.

**Rust layer (shared with the web app's engine)**
- A file queued under two ids is fetched once; cancelling one id keeps the other's bytes.
- One open library per folder, so two instances can no longer delete each other's files.
- The downloaded-FLAC check that can stand in for a stream now runs in the background and never delays the stream's start.
- A trickling server can no longer hold a read open for ever: three minutes for a whole body, at most.
- New TLS tests prove self-signed and wrong-name certificates are refused on every path, and that a trusted certificate is accepted.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

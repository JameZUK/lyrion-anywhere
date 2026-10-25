# Lyrion Anywhere for Android 1.17.1

Installs over 1.17.0 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Needs plugin [0.7.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.3) or later to play on the phone, as 1.17.0 did; the player protocol is unchanged. Plugin [0.7.4](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.4), released alongside, makes the same mini-player change in the web app.

## What changed

- **The mini player's chip is the verdict word alone**: *Hi-Res*, *Lossless*, *Transcoded* or *Lossy*, amber when the badge is, after the artist, which keeps its room. In 1.17.0 the full badge could squeeze the artist's name down to a letter and an ellipsis. On a narrow line the chip gives way rather than the name, and TalkBack still reads the full badge. The full badge stays on Now playing.
- **Manifest changes:** the version only; no new permissions.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

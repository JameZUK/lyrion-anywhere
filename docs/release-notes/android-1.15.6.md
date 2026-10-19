# Lyrion Anywhere for Android 1.15.6

Installs over 1.15.5 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

One fix, the same one as plugin [0.6.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.6.3) carries for the web app. Works with plugin 0.5.0 or later.

## What changed

- **A seek into a FLAC at Original quality plays on instead of skipping the track.** LMS serves a seek by jumping into the file, with no FLAC header in front of the frames (its stock transcoding rule is a pass-through, and FLAC is the one format LMS never writes an initial block for). The app's decoder needed that header, so on a stock LMS every seek into a FLAC streamed at Original ended in "Decoder does not support file format" and LMS moved to the next track. Tracks the app had downloaded were unaffected, as it plays those from the file. The shared player core now makes the header from the first frame (checked against the next one) and plays from the seek point. Proven on a stock LMS 9.1.1 with CD-quality and 24/96 FLAC.

Nothing else changed in the app. The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

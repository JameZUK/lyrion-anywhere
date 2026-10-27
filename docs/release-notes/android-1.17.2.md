# Lyrion Anywhere for Android 1.17.2

Installs over 1.17.1 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Needs plugin [0.7.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.3) or later to play on the phone, as 1.17.0 did; the player protocol is unchanged, and there is no plugin release alongside.

## What changed

- **The "buffered ahead" figure counts the next track once it is on the phone** ([#30](https://github.com/JameZUK/lyrion-anywhere/issues/30)). With read-ahead on and a fast link, the figure sat at about a minute, showed four or five minutes as a track began and fell at once, and counted down to nothing at each track's end, although the music carried on to the end of the following track with the signal gone. Read-ahead was working; the figure was wrong. A track of ordinary size (an MP3 or Opus track) goes into the player whole, and the next one is then held in the player's queued slot rather than the spool, where the figure did not look; it also took those bytes as already heard when working out the byte rate, so the rest of the current track was scaled down too. The figure now counts every byte not yet decoded, and works the time out from the track's own byte rate (tags and artwork left out), so it reads about "the rest of this track plus the next" once the next has arrived.
- **Manifest changes:** the version only; no new permissions.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

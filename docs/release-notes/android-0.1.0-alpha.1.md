# Lyrion Anywhere for Android 0.1.0-alpha.1 (test build)

The first test build of the native Android app. It needs Android 10 or later and a Lyrion Anywhere gateway (plugin 0.1.0 or later), reached over HTTPS.

**This is an alpha for testing.** Expect rough edges, and please report problems as GitHub issues.

## What's in it
- **Sign in** with your Lyrion Anywhere account (two-step verification supported), or with a pairing code.
- **Browse and control:** home, albums, artists, search, Now playing with a waveform seek bar, and remote control of any LMS player you're allowed to use.
- **Play on this phone:** the phone becomes an LMS player, with native audio, lock-screen and Bluetooth controls, gapless playback and a signal-path panel.
- **Audiophile options** (all off by default, bit-perfect):
  - bit-perfect output to a USB DAC on Android 14+;
  - loudness levelling (ReplayGain or the server's R128 analysis);
  - parametric EQ with AutoEq import, crossfeed and dither.

## Install
1. On the phone, download `LyrionAnywhere-android-0.1.0-alpha.1.apk` from this release.
2. Open it, and allow your browser or file manager to install apps when Android asks.
3. Open *Lyrion Anywhere* and enter your server's address, e.g. `https://music.example.com`.

Check the download if you like: its SHA-256 is in `SHA256SUMS`. The signing certificate's SHA-256 is `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`. Later builds are signed with the same key, so they install as updates.

## Known limitations
- Not yet tested on real hardware: USB DAC bit-perfect output, Bluetooth and lock-screen behaviour, and background survival on aggressive battery managers.
- The phone's volume keys change Android's volume, not LMS's.
- The new web-app browse pages (New Music, Genres, Years, Music Folder…) come to Android in the next milestone.
- No pairing QR code is shown by the web app or plugin yet; type the pairing code.

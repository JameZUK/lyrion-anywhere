# Lyrion Anywhere for Android 1.18.0

Installs over 1.17.3 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Needs plugin [0.7.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.3) or later to play on the phone, as 1.17.0 did; the player protocol is unchanged. Plugin [0.8.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.8.0) goes out alongside with the server's share of the same review, and updating both is recommended: keeping downloads after an expired sign-in needs it.

## What changed

The fixes from a full review of the project for security, stability and reliability.

- **Signing in again works without restarting the app.** After signing out, after the server ended the sign-in, or after following a pairing link, signing in on the same server failed until the app was restarted.
- **Downloads survive an expired sign-in** (with plugin 0.8.0). Only a device the server revokes loses them. A phone used offline for longer than the sign-in lasts lost every download when it came back online.
- **Signing out really signs the device out on the server**, even just after the server restarted; the app says so when the server could not be told.
- **Sign-in renewals always finish**, so a renewal cut short by the player or the downloader stopping can no longer end in the device being signed out.
- **Resumed downloads** check they continue the same file and start again if it changed on the server.
- **Car mode opens by itself** when the car's Bluetooth connects.
- **Sturdier:** another app can no longer crash the app by starting its media service; a damaged settings file, a refused background start, an odd camera frame in the QR scanner or a slow artwork fetch in a widget no longer stop it; searching downloads from the car or by voice no longer stalls it.
- **Covers are kept per server:** after a change of server, covers from the previous one could show.
- **Hostile streams:** a stream of repeating WAV markers, or one declaring a sample rate of 0 or an absurd one, is refused instead of hanging the player or filling memory.
- The two-step code field offers codes from authenticator apps, not from text messages; playlist names refuse every character that draws nothing (emoji are fine).
- **Manifest changes:** the version only; no new permissions.

The APKs and the bundle are now signed in a job of their own, after an unsigned build, with the same key as before.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

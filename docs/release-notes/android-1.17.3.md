# Lyrion Anywhere for Android 1.17.3

Installs over 1.17.2 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Needs plugin [0.7.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.3) or later to play on the phone, as 1.17.0 did; the player protocol is unchanged. Plugin 0.7.7 goes out alongside with the same fix on the server side, and either is enough on its own.

## What changed

- **Streams show their artwork** ([#37](https://github.com/JameZUK/lyrion-anywhere/issues/37)). Playing BBC Radio 2 from the BBC Sounds plugin showed no artwork, and nor did any stream whose image is at an external URL, or a plugin menu icon at one. LMS names such images through its own image proxy with no size, and answers a request for exactly that with a redirect rather than the image, which the gateway never follows. The app now asks for every remote image at the size the view needs, as it already did for images it was given as plain URLs.
- **Manifest changes:** the version only; no new permissions.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

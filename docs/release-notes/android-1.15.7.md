# Lyrion Anywhere for Android 1.15.7

Installs over 1.15.6 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Two fixes to *Scan QR code*. Works with plugin 0.5.0 or later.

## What changed

- **Scan QR code reads the pairing code from a normal distance.** The in-app scanner decoded the camera's default 640 × 480 analysis frames, on which the 232-pixel QR code on another device's screen covered under two pixels per module from arm's length, so it was never read, while a third-party scanner app read the same code. The scanner now asks the camera for 1920 × 1440 frames (or the nearest size it offers) and also reads a light-on-dark code, as on a page in dark mode. Typing the code, and opening a pairing link from another scanner, were unaffected.
- **The scanner fits the screen in landscape.** Held sideways, the preview square was taller than the screen, so the code could not be framed. It is now sized to the window in either orientation.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

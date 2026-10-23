# Lyrion Anywhere for Android 1.15.8

Installs over 1.15.7 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

A dependency refresh with no feature changes, released alongside plugin [0.6.4](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.6.4). Works with plugin 0.5.0 or later.

## What changed

- **Updated Rust libraries**, including the three security patches published on 23 October against the TLS and hashing crates the player's engine links: `rustls-webpki` 0.103.16 (RUSTSEC-2026-0382), `webpki-root-certs` 1.0.11 (RUSTSEC-2026-0383) and `ctutils` 0.4.5 (RUSTSEC-2026-0385). Also UniFFI 0.32 for the Kotlin bindings, getrandom 0.4, tokio 1.53.3 and serde_json 1.0.154.
- **Nothing else changed.** The app's Kotlin code, its Android dependencies and its manifest are the same as in 1.15.7.
- This is the first Android build published as a full GitHub release rather than a pre-release.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

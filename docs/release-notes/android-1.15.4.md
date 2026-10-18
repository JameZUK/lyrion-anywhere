# Lyrion Anywhere for Android 1.15.4

Installs over 1.15.3 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

A small, careful release: one security hardening from the October code review, and nothing else changes in the app. Works with plugin 0.5.0 or later; *Your devices* needs 0.5.0, and the gateway improvements in [0.5.3](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.5.3) help every client.

## What changed

- **Continue on another player resolves against your own library only.** Each user's downloads live in their own library on the phone, keyed by server and account. The review found that the step that turns your queued downloads back into server track ids for *Continue on* took no library at all, so it could not check whose downloads it was looking at; a stale handle held across an account switch could in principle have resolved another user's. It now takes the owner's library and refuses, with no network request, unless that library belongs to the account signed in, as syncing plays and downloading already did. The app already checked the queue's owner before offering *Continue on*, so this closes the last gap underneath that check.

## Tests

- Rust: resolving with another account's library is refused and makes no request; resolving with your own works as before.
- Kotlin: the *Continue on* use case passes the queue owner's library.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

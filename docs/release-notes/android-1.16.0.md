# Lyrion Anywhere for Android 1.16.0

Installs over 1.15.8 (same signing key). Signing certificate SHA-256: `86:74:DA:2B:CE:50:26:22:F0:A0:2E:7A:22:50:96:DF:FB:D0:A9:45:17:82:66:25:C9:6C:DD:AF:37:90:D2:85`.

Released alongside plugin [0.7.0](https://github.com/JameZUK/lyrion-anywhere/releases/tag/v0.7.0). Works with plugin 0.5.0 or later; connecting over your own VPN without HTTPS needs 0.7.0 on the server.

## What changed

- **Your own VPN, no HTTPS (trusted networks).** If the admin has listed your VPN as a trusted network on the server (plugin 0.7.0), you can enter the server as `http://` and its private address, e.g. `http://10.8.0.1:9443`. Before signing in, the app shows *Unencrypted connection*: anyone on the same network could read your password and what you play, so only continue if this is your own VPN or a network you trust. *Connect anyway* signs in; *Cancel* leaves the address as typed. While you are signed in to such a server, *Settings › Account* marks it **Unencrypted**. Pairing codes and links may name such a server too, with the same warning. `http://` to a name (which could lead anywhere) or to a public address is still refused, and `https://` is unchanged. The private ranges are checked by the shared Rust rule and again in the app against the same test vectors; the release build's network security configuration (no cleartext for Android's own HTTP stacks, system certificates only) is unchanged, since every connection goes through the app's Rust layer.
- **Android Auto on a sideloaded build.** Android Auto hides media apps not installed from Google Play until *Unknown sources* is on in its developer settings: in the Android Auto app on your phone, *Settings*, scroll to *Version* and tap it ten times, then ⋮ › *Developer settings* › *Unknown sources*; open Lyrion Anywhere once and restart Android Auto. The app's build was already correct; the docs now say this, and two things make a car problem easier to pin down:
  - **Car connections** under *Settings › About*: the last ten times a car app asked to connect, with the outcome (allowed, controls only, refused, or refused because its certificate wasn't recognised). Kept on the phone only, until the app closes.
  - The manifest now declares the three car and assistant apps whose certificates the app checks (`<queries>`, Android 11+ package visibility), so the check can read them whatever Android's visibility rules do.
- **Manifest changes:** the `<queries>` element above, and the version. Nothing else; no new permissions.

The full list is in the [changelog](https://github.com/JameZUK/lyrion-anywhere/blob/main/CHANGELOG.md).

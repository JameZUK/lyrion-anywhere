# Security and privacy

How Lyrion Anywhere keeps your server and your listening safe, in brief, and how to report a vulnerability.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's *Report a vulnerability* (the repository's Security tab), not in a public issue.

## Security in brief

- **Nothing of LMS is exposed.** The gateway forwards only allow-listed commands with validated arguments; everything else is refused. Answers are scrubbed of file paths, LAN addresses, server identifiers and other servers' players.
- **Each device is held to its own player.** The Slimproto tunnel accepts only what a software player sends, pins the device's assigned player address, and never lets LMS redirect a device to another server.
- **Accounts of its own:** argon2id passwords, optional TOTP two-step verification, sign-in lockouts that strangers can't use to lock you out, 15-minute access tokens, rotating refresh tokens with reuse detection, and per-device revocation that takes effect at once. Each user can see their own signed-in devices and sign out any of them (*Your devices*).
- **Least privilege:** listeners control only their own devices and the home players an admin grants them. Plugins can be reserved for admins.
- **Hardened web app:** a strict Content Security Policy with no inline script, no third-party code at runtime, and metadata always rendered as text.
- **Hardened Android app:** no cleartext traffic, system certificates only, backups off, and the sign-in sealed by the Android Keystore.
- **Tested like it matters:** negative tests for every endpoint, robustness tests for every parser, a regression test for every security fix, and reviews recorded in [`security/`](security).

Every gateway-enforced protection is listed in [`FEATURES.md`](FEATURES.md#security-features). The trust boundary and threat model are in [`ARCHITECTURE.md`](ARCHITECTURE.md#4-trust-boundary-and-threat-model) and [`SPEC.md`](../SPEC.md#9-authentication-and-security) §9; the command policy and security rules of the API are in [`PROTOCOL.md`](PROTOCOL.md).

## Privacy in brief

- **Nothing goes to the project.** No analytics, advertising, tracking or third-party SDKs. The apps talk only to your own gateway.
- **Your server keeps what it needs:** accounts, devices (including each device's last address, for admins), an audit log for a year, and, per user, a listening history kept for 13 months that you can view, clear or switch off. No one else, admins included, can see your history through the apps or the API.
- **Your phone keeps your sign-in sealed by the Android Keystore,** your settings and your downloads. Signing out can remove the downloads; a revoked device deletes them.
- **Lyrics, biographies and reviews** come from your server's own plugins; Lyrion Anywhere never calls those services itself.

The full policy, including Google Play's Data safety answers, is [`PRIVACY.md`](PRIVACY.md); the privacy controls in the apps are listed in [`FEATURES.md`](FEATURES.md#privacy-controls).

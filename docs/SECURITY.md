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
- **An optional Cloudflare Tunnel with one secret kept:** a pinned, checksum-checked `cloudflared` that never updates itself, run without a shell; the tunnel token is the only secret stored, and a Cloudflare API token is used once and never stored ([below](#the-built-in-cloudflare-tunnel)).
- **Tested like it matters:** negative tests for every endpoint, robustness tests for every parser, a regression test for every security fix, and reviews recorded in [`security/`](security).

Every gateway-enforced protection is listed in [`FEATURES.md`](FEATURES.md#security-features). The trust boundary and threat model are in [`ARCHITECTURE.md`](ARCHITECTURE.md#4-trust-boundary-and-threat-model) and [`SPEC.md`](../SPEC.md#9-authentication-and-security) §9; the command policy and security rules of the API are in [`PROTOCOL.md`](PROTOCOL.md).

## The built-in Cloudflare Tunnel

The plugin can have the gateway run Cloudflare's `cloudflared` in place of a reverse proxy ([`INSTALL.md`](INSTALL.md#option-c-built-in-cloudflare-tunnel), SPEC D27). It is off until an admin asks for it on the plugin's settings page, and this is what it trusts and keeps.

- **Who can drive it.** Only the plugin's settings page: its `anywhere tunnel_…` commands need the page's token, `tunnel_status` and `tunnel_log` included, and the plugin passes them to the gateway's admin API on loopback with the admin secret. The plugin checks the inputs before sending them, never logs a token, and never echoes one back to the page.
- **What is stored**, all under `<data>/tunnel/` (mode 0700, inside the 0700 data folder): the **tunnel token**, the only secret kept (`token`, mode 0600, shown nowhere); `state.json` (the hostname, the installed version, and whether the tunnel starts with the gateway); the `cloudflared` program (0700); and `pid` while it runs. *Remove* deletes all of it.
- **What is never stored.** The **Cloudflare API token** for *Create it for me* travels once from the admin's browser to LMS's settings page, over the LAN like every LMS setting, and once from the plugin to the gateway over loopback. The gateway holds it in memory for that one exchange with Cloudflare's API and then drops it. It is never written to disk or logged, and is masked should it appear in a log line. Nothing about the **temporary tunnel** is stored: no token and no address.
- **How `cloudflared` is fetched.** Only the version pinned in this release, from Cloudflare's GitHub releases over HTTPS, and only if it matches the SHA-256 compiled into the gateway. At most two redirects are followed, each only to `github.com` or a `githubusercontent.com` host over HTTPS. The download is capped at 64 MiB and 10 minutes, written to a temporary file and moved into place only once its checksum matches. It never updates itself (`--no-autoupdate`); a new version comes only with a Lyrion Anywhere release. `--cloudflared` names a system-installed one instead, which the gateway runs but doesn't check.
- **How `cloudflared` is run.** Directly, without a shell, with a fixed argument list, stdin closed and an environment of only `TUNNEL_TOKEN` (registered tunnels), `HOME` (the tunnel folder) and `PATH`. The token is never on a command line. Its output is read a line at a time (at most 4 KiB a line), scrubbed of tokens, keys and ids, and only the last 200 lines are kept, in memory. A `cloudflared` left behind by a gateway that was killed is stopped at the next start only if `/proc/<pid>/exe` shows it is the gateway's own.
- **Outbound only.** `cloudflared` dials out to Cloudflare and listens only on a loopback port for its metrics, which the gateway reads to tell whether it is connected. The gateway's own listen address doesn't change, and nothing about the tunnel exposes it to the LAN.
- **The gateway's rules follow the tunnel.** While a tunnel runs, `https://<its hostname>` is an accepted origin, and the client's address is read from `CF-Connecting-IP` on requests from the tunnel's source: loopback, or the gateway's listen address when that isn't loopback, trusted as a proxy meanwhile. Both revert when it stops. Because every loopback connection then counts as the tunnel, **don't run another reverse proxy on the same machine at the same time unless it removes or overwrites `CF-Connecting-IP`**, or a client of that proxy could choose the address the lockouts see.
- **A tunnel address is a public address.** Sign-in, two-step verification, lockouts and rate limits apply exactly as behind a reverse proxy. The temporary tunnel's random address reaches the sign-in page for anyone who has it, until it stops (after an hour at most).
- **Cloudflare is in the path.** Cloudflare ends HTTPS at its edge, so it handles the traffic unencrypted there, as for any site it serves. That is the trade for needing no certificate or open port at home; use your own reverse proxy if you would rather not.

## Privacy in brief

- **Nothing goes to the project.** No analytics, advertising, tracking or third-party SDKs. The apps talk only to your own gateway.
- **Your server keeps what it needs:** accounts, devices (including each device's last address, for admins), an audit log for a year, and, per user, a listening history kept for 13 months that you can view, clear or switch off. No one else, admins included, can see your history through the apps or the API.
- **Your phone keeps your sign-in sealed by the Android Keystore,** your settings and your downloads. Signing out can remove the downloads; a revoked device deletes them.
- **Lyrics, biographies and reviews** come from your server's own plugins; Lyrion Anywhere never calls those services itself.

The full policy, including Google Play's Data safety answers, is [`PRIVACY.md`](PRIVACY.md); the privacy controls in the apps are listed in [`FEATURES.md`](FEATURES.md#privacy-controls).

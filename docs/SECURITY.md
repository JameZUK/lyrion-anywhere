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
- **Hardened Android app:** HTTPS with system certificates only; plain HTTP only to a literal private address (your own VPN, where the admin has listed the network: [below](#trusted-networks)), after a warning, and marked *Unencrypted* in Settings, never to a name or a public address; backups off, and the sign-in sealed by the Android Keystore.
- **HTTPS required,** with one opt-in exception: plain HTTP from private networks the admin lists, such as a VPN ([below](#trusted-networks)).
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
- **The tunnel has a listener of its own.** `cloudflared` sends the tunnel's requests to the gateway's tunnel listener (`--tunnel-listen`, default `127.0.0.1:9444`, loopback only, not opened with `--no-tunnel`), never to the public listen address. Requests that arrive on that listener, and only they, have the client's address read from `CF-Connecting-IP` (a single valid address; otherwise the usual `X-Forwarded-For` rule), and while a tunnel runs only they are accepted from pages at `https://<its hostname>`. The mark is set by the listener itself, so nothing a client sends can claim it. The public listener behaves exactly as configured, tunnel or not: its trusted proxies and per-address connection cap don't change, and it never reads `CF-Connecting-IP` unless *Client address header* says so. So a reverse proxy on the same machine that passes a client's own `CF-Connecting-IP` through can't let that client choose the address the lockouts and the audit log see. HTTPS is still required on the tunnel listener: `cloudflared` reports `X-Forwarded-Proto: https` from loopback, which the usual rule accepts.
- **A tunnel address is a public address.** Sign-in, two-step verification, lockouts and rate limits apply exactly as behind a reverse proxy. The temporary tunnel's random address reaches the sign-in page for anyone who has it, until it stops (after an hour at most).
- **Cloudflare is in the path.** Cloudflare ends HTTPS at its edge, so it handles the traffic unencrypted there, as for any site it serves. That is the trade for needing no certificate or open port at home; use your own reverse proxy if you would rather not.

## Trusted networks

The gateway insists on HTTPS: a plain-HTTP request from anything but the LMS machine itself is refused with `403 TLS required`. An admin who reaches home over a VPN may list private networks as *trusted* (the plugin's *Trusted networks* setting, `--trusted-network`; SPEC D28, [`INSTALL.md`](INSTALL.md#option-d-your-own-vpn-no-https) Option D), and the gateway then serves plain HTTP to devices inside them too. With none listed, which is the default, nothing changes.

- **What it costs.** On a trusted network, traffic crosses unencrypted: passwords at sign-in, two-step codes, sign-in tokens, pairing codes, the library and the music. The VPN's own encryption is what protects it between the device and home. Anyone who can read or inject traffic on that network can read or replay a sign-in.
- **The network boundary is the admin's.** The gateway can't tell whether a packet from `10.8.0.5` really crossed the VPN. That is why only private networks can be listed, why the docs and the settings page say to bind the *Listen address* to the VPN interface (never `0.0.0.0`) and firewall the port to it, and why the page points out a listen address that doesn't fit.
- **Only private networks.** Each must lie within `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `100.64.0.0/10`, `fc00::/7` or `fe80::/10`. Public ranges, catch-alls (`0.0.0.0/0`, `::/0`), anything including loopback, and IPv4 networks written as IPv6 are refused when the setting is saved and again when the gateway starts. One rule (`core::privnet`), with shared test vectors, serves the gateway, the plugin, the web app and the Android app.
- **Nothing a client sends widens it.** The networks come only from the gateway's command line, which only the settings page sets (LMS's `pref` command can't change them, as for the other gateway settings). A request from a trusted network that carries `X-Forwarded-For` or `X-Forwarded-Proto` is not believed: it is refused unless it comes from a trusted proxy saying `https`. The client's address for lockouts and the audit log is the TCP peer's, as for any direct client.
- **Still refused, as before:** plain HTTP from every address outside the trusted networks; the admin API from anywhere but loopback; LMS's own ports, which still never need exposing. The apps accept `http://` only to a literal private address, never to a name (which could resolve to a public address), whatever a server or a pairing link says; the Android app warns before signing in and shows an *Unencrypted* badge, and browsers show a banner and withhold installing and offline use.
- **The cookie.** The browser's refresh cookie loses `Secure` only on a plain-HTTP request from a trusted network, since browsers would drop it otherwise; it stays `HttpOnly`, `SameSite=Strict` and scoped to `/v1/auth`.

To keep HTTPS over a VPN instead, use Tailscale Serve or a reverse proxy with an internal certificate: both work with no change and keep every browser feature.

## Privacy in brief

- **Nothing goes to the project.** No analytics, advertising, tracking or third-party SDKs. The apps talk only to your own gateway.
- **Your server keeps what it needs:** accounts, devices (including each device's last address, for admins), an audit log for a year, and, per user, a listening history kept for 13 months that you can view, clear or switch off. No one else, admins included, can see your history through the apps or the API.
- **Your phone keeps your sign-in sealed by the Android Keystore,** your settings and your downloads. Signing out can remove the downloads; a revoked device deletes them.
- **Lyrics, biographies and reviews** come from your server's own plugins; Lyrion Anywhere never calls those services itself.

The full policy, including Google Play's Data safety answers, is [`PRIVACY.md`](PRIVACY.md); the privacy controls in the apps are listed in [`FEATURES.md`](FEATURES.md#privacy-controls).

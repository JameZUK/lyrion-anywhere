# Trusted networks (unencrypted connections over a VPN): agreed design

Agreed with the owner on 2026-10-24. Ships with the next plugin release and the matching Android release. It covers the gateway, the LMS plugin's settings page, the Android app, the web app and the documentation.

## Why

Until now the only supported way in was HTTPS: a reverse proxy with a certificate, or the built-in Cloudflare Tunnel. Some people would rather not run either and already have a VPN into their home (WireGuard, Tailscale, a router's VPN). Inside the VPN the traffic is encrypted by the VPN itself, so insisting on HTTPS buys little and costs a certificate and a proxy. This lets an admin say "connections from these private networks may be unencrypted", and nothing else changes.

## Choices made

- **Off by default, and the default is byte for byte what it was.** With the setting empty, every code path behaves exactly as before: plain HTTP from anything but loopback is refused with `403 TLS required`, cookies are `Secure`, the apps refuse `http://` addresses. The existing test suite is the proof, and it stays unchanged.
- **The admin decides, server-side.** The plugin's settings page gains *Trusted networks*: a comma-separated list of private networks (CIDR), passed to the gateway as repeated `--trusted-network` flags. A user can't turn it on from a phone or a browser.
- **Private networks only.** A network is accepted only if it lies wholly within one of: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `100.64.0.0/10` (Tailscale's range), `fc00::/7`, `fe80::/10`. Anything else is refused when the setting is saved and again when the gateway starts: a public range, `0.0.0.0/0`, `::/0`, multicast, and loopback (always allowed anyway, so listing it is a mistake worth saying). One shared definition in `core::privnet`, used by the gateway and the Android app, with the same vectors run against the plugin's Perl check and the web app's JavaScript check (`web/app/tests/fixtures/private-networks.json`).
- **The gateway listens where the admin says.** The docs tell admins to set the *Listen address* to the VPN interface's address (e.g. `10.8.0.1:9443`), not `0.0.0.0`, so the LAN and the internet never see a plain listener. The gateway does not widen its own listener.
- **Android: literal private addresses only.** The app accepts `http://10.8.0.1:9443`, never `http://name`, because a name could resolve to a public address and send the password in clear. Before signing in to such a server the app shows a warning and asks for confirmation, and an *Unencrypted* badge stays in Settings beside the server. The release build's network security configuration keeps forbidding cleartext for Android's own HTTP stacks; the Rust layer makes the connections and applies the rule above.
- **Web: full support, with a banner.** Browsers treat `http://` on anything but localhost as an insecure context, which removes AudioWorklet, Web Locks, `navigator.storage`, the service worker and the Cache API. The web app plays through its media-element path there (Media Source with FLAC in MP4, as on the iPad), falls back where an API is missing, and shows a banner saying the connection is unencrypted and that installing, offline use and some player features need HTTPS. Verified in Chromium and Firefox against a gateway on a LAN address.
- **Pairing codes and QR links** may point at `http://<private address>` too; the plugin and the web app apply the same private-literal rule when they build or read one.
- **Alternatives kept in the docs:** Tailscale Serve or any VPN plus a proxy with an internal certificate already work with no change, and keep every browser feature. The docs say so before describing trusted networks. The gateway terminating TLS with its own certificate was rejected: every device would need the CA installed, and trusting user CAs in the Android app would weaken its system-only posture.

## How it works

### Gateway

- `--trusted-network <CIDR>` (repeatable). Each value is parsed and checked by `anywhere_core::privnet::trusted_network`; an invalid one stops the gateway from starting with a sentence saying why. At start-up a warning line names the networks accepted unencrypted.
- `require_secure_transport` admits a request with no proxy headers from a peer inside a trusted network, exactly as it admits one from loopback. A peer inside a trusted network that sends `X-Forwarded-For` or `X-Forwarded-Proto` is still refused unless it is a configured trusted proxy saying `https`: nothing a client sends can change the rule.
- The client's address, the lockout key and the audit log are the peer's address, as for any direct client today.
- The refresh cookie is set without `Secure` only on a plain-HTTP request from a trusted network (browsers refuse a `Secure` cookie set over plain HTTP from anything but localhost). On loopback and on every HTTPS request the cookie is unchanged. Logging out clears it the same way.
- `origin_allowed` already accepts `http://<Host>` as the same origin on a plain-HTTP request, so the web app served by the gateway at `http://10.8.0.1:9443` works without an `--allowed-origin`.

### Plugin

- New preference `trusted_networks` (string, comma-separated CIDRs), validated in Perl against the same rule and vectors, saved only when every entry is valid; passed as `--trusted-network` flags. The settings page shows it under a warning, with a note that the *Listen address* must be the VPN interface's address for it to be of any use. The gateway's listener check copes with a non-loopback listen address.
- The *Public URL* may be `http://<private address>[:port]` (the existing "Allow http:// anyway" tick), and the pairing link and QR code are built for it.

### Android

- `parse_server_url` accepts `http://` when the host is a literal address inside a private range (`privnet::is_private`); `http://name` is refused with a message saying why, and public literals too. `Server::is_secure()` is false for such a server, and the Kotlin layer's `PLAIN_HTTP_REFUSED` check becomes a confirmation step instead of a refusal for private literals.
- Sign-in shows a dialog ("This connection isn't encrypted. Only use it over a VPN or on a network you trust.") with *Connect anyway* and *Cancel*. Settings shows an *Unencrypted* badge beside the server while signed in to one.
- Artwork and every other request go through the Rust layer, which applies the rule; the network security configuration is unchanged.

### Web

- Capability detection checks `audioWorklet` on a context (Chromium exposes `AudioWorkletNode` in insecure contexts but not `audioWorklet`), so the mode chooser picks the media element over plain HTTP.
- Guards for the APIs missing in an insecure context: Web Locks (already optional), `navigator.storage` (persist and estimate), the service worker and the Cache API (update flow and offline shell), `crypto.randomUUID` (already optional).
- A banner on `location.protocol === 'http:'` outside localhost, dismissible per session.
- An opt-in end-to-end check (Chromium and Firefox through Playwright) against a local gateway bound to a LAN address with `--trusted-network`, exercising sign-in, the player in media mode and the banner.

## Tests

- `core::privnet`: parsing and normalisation, containment, every allowed range, the refusals (public, catch-all, loopback, multicast, mapped IPv6, garbage), from the shared vectors file.
- Gateway: the transport decision table with and without trusted networks (loopback, LAN peer inside and outside, proxy headers from inside, mapped IPv6 peer); the cookie's `Secure` flag per case; the gateway refusing to start on an invalid network; the existing hardening tests unchanged.
- Plugin: Perl validation against the vectors, the arguments passed, the page's warning, the pairing link for an `http://` private URL; JavaScript tests for the page script.
- Android: `parse_server_url` cases (private literal accepted, name refused, public literal refused, loopback as before); the confirmation dialog and the badge (Compose and Robolectric); the Kotlin repository's decision.
- Web: capability detection in an insecure context, the mode choice, every guard with the API absent, the banner; the opt-in browser check above.

## Security notes

- The gateway cannot tell whether a packet from `10.8.0.5` really crossed the VPN: the network boundary is the admin's. That is why the networks must be private, why the listener should be bound to the VPN address, and why the docs say so in the first sentence of the section.
- Passwords and tokens do cross a trusted network in clear. SECURITY.md and PRIVACY.md say "plain HTTP is refused unless the admin allows it for a private network", and the Android data-safety wording follows.
- Nothing a client sends can widen the rule: the networks come from the command line, proxy headers from a trusted-network peer are not believed, and the apps refuse `http://` to anything public whatever the server says.

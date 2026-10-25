# The README, refreshed: a landing page, two walkthroughs, new screenshots

_Agreed with the owner on 2026-10-25. Goal in the owner's words: refactor, standardise and simplify the README; the complex material on separate pages; screenshots that show the new signal path and how to set up a connection with and without a VPN; easy for the average person, impressive, reading as if a person wrote it; British English throughout._

## Principles

- **The README is a landing page, under 120 lines.** Pictures first, then plain words. A visitor should know in ten seconds what this is, what it needs, and how to start.
- **Plain and direct, to "you".** Short sentences. No superlatives, no jargon on the first screen; a technical word appears only where the reader needs it, explained once. British English.
- **One idea per line in lists.** A bullet is a sentence, not a paragraph.
- **Every picture earns its place** and has an alt text that says what is on it. The library, players, people and artwork in every screenshot are invented, and the page says so once.
- **Nothing personal or homelab-specific** in words or pixels (the standing rule). Example names only: `music.example.com`, `10.8.0.1`.
- **The complex material moves, it does not vanish.** Each technical paragraph that leaves the README lands on a page that is linked from where it used to be.

## The README's shape

1. **Logo, title, tagline.** "Your music, on any device, anywhere."
2. **A row of quiet badges:** latest plugin release, latest Android release, licence, CI.
3. **Three short paragraphs.** What it is: Lyrion Music Server is the server for your music at home; Lyrion Anywhere lets every phone, tablet and browser be a player of it, wherever you are, in step with the players at home, lossless. What it needs: LMS 9 or later and one of two ways of reaching it from outside. What stays private: LMS itself never faces the internet; one small, locked-down gateway does, and only over HTTPS or your VPN.
4. **The hero strip:** three images side by side. Android Now playing with the badge *Hi-Res · FLAC 24/96*; the Android signal path as the block diagram with a *Transcoded* stream and the amber gateway block; the web app's Now playing on a phone.
5. **What you get:** eight one-line bullets. Any phone, tablet or browser is a real player. It plays in step with the players at home. Lossless and gapless, decoded on the device. See what you hear: the badge and the signal path. Download albums for offline listening. Frugal on mobile data when you want it. Everything LMS has: apps, radio, alarms, rooms. Nothing exposed but one door. Then one line to FEATURES.md.
6. **Screenshots:** a strip of six (web and Android), and a link to a gallery page with the rest.
7. **Get started, in three steps.**
   1. Install the plugin in LMS (the repository URL).
   2. Choose how to reach it, as two cards side by side: **Got a domain?** The built-in Cloudflare Tunnel: no open port, no certificate, set up from the plugin's settings page, three lines, link to the walkthrough. **Use a VPN?** Trusted networks: your VPN's subnet on the settings page, no HTTPS, the phone enters the server's private address, three lines, link to the walkthrough. Under the cards, one line: already running your own reverse proxy? nginx and Caddy are in the domain walkthrough.
   3. Sign in: open the address in a browser, or install the Android app and scan the QR code from *Add a device*.
   Then one line: the app and the plugin move together; each tells you when the other needs updating.
8. **How it works:** the existing diagram and four lines (clients speak only HTTPS or your VPN; the gateway is the only thing reachable; the plugin runs it; LMS's ports stay at home). A link to *Under the bonnet* and to ARCHITECTURE.md.
9. **Documentation:** a six-row table (walkthroughs, install guide, features, security and privacy, under the bonnet, everything else).
10. **Licence and acknowledgements,** two lines each.

## New and moved pages

- **`docs/SETUP-DOMAIN.md`:** a numbered, screenshot-led walkthrough of one screen's reading: install the plugin; the Cloudflare Tunnel section (Install, Register, or Try it first); the Public URL; create a user; sign in on the phone with the QR code. A second, shorter section for your own reverse proxy (nginx and Caddy snippets, the Public URL, trusted proxies). Links to INSTALL.md for every detail.
- **`docs/SETUP-VPN.md`:** the same for a VPN (WireGuard, Tailscale or your router's): the Listen address on the VPN interface, Trusted networks, the Public URL as `http://10.8.0.1:9443` with *Allow http://*, the QR code, what the Android app shows (the warning, the *Unencrypted* badge, why it needs the IP address, not a name), what the browser shows, and what crosses the VPN in clear.
- **`docs/TECHNICAL.md`, "Under the bonnet":** the README's technical section as one readable page (one Rust core, its own decoders, 64-bit DSP that steps aside, what you hear shown: the shared quality model and protocol 2, one small gateway, a player link that survives the network, the measurements), with the request-flow detail still in ARCHITECTURE.md.
- **`docs/SCREENSHOTS.md`:** every screenshot with a one-line caption, grouped web then Android, with the settings page shots last.
- **`docs/INSTALL.md`** stays the complete reference; its introduction points to the two walkthroughs. **`docs/README.md`** gains the new pages. **`docs/FEATURES.md`** keeps its rows; its images are regenerated.

## Screenshots

All regenerated, because every current one shows the old badge, and the signal path's look has changed.

- **Web (the demo, `tools/demo/shots.sh`, Playwright):** every existing scene again; new scenes: the signal-path sheet on a phone with a *Transcoded · Opus 160* stream (the amber gateway block and the quality switch), the same on a desktop as a row of blocks, and Now playing with the badge in both layouts. The demo's gateway needs `ffmpeg` on this machine and the live transcode enabled so a High stream really is Opus; the script chooses High in the sheet before the shot.
- **Android (`DocScreenshots.kt`, Roborazzi):** every existing scene again; new scenes: the signal-path sheet as the block diagram (the Transcoded case), Now playing with *Hi-Res · FLAC 24/96*, Sound settings with *Fix output level at 100%*, the sign-in screen showing an `http://` private address with its warning (for the VPN walkthrough), the mini player with the badge.
- **The plugin's settings page (new):** a throwaway stock LMS 9.1.1 in Docker on this machine with the plugin and gateway installed and example values (`https://music.example.com`, `10.8.0.0/24`), shot with Playwright: the Public URL with the *Allow http://* box, the Cloudflare Tunnel section, Trusted networks, Users with *Add a device* and its QR code, the Devices list with a *needs updating* tag.
- Every PNG optimised as the scripts do; alt text written with the image, not after.

## Who does what

Three agents in parallel, then the coordinator writes the words: one agent regenerates and extends the web screenshots, one the Android screenshots, one builds the stock-LMS container and shoots the settings page. The README, the two walkthroughs, *Under the bonnet* and the gallery are written by the coordinator in one voice, then read once more end to end for British English, plainness and length before the commit. Nothing is pushed until the owner has read the README.

## Out of scope

Rewriting FEATURES.md, INSTALL.md or ARCHITECTURE.md beyond the links and introductions; a new logo; anything that changes the apps.

<p align="center">
  <img src="docs/images/logo.svg" alt="Lyrion Anywhere logo: a waveform turning into broadcast waves" width="112" height="112">
</p>

<h1 align="center">Lyrion Anywhere</h1>

<p align="center"><strong>Your music, on any device, anywhere.</strong></p>

<p align="center">
  <a href="https://github.com/JameZUK/lyrion-anywhere/releases/latest"><img alt="Latest plugin release" src="https://img.shields.io/github/v/release/JameZUK/lyrion-anywhere?label=plugin&style=flat-square"></a> <a href="https://github.com/JameZUK/lyrion-anywhere/releases?q=android-v&expanded=true"><img alt="Latest Android release" src="https://img.shields.io/github/v/release/JameZUK/lyrion-anywhere?filter=android-v*&label=android&style=flat-square"></a> <a href="https://github.com/JameZUK/lyrion-anywhere/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/JameZUK/lyrion-anywhere/ci.yml?branch=main&label=CI&style=flat-square"></a> <a href="LICENSE"><img alt="Licence: GPL-2.0-or-later" src="https://img.shields.io/badge/licence-GPL--2.0--or--later-blue?style=flat-square"></a>
</p>

[Lyrion Music Server](https://lyrion.org) is the server for the music you keep at home, playing it on Squeezeboxes and other players around the house. Lyrion Anywhere takes it with you. Sign in on your phone, your tablet or any browser and that device becomes one of your players: on the train, at work, in the car. It plays your library losslessly, keeps in step with the players at home, and can control them.

You need Lyrion Music Server 9 or later and one of two ways of reaching it from outside: a domain name (the plugin can set up a Cloudflare Tunnel for you, with no port to open) or a VPN you already use. Your server itself never faces the internet. One small, locked-down gateway does, over HTTPS or your VPN, and that is all.

<p align="center">
  <img src="docs/images/android/now-playing.png" alt="Now playing in the Android app: the album cover, the title, the artist and album with a badge reading Hi-Res, FLAC 24/96, a waveform seek bar and the transport controls" width="250">
  &nbsp;
  <img src="docs/images/android/signal-path.png" alt="The signal path in the Android app as three blocks: File, FLAC 24/96; Lyrion gateway, tinted amber, re-encoded to Opus 160 kb/s on mobile data, with Original, High and Data saver buttons beneath; This device, Opus 48 kHz on the phone speaker" width="250">
  &nbsp;
  <img src="docs/images/web/now-playing-phone.png" alt="Now playing in the web app on a phone: the cover, the title with a rating star, the quality badge, the waveform seek bar and large transport controls" width="250">
</p>

> The library, players, people and artwork in every screenshot are invented. They come from the built-in demo and the apps' screenshot tests.

## What you get

- **Any phone, tablet or browser is a real player.** It appears in Lyrion like a Squeezebox, so gapless playback, alarms, plugins and play counts all work.
- **It plays in step with the players at home.** Join a group with the Squeezeboxes and the music stays together, even through a dropped connection.
- **Lossless and gapless, decoded on the device.** FLAC plays as FLAC. On Android the output is bit-perfect to a USB DAC where the phone allows it.
- **See what you hear.** A badge says whether the stream is Hi-Res, Lossless, Transcoded or Lossy; tap it for the path from file to speaker, and change the streaming quality there and then.
- **Take albums with you.** Download your own library for offline listening; plays are counted when you are back.
- **Frugal on mobile data when you want it.** The gateway re-encodes to Opus on the fly, and the apps choose for you on mobile data.
- **Everything Lyrion has.** Your whole library, every app, radio station and plugin, alarms, rooms, and moving playback between players.
- **Nothing exposed but one door.** Your own user accounts, two-step sign-in, a list of your signed-in devices, and a gateway that passes only an allow-list of commands.

The full list, by platform, is in [`docs/FEATURES.md`](docs/FEATURES.md).

<p align="center">
  <img src="docs/images/web/now-playing-desktop.png" alt="Now playing in the web app on a desktop browser: a large cover beside the title, artist and album with the quality badge, the current lyric line, the waveform seek bar and controls, with the queue pinned on the right" width="820">
  <img src="docs/images/web/signal-path-phone.png" alt="The signal path in the web app on a phone: the blocks File, Lyrion gateway and This device stacked, the gateway block amber for a re-encoded stream, with the streaming quality buttons beneath it" width="200">
  <img src="docs/images/android/play-on.png" alt="The Play on sheet in the Android app: this phone, then every player at home with what it is doing, and Group players" width="200">
  <img src="docs/images/android/downloads.png" alt="Downloads in the Android app: storage used, downloads in progress and downloaded albums" width="200">
  <img src="docs/images/web/rooms-desktop.png" alt="Rooms in the web app on a desktop browser: a card for each player with its artwork, track, play and pause and volume" width="200">
</p>

More in [`docs/SCREENSHOTS.md`](docs/SCREENSHOTS.md).

## Get started

**1. Install the plugin.** In Lyrion's web interface open *Settings › Plugins*, add this repository under *Additional repositories*, tick **Lyrion Anywhere** and restart the server:

```
https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml
```

**2. Choose how you will reach it from outside.**

<table>
<tr>
<td width="50%" valign="top">

**Got a domain name?**

Use the built-in Cloudflare Tunnel. Nothing to open on your router and no certificate to look after: the plugin's settings page installs and runs it for you, and your server is reachable as `https://music.example.com`.

[Set up with a domain →](docs/SETUP-DOMAIN.md)

</td>
<td width="50%" valign="top">

**Use a VPN?**

Keep the gateway inside your VPN (WireGuard, Tailscale, your router's). List the VPN's network as trusted on the settings page and connect to the server's private address, no HTTPS needed.

[Set up over a VPN →](docs/SETUP-VPN.md)

</td>
</tr>
</table>

Already running your own reverse proxy? nginx and Caddy are covered in the domain walkthrough too.

**3. Sign in.** Open the address in a browser and sign in, or install the [Android app](https://github.com/JameZUK/lyrion-anywhere/releases?q=android-v&expanded=true) and scan the QR code from *Add a device* on the settings page. Pressing Play plays on that device straight away.

The app and the plugin move forward together. When one needs updating, the other tells you.

## How it works

<p align="center">
  <img src="docs/images/how-it-works.svg" alt="The web app and the Android app reach a reverse proxy or a Cloudflare Tunnel over HTTPS only, which forwards to the Lyrion Anywhere gateway; the gateway talks to Lyrion Music Server at home, whose own ports are never forwarded; the plugin starts and supervises the gateway" width="600">
</p>

- Your devices speak only HTTPS, or your VPN, to a single gateway.
- The gateway signs devices in, passes an allow-list of commands to Lyrion and scrubs the answers, carries each device's player connection and relays the audio.
- The Lyrion plugin starts the gateway, restarts it if it stops, and gives you a settings page for users, devices and options.
- Lyrion's own ports stay on your home network.

The engineering behind it is in [`docs/TECHNICAL.md`](docs/TECHNICAL.md), and the full design in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Documentation

| Page | What it covers |
|---|---|
| [Set up with a domain](docs/SETUP-DOMAIN.md) · [Set up over a VPN](docs/SETUP-VPN.md) | The two walkthroughs, with screenshots of each step |
| [`docs/INSTALL.md`](docs/INSTALL.md) | The complete installation guide: every option, users, checks, troubleshooting |
| [`docs/FEATURES.md`](docs/FEATURES.md) · [`docs/PLATFORMS.md`](docs/PLATFORMS.md) | Every feature by area, and where it runs |
| [`docs/SECURITY.md`](docs/SECURITY.md) · [`docs/PRIVACY.md`](docs/PRIVACY.md) | What is protected, what is stored, and how to report a problem |
| [`docs/TECHNICAL.md`](docs/TECHNICAL.md) | Under the bonnet: the player core, the gateway, the measurements |
| [`docs/README.md`](docs/README.md) | Everything else: architecture, protocol, building, releasing, design history |

## Licence

GPL-2.0-or-later ([`LICENSE`](LICENSE)); binaries that include Apache-2.0 dependencies are distributed under GPLv3. An additional permission lets the apps be distributed through app stores as long as the full source stays available under the GPL ([`LICENSE-EXCEPTION`](LICENSE-EXCEPTION)).

## Acknowledgements

With thanks to [Lyrion Music Server](https://lyrion.org) and its community (this is an independent project, not an official Lyrion one), to [squeezelite](https://github.com/ralph-irving/squeezelite) and [Material Skin](https://github.com/CDrummond/lms-material) as references for behaviour in this clean-room implementation, and to the open-source building blocks listed in [`docs/BUILDING.md`](docs/BUILDING.md#acknowledgements).

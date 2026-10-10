# Installing Lyrion Anywhere

This guide takes you from a working LMS at home to playing your music on your phone over the internet. It takes about half an hour if you already have a domain name; most of that time is setting up HTTPS.

Every name and address here is an example. Throughout:

| In this guide | Means | Replace with |
|---|---|---|
| `music.example.com` | the public address people will use | your own (sub)domain |
| `lms.example.lan` / `10.0.0.20` | the machine running LMS | your LMS machine |
| `10.0.0.10` | the machine running the reverse proxy, if it isn't the LMS machine | your proxy |

**Contents**

1. [Before you start](#1-before-you-start)
2. [Install the gateway](#2-install-the-gateway)
   - [Option A: the LMS plugin (recommended)](#option-a-the-lms-plugin-recommended)
   - [Option B: Docker](#option-b-docker)
3. [Configure the plugin](#3-configure-the-plugin)
4. [Set up HTTPS (reverse proxy)](#4-set-up-https-reverse-proxy)
5. [Create users](#5-create-users)
6. [First sign-in, and installing the app](#6-first-sign-in-and-installing-the-app)
7. [Check your set-up](#7-check-your-set-up)
8. [Everyday administration](#8-everyday-administration)
9. [Troubleshooting](#9-troubleshooting)
10. [Updating and uninstalling](#10-updating-and-uninstalling)

---

## 1. Before you start

You need:

- **Lyrion Music Server 9.0 or later**, working on your home network.
- **A domain name** you can point at your home, e.g. `music.example.com`. A free dynamic-DNS name works, as does a subdomain of a domain you own.
- **A reverse proxy with an HTTPS certificate** for that name: nginx, Caddy, Traefik or similar. Let's Encrypt certificates are free. If you have nothing yet, [Caddy](https://caddyserver.com) is the quickest (§4.2): it gets and renews certificates by itself.
- **Port 443 forwarded** from your router to the reverse proxy (and port 80, if your certificate tool uses HTTP validation).
- **Optional: `ffmpeg`** on the machine running the gateway, for offline downloads as MP3 or Opus (e.g. `sudo apt install ffmpeg`; it needs the `libmp3lame` and `libopus` encoders, which distribution packages include). Without it, downloads are offered in their original format only. Lyrion Anywhere doesn't bundle ffmpeg.

What you must **not** do: forward or expose LMS's own ports (3483, 9000, 9090) to the internet. Lyrion Anywhere exists so that you never have to.

```
 phone / browser ──HTTPS──▶ reverse proxy ──HTTP──▶ gateway ──▶ LMS
  (internet)                  (port 443)          (port 9443)   (LAN only)
```

## 2. Install the gateway

### Option A: the LMS plugin (recommended)

The plugin bundles the gateway for Linux on x86_64, aarch64 (64-bit Raspberry Pi and similar) and armhf (32-bit Raspberry Pi OS), plus the web app. It starts the gateway with LMS, restarts it if it stops, and gives you a settings page for everything.

**From a release** (once releases are published):

1. In LMS, open *Settings › Plugins*.
2. At the bottom, under *Additional repositories*, add the release's `repo.xml` URL, shown on the project's [Releases](https://github.com/JameZUK/lyrion-anywhere/releases) page. Click *Apply*.
3. Find **Lyrion Anywhere** in the list, tick it, click *Apply*, and restart LMS when asked.

**From source:**

```sh
git clone https://github.com/JameZUK/lyrion-anywhere.git
cd lyrion-anywhere
tools/build-release.sh          # needs Docker with BuildKit; ~10 minutes the first time
ls dist/                        # LyrionAnywhere-<version>.zip, repo.xml
```

Unzip `dist/LyrionAnywhere-<version>.zip` into a folder called `LyrionAnywhere` inside LMS's plugin folder, then restart LMS. *Settings › Information* in LMS lists its plugin folders. On the Debian/Ubuntu package it is `/var/lib/squeezeboxserver/Plugins/`:

```sh
sudo unzip dist/LyrionAnywhere-0.1.0.zip -d /var/lib/squeezeboxserver/Plugins/LyrionAnywhere
sudo chown -R squeezeboxserver: /var/lib/squeezeboxserver/Plugins/LyrionAnywhere
sudo systemctl restart lyrionmusicserver      # or: logitechmediaserver / squeezeboxserver
```

Keep exactly one copy of the plugin in that folder. A backup copy left next to it (e.g. `LyrionAnywhere.old`) confuses LMS's plugin loader.

> **How the bundled gateway is checked.** The plugin refuses to run a bundled gateway whose checksum doesn't match `Bin/SHA256SUMS`, which catches a corrupt or accidentally replaced file. It isn't a signature, so only install releases from the project's own GitHub page.

### Option B: Docker

Use this if your LMS runs somewhere the plugin can't run its binary (e.g. Windows or macOS), or if you want the gateway on a different machine from LMS. You then manage accounts from the command line instead of LMS's settings page.

```sh
git clone https://github.com/JameZUK/lyrion-anywhere.git
cd lyrion-anywhere
docker build -t lyrion-anywhere .
docker volume create anywhere-data

# Your first admin account (you'll be asked for the password on standard input):
docker run --rm -i -v anywhere-data:/data lyrion-anywhere \
    admin --data-dir /data create-user alice --role admin

# Run the gateway:
docker run -d --name lyrion-anywhere --restart unless-stopped --network host \
    -v anywhere-data:/data lyrion-anywhere \
    serve --data-dir /data --static-dir /web \
          --listen 127.0.0.1:9443 \
          --lms lms.example.lan \
          --allowed-origin https://music.example.com
```

The main `serve` options:

| Option | Default | Purpose |
|---|---|---|
| `--listen` | `127.0.0.1:9443` | Where your reverse proxy connects. Keep it on loopback, or a LAN address firewalled to the proxy. |
| `--lms` | `127.0.0.1` | LMS's host name or address. |
| `--lms-http-port`, `--lms-cli-port`, `--lms-slimproto-port` | 9000, 9090, 3483 | LMS's ports, if you changed them. |
| `--allowed-origin` | | Your public URL, e.g. `https://music.example.com`. |
| `--trusted-proxy` | | The proxy's address, if it isn't on the same machine (repeatable). |
| `--admin-listen` | `127.0.0.1:9445` | The admin API, for the plugin and the `admin` commands. Never expose it. |
| `--ffmpeg` | `ffmpeg` on `PATH` | ffmpeg for MP3/Opus downloads (an absolute path). |
| `--no-transcode` | | Offer downloads in their original format only, even with ffmpeg installed. |
| `--download-cache-mb` | 2048 | Disk space (MiB, 16–1048576) for transcoded downloads, kept in `<data-dir>/download-cache`. The least recently used are removed first. |

The Docker image has no ffmpeg, so downloads from it are original-format only.

Account commands: `docker exec lyrion-anywhere anywhere-gateway admin --data-dir /data <command>`, where `<command>` is `list-users`, `list-devices`, `create-user NAME --role admin|listener`, `set-password NAME`, `pair NAME` or `reset-totp NAME`.

The image is static, has no shell and runs as an unprivileged user. Host networking keeps clients' real addresses visible to the gateway (needed for sign-in lockouts). With a bridge network instead:
- pass `--listen 0.0.0.0:9443` inside the container;
- publish with `-p 127.0.0.1:9443:9443`;
- add the bridge's gateway address as `--trusted-proxy`.

## 3. Configure the plugin

Open *Settings › Plugins › Lyrion Anywhere* in LMS. (Docker users: the same settings are the `serve` options above.)

| Setting | What to enter |
|---|---|
| **Gateway: Enabled** | Ticked. |
| **Listen address** | `127.0.0.1:9443` if the reverse proxy runs on the LMS machine. Otherwise the LMS machine's LAN address, e.g. `10.0.0.20:9443`; then firewall that port so only the proxy can reach it. |
| **Public URL** | `https://music.example.com`, exactly as people will type it. |
| **Trusted proxies** | The proxy's LAN address (e.g. `10.0.0.10`) if it runs on another machine; otherwise leave empty. |
| **Gateway path** | Leave empty. It's for development and ignored unless LMS is started with `ANYWHERE_ALLOW_GATEWAY_PATH=1`. |
| **Transcoded downloads** | Ticked (the default) to offer MP3 and Opus downloads as well as the original. Needs ffmpeg (§1). |
| **ffmpeg path** | Leave empty to use `ffmpeg` from the system path. Otherwise the absolute path to a program named `ffmpeg`, e.g. `/usr/local/bin/ffmpeg`. |
| **Download cache (MiB)** | Disk space for transcoded downloads (default 2048), so repeat downloads cost nothing. |

Click *Apply*. The **Status** panel should show the gateway as running. If it shows an error, see [Troubleshooting](#9-troubleshooting).

The same page has these sections:
- **Users:** add users; for each one, *Disable*/*Enable*, *Delete*, set a *New password*, *Add a player…* (grant a home player), *Reset two-step*, *Pairing code*, and *Turn off downloads*/*Allow downloads* (offline downloads are allowed for every user by default; the *Downloads* column shows the formats each may use).
- **Sign-in › Stay signed in:** how many days an unused device stays signed in (1–365, default 30). Every use renews it.
- **Plugins › Admins only:** the plugins LMS offers remote devices. Tick one to hide it from listeners and refuse its use, e.g. a plugin with server maintenance actions.
- **Devices:** every signed-in device, when it was last seen, its *Diagnostics*, and *Sign out* (e.g. for a lost phone).
- **Recent activity:** sign-ins, failures and administrative changes.

> **Your LAN and LMS's own security.** By default, LMS's web interface and control ports (9000, 9090) don't ask for a password on your home network. Anyone on your network can therefore open this settings page. Changes to remote accounts are only accepted from this page (other web sites can't make them through LMS), but if other people use your network, turn on LMS's password protection under *Settings › Advanced › Security*.

## 4. Set up HTTPS (reverse proxy)

The proxy terminates HTTPS and passes requests, including WebSockets, to the gateway. Whatever proxy you use, it must:

- **Replace `X-Forwarded-For` with the client's address, never append to it.** The gateway uses that address for sign-in lockouts, and an appended header would let a client choose its own.
- **Pass WebSocket upgrades** and allow long-lived connections (players and audio streams stay open for hours).
- **Add no Content-Security-Policy of its own.** The gateway sets its own strict headers. Adding HSTS is good.

### 4.1 nginx

A complete, tested configuration is in [`deploy/nginx/lyrion-anywhere.conf`](../deploy/nginx/lyrion-anywhere.conf); CI runs it in front of the gateway on every change. Copy it, then replace `music.example.com`, the certificate paths and the `upstream` address:

```nginx
upstream lyrion_anywhere {
    server 127.0.0.1:9443;          # the gateway's listen address (or 10.0.0.20:9443)
    keepalive 16;
}
```

```sh
sudo cp deploy/nginx/lyrion-anywhere.conf /etc/nginx/sites-available/
sudo ln -s ../sites-available/lyrion-anywhere.conf /etc/nginx/sites-enabled/
sudo certbot certonly --nginx -d music.example.com     # or your usual certificate tool
sudo nginx -t && sudo systemctl reload nginx
```

### 4.2 Caddy

Caddy gets a certificate automatically and replaces `X-Forwarded-For` by default. A complete `Caddyfile`:

```caddy
music.example.com {
    reverse_proxy 127.0.0.1:9443
    header Strict-Transport-Security "max-age=31536000"
}
```

### 4.3 Behind Cloudflare or another CDN

This works, with two points to watch:

- **Pass the real client address.** Otherwise every request seems to come from the CDN, and one bad password locks everyone out. In nginx, use the realip module, trusting only the CDN's published ranges:

  ```nginx
  real_ip_header CF-Connecting-IP;
  set_real_ip_from 173.245.48.0/20;   # …and the rest of Cloudflare's published IPv4/IPv6 ranges
  ```

- **Idle timeouts:** Cloudflare closes idle WebSockets after 100 seconds. The gateway pings every 30 seconds, so no change is needed.

## 5. Create users

On the plugin's settings page, under **Users › Add a user**:

| Role | Can do |
|---|---|
| **Admin** | Control every player; edit saved playlists and favourites; use every plugin. |
| **Listener** | Control their own devices, plus the home players you grant them (*Add a player…*). Play saved playlists and favourites, but not change them. Plugins you mark *Admins only* are hidden from them. |

Passwords need at least 10 characters. Give each person their own account, so you can see and revoke their devices separately.

**Two-step verification.** Each user can turn it on in the app under *Settings › Account*: scan the QR code with an authenticator app and confirm with a code. Strongly recommended for admins. If someone loses their phone, use **Reset two-step** on the settings page.

**Pairing codes** (for future native apps): **Pairing code** next to a user gives a one-time code, valid for 10 minutes, that signs a new app in without typing a password.

Docker users run these from the command line instead, e.g. `docker exec -i lyrion-anywhere anywhere-gateway admin --data-dir /data create-user bob --role listener`.

## 6. First sign-in, and installing the app

Open `https://music.example.com` on your phone and sign in.

<p align="center">
  <img src="images/sign-in-phone.png" alt="The sign-in page" width="250">
  &nbsp;
  <img src="images/now-playing-phone.png" alt="Now playing" width="250">
  &nbsp;
  <img src="images/appearance-phone.png" alt="Appearance settings" width="250">
</p>

- **Choose a player:** tap the player name at the top of *Now playing*. To play on the phone itself, choose **Play on this device**. It then appears in LMS as a player called e.g. "Chrome on Android", which you can also control from your other LMS apps.
- **Install it:** *Settings › Install the app* offers an **Install** button on Android and desktop Chrome and Edge. On iPhone and iPad, tap **Share › Add to Home Screen**. The app then opens full screen, with its own icon.
- **Make it yours:** *Settings › Appearance* has three themes (Material, Classic, High contrast), light/dark/automatic mode and an accent colour.

**Background playback.** When you play on the phone itself, the app picks the playback mode that keeps going with the screen locked, after a quick check of what that browser supports. *Settings › This device* shows the choice and why it was made.

## 7. Check your set-up

From a phone **off your Wi-Fi** (on mobile data):

- [ ] `https://music.example.com/healthz` shows `ok`.
- [ ] `http://music.example.com` redirects to `https://`.
- [ ] LMS's ports are **not** reachable from outside: `http://<your public IP>:9000` must fail. Check 3483 and 9090 the same way.
- [ ] You can sign in, see your library and control a player.
- [ ] *Play on this device* plays, and keeps playing when you lock the screen.
- [ ] Five wrong passwords within 15 minutes lock that username (and that address) for a while.

## 8. Everyday administration

- **A lost or stolen phone:** under *Devices*, **Sign out** that device. Its sign-in and any live playback end at once.
- **Someone leaves:** **Disable** or **Delete** their account. All their devices are signed out.
- **Password reset:** **New password** signs the user out everywhere, so they sign in again with the new one.
- **Why won't it play on Dad's iPad?** Under *Devices*, **Diagnostics** shows what that device reported: its browser, the playback mode it chose and why, and its recent connection log. You don't need the device in front of you.
- **Keep a plugin for admins:** *Plugins › Admins only*. It's hidden from listeners at once.

## 9. Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| The settings page shows **gateway exited: …** | The message after the colon is the gateway's own reason: usually a port already in use, or a bad *Listen address*. Fix it, then click **Restart gateway**. |
| **502 Bad Gateway** from the proxy | The gateway isn't running, or the proxy points to the wrong address. Check the *Status* panel and the proxy's `upstream`/`reverse_proxy` address. |
| **Sign-in fails with "too many attempts"** | The lockout is working (5 failures in 15 minutes). Wait 15 minutes; repeated lockouts last longer, up to a day. If it happens to everyone, the proxy isn't passing the real client address (see §4 and §4.3). |
| **Two-step codes are always wrong** | The device's clock, or the server's, is wrong. Both need the correct time (NTP). |
| **"Can't reach the music server. Trying again…"** | The phone is offline, or the gateway is down. The app keeps your sign-in and reconnects by itself. |
| **No sound after reloading the page** | Browsers only start sound after a tap. Tap anywhere on the page (a banner says so). |
| **Playback stops when the screen locks (iPhone/iPad)** | Check *Settings › This device*. If it says Web Audio, the browser refused the background-capable mode. The device's **Diagnostics** show why; please include them in a bug report. |
| **A plugin shows "Needs setting up in LMS"** | It needs an account or settings in LMS itself (e.g. a streaming service). Set it up in LMS's own settings. |
| **The settings page says "refused: use the Lyrion Anywhere settings page"** | LMS was restarted since the page was opened. Reload the page. |

**Logs:**
- **Gateway:** `lyrion-anywhere.log` in LMS's log folder, e.g. `/var/log/squeezeboxserver/` (Docker: `docker logs lyrion-anywhere`).
- **Plugin:** LMS's `server.log`. For more detail, raise *Settings › Advanced › Logging › Lyrion Anywhere* to *Debug*.

## 10. Updating and uninstalling

- **Updating:**
  - *Plugin from a release:* LMS offers the update under *Settings › Plugins*.
  - *Manual install:* replace the `LyrionAnywhere` folder with the new zip's contents, then restart LMS.
  - *Docker:* rebuild the image and re-create the container.
  - Accounts and devices live in the data folder (`…/prefs/lyrion-anywhere/` or the Docker volume) and carry over.
- **Uninstalling:** untick and remove the plugin in *Settings › Plugins*, then remove the proxy's site configuration. To delete all accounts too, delete the data folder.

---

For more depth:
- [`docs/DEPLOY.md`](DEPLOY.md): the deployment reference (proxy requirements in detail, Docker networking);
- [`docs/PROTOCOL.md`](PROTOCOL.md): the API, the command policy and the security rules;
- [`SPEC.md`](../SPEC.md): the design and its decisions.

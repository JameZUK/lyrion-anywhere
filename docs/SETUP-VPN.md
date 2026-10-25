# Set up over a VPN

If your phone already reaches home through a VPN (WireGuard, Tailscale, or the one built into your router), you can keep Lyrion Anywhere inside it: no domain name, no certificate, nothing opened on your router. The VPN encrypts the journey from your phone to your house; inside the VPN, the gateway then speaks plain HTTP.

About ten minutes. The complete reference, with every option and the troubleshooting table, is [`INSTALL.md`](INSTALL.md).

## Before you choose this

Inside the VPN, sign-ins, passwords and the music itself cross your home network unencrypted. That is fine for a network you control and nobody else can join. If you would rather keep HTTPS everywhere, two ways need no change to the plugin at all: **Tailscale Serve** gives the server an `https://` address on your tailnet with a certificate Tailscale manages, and any VPN plus a reverse proxy with a certificate your devices trust works as in the [domain walkthrough](SETUP-DOMAIN.md#using-your-own-reverse-proxy-instead). Both keep every browser feature, including installing the web app and offline downloads.

## What you need

- **Lyrion Music Server 9 or later** on Linux, with the VPN reaching the machine it runs on.
- **The VPN's details:** the server's address on the VPN (for example `10.8.0.1`) and the VPN's network (for example `10.8.0.0/24`). Tailscale uses `100.64.0.0/10`.

## 1. Install the plugin

In Lyrion's web interface open *Settings › Plugins*. At the bottom, under *Additional repositories*, add this address and click *Apply*:

```
https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml
```

Find **Lyrion Anywhere** in the list, tick it, click *Apply* and restart the server when asked.

## 2. Point the gateway at the VPN

Open *Settings › Plugins › Lyrion Anywhere* and set three things.

<p align="center">
  <img src="images/plugin/trusted-networks.png" alt="The settings page with the Listen address set to the server's VPN address and port, and Trusted networks set to the VPN's network, with the page's warning that devices on it connect unencrypted" width="700">
</p>

| Setting | What to enter |
|---|---|
| **Listen address** | The server's address **on the VPN**, with the port: `10.8.0.1:9443`. Not `127.0.0.1`, which the VPN cannot reach, and not `0.0.0.0`, which would offer the unencrypted gateway to every network the machine is on. |
| **Trusted networks** | The VPN's network: `10.8.0.0/24`. Several, comma-separated, if you have more than one. Only private ranges are accepted. |
| **Public URL** | The same address as a web address, `http://10.8.0.1:9443`, and tick **Allow http:// anyway** underneath it. The page keeps the tick only while you set or change an `http://` address, and confirms what it saved. |

<p align="center">
  <img src="images/plugin/public-url-http.png" alt="The Public URL field holding http://10.8.0.1:9443 with the warning that an http:// address sends passwords unencrypted and the Allow http:// anyway box beneath it" width="700">
</p>

Click *Apply*. Then, on the server, allow the port only on the VPN interface so nothing else can reach it even by mistake; with `ufw` and a WireGuard interface called `wg0`:

```sh
sudo ufw allow in on wg0 to any port 9443 proto tcp
```

## 3. Add a user and a device

Under **Users**, add a user, then press **Add a device** next to them: the page shows a one-time code and a QR code. Because the Public URL is a private address, the QR code carries it, so the phone needs nothing typed.

## 4. Sign in from the VPN

Connect your phone to the VPN first.

- **The Android app:** install it from the [releases page](https://github.com/JameZUK/lyrion-anywhere/releases?q=android-v&expanded=true), choose **Scan QR code** and scan the code, or enter `http://10.8.0.1:9443` by hand. The app explains that the connection is unencrypted and asks you to confirm; while you are signed in, *Settings* shows an *Unencrypted* badge beside the server.

<p align="center">
  <img src="images/android/sign-in-vpn.png" alt="The Android app's server screen with http://10.8.0.1:9443 entered and the Unencrypted connection warning, explaining that anyone on the same network could read the traffic and asking to confirm" width="250">
</p>

- **A browser:** open `http://10.8.0.1:9443`. A banner says the connection is unencrypted. Browsers keep some features back from plain-HTTP pages: the web app cannot be installed to the home screen or keep music for offline use, and it plays through the browser's media player rather than its background-capable one. Everything else works.

## Two things to know

- **The app needs the address, not a name.** Over `http://` the Android app accepts only a private IP address such as `10.8.0.1`, never `lms.local` or any other name, because a name could lead somewhere public and carry your password in clear. A browser on the same phone will happily take the name; the app will tell you to use the address instead. Over `https://` either is fine.
- **Off the VPN, nothing changes.** A plain-HTTP request from any address outside the trusted networks is refused. Lyrion's own ports are never reachable from outside.

## If something does not work

The settings page tells you when the *Listen address* is one the VPN cannot use, and refuses a network that is not private. The troubleshooting table in [`INSTALL.md`](INSTALL.md#9-troubleshooting) covers *403 TLS required* (the device is not inside a trusted network) and the app refusing a name.

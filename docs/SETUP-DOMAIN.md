# Set up with a domain name

The quickest way to reach your Lyrion Music Server from anywhere: a name such as `music.example.com` that serves the app over HTTPS. The plugin can do the hard part for you with a Cloudflare Tunnel, which needs nothing opened on your router and no certificate to renew. If you already run your own reverse proxy, the second half of this page is for you.

About fifteen minutes. The complete reference, with every option and the troubleshooting table, is [`INSTALL.md`](INSTALL.md).

## What you need

- **Lyrion Music Server 9 or later** on Linux (x86_64, a 64-bit Raspberry Pi or a 32-bit one).
- **A domain name whose DNS is on Cloudflare.** The free plan is enough. Any registrar's domain can be moved to Cloudflare's DNS.
- A phone or a browser to sign in with at the end.

## 1. Install the plugin

In Lyrion's web interface open *Settings › Plugins*. At the bottom, under *Additional repositories*, add this address and click *Apply*:

```
https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml
```

Find **Lyrion Anywhere** in the list, tick it, click *Apply* and restart the server when asked. Lyrion then offers its updates automatically.

## 2. Open the settings page

*Settings › Plugins › Lyrion Anywhere*. The gateway is already running: the top of the page shows it, with the address it listens on. Leave the *Listen address* as it is.

<p align="center">
  <img src="images/plugin/settings-overview.png" alt="The top of the Lyrion Anywhere settings page: Gateway enabled, the listen address, and the Public URL field" width="700">
</p>

## 3. Start the tunnel

Scroll to **Cloudflare Tunnel**.

<p align="center">
  <img src="images/plugin/cloudflare-tunnel.png" alt="The Cloudflare Tunnel section of the settings page before anything is installed: an Install button, a box to try a temporary tunnel, and the two registration forms" width="700">
</p>

1. Press **Install**. The plugin downloads Cloudflare's connector at the version it pins, checks it against a checksum built into the gateway, and refuses anything that does not match. Nothing is downloaded or run until you press the button.
2. Register the tunnel, either way:
   - **Create it for me.** In Cloudflare's dashboard, under *My Profile › API Tokens*, make a token with three permissions: *Account › Cloudflare Tunnel › Edit*, *Zone › DNS › Edit* and *Zone › Zone › Read*. Paste it into the form with the name you want (`music.example.com`) and press **Create and start**. The gateway creates the tunnel and the DNS record, and the token is used once and never stored.
   - **I have a tunnel token.** Create the tunnel yourself in Cloudflare under *Networking › Tunnels*, give it your name with the service URL `http://127.0.0.1:9444`, copy its token, and paste it into the form with the name. Press **Register and start**.
3. The section says *Connected to Cloudflare*. From a phone on mobile data, open `https://music.example.com/healthz`: it answers `ok`.

Not sure yet? **Try a temporary tunnel** needs no account and no domain: it gives you a throwaway address and a QR code for an hour's test from your phone, and stores nothing.

The tunnel starts again by itself whenever the server starts. Your music then flows through Cloudflare's network under Cloudflare's terms.

## 4. Add a user and a device

Under **Users**, add a user. An *admin* can control every player and change playlists; a *listener* controls their own devices and the home players you grant them. Passwords need ten characters or more, and each person should have their own account so you can see and sign out their devices separately.

Next to the user, press **Add a device**. The page shows a one-time code and a QR code that carries your server's address with it.

<p align="center">
  <img src="images/plugin/users-add-device.png" alt="The Users section with one user and the Add a device panel open, showing a one-time code in large letters, its QR code and the time left" width="700">
</p>

## 5. Sign in

- **In a browser:** open `https://music.example.com`, sign in with the user's name and password, and press Play. The browser is now a player; *Settings › Install the app* puts it on the home screen with its own icon.
- **On an Android phone:** install the [Android app](https://github.com/JameZUK/lyrion-anywhere/releases?q=android-v&expanded=true) (the `arm64-v8a` APK suits most phones; the universal one works on any), choose **Scan QR code** and point it at the code. The app shows which server it will join and asks you to confirm.

That is it. Two-step verification can be turned on by each user under *Settings › Account* in the app.

## Using your own reverse proxy instead

If you already have nginx, Caddy or another proxy with a certificate, skip the tunnel and point the proxy at the gateway's listen address (`127.0.0.1:9443` on the same machine). The proxy must pass the client's address in `X-Forwarded-For`, pass WebSocket upgrades, and allow long-lived connections; it must not add a Content-Security-Policy of its own.

**Caddy** is two lines:

```caddy
music.example.com {
    reverse_proxy 127.0.0.1:9443
    header Strict-Transport-Security "max-age=31536000"
}
```

**nginx:** copy the tested configuration from [`deploy/nginx/lyrion-anywhere.conf`](../deploy/nginx/lyrion-anywhere.conf), change the name, the certificate paths and the upstream address, and reload.

Then, on the settings page, set the **Public URL** to `https://music.example.com` (it is what the QR codes carry) and, if the proxy runs on another machine, its address under **Trusted proxies**. Carry on from step 4.

## If something does not work

The settings page's status lines say what failed, and *Copy log* gives you the tunnel's own log with secrets masked. The troubleshooting tables in [`INSTALL.md`](INSTALL.md#if-something-goes-wrong) cover the common messages, from a blocked outbound port 7844 to a DNS record that already exists.

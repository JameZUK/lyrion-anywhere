# Deploying Lyrion Anywhere

This is the deployment reference. For a step-by-step walkthrough, see [`INSTALL.md`](INSTALL.md).

The gateway is the only part reachable from the internet. Put it behind a TLS reverse proxy and never expose LMS's own ports (3483, 9000, 9090).

```
phone/browser ──HTTPS──▶ reverse proxy ──HTTP──▶ gateway ──▶ LMS (LAN)
```

There are two ways to run the gateway.

## 1. LMS plugin (recommended)

Install the plugin from a release: in LMS, *Settings › Plugins › Additional repositories*, add the release's `repo.xml` URL, then install **Lyrion Anywhere**. Or unzip `LyrionAnywhere-<version>.zip` into a folder named `LyrionAnywhere` in LMS's plugin directory (e.g. `/var/lib/squeezeboxserver/Plugins/` on Debian). Don't leave a second copy of the plugin anywhere in that directory. Restart LMS.

The release bundles static gateways for x86_64, aarch64 and armhf Linux, with a `Bin/SHA256SUMS` list. The plugin refuses to run a bundled gateway that doesn't match it. That catches a corrupt, truncated or accidentally replaced binary. It is not a signature: the list ships in the same zip, so it can't detect a maliciously altered release. The release also bundles the browser player, which the gateway serves at `/`.

Then open *Settings › Plugins › Lyrion Anywhere*:

- **Enabled:** ticked.
- **Listen address:** `127.0.0.1:9443` if the proxy runs on the LMS machine. Otherwise use the LMS machine's LAN address (e.g. `10.0.0.20:9443`), and allow only the proxy to reach that port.
- **Public URL:** `https://music.example.com`.
- **Trusted proxies:** the proxy's address when it runs on another machine (e.g. `10.0.0.10`).

Add users on the same page. The gateway refuses plain-HTTP requests from anything except loopback, so the LAN port can't be used to bypass TLS. Still, firewall it to the proxy.

LMS's own web and control ports (9000 and 9090) need no password on the local network by default, so anyone on your network can use LMS and open its settings. Remote accounts can only be changed from the Lyrion Anywhere settings page: the plugin's account commands need a token that only that page carries, so other web sites can't change accounts through LMS. Anyone who can open the page can, though. If others use your network, turn on LMS password protection (*Settings › Advanced › Security*).

**Gateway path (development only).** The settings page has a field for running your own gateway build instead of the bundled, checked one. The plugin ignores it (and logs a warning) unless LMS itself was started with `ANYWHERE_ALLOW_GATEWAY_PATH=1` in its environment, so a changed setting alone can't make LMS run an arbitrary program. Don't set the variable on a server you use day to day.

## 2. Docker

For LMS installs where a plugin can't run the binary, or to run the gateway on another host:

```sh
docker build -t lyrion-anywhere .
docker volume create anywhere-data

# First admin user (password on standard input):
docker run --rm -i -v anywhere-data:/data lyrion-anywhere \
    admin --data-dir /data create-user alice --role admin

docker run -d --name lyrion-anywhere --restart unless-stopped --network host \
    -v anywhere-data:/data lyrion-anywhere \
    serve --data-dir /data --static-dir /web \
          --listen 127.0.0.1:9443 --lms lms.example.lan \
          --allowed-origin https://music.example.com
```

- Manage accounts with `docker exec lyrion-anywhere anywhere-gateway admin --data-dir /data …` (`list-users`, `list-devices`, `pair`, `reset-totp`, `set-password`).
- The image is static, contains no shell and runs as a non-root user. It includes the browser player page at `/`.
- Host networking keeps the client address real. With a bridge network, the gateway must listen on all interfaces inside the container (`--listen 0.0.0.0:9443`; `127.0.0.1` there is unreachable from the host). Publish the port to `127.0.0.1:9443` only (`-p 127.0.0.1:9443:9443`), and pass the bridge gateway address as `--trusted-proxy`.

## Reverse proxy

[`deploy/nginx/lyrion-anywhere.conf`](../deploy/nginx/lyrion-anywhere.conf) is a tested example. `tools/test-deploy.sh` runs it in front of the Docker image on every CI push. What matters:

- **`X-Forwarded-For $remote_addr`:** overwrite the header, never append to it (`$proxy_add_x_forwarded_for`). The gateway uses the first address for sign-in lockouts, so an appended header would let clients choose their own address. The test proves lockouts still apply when a client spoofs it.
- **WebSocket upgrade headers,** with `proxy_buffering off` and long read timeouts (audio streams and a paused player are long-lived).
- **Downloads aren't buffered:** `proxy_buffering off` (and `proxy_max_temp_file_size 0`, in case buffering is turned back on) stream offline downloads straight through, instead of nginx spooling a whole album to its own disk first. A transcoded download starts only once ffmpeg has finished, so keep the long `proxy_read_timeout`.
- **ffmpeg (optional):** install it where the gateway runs for MP3/Opus downloads (`--ffmpeg`, `--no-transcode`, `--download-cache-mb`; see [`PROTOCOL.md`](PROTOCOL.md#downloads)). The Docker image doesn't include it.
- **No CSP of your own:** the gateway sets its security headers. The proxy adds only HSTS.
- **Behind a CDN** (e.g. Cloudflare): use the realip module so `$remote_addr` is the real client, trusting only the CDN's address ranges. The gateway pings quiet WebSockets every 30 s, so a CDN's idle timeout (100 s on Cloudflare) doesn't close them.

Caddy works too: `reverse_proxy 127.0.0.1:9443` replaces `X-Forwarded-For` by default.

## Checklist

- [ ] LMS ports unreachable from the internet (check from outside, e.g. a phone off Wi-Fi).
- [ ] The gateway's port reachable only from the proxy.
- [ ] `https://music.example.com/healthz` returns `ok`; plain HTTP redirects.
- [ ] Sign in with a test user; turn on two-step for admins.

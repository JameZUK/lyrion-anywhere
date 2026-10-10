# Deploying Lyrion Anywhere

The gateway is the only part reachable from the internet. Put it behind a TLS reverse proxy and never expose LMS's own ports (3483, 9000, 9090).

```
phone/browser ──HTTPS──▶ reverse proxy ──HTTP──▶ gateway ──▶ LMS (LAN)
```

There are two ways to run the gateway.

## 1. LMS plugin (recommended)

Install the plugin and open *Settings › Plugins › Lyrion Anywhere*:

- **Enabled:** ticked.
- **Listen address:** `127.0.0.1:9443` if the proxy runs on the LMS machine. Otherwise use the LMS machine's LAN address (e.g. `10.0.0.20:9443`), and allow only the proxy to reach that port.
- **Public URL:** `https://music.example.com`.
- **Trusted proxies:** the proxy's address when it runs on another machine (e.g. `10.0.0.10`).

Add users on the same page. The gateway refuses plain-HTTP requests from anything except loopback, so the LAN port can't be used to bypass TLS. Still, firewall it to the proxy.

Anyone who can open LMS settings can manage remote accounts. If others use your network, turn on LMS password protection (*Settings › Advanced › Security*).

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
- Host networking keeps the client address real. With a bridge network, publish to `127.0.0.1:9443` only, and pass the bridge gateway address as `--trusted-proxy`.

## Reverse proxy

[`deploy/nginx/lyrion-anywhere.conf`](../deploy/nginx/lyrion-anywhere.conf) is a tested example. `tools/test-deploy.sh` runs it in front of the Docker image on every CI push. What matters:

- **`X-Forwarded-For $remote_addr`:** overwrite the header, never append to it (`$proxy_add_x_forwarded_for`). The gateway uses the first address for sign-in lockouts, so an appended header would let clients choose their own address. The test proves lockouts still apply when a client spoofs it.
- **WebSocket upgrade headers,** with `proxy_buffering off` and long read timeouts (audio streams and a paused player are long-lived).
- **No CSP of your own:** the gateway sets its security headers. The proxy adds only HSTS.
- **Behind a CDN** (e.g. Cloudflare): use the realip module so `$remote_addr` is the real client, trusting only the CDN's address ranges. The gateway pings quiet WebSockets every 30 s, so a CDN's idle timeout (100 s on Cloudflare) doesn't close them.

Caddy works too: `reverse_proxy 127.0.0.1:9443` replaces `X-Forwarded-For` by default.

## Checklist

- [ ] LMS ports unreachable from the internet (check from outside, e.g. a phone off Wi-Fi).
- [ ] The gateway's port reachable only from the proxy.
- [ ] `https://music.example.com/healthz` returns `ok`; plain HTTP redirects.
- [ ] Sign in with a test user; turn on two-step for admins.

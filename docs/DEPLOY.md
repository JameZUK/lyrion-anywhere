# Deploying Lyrion Anywhere

This is the deployment reference. For a step-by-step walkthrough, see [`INSTALL.md`](INSTALL.md).

The gateway is the only part reachable from the internet. Put it behind a TLS reverse proxy, or a Cloudflare Tunnel, and never expose LMS's own ports (3483, 9000, 9090).

```
phone/browser ──HTTPS──▶ reverse proxy ──HTTP──▶ gateway ──▶ LMS (LAN)
phone/browser ──HTTPS──▶ Cloudflare ◀── cloudflared (dials out) ──HTTP──▶ gateway ──▶ LMS (LAN)
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

**Built-in Cloudflare Tunnel.** On Linux the plugin's settings page can install and run `cloudflared` for you, instead of a reverse proxy: register a tunnel token from Cloudflare's dashboard, let the gateway create the tunnel with a Cloudflare API token (used once, never stored), or try a temporary `trycloudflare.com` tunnel for an hour. The walkthrough is [`INSTALL.md`](INSTALL.md#option-c-built-in-cloudflare-tunnel) Option C; the security model is in [`SECURITY.md`](SECURITY.md#the-built-in-cloudflare-tunnel), and the admin endpoints are in [`PROTOCOL.md`](PROTOCOL.md#admin-api). Nothing is downloaded or run until an admin asks for it there. While the tunnel runs, the gateway accepts its `https://` address as an origin and reads `CF-Connecting-IP` on requests from it, so *Public URL* and *Client address header* need no change.

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

### Cloudflare Tunnel with Docker: a sidecar

Docker has no plugin settings page, and the image is distroless (no shell, no package manager) and stays that way, so the gateway's built-in tunnel isn't the documented way here. Run Cloudflare's own `cloudflared` image as a second container instead, on a network shared with the gateway. A `docker-compose.yml`:

```yaml
services:
  gateway:
    image: lyrion-anywhere                 # built as above
    restart: unless-stopped
    volumes:
      - anywhere-data:/data
    command:
      - serve
      - --data-dir=/data
      - --static-dir=/web
      - --listen=0.0.0.0:9443              # reachable on the compose network only: no ports published
      - --lms=lms.example.lan
      - --allowed-origin=https://music.example.com
      - --trusted-proxy=10.0.9.3           # the cloudflared container
      - --client-ip-header=CF-Connecting-IP
      - --no-tunnel                        # the sidecar is the tunnel
    networks:
      tunnel:
        ipv4_address: 10.0.9.2

  cloudflared:
    image: cloudflare/cloudflared:2026.10.1   # pin a tag; move it when you choose to
    restart: unless-stopped
    command: tunnel --no-autoupdate run
    env_file: cloudflared.env               # TUNNEL_TOKEN=<the tunnel token>; mode 0600, not in version control
    depends_on:
      - gateway
    networks:
      tunnel:
        ipv4_address: 10.0.9.3

networks:
  tunnel:
    ipam:
      config:
        - subnet: 10.0.9.0/24              # any private range not used on your LAN

volumes:
  anywhere-data:
```

- **The tunnel:** create it in Cloudflare's dashboard (*Networking › Tunnels › Create a tunnel*), copy the token from the connector command it shows into `cloudflared.env` as `TUNNEL_TOKEN=…`, and give it the public hostname `music.example.com` with the service URL `http://gateway:9443`, the gateway's name on the compose network. Compose's `secrets:` work too, if your `cloudflared` version reads the token from a file.
- **The client address:** the gateway sees every request coming from the `cloudflared` container, so that container gets a fixed address, passed as `--trusted-proxy`, and `--client-ip-header CF-Connecting-IP` takes each visitor's address from Cloudflare's header. The gateway believes that header only from a trusted proxy, and nothing else can reach the gateway's port, since it isn't published. Without these, every visitor shares one address and one bad password locks everyone out.
- **The origin:** `--allowed-origin` is the tunnel's `https://` address.
- **`--no-tunnel`** turns off the gateway's built-in tunnel, so its admin actions are refused and only the sidecar runs. It's optional: the built-in tunnel never starts unless asked through the admin API.
- **Updates:** the sidecar moves only when you change its tag. Keep `--no-autoupdate`; the container is replaced, not updated in place.
- Accounts are managed as above, e.g. `docker compose exec gateway anywhere-gateway admin --data-dir /data create-user alice --role admin`.

## Reverse proxy

[`deploy/nginx/lyrion-anywhere.conf`](../deploy/nginx/lyrion-anywhere.conf) is a tested example. `tools/test-deploy.sh` runs it in front of the Docker image on every CI push. What matters:

- **`X-Forwarded-For $remote_addr`:** the example overwrites the header. Appending (`$proxy_add_x_forwarded_for`) is safe too: for sign-in lockouts the gateway reads the header from the right and takes the first address that isn't a trusted proxy, so whatever a client puts at the start is ignored. The test proves lockouts still apply when a client spoofs it.
- **WebSocket upgrade headers,** with `proxy_buffering off` and long read timeouts (audio streams and a paused player are long-lived).
- **Downloads aren't buffered:** `proxy_buffering off` (and `proxy_max_temp_file_size 0`, in case buffering is turned back on) stream offline downloads straight through, instead of nginx spooling a whole album to its own disk first. A transcoded download starts only once ffmpeg has finished, so keep the long `proxy_read_timeout`.
- **ffmpeg (optional):** install it where the gateway runs for MP3/Opus downloads and for the lower-data streams phones ask for on mobile data (`--ffmpeg`, `--no-transcode`, `--download-cache-mb`, `--live-transcode-jobs`; see [`PROTOCOL.md`](PROTOCOL.md#downloads) and [Streams](PROTOCOL.md#streams)). `--no-transcode` turns off both. Each live re-encode is one ffmpeg process for the length of a track; `--live-transcode-jobs` (default 4) caps how many run at once, and streams beyond it are sent as they are. The Docker image doesn't include ffmpeg.
- **Long-lived player sessions:** a phone that loses its connection keeps its LMS player for up to `--max-resume-grace` seconds (default 600; players that don't ask get 60 s), so music carries on through tunnels and lifts. Each held session is still one LMS connection per device. Lower it on a server with many devices if you prefer LMS to drop absent players sooner.
- **Track analysis (on by default):** the gateway decodes a track the first time an app asks for its waveform or loudness, one track at a time, and keeps the result (about 2 KiB) in `<data-dir>/analysis-cache`. On a very slow host, turn it off with `--no-analysis` (the plugin's *Track analysis* setting); see [`PROTOCOL.md`](PROTOCOL.md#track-analysis-get-v1analysistrackid). An analysis that is still running answers 202 within 10 s, so the proxy's ordinary timeouts are enough.
- **No CSP of your own:** the gateway sets its security headers. The proxy adds only HSTS.
- **Behind a CDN** (e.g. Cloudflare): either use the realip module so `$remote_addr` is the real client, trusting only the CDN's address ranges, or pass the CDN's own header through and start the gateway with `--client-ip-header CF-Connecting-IP` (the plugin's *Client address header* setting). The gateway believes that header only on requests from a trusted proxy (loopback or `--trusted-proxy`), so use it only when the proxy accepts connections from the CDN alone, or with a Cloudflare Tunnel. The gateway pings quiet WebSockets every 5 s, so a CDN's idle timeout (100 s on Cloudflare) doesn't close them.

Caddy works too: `reverse_proxy 127.0.0.1:9443` replaces `X-Forwarded-For` by default.

## Checklist

- [ ] LMS ports unreachable from the internet (check from outside, e.g. a phone off Wi-Fi).
- [ ] The gateway's port reachable only from the proxy (or, with a Cloudflare Tunnel, from `cloudflared` only).
- [ ] `https://music.example.com/healthz` returns `ok`; plain HTTP redirects.
- [ ] Sign in with a test user; turn on two-step for admins.

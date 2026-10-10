#!/bin/sh
# Deployment test: the Docker image behind nginx with deploy/nginx/lyrion-anywhere.conf.
# Proves the example config is safe as written: TLS-only, client addresses can't be
# spoofed, LMS and admin paths are unreachable, WebSockets pass, origin checks hold.
#
#   tools/test-deploy.sh [image]      (default: builds lyrion-anywhere:test)
#
# Needs docker, curl and openssl. Uses host networking on high ports.
set -eu
cd "$(dirname "$0")/.."

IMAGE=${1:-}
NGINX_IMAGE=nginx:1.29-alpine
NAME=music.example.com
GW_PORT=19443 HTTPS_PORT=18443 HTTP_PORT=18080
WORK=$(mktemp -d)
GW=anywhere-deploy-gw-$$ NG=anywhere-deploy-nginx-$$ VOL=anywhere-deploy-data-$$
PASS=0 FAIL=0

cleanup() {
	docker rm -f "$GW" "$NG" >/dev/null 2>&1 || true
	docker volume rm "$VOL" >/dev/null 2>&1 || true
	rm -rf "$WORK"
}
trap cleanup EXIT

ok()   { PASS=$((PASS + 1)); echo "ok   - $1"; }
bad()  { FAIL=$((FAIL + 1)); echo "FAIL - $1"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (got '$2', want '$3')"; fi; }

if [ -z "$IMAGE" ]; then
	IMAGE=lyrion-anywhere:test
	docker build -q -t "$IMAGE" . >/dev/null
fi

# --- gateway ------------------------------------------------------------------------------
docker volume create "$VOL" >/dev/null
printf 'deploy test password\n' | docker run --rm -i -v "$VOL:/data" "$IMAGE" \
	admin --data-dir /data create-user alice --role listener >/dev/null
# No LMS here: the checks below never need one.
docker run -d --name "$GW" --network host -v "$VOL:/data" "$IMAGE" \
	serve --data-dir /data --static-dir /web --listen "127.0.0.1:$GW_PORT" \
	--lms 127.0.0.1 --lms-slimproto-port 1 --lms-http-port 1 --lms-cli-port 1 \
	--allowed-origin "https://$NAME" >/dev/null

# --- nginx with the example config, only ports and certificate swapped ---------------------
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=$NAME" \
	-keyout "$WORK/privkey.pem" -out "$WORK/fullchain.pem" 2>/dev/null
sed -e "s/127.0.0.1:9443/127.0.0.1:$GW_PORT/" \
    -e "s/listen 443 ssl/listen $HTTPS_PORT ssl/; s/listen \[::\]:443 ssl/listen [::]:$HTTPS_PORT ssl/" \
    -e "s/listen 80;/listen $HTTP_PORT;/; s/listen \[::\]:80;/listen [::]:$HTTP_PORT;/" \
    -e "s#/etc/ssl/$NAME/#/certs/#" \
    deploy/nginx/lyrion-anywhere.conf > "$WORK/site.conf"
docker run -d --name "$NG" --network host \
	-v "$WORK/site.conf:/etc/nginx/conf.d/default.conf:ro" -v "$WORK:/certs:ro" \
	"$NGINX_IMAGE" >/dev/null

BASE="https://$NAME:$HTTPS_PORT"
C="curl -sk --resolve $NAME:$HTTPS_PORT:127.0.0.1 --resolve $NAME:$HTTP_PORT:127.0.0.1"
for _ in $(seq 1 50); do
	[ "$($C -o /dev/null -w '%{http_code}' "$BASE/healthz")" = 200 ] && break
	sleep 0.2
done

# --- checks -------------------------------------------------------------------------------
echo "# transport"
check "health check through nginx" "$($C -o /dev/null -w '%{http_code}' "$BASE/healthz")" 200
check "plain HTTP redirects to HTTPS" "$($C -o /dev/null -w '%{http_code} %{redirect_url}' "http://$NAME:$HTTP_PORT/v1/me")" "301 https://$NAME/v1/me"
H=$($C -D- -o /dev/null "$BASE/healthz" | tr -d '\r')
check "HSTS set" "$(echo "$H" | grep -ci '^strict-transport-security:')" 1
check "exactly one CSP (the gateway's)" "$(echo "$H" | grep -ci '^content-security-policy:')" 1

echo "# image"
check "runs as non-root" "$(docker inspect -f '{{.Config.User}}' "$IMAGE")" nonroot
check "data directory and files are private" "$(docker run --rm -v "$VOL:/data:ro" alpine:3.22 sh -c 'stat -c %a /data; find /data -type f ! -perm 600 | wc -l' | tr '\n' ' ')" "700 0 "
check "web player page served" "$($C -o /dev/null -w '%{http_code}' "$BASE/")" 200
check "WebAssembly served as application/wasm" "$($C -o /dev/null -w '%{content_type}' "$BASE/player/pkg/anywhere_wasm_bg.wasm")" application/wasm
check "player worker served" "$($C -o /dev/null -w '%{http_code}' "$BASE/player/player-worker.js")" 200
check "web app page has no inline script" "$($C "$BASE/" | grep -c -E '<script>|<script [^>]*>[^<]' || true)" 0

echo "# nothing else is reachable"
for p in /jsonrpc.js /settings/index.html /admin/v1/users /music/1/download /plugins/ /stream.mp3 /cometd /..%2f..%2fetc/passwd; do
	code=$($C -o /dev/null -w '%{http_code}' "$BASE$p")
	case $code in 4??) ok "refused: $p ($code)" ;; *) bad "reachable: $p ($code)" ;; esac
done
check "oversize bodies refused by nginx" "$(head -c 70000 /dev/zero | $C -o /dev/null -w '%{http_code}' -X POST --data-binary @- "$BASE/v1/rpc")" 413

echo "# sign-in"
LOGIN='{"username":"alice","password":"deploy test password","device_name":"deploy test"}'
R=$($C -D "$WORK/h" -H 'Content-Type: application/json' -H 'X-Anywhere-Client: 1' \
	-H "Origin: https://$NAME" -d "$LOGIN" "$BASE/v1/auth/login")
TOKEN=$(echo "$R" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
if [ -n "$TOKEN" ]; then ok "login through nginx"; else bad "login through nginx: $R"; fi
check "refresh cookie is Secure and HttpOnly" "$(grep -i '^set-cookie:' "$WORK/h" | grep -ci 'secure.*httponly\|httponly.*secure')" 1
check "cross-origin login refused" "$($C -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -H 'X-Anywhere-Client: 1' -H 'Origin: https://evil.example' -d "$LOGIN" "$BASE/v1/auth/login")" 403

echo "# WebSockets"
WS="-o /dev/null -w %{http_code} --http1.1 -m 3 -H Connection:Upgrade -H Upgrade:websocket -H Sec-WebSocket-Version:13 -H Sec-WebSocket-Key:dGhlIHNhbXBsZSBub25jZQ=="
check "events socket upgrades through nginx" "$($C $WS -H "Origin: https://$NAME" "$BASE/v1/events" || true)" 101
check "player socket upgrades through nginx" "$($C $WS -H "Origin: https://$NAME" "$BASE/v1/player" || true)" 101
check "cross-origin socket refused" "$($C $WS -H 'Origin: https://evil.example' "$BASE/v1/events" || true)" 403

echo "# client address can't be spoofed"
# Six failures from one client, each claiming a different address and user. If nginx
# appended to X-Forwarded-For, the gateway would see six clients and never lock out.
codes=""
for i in 1 2 3 4 5 6; do
	codes="$codes $($C -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -H 'X-Anywhere-Client: 1' \
		-H "X-Forwarded-For: 203.0.113.$i" -H "X-Real-IP: 203.0.113.$i" \
		-d "{\"username\":\"nobody$i\",\"password\":\"wrong password\",\"device_name\":\"x\"}" "$BASE/v1/auth/login")"
done
check "per-client lockout despite spoofed headers" "$(echo $codes | awk '{print $NF}')" 429

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

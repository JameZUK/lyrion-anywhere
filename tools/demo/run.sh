#!/bin/bash
# Run Lyrion Anywhere against the demo LMS (invented library), for trying the web app and
# for the documentation screenshots. Nothing here touches a real LMS.
#
#   tools/demo/run.sh            # then open http://127.0.0.1:19443 (user demo, password below)
#
# Needs: a built gateway (cargo build -p anywhere-gateway), a built web app
# (tools/build-web.sh), python3 with Pillow. Stops everything on Ctrl-C.
set -euo pipefail
cd "$(dirname "$0")/../.."

DATA=$(mktemp -d)
GW=target/debug/anywhere-gateway
PASSWORD=${DEMO_PASSWORD:-demo-password-123}
[ -x "$GW" ] || { echo "build the gateway first: cargo build -p anywhere-gateway" >&2; exit 1; }
[ -f web/app/build/index.html ] || { echo "build the web app first: tools/build-web.sh" >&2; exit 1; }

cleanup() { kill $(jobs -p) 2>/dev/null || true; rm -rf "$DATA"; }
trap cleanup EXIT

python3 tools/demo/fake_lms.py --port 19000 &
printf '%s\n' "$PASSWORD" | "$GW" admin --data-dir "$DATA" create-user demo --role admin >/dev/null
"$GW" serve --data-dir "$DATA" --listen 127.0.0.1:19443 --admin-listen 127.0.0.1:19445 \
	--lms 127.0.0.1 --lms-http-port 19000 --lms-cli-port 19090 --lms-slimproto-port 13483 \
	--static-dir web/app/build >"$DATA/gateway.log" 2>&1 &

for _ in $(seq 50); do curl -sf http://127.0.0.1:19443/healthz >/dev/null && break; sleep 0.2; done
echo "Demo running at http://127.0.0.1:19443  (user: demo, password: $PASSWORD)"
wait

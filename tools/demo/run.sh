#!/bin/bash
# Run Lyrion Anywhere against the demo LMS (invented library), for trying the web app and
# for the documentation screenshots. Nothing here touches a real LMS.
#
#   tools/demo/run.sh            # then open http://127.0.0.1:19443 (user demo, password below)
#
# Needs: a debug build of the gateway (cargo build -p anywhere-gateway), a built web app
# (tools/build-web.sh), python3 with Pillow. With ffmpeg (libopus) on PATH, this browser's
# own player plays through the gateway and can be re-encoded to Opus (the signal-path shots).
# Stops everything on Ctrl-C.
#
# Ports (environment, defaults in brackets): DEMO_GW_PORT [19443], DEMO_ADMIN_PORT [19445],
# DEMO_LMS_PORT, the fake LMS's web port [19000], DEMO_SLIMPROTO_PORT [13483],
# DEMO_CLI_PORT [19090]. DEMO_GATEWAY names the gateway binary
# [$CARGO_TARGET_DIR/debug/anywhere-gateway, else target/debug/anywhere-gateway];
# DEMO_GATEWAY_LOG keeps its log [in the scratch directory, removed at the end].
set -euo pipefail
cd "$(dirname "$0")/../.."

DATA=$(mktemp -d)
GW=${DEMO_GATEWAY:-${CARGO_TARGET_DIR:-target}/debug/anywhere-gateway}
PASSWORD=${DEMO_PASSWORD:-demo-password-123}
GW_PORT=${DEMO_GW_PORT:-19443}
ADMIN_PORT=${DEMO_ADMIN_PORT:-19445}
LMS_PORT=${DEMO_LMS_PORT:-19000}
SLIM_PORT=${DEMO_SLIMPROTO_PORT:-13483}
CLI_PORT=${DEMO_CLI_PORT:-19090}
[ -x "$GW" ] || { echo "build the gateway first: cargo build -p anywhere-gateway" >&2; exit 1; }
[ -f web/app/build/index.html ] || { echo "build the web app first: tools/build-web.sh" >&2; exit 1; }

cleanup() { kill $(jobs -p) 2>/dev/null || true; rm -rf "$DATA"; }
trap cleanup EXIT

python3 tools/demo/fake_lms.py --port "$LMS_PORT" --slimproto-port "$SLIM_PORT" &
printf '%s\n' "$PASSWORD" | "$GW" admin --data-dir "$DATA" create-user demo --role admin >/dev/null
# Invented listening history for the stats and "Your year in sound". The marker tells the
# (debug-build-only) seeding command that this scratch directory is a demo's.
touch "$DATA/demo-data-dir"
python3 tools/demo/fake_lms.py --history-json | "$GW" admin --data-dir "$DATA" demo-seed-history demo >/dev/null
"$GW" serve --data-dir "$DATA" --listen "127.0.0.1:$GW_PORT" --admin-listen "127.0.0.1:$ADMIN_PORT" \
	--lms 127.0.0.1 --lms-http-port "$LMS_PORT" --lms-cli-port "$CLI_PORT" --lms-slimproto-port "$SLIM_PORT" \
	--static-dir web/app/build >"${DEMO_GATEWAY_LOG:-$DATA/gateway.log}" 2>&1 &

for _ in $(seq 50); do curl -sf "http://127.0.0.1:$GW_PORT/healthz" >/dev/null && break; sleep 0.2; done
echo "Demo running at http://127.0.0.1:$GW_PORT  (user: demo, password: $PASSWORD)"
wait

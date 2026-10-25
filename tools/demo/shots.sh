#!/bin/bash
# Take the web documentation screenshots (docs/images/web) in one go: start a fresh demo
# (tools/demo/run.sh: invented library, players and listening history), run
# tools/demo/screenshots.mjs against it, then stop the demo.
#
#   tools/demo/shots.sh [out-dir]       # ONLY=name,name to retake some
#
# Needs what run.sh needs (a debug gateway build and a built web app), plus Node with
# playwright-core and a Chromium (PLAYWRIGHT_CORE=/path/to/playwright-core if it isn't
# resolvable from tools/demo). PNGs are optimised with oxipng when it is installed, else
# with Pillow (tools/demo/optimise_png.py). The demo's ports can be moved with run.sh's
# DEMO_*_PORT variables (when another LMS holds 19000 or 13483), and are passed on here.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:-docs/images/web}
export DEMO_URL=${DEMO_URL:-http://127.0.0.1:${DEMO_GW_PORT:-19443}}
export DEMO_LMS_URL=${DEMO_LMS_URL:-http://127.0.0.1:${DEMO_LMS_PORT:-19000}}

if curl -sf "$DEMO_URL/healthz" >/dev/null 2>&1; then
	echo "a demo is already running at $DEMO_URL; stop it first (shots need a fresh one)" >&2
	exit 1
fi
tools/demo/run.sh &
DEMO=$!
trap 'kill -TERM $DEMO 2>/dev/null || true; wait $DEMO 2>/dev/null || true' EXIT
for _ in $(seq 100); do curl -sf "$DEMO_URL/healthz" >/dev/null 2>&1 && break; sleep 0.2; done

node tools/demo/screenshots.mjs "$OUT"
if command -v oxipng >/dev/null; then oxipng -q -o 3 --strip safe "$OUT"/*.png; else python3 tools/demo/optimise_png.py "$OUT"/*.png; fi

#!/bin/bash
# Checks a built release (tools/build-release.sh): layout, checksums, binaries, repo.xml.
#   tools/test-release.sh [dist-dir]
set -eu
DIST=${1:-dist}
PASS=0 FAIL=0
ok()  { PASS=$((PASS + 1)); echo "ok   - $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL - $1"; }
t()   { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

ZIP=$(ls "$DIST"/LyrionAnywhere-*.zip | head -1)
VERSION=$(basename "$ZIP" .zip | sed 's/^LyrionAnywhere-//')
X=$(mktemp -d); trap 'rm -rf "$X"' EXIT
unzip -q "$ZIP" -d "$X"

echo "# layout"
t "install.xml at the zip root" '[ -f "$X/install.xml" ]'
t "install.xml version matches the zip name" 'grep -q "<version>$VERSION</version>" "$X/install.xml"'
t "plugin module present" '[ -f "$X/Plugin.pm" ] && [ -f "$X/Gateway.pm" ]'
t "settings page present" '[ -f "$X/HTML/EN/plugins/LyrionAnywhere/settings/basic.html" ]'
t "settings page logo where LMS serves images" '[ -f "$X/HTML/EN/plugins/LyrionAnywhere/html/images/logo.png" ] && grep -q "src=\"/plugins/LyrionAnywhere/html/images/logo.png\"" "$X/HTML/EN/plugins/LyrionAnywhere/settings/basic.html"'
t "settings page QR encoder present" '[ -f "$X/HTML/EN/plugins/LyrionAnywhere/settings/qr.js" ]'
t "settings page files are ASCII (no mojibake)" '! LC_ALL=C grep -rlP "[^\x00-\x7f]" "$X/HTML" --include=*.html --include=*.js --include=*.css | grep -q .'
t "web app present" '[ -f "$X/web/index.html" ] && [ -d "$X/web/_app/immutable" ] && [ -f "$X/web/player/pkg/anywhere_wasm_bg.wasm" ] && [ -f "$X/web/player/player-worker.js" ]'
# Settings › About shows the plugin's version (vite.config.ts reads install.xml at build time;
# the 0.5.3 zip shipped "0.1.0" because the Docker web stage had no install.xml to read).
t "web app shows the plugin version" 'grep -rqF "Version $VERSION" "$X/web/_app/immutable"'
t "web app has no inline script (CSP)" '! grep -q -E "<script>|<script [^>]*>[^<]" "$X/web/index.html"'
t "no tests, dotfiles, or local config in the zip" '[ -z "$(cd "$X" && find . \( -name "*.t" -o -name ".*" -o -name "*.bak" -o -name "*.env" -o \( -name "local.*" ! -name "*.js" \) \) ! -name . | head -1)" ]'
# The web app's own code may be called local.js (web/player/local.js); local config never is.
t "no web sources or tests in the zip" '[ ! -d "$X/web/tests" ] && [ ! -d "$X/web/src" ] && [ ! -d "$X/web/node_modules" ]'

echo "# gateways"
t "checksum list present" '[ -f "$X/Bin/SHA256SUMS" ]'
t "every binary matches its checksum" '(cd "$X/Bin" && sha256sum --quiet -c SHA256SUMS)'
t "every binary is listed" '[ "$(find "$X/Bin" -name anywhere-gateway -type f | wc -l)" = "$(wc -l < "$X/Bin/SHA256SUMS")" ]'
for pair in x86_64-linux:x86-64 aarch64-linux:aarch64 armhf-linux:ARM; do
	plat=${pair%%:*} arch=${pair##*:}
	f="$X/Bin/$plat/anywhere-gateway"
	t "$plat gateway is a static $arch executable" '[ -f "$f" ] && file -b "$f" | grep -q "$arch" && file -b "$f" | grep -q -E "statically linked|static-pie linked"'
done
if [ "$(uname -m)" = x86_64 ]; then
	t "x86_64 gateway runs" '"$X/Bin/x86_64-linux/anywhere-gateway" --version >/dev/null'
fi

if [ "${ANYWHERE_RUN_ARM:-}" = 1 ]; then
	# CI registers QEMU emulation, so the ARM builds can be run, not just inspected. The image
	# is pinned by digest like the Dockerfiles, but by each platform's own manifest, not the
	# multi-arch index (alpine:3.22@sha256:5291449c…): Docker refuses to pull a second platform
	# of one index digest ("cannot overwrite digest"), which failed the armhf check after the
	# aarch64 one had pulled (0.5.6).
	for triple in arm64:aarch64:sha256:2e1a7aa4cbc4e9e5222bb4c24a839aa1a6170ea5492d644777ce7b178824e44f \
	              arm/v7:armhf:sha256:947bab19f99aef448855af6d1886613d95d311a1b8bf9d32e7b329eb76ab4e44; do
		p=${triple%%:*} rest=${triple#*:} d=${rest%%:*} digest=${rest#*:}
		# The output is kept and shown on failure: QEMU and Docker errors are otherwise invisible.
		t "$d gateway runs (emulated)" 'docker run --rm --platform "linux/$p" -v "$X/Bin/$d-linux:/b:ro" "alpine@$digest" /b/anywhere-gateway --version >"$X/emulated.out" 2>&1 || { sed "s/^/    /" "$X/emulated.out"; false; }'
	done
fi

echo "# repo.xml"
SHA1=$(sha1sum "$ZIP" | cut -c1-40)
t "repo.xml is well-formed" 'python3 -c "import sys,xml.dom.minidom; xml.dom.minidom.parse(sys.argv[1])" "$DIST/repo.xml"'
t "repo.xml names the plugin icon" 'grep -q "<icon>https://raw.githubusercontent.com/[^<]*/logo-256.png</icon>" "$DIST/repo.xml"'
t "repo.xml sha1 matches the zip" 'grep -q "<sha>$SHA1</sha>" "$DIST/repo.xml"'
t "repo.xml version and URL match" 'grep -q "version=\"$VERSION\"" "$DIST/repo.xml" && grep -q "<url>https://[^<]*/LyrionAnywhere-$VERSION.zip</url>" "$DIST/repo.xml"'

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

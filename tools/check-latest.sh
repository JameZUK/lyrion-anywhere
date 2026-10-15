#!/usr/bin/env bash
# Check that the plugin repository URL LMS installs use still serves the plugin's repo.xml.
#
#   tools/check-latest.sh [--expect-version <version>] [url]
#
# LMS finds plugin updates at releases/latest/download/repo.xml. GitHub's "latest" is the
# newest full (non-pre-)release, so an Android release promoted from pre-release makes that
# URL a 404 and every install silently stops seeing plugin updates. Android releases must
# therefore stay pre-releases (docs/RELEASING.md). CI runs this after every release and
# weekly; run it by hand after changing anything on the releases page.
#
# CHECK_LATEST_TRIES (default 1) and CHECK_LATEST_DELAY (seconds, default 10) retry while
# GitHub's download redirect catches up with a release just published.
set -eu

URL=https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml
EXPECT=
while [ $# -gt 0 ]; do
	case $1 in
		--expect-version) EXPECT=${2:-}; shift 2 ;;
		-h|--help) sed -n '2,13p' "$0"; exit 0 ;;
		*) URL=$1; shift ;;
	esac
done
TRIES=${CHECK_LATEST_TRIES:-1}
DELAY=${CHECK_LATEST_DELAY:-10}
case $TRIES in ''|*[!0-9]*) TRIES=1 ;; esac
case $DELAY in ''|*[!0-9]*) DELAY=10 ;; esac

fail() {
	echo "check-latest: $URL: $1" >&2
	echo "check-latest: LMS installs using this repository URL will not see plugin updates." >&2
	echo "check-latest: if an android-v* release was marked Latest or promoted from pre-release," \
		"mark it a pre-release again and mark the newest v* plugin release as Latest (docs/RELEASING.md)." >&2
	exit 1
}

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

n=0
while :; do
	n=$((n + 1))
	if curl -fsSL --proto '=https,http' --max-time 30 --max-filesize 1048576 -o "$tmp" "$URL" 2>/dev/null; then
		break
	fi
	[ "$n" -lt "$TRIES" ] || fail "could not be fetched (a 404 here usually means the newest full release is not a plugin release)"
	sleep "$DELAY"
done

# Parsed with Python's standard library only (no network, no external entities).
version=$(python3 -I - "$tmp" <<'PY'
import re, sys
import xml.etree.ElementTree as ET
try:
    root = ET.parse(sys.argv[1]).getroot()
except Exception as e:
    sys.exit(f"not well-formed XML: {e}")
if root.tag != "extensions":
    sys.exit(f"root element is <{root.tag}>, not <extensions>")
plugins = [p for p in root.iter("plugin") if p.get("name") == "LyrionAnywhere"]
if len(plugins) != 1:
    sys.exit("no single LyrionAnywhere plugin entry")
p = plugins[0]
v = p.get("version") or ""
if not re.fullmatch(r"[0-9]+(\.[0-9]+)*", v):
    sys.exit(f"bad plugin version {v!r}")
url = (p.findtext("url") or "").strip()
if not re.fullmatch(r"https://\S+/LyrionAnywhere-" + re.escape(v) + r"\.zip", url):
    sys.exit(f"zip URL {url!r} does not match version {v}")
if not re.fullmatch(r"[0-9a-f]{40}", (p.findtext("sha") or "").strip()):
    sys.exit("no SHA1 for the zip")
print(v)
PY
) || fail "is not the plugin's repo.xml"

if [ -n "$EXPECT" ] && [ "$version" != "$EXPECT" ]; then
	fail "serves plugin $version, not the $EXPECT just released"
fi
echo "ok: $URL serves the Lyrion Anywhere plugin $version"

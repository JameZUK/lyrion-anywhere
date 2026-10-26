#!/bin/bash
# Build a plugin release: dist/LyrionAnywhere-<version>.zip and dist/repo.xml.
#
#   tools/build-release.sh [download-url-prefix]
#
# The zip holds the plugin, static gateways for x86_64/aarch64/armhf Linux with
# Bin/SHA256SUMS (checked by the plugin before it runs one), and the browser player.
# Needs docker (with BuildKit), python3 and sha1sum/sha256sum.
# SKIP_BUILD=1 re-packages from an earlier build in dist/build.
set -eu
cd "$(dirname "$0")/.."

PLUGIN=plugin/Plugins/LyrionAnywhere
VERSION=$(sed -n 's:.*<version>\(.*\)</version>.*:\1:p' "$PLUGIN/install.xml")
URL_PREFIX=${1:-https://github.com/JameZUK/lyrion-anywhere/releases/download/v$VERSION}
case $VERSION in *[!0-9.]*|'') echo "bad version in install.xml: $VERSION" >&2; exit 1 ;; esac

if [ "${SKIP_BUILD:-}" = 1 ]; then
	[ -d dist/build/Bin ] || { echo "no earlier build in dist/build" >&2; exit 1; }
	rm -rf dist/LyrionAnywhere dist/LyrionAnywhere-*.zip dist/repo.xml
	mkdir -p dist/LyrionAnywhere
else
	# Only this script's own outputs: dist/ also holds deploy binaries and Android builds.
	rm -rf dist/build dist/LyrionAnywhere dist/LyrionAnywhere-*.zip dist/repo.xml
	mkdir -p dist/build dist/LyrionAnywhere
	docker build -f tools/release/Dockerfile --target out --output type=local,dest=dist/build .
fi

# Plugin files tracked by git (no local build leftovers such as Bin/<platform>/ or web/).
git ls-files -z "$PLUGIN" | while IFS= read -r -d '' f; do
	rel=${f#"$PLUGIN"/}
	mkdir -p "dist/LyrionAnywhere/$(dirname "$rel")"
	cp -p "$f" "dist/LyrionAnywhere/$rel"
done
cp -R dist/build/Bin/. dist/LyrionAnywhere/Bin/
cp -R dist/build/web dist/LyrionAnywhere/web
(cd dist/LyrionAnywhere/Bin && find . -name anywhere-gateway -type f | sed 's:^\./::' | sort \
  | xargs sha256sum > SHA256SUMS)

ZIP=LyrionAnywhere-$VERSION.zip
# Sorted entries, Unix modes kept so the gateways stay executable, and every member dated
# with the commit being built (UTC): the same for every build of a release, and later for
# each release. Never a constant: LMS keeps the dates when it installs the plugin and goes
# on using its compiled copy of a page template while the template's date is unchanged, so
# a constant date hid changes to the settings page from updates (issue #29).
ZIP_DATE=$(git log -1 --format=%ct)
python3 - "dist/LyrionAnywhere" "dist/$ZIP" "$ZIP_DATE" <<'PY'
import os, sys, time, zipfile
src, out, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
stamp = time.gmtime(max(epoch, 315532800))[:6]   # zip dates start in 1980
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for root, dirs, files in os.walk(src):
        dirs.sort()
        for name in sorted(files):
            path = os.path.join(root, name)
            info = zipfile.ZipInfo(os.path.relpath(path, src), date_time=stamp)
            info.external_attr = (os.stat(path).st_mode & 0o777 | 0o100000) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as f:
                z.writestr(info, f.read())
PY
SHA1=$(sha1sum "dist/$ZIP" | cut -c1-40)

cat > dist/repo.xml <<XML
<?xml version="1.0" encoding="UTF-8"?>
<extensions>
	<details>
		<title lang="EN">Lyrion Anywhere</title>
	</details>
	<plugins>
		<plugin name="LyrionAnywhere" version="$VERSION" minTarget="9.0" maxTarget="*">
			<title lang="EN">Lyrion Anywhere</title>
			<desc lang="EN">Secure remote access to this server from phones and browsers, through an authenticated gateway behind your HTTPS reverse proxy.</desc>
			<creator>JameZUK</creator>
			<link>https://github.com/JameZUK/lyrion-anywhere</link>
			<icon>https://raw.githubusercontent.com/JameZUK/lyrion-anywhere/main/docs/images/logo-256.png</icon>
			<category>tools</category>
			<url>$URL_PREFIX/$ZIP</url>
			<sha>$SHA1</sha>
		</plugin>
	</plugins>
</extensions>
XML

echo "dist/$ZIP  sha1 $SHA1"
cat dist/LyrionAnywhere/Bin/SHA256SUMS

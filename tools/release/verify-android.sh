#!/usr/bin/env bash
# Verify a published Android release and keep its files under dist/android/<version>/.
#
#   tools/release/verify-android.sh <version> [<previous version>]
#
# Downloads the android-v<version> release into a fresh dist/android/<version>/ and checks:
# SHA256SUMS; the build provenance of every APK and the bundle (gh attestation verify);
# that every APK is signed with the release certificate (its SHA-256 is below, the one in
# every Android release note); that the universal APK's versionName and versionCode are the
# ones in apps/android/app/build.gradle.kts; that the merged manifest differs from the
# previous release's (dist/android/<previous>/, when given and present) only in the version
# lines, and lists its providers and androidx.startup initialisers (docs/RELEASING.md: a new
# initialiser once crashed the app before onCreate); and that the release is not marked
# Latest, which must stay the plugin's (the plugin update URL depends on it). Needs the gh
# CLI and ANDROID_HOME (or ~/Android/Sdk) with build-tools; keytool (a JDK) for the bundle's
# signer, skipped with a note when absent.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
v=${1:?usage: verify-android.sh <version> [<previous version>]}
prev=${2:-}
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
CERT_SHA256=8674da2bce502622f0a02e7a225096dffbd0a94517826625c96cddaf3790d285
sdk=${ANDROID_HOME:-$HOME/Android/Sdk}
bt=$(ls -d "$sdk"/build-tools/*/ 2>/dev/null | sort -V | tail -1)
[ -n "$bt" ] || { echo "no build-tools under $sdk" >&2; exit 1; }
aapt2=${bt}aapt2
apksigner=${bt}apksigner
dir=$root/dist/android/$v
apk="LyrionAnywhere-android-$v.apk"

fail() { echo "verify-android.sh: $*" >&2; exit 1; }

rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"
gh release download "android-v$v" -R "$repo" -D . --clobber
[ -f "$apk" ] || fail "$apk was not among the release's files: $(ls)"
echo "== checksums"
sha256sum -c SHA256SUMS
echo "== build provenance (silent when good)"
# Built by this repository's CI workflow, on GitHub's own runners, from this tag: an
# attestation from any other workflow in the repository, or for another ref, does not count.
for f in *.apk *.aab; do
	gh attestation verify "$f" -R "$repo" --signer-workflow "$repo/.github/workflows/ci.yml" \
		--source-ref "refs/tags/android-v$v" --deny-self-hosted-runners
done
echo "== signing certificate"
for f in *.apk; do
	have=$("$apksigner" verify --print-certs "$f" | sed -n 's/.*SHA-256 digest: *//p' | head -1)
	[ "$have" = "$CERT_SHA256" ] || fail "$f is signed with $have, not the release certificate"
	echo "$f: release certificate"
done
if command -v keytool >/dev/null; then
	for f in *.aab; do
		have=$(keytool -printcert -jarfile "$f" | sed -n 's/.*SHA256: *//p' | head -1 | tr -d ':' | tr 'A-F' 'a-f')
		[ "$have" = "$CERT_SHA256" ] || fail "$f is signed with $have, not the release certificate"
		echo "$f: release certificate"
	done
else
	echo "note: keytool not found; the bundle's signer was not checked"
fi
echo "== version"
badging=$("$aapt2" dump badging "$apk" | grep '^package:')
code=$(sed -n "s/.*versionCode='\([0-9]*\)'.*/\1/p" <<<"$badging")
name=$(sed -n "s/.*versionName='\([^']*\)'.*/\1/p" <<<"$badging")
gradle=$root/apps/android/app/build.gradle.kts
want_code=$(sed -n 's/.*versionCode = \([0-9]*\).*/\1/p' "$gradle")
want_name=$(sed -n 's/.*versionName = "\(.*\)".*/\1/p' "$gradle")
[ "$name" = "$v" ] || fail "the APK's versionName is $name, not $v"
if [ "$want_name" = "$v" ]; then
	[ "$code" = "$want_code" ] || fail "the APK has versionCode $code; build.gradle.kts says $want_code for $v"
	echo "versionName $name, versionCode $code (as build.gradle.kts)"
else
	echo "versionName $name, versionCode $code (the tree is at $want_name: not compared)"
fi
echo "== manifest"
"$aapt2" dump xmltree --file AndroidManifest.xml "$apk" >"$dir/manifest.txt"
grep -E 'E: provider|androidx\.startup|android:name\(0x01010003\)=".*Initializer"' "$dir/manifest.txt" | sed 's/^ *//' | sort -u | sed 's/^/  /'
if [ -n "$prev" ]; then
	old=$root/dist/android/$prev/LyrionAnywhere-android-$prev.apk
	if [ -f "$old" ]; then
		"$aapt2" dump xmltree --file AndroidManifest.xml "$old" >"$dir/manifest-$prev.txt"
		changed=$(diff "$dir/manifest-$prev.txt" "$dir/manifest.txt" | grep -E '^[<>]' || true)
		if [ -z "$changed" ]; then
			echo "manifest identical to $prev"
		elif grep -vqE 'versionCode|versionName' <<<"$changed"; then
			echo "$changed" >&2
			fail "the manifest differs from $prev beyond the version lines (above)"
		else
			echo "manifest differs from $prev only in the version"
		fi
	else
		echo "note: $old not present; manifest not compared"
	fi
fi
echo "== Latest"
latest=$(gh api "repos/$repo/releases/latest" --jq .tag_name)
[ "$latest" != "android-v$v" ] || fail "android-v$v is marked Latest: run gh release edit <plugin tag> --latest, then tools/check-latest.sh"
echo "Latest is $latest"
[ -f BUILDINFO.txt ] && { echo "== BUILDINFO.txt"; sed 's/^/  /' BUILDINFO.txt; }
echo "verified android-v$v in $dir"

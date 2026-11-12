#!/usr/bin/env bash
# Sign an unsigned Android release build with the release key, using only the SDK's apksigner
# and the JDK's jarsigner (no build code runs here), then check every signature.
#
#   tools/release/sign-android.sh <unsigned dir> <out dir> <version>
#
# <unsigned dir> holds what `./gradlew -Panywhere.splitApks=true assembleRelease` and
# `./gradlew bundleRelease` leave when no key is given (app/build/outputs, or a copy of it):
# apk/release/app-<abi>-release-unsigned.apk (universal and one per ABI) and
# bundle/release/app-release.aab. <out dir> gets LyrionAnywhere-android-<version>.apk (the
# universal one), LyrionAnywhere-android-<version>-<abi>.apk and
# LyrionAnywhere-android-<version>.aab.
#
# The key comes from the environment: ANYWHERE_KEYSTORE (a path), ANYWHERE_KEYSTORE_PASSWORD,
# ANYWHERE_KEY_ALIAS and ANYWHERE_KEY_PASSWORD. The passwords reach the tools by variable
# name, never on a command line, and are never printed. The tools: APKSIGNER and ZIPALIGN, or
# those in ANDROID_HOME's build-tools/$BUILD_TOOLS (the newest there if BUILD_TOOLS is unset);
# JARSIGNER and KEYTOOL, or JAVA_HOME's, or the PATH's.
#
# The APKs are signed as the Android Gradle plugin signed them: APK Signature Scheme v2 only
# (minSdk 29 needs no v1; one key needs no v3; no v4 .idsig files), keeping the alignment the
# build gave them, so that a reproducible build of the same commit (F-Droid) still matches
# once the signature is copied onto it: the entries and the central directory stay byte for
# byte the build's, and apksigner adds only zero padding (so that its signing block starts on
# a 4 KiB boundary) and the block. An APK that is not aligned (4 bytes, native libraries on
# 16 KiB pages) is aligned first, with a warning. The bundle is JAR-signed with SHA-256
# digests and SHA256withRSA, as the plugin signed it; Google Play checks it as the upload key.
set -euo pipefail
usage="usage: sign-android.sh <unsigned dir> <out dir> <version>"
in=${1:?$usage}
out=${2:?$usage}
v=${3:?$usage}

fail() { echo "sign-android.sh: $*" >&2; exit 1; }

case $v in ''|*[!0-9A-Za-z.-]*) fail "not a version: $v" ;; esac
[ -n "${ANYWHERE_KEYSTORE:-}" ] && [ -f "$ANYWHERE_KEYSTORE" ] || fail "ANYWHERE_KEYSTORE must name the keystore file"
[ -n "${ANYWHERE_KEY_ALIAS:-}" ] || fail "ANYWHERE_KEY_ALIAS is not set"
[ -n "${ANYWHERE_KEYSTORE_PASSWORD:-}" ] || fail "ANYWHERE_KEYSTORE_PASSWORD is not set"
[ -n "${ANYWHERE_KEY_PASSWORD:-}" ] || fail "ANYWHERE_KEY_PASSWORD is not set"

sdk=${ANDROID_HOME:-$HOME/Android/Sdk}
if [ -n "${BUILD_TOOLS:-}" ]; then bt=$sdk/build-tools/$BUILD_TOOLS/
else bt=$(ls -d "$sdk"/build-tools/*/ 2>/dev/null | sort -V | tail -1); fi
apksigner=${APKSIGNER:-${bt}apksigner}
zipalign=${ZIPALIGN:-${bt}zipalign}
jdk=${JAVA_HOME:+$JAVA_HOME/bin/}
jarsigner=${JARSIGNER:-${jdk}jarsigner}
keytool=${KEYTOOL:-${jdk}keytool}
for t in "$apksigner" "$zipalign" "$jarsigner" "$keytool"; do
	command -v "$t" >/dev/null 2>&1 || fail "$t not found"
done

aab=$in/bundle/release/app-release.aab
[ -f "$in/apk/release/app-universal-release-unsigned.apk" ] || fail "no universal APK in $in/apk/release"
[ -f "$aab" ] || fail "no app bundle at $aab"
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "== signing the APKs (v2 only, alignment kept)"
for apk in "$in"/apk/release/app-*-release-unsigned.apk; do
	abi=$(basename "$apk" | sed 's/^app-\(.*\)-release-unsigned\.apk$/\1/')
	case $abi in
		universal) dest=$out/LyrionAnywhere-android-$v.apk ;;
		*[!0-9a-z_-]*|'') fail "unexpected APK name: $apk" ;;
		*) dest=$out/LyrionAnywhere-android-$v-$abi.apk ;;
	esac
	src=$apk
	if ! "$zipalign" -c -P 16 4 "$apk" >/dev/null 2>&1; then
		echo "::warning::$(basename "$apk") was not aligned; aligning it (a reproducible build may no longer match it)"
		src=$work/$(basename "$apk")
		"$zipalign" -P 16 -f 4 "$apk" "$src"
	fi
	"$apksigner" sign --ks "$ANYWHERE_KEYSTORE" --ks-pass env:ANYWHERE_KEYSTORE_PASSWORD \
		--ks-key-alias "$ANYWHERE_KEY_ALIAS" --key-pass env:ANYWHERE_KEY_PASSWORD \
		--v1-signing-enabled false --v2-signing-enabled true --v3-signing-enabled false --v4-signing-enabled false \
		--alignment-preserved true --out "$dest" "$src"
	echo "$(basename "$dest")"
done

echo "== signing the app bundle"
cp "$aab" "$out/LyrionAnywhere-android-$v.aab"
"$jarsigner" -keystore "$ANYWHERE_KEYSTORE" -storepass:env ANYWHERE_KEYSTORE_PASSWORD -keypass:env ANYWHERE_KEY_PASSWORD \
	-sigalg SHA256withRSA -digestalg SHA-256 "$out/LyrionAnywhere-android-$v.aab" "$ANYWHERE_KEY_ALIAS" >/dev/null
echo "LyrionAnywhere-android-$v.aab"

echo "== checking the signatures"
ls "$out"/*.idsig >/dev/null 2>&1 && fail "apksigner wrote v4 .idsig files"
cert=$("$apksigner" verify --print-certs "$out/LyrionAnywhere-android-$v.apk" | sed -n 's/.*certificate SHA-256 digest: *//p' | head -1)
[ -n "$cert" ] || fail "the universal APK does not verify"
for f in "$out"/LyrionAnywhere-android-"$v"*.apk; do
	report=$("$apksigner" verify -v --print-certs "$f") || fail "$(basename "$f") does not verify"
	grep -q '^Verified using v2 scheme (APK Signature Scheme v2): true' <<<"$report" || fail "$(basename "$f") has no v2 signature"
	have=$(sed -n 's/.*certificate SHA-256 digest: *//p' <<<"$report" | head -1)
	[ "$have" = "$cert" ] || fail "$(basename "$f") is signed with $have, not $cert"
	"$zipalign" -c -P 16 4 "$f" >/dev/null || fail "$(basename "$f") is not aligned after signing"
done
"$jarsigner" -verify "$out/LyrionAnywhere-android-$v.aab" | grep -q '^jar verified' || fail "the app bundle does not verify"
got=$("$keytool" -printcert -jarfile "$out/LyrionAnywhere-android-$v.aab" | sed -n 's/^[[:space:]]*SHA256: //p' | head -1 | tr -d ':' | tr 'A-F' 'a-f')
[ "$got" = "$cert" ] || fail "the app bundle is signed with $got, not the APKs' $cert"
if [ -n "${ANYWHERE_EXPECT_CERT:-}" ] && [ "$cert" != "$ANYWHERE_EXPECT_CERT" ]; then
	fail "signed with $cert, not the expected release certificate $ANYWHERE_EXPECT_CERT"
fi
echo "signed and verified: certificate SHA-256 $cert"

#!/usr/bin/env bash
# Test tools/release/sign-android.sh, the release job's signing step, with a throwaway key.
#
#   tools/release/test-sign-android.sh <apk>
#
# <apk> is any APK built from this repository (CI uses the debug build); it stands in for
# each unsigned release APK, and a copy of it for the app bundle (jarsigner signs any zip).
# Needs the SDK's build-tools (see sign-android.sh) and a JDK. Checks that every file comes out
# signed with the throwaway key and verified (APKs: v2 only, aligned, no .idsig), and that
# the script refuses to run without the key or its passwords, fails on a wrong password, and
# never prints a password.
set -euo pipefail
cd "$(dirname "$0")/../.."
apk=${1:?usage: test-sign-android.sh <apk>}
[ -f "$apk" ] || { echo "no APK at $apk" >&2; exit 2; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
PASS=0 FAIL=0
ok()   { PASS=$((PASS + 1)); echo "ok   - $1"; }
fail() { FAIL=$((FAIL + 1)); echo "FAIL - $1"; }
jdk=${JAVA_HOME:+$JAVA_HOME/bin/}

# A throwaway key, and the unsigned layout the release build leaves.
store_pw="store-$RANDOM-$RANDOM-secret"
key_pw="key-$RANDOM-$RANDOM-secret"
"${jdk}keytool" -genkeypair -keystore "$work/test.jks" -storetype PKCS12 -alias test-release -keyalg RSA -keysize 2048 \
	-validity 2 -dname 'CN=Lyrion Anywhere test key' -storepass "$store_pw" -keypass "$store_pw" >/dev/null 2>&1
# PKCS12 keeps one password for both; the script is given it twice, as CI gives the two secrets.
key_pw=$store_pw
want=$("${jdk}keytool" -list -v -keystore "$work/test.jks" -storepass "$store_pw" -alias test-release \
	| sed -n 's/^[[:space:]]*SHA256: //p' | head -1 | tr -d ':' | tr 'A-F' 'a-f')
mkdir -p "$work/in/apk/release" "$work/in/bundle/release"
for abi in universal arm64-v8a armeabi-v7a x86_64; do
	cp "$apk" "$work/in/apk/release/app-$abi-release-unsigned.apk"
done
cp "$apk" "$work/in/bundle/release/app-release.aab"

sign() {
	rc=0
	out=$(env ANYWHERE_KEYSTORE="$work/test.jks" ANYWHERE_KEY_ALIAS=test-release \
		ANYWHERE_KEYSTORE_PASSWORD="$store_pw" ANYWHERE_KEY_PASSWORD="$key_pw" "$@" \
		tools/release/sign-android.sh "$work/in" "$work/out" 9.9.9 2>&1) || rc=$?
}

echo "# signing"
sign
if [ "$rc" = 0 ]; then ok "signs"; else fail "sign-android.sh exited $rc: $out"; fi
case $out in *"certificate SHA-256 $want"*) ok "with the throwaway key";; *) fail "not the throwaway key ($want): $out";; esac
for f in LyrionAnywhere-android-9.9.9.apk LyrionAnywhere-android-9.9.9-arm64-v8a.apk LyrionAnywhere-android-9.9.9-armeabi-v7a.apk \
         LyrionAnywhere-android-9.9.9-x86_64.apk LyrionAnywhere-android-9.9.9.aab; do
	if [ -f "$work/out/$f" ]; then ok "$f written"; else fail "$f missing"; fi
done
if ls "$work/out"/*.idsig >/dev/null 2>&1; then fail "v4 .idsig files written"; else ok "no .idsig files"; fi
case $out in *"$store_pw"*|*"$key_pw"*) fail "a password was printed";; *) ok "no password printed";; esac
bt=${BUILD_TOOLS:+${ANDROID_HOME:-$HOME/Android/Sdk}/build-tools/$BUILD_TOOLS/}
[ -n "$bt" ] || bt=$(ls -d "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools/*/ | sort -V | tail -1)
report=$("${APKSIGNER:-${bt}apksigner}" verify -v "$work/out/LyrionAnywhere-android-9.9.9.apk")
case $report in *"v2 scheme (APK Signature Scheme v2): true"*) ok "v2 signature";; *) fail "no v2 signature: $report";; esac
case $report in *"v1 scheme (JAR signing): false"*"v3 scheme (APK Signature Scheme v3): false"*) ok "and no v1 or v3, as the Android Gradle plugin signed";; *) fail "schemes differ: $report";; esac
if "${jdk}jarsigner" -verify "$work/out/LyrionAnywhere-android-9.9.9.aab" | grep -q '^jar verified'; then ok "the bundle verifies"; else fail "the bundle does not verify"; fi

echo "# refusals"
rm -rf "$work/out"
for missing in ANYWHERE_KEYSTORE ANYWHERE_KEY_ALIAS ANYWHERE_KEYSTORE_PASSWORD ANYWHERE_KEY_PASSWORD; do
	sign -u "$missing"
	if [ "$rc" != 0 ] && [ ! -e "$work/out" ]; then ok "without $missing: refused, nothing written"; else fail "ran without $missing (exit $rc)"; fi
done
sign ANYWHERE_KEYSTORE_PASSWORD="wrong-$RANDOM"
if [ "$rc" != 0 ]; then ok "a wrong keystore password fails"; else fail "signed with a wrong password"; fi
case $out in *"$store_pw"*) fail "a password was printed on failure";; *) ok "no password printed on failure";; esac
rm -rf "$work/out"
mv "$work/in/apk/release/app-universal-release-unsigned.apk" "$work/universal.apk"
sign
if [ "$rc" != 0 ]; then ok "no universal APK: refused"; else fail "signed without a universal APK"; fi
mv "$work/universal.apk" "$work/in/apk/release/app-universal-release-unsigned.apk"
rm "$work/in/bundle/release/app-release.aab"
sign
if [ "$rc" != 0 ]; then ok "no app bundle: refused"; else fail "signed without an app bundle"; fi
out=
rc=0
out=$(tools/release/sign-android.sh "$work/in" "$work/out" '9.9.9;x' 2>&1) || rc=$?
if [ "$rc" != 0 ]; then ok "a version with odd characters: refused"; else fail "accepted the version 9.9.9;x"; fi

echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

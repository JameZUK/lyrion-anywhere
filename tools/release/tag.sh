#!/usr/bin/env bash
# Tag a release on HEAD once the tree is ready, and print (or run) the push.
#
#   tools/release/tag.sh [--push] [plugin <version>] [android <version>]
#
# Checks before tagging: a clean working tree (untracked files aside); HEAD is on
# origin/main (push main first and wait for it to be green with ci-wait.sh: its release job
# proves the Docker gateway build); the plugin version is the one in
# plugin/Plugins/LyrionAnywhere/install.xml and docs/release-notes/v<version>.md exists;
# the Android version is the app's versionName, docs/release-notes/android-<version>.md
# exists, and apps/android/fastlane/metadata/android/en-GB/changelogs/<versionCode>.txt
# exists (F-Droid). Then makes the annotated tags v<version> and android-v<version>.
#
# Without --push the exact push command is printed, to run as its own command (no
# git pushes on weekdays 07:30–17:00 UK; see docs/RELEASING.md). Each tag's run publishes
# its release about 35 minutes later; wait with `tools/release/ci-wait.sh tag <tag>`, then
# verify with verify-plugin.sh / verify-android.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."

push=0
plugin=""
android=""
while [ $# -gt 0 ]; do
	case "$1" in
	--push) push=1 ;;
	plugin) plugin=${2:?plugin <version>}; shift ;;
	android) android=${2:?android <version>}; shift ;;
	*) sed -n '2,5p' "$0" >&2; exit 2 ;;
	esac
	shift
done
if [ -z "$plugin" ] && [ -z "$android" ]; then
	sed -n '2,5p' "$0" >&2
	exit 2
fi

fail() { echo "tag.sh: $*" >&2; exit 1; }

[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the working tree has uncommitted changes"
git fetch -q origin main
git merge-base --is-ancestor HEAD origin/main || fail "HEAD is not on origin/main: push main first and wait for it to be green"

tags=()
if [ -n "$plugin" ]; then
	have=$(sed -n 's/.*<version>\(.*\)<\/version>.*/\1/p' plugin/Plugins/LyrionAnywhere/install.xml)
	[ "$have" = "$plugin" ] || fail "install.xml says $have, not $plugin"
	[ -f "docs/release-notes/v$plugin.md" ] || fail "docs/release-notes/v$plugin.md is missing"
	git rev-parse -q --verify "refs/tags/v$plugin" >/dev/null && fail "tag v$plugin exists already"
	tags+=("v$plugin")
fi
if [ -n "$android" ]; then
	gradle=apps/android/app/build.gradle.kts
	name=$(sed -n 's/.*versionName = "\(.*\)".*/\1/p' "$gradle")
	code=$(sed -n 's/.*versionCode = \([0-9]*\).*/\1/p' "$gradle")
	[ "$name" = "$android" ] || fail "$gradle says versionName $name, not $android"
	[ -n "$code" ] || fail "no versionCode in $gradle"
	[ -f "docs/release-notes/android-$android.md" ] || fail "docs/release-notes/android-$android.md is missing"
	cl=apps/android/fastlane/metadata/android/en-GB/changelogs/$code.txt
	[ -f "$cl" ] || fail "$cl is missing (the F-Droid changelog for versionCode $code)"
	[ "$(wc -c <"$cl")" -le 500 ] || fail "$cl is over 500 characters"
	git rev-parse -q --verify "refs/tags/android-v$android" >/dev/null && fail "tag android-v$android exists already"
	tags+=("android-v$android")
fi

for t in "${tags[@]}"; do
	case "$t" in
	android-*) git tag -a "$t" -m "Lyrion Anywhere for Android ${t#android-v}" ;;
	*) git tag -a "$t" -m "Lyrion Anywhere ${t#v}" ;;
	esac
	echo "tagged $t at $(git rev-parse --short HEAD)"
done

if [ "$push" = 1 ]; then
	git push origin "${tags[@]}"
	echo "pushed; wait with: tools/release/ci-wait.sh tag <tag>"
else
	echo "now push the tags (outside the no-push window):"
	echo "  git push origin ${tags[*]}"
fi

#!/usr/bin/env bash
# Verify a published plugin release and unpack it for deployment.
#
#   tools/release/verify-plugin.sh <version> [<dir>]     # default dir: dist/releases/v<version>
#
# Downloads the release's files into a fresh <dir>, checks SHA256SUMS, the build provenance
# of the zip and repo.xml (gh attestation verify, signed by the CI workflow), that the
# plugin repository URL LMS installs use still serves this version (tools/check-latest.sh),
# unpacks the zip into <dir>/unpacked without trusting the archive's paths, checks
# install.xml's version and Bin/SHA256SUMS inside, and prints the newest file date (release
# zips are dated with their commit since 0.7.6, so LMS's caches see an update). The last
# line printed is the unpacked plugin directory, for the deploy step.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
v=${1:?usage: verify-plugin.sh <version> [<dir>]}
dir=${2:-$root/dist/releases/v$v}
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
zip="LyrionAnywhere-$v.zip"

fail() { echo "verify-plugin.sh: $*" >&2; exit 1; }

rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"
gh release download "v$v" -R "$repo" -D . --clobber
[ -f "$zip" ] || fail "$zip was not among the release's files: $(ls)"
echo "== checksums"
sha256sum -c SHA256SUMS
echo "== build provenance (silent when good)"
# Built by this repository's CI workflow, on GitHub's own runners, from this tag: an
# attestation from any other workflow in the repository, or for another ref, does not count.
provenance=(-R "$repo" --signer-workflow "$repo/.github/workflows/ci.yml" --source-ref "refs/tags/v$v" --deny-self-hosted-runners)
gh attestation verify "$zip" "${provenance[@]}"
gh attestation verify repo.xml "${provenance[@]}"
echo "== plugin repository URL"
"$root/tools/check-latest.sh" --expect-version "$v"

echo "== contents"
python3 -I - "$zip" "$dir/unpacked" <<'EOF'
import os, pathlib, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
out = pathlib.Path(sys.argv[2]).resolve()
newest = None
for m in z.infolist():
    p = (out / m.filename).resolve()
    if p != out and not str(p).startswith(str(out) + os.sep):
        raise SystemExit(f"unsafe path in the zip: {m.filename}")
    if not m.is_dir():
        newest = max(newest or m.date_time, m.date_time)
z.extractall(out)
print(f"{len(z.infolist())} entries, newest file date {newest[0]:04d}-{newest[1]:02d}-{newest[2]:02d} {newest[3]:02d}:{newest[4]:02d}:{newest[5]:02d}")
EOF
plugin=$(find "$dir/unpacked" -maxdepth 2 -name install.xml -printf '%h\n' | head -1)
[ -n "$plugin" ] || fail "no install.xml inside the zip"
have=$(sed -n 's/.*<version>\(.*\)<\/version>.*/\1/p' "$plugin/install.xml")
[ "$have" = "$v" ] || fail "the zip's install.xml says $have, not $v"
echo "install.xml: $have"
[ -f "$plugin/Bin/SHA256SUMS" ] || fail "no Bin/SHA256SUMS inside the zip"
(cd "$plugin/Bin" && sha256sum -c --quiet SHA256SUMS) && echo "Bin/SHA256SUMS: ok ($(ls "$plugin/Bin" | grep -c linux) targets)"
[ -f "$plugin/web/index.html" ] || fail "no web/index.html inside the zip"
if [ -f "$root/dist/build/Bin/SHA256SUMS" ]; then
	if cmp -s "$plugin/Bin/SHA256SUMS" "$root/dist/build/Bin/SHA256SUMS"; then
		echo "Bin/SHA256SUMS matches the local build (dist/build)"
	else
		echo "note: Bin/SHA256SUMS differs from the local build in dist/build (only a concern if that build is of this very commit)"
	fi
fi
echo "verified v$v"
echo "$plugin"

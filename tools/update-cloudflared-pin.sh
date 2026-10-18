#!/bin/bash
# Pin the gateway's built-in Cloudflare Tunnel to a `cloudflared` release: downloads the
# three Linux assets from GitHub into a fresh temporary directory, sums them, and rewrites
# gateway/src/cftunnel/pins.rs (its comments are kept). Run it before each release with the
# newest `cloudflared` release and note the move in CHANGELOG.md (docs/RELEASING.md).
#   tools/update-cloudflared-pin.sh <version>      e.g. 2026.10.1
set -eu
VERSION=${1:-}
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]{4}\.[0-9]{1,2}\.[0-9]+$'; then
	echo "usage: $0 <version>   (a cloudflared release such as 2026.10.1)" >&2
	exit 2
fi
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PINS="$ROOT/gateway/src/cftunnel/pins.rs"
[ -f "$PINS" ] || { echo "$PINS not found" >&2; exit 1; }
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
BASE="https://github.com/cloudflare/cloudflared/releases/download/$VERSION"

sum_of() {
	curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --max-redirs 3 --max-time 600 \
		-o "$WORK/$1" "$BASE/$1"
	sha256sum "$WORK/$1" | cut -c1-64
}

AMD64=$(sum_of cloudflared-linux-amd64)
ARM64=$(sum_of cloudflared-linux-arm64)
ARMHF=$(sum_of cloudflared-linux-armhf)
for s in "$AMD64" "$ARM64" "$ARMHF"; do
	printf '%s' "$s" | grep -Eq '^[0-9a-f]{64}$' || { echo "bad sum: $s" >&2; exit 1; }
done

sed -E -i \
	-e "s/^(pub const CLOUDFLARED_VERSION: &str = \")[^\"]*(\";)/\1$VERSION\2/" \
	-e "s/^(pub const SHA256_AMD64: &str = \")[^\"]*(\";)/\1$AMD64\2/" \
	-e "s/^(pub const SHA256_ARM64: &str = \")[^\"]*(\";)/\1$ARM64\2/" \
	-e "s/^(pub const SHA256_ARMHF: &str = \")[^\"]*(\";)/\1$ARMHF\2/" \
	"$PINS"
if ! { grep -q "\"$VERSION\"" "$PINS" && grep -q "$AMD64" "$PINS" \
	&& grep -q "$ARM64" "$PINS" && grep -q "$ARMHF" "$PINS"; }; then
	echo "pins.rs was not rewritten as expected" >&2
	exit 1
fi

echo "cloudflared $VERSION pinned in gateway/src/cftunnel/pins.rs:"
echo "  cloudflared-linux-amd64  $AMD64"
echo "  cloudflared-linux-arm64  $ARM64"
echo "  cloudflared-linux-armhf  $ARMHF"

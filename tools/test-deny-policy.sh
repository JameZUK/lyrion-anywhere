#!/bin/bash
# The licence policy in deny.toml refuses GPL and LGPL dependencies (LICENSE-EXCEPTION
# covers only this project's own code), while this project's crates and dual-licensed
# crates that offer a permissive choice pass. Builds throwaway workspaces with path
# dependencies (no network, nothing compiled) and runs `cargo deny check licenses` on them.
#   tools/test-deny-policy.sh          (needs cargo and cargo-deny)
set -eu
cd "$(dirname "$0")/.."

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# A workspace with one of the project's crates (anywhere-mobile) depending on a crate
# under licence $1. Prints its directory.
workspace() {
	local dir="$WORK/$2"
	mkdir -p "$dir/app/src" "$dir/dep/src"
	cp deny.toml "$dir/"
	printf '[workspace]\nmembers = ["app"]\nresolver = "2"\n' > "$dir/Cargo.toml"
	printf '[package]\nname = "anywhere-mobile"\nversion = "0.0.1"\nedition = "2021"\nlicense = "GPL-2.0-or-later"\npublish = false\n\n[dependencies]\ndep = { path = "../dep" }\n' > "$dir/app/Cargo.toml"
	printf '[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\nlicense = "%s"\n' "$1" > "$dir/dep/Cargo.toml"
	touch "$dir/app/src/lib.rs" "$dir/dep/src/lib.rs"
	echo "$dir"
}

deny() { cargo deny --manifest-path "$1/Cargo.toml" check licenses 2>&1; }
# Refused for its licence (not for some other failure, which would pass unnoticed).
refused() { local out; if out=$(deny "$1"); then return 1; fi; grep -q 'license is not explicitly allowed' <<<"$out"; }

for lic in GPL-2.0-or-later GPL-3.0-or-later GPL-2.0-only LGPL-2.1-or-later LGPL-3.0-only AGPL-3.0-or-later; do
	d=$(workspace "$lic" "refuse-$lic")
	if refused "$d"; then ok "a dependency under $lic is refused"; else bad "a dependency under $lic is refused"; fi
done
for lic in MIT "MIT OR Apache-2.0 OR LGPL-2.1-or-later" MPL-2.0; do
	d=$(workspace "$lic" "allow-${lic// /_}")
	if deny "$d" >/dev/null; then ok "a dependency under $lic is allowed"; else bad "a dependency under $lic is allowed"; fi
done

exit $fail

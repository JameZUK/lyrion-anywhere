#!/bin/sh
# disk-gc test: the artefact sweep is found through cargo's own subcommand list, not the
# shell's PATH. A timer's PATH has no ~/.cargo/bin, so a `command -v cargo-sweep` check once
# skipped the sweep on every scheduled run while reporting the tool missing. Runs
# tools/disk-gc.sh in --dry-run mode with a fake `cargo`, a throwaway target directory and
# an empty temp folder, so nothing is removed.
#
#   tools/test-disk-gc.sh
set -eu
cd "$(dirname "$0")/.."
ROOT=$(pwd -P)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
PASS=0 FAIL=0
ok()   { PASS=$((PASS + 1)); echo "ok   - $1"; }
fail() { FAIL=$((FAIL + 1)); echo "FAIL - $1"; }

mkdir -p "$WORK/bin" "$WORK/target" "$WORK/tmp"
# A fake cargo: `cargo --list` prints the lines given; any other call is an error.
fake_cargo() {
	printf 'Installed Commands:\n%s\n' "$1" >"$WORK/list"
	cat >"$WORK/bin/cargo" <<SH
#!/bin/sh
[ "\$1" = "--list" ] || { echo "fake cargo: unexpected call: \$*" >&2; exit 9; }
cat "$WORK/list"
SH
	chmod +x "$WORK/bin/cargo"
}
# gc <target dir> [disk-gc options]: a dry run; its output lands in $out.
gc() {
	target=$1; shift
	if out=$(PATH="$WORK/bin:$PATH" CARGO_TARGET_DIR="$target" TMPDIR="$WORK/tmp" tools/disk-gc.sh --dry-run "$@" 2>&1); then
		return 0
	fi
	rc=$?
	echo "$out"
	if [ "$rc" = 3 ]; then
		echo "a cargo, gradle or docker build is running here; run this test again later" >&2
		exit 3
	fi
	fail "disk-gc.sh --dry-run exited $rc"
}
has() { case "$out" in *"$1"*) return 0;; esac; return 1; }

echo "# sweep listed by cargo"
fake_cargo '    add                  Add dependencies to a Cargo.toml manifest file
    sweep                Clean cargo target directories
    tree                 Display a tree visualization of a dependency graph'
gc "$WORK/target"
if has "would: cargo sweep --time 3 -r $ROOT"; then ok "sweep found through cargo --list; would sweep artefacts unused for 3 days"; else fail "sweep not run: $out"; fi
if has "not installed"; then fail "reported cargo-sweep missing although cargo lists it"; else ok "no missing-tool message"; fi
gc "$WORK/target" --days 7
if has "would: cargo sweep --time 7 -r $ROOT"; then ok "--days reaches the sweep"; else fail "--days 7 not passed on: $out"; fi

echo "# sweep not among cargo's subcommands (look-alike names present)"
fake_cargo '    sweeper              Not the tool
    unsweep              Nor this
    sweep-all            Nor this'
gc "$WORK/target"
if has "cargo-sweep not installed; skipping artefact sweep"; then ok "missing tool reported"; else fail "no missing-tool message: $out"; fi
if has "would: cargo sweep"; then fail "swept without the tool"; else ok "no sweep without the tool"; fi

echo "# no target directory"
fake_cargo '    sweep                Clean cargo target directories'
gc "$WORK/absent"
if has "cargo sweep" || has "not installed"; then fail "sweep step ran without a target directory: $out"; else ok "sweep step skipped without a target directory"; fi

echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

#!/bin/sh
# disk-gc test: the artefact sweep is found through cargo's own subcommand list, not the
# shell's PATH. A timer's PATH has no ~/.cargo/bin, so a `command -v cargo-sweep` check once
# skipped the sweep on every scheduled run while reporting the tool missing. Also: the run
# says that the Docker prune is host-wide (every project's), --no-docker skips it, a dry run
# shows the Docker commands instead of swallowing them, and the shared build lock in
# LYRION_BUILD_LOCK, which the header promises, is honoured (review 2026-11-12). Runs
# tools/disk-gc.sh in --dry-run mode with a fake `cargo` and `docker`, a throwaway target
# directory and an empty temp folder, so nothing is removed.
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
# A fake docker: `docker info` succeeds (or fails, with DOCKER_DOWN=1); every other call is
# logged, so a dry run can be shown to prune nothing.
cat >"$WORK/bin/docker" <<SH
#!/bin/sh
if [ "\$1" = info ]; then [ -z "\${DOCKER_DOWN:-}" ]; exit; fi
echo "\$*" >>"$WORK/docker.log"
SH
chmod +x "$WORK/bin/docker"
# A fake pgrep, so builds running on this machine don't stop the test: it finds a build
# only with FAKE_BUILD set to the pattern disk-gc asks about (cargo, rustc, ...).
cat >"$WORK/bin/pgrep" <<'SH'
#!/bin/sh
for a; do last=$a; done
[ -n "${FAKE_BUILD:-}" ] && [ "$last" = "$FAKE_BUILD" ]
SH
chmod +x "$WORK/bin/pgrep"
# gc <target dir> [disk-gc options]: a dry run; its output lands in $out, its status in $rc.
gc() {
	target=$1; shift
	rc=0
	out=$(PATH="$WORK/bin:$PATH" CARGO_TARGET_DIR="$target" TMPDIR="$WORK/tmp" tools/disk-gc.sh --dry-run "$@" 2>&1) || rc=$?
	# 4 is "done, but the disk is nearly full": a fact about this machine, not a failure
	# of the script, so the test passes on a full build host too.
	[ "$rc" = 0 ] || [ "$rc" = 4 ] && return 0
	echo "$out"
	fail "disk-gc.sh --dry-run $* exited $rc"
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

echo "# Docker: host-wide, said so, shown in a dry run, skipped with --no-docker"
rm -f "$WORK/docker.log"
gc "$WORK/target"
if has "pruning Docker for the whole host (every project, not only this one)"; then ok "the run says the Docker prune is host-wide"; else fail "no host-wide notice: $out"; fi
if has "would: docker builder prune -f --filter until=72h"; then ok "dry run shows the build cache prune (unused for 3 days)"; else fail "dry run hid the build cache prune: $out"; fi
if has "would: docker image prune -f"; then ok "dry run shows the dangling image prune"; else fail "dry run hid the image prune: $out"; fi
if [ -s "$WORK/docker.log" ]; then fail "a dry run called docker: $(cat "$WORK/docker.log")"; else ok "a dry run prunes nothing"; fi
gc "$WORK/target" --days 7
if has "would: docker builder prune -f --filter until=168h"; then ok "--days reaches the Docker prune"; else fail "--days 7 not passed to docker: $out"; fi
gc "$WORK/target" --no-docker
if has "would: docker"; then fail "--no-docker still pruned Docker: $out"; else ok "--no-docker leaves Docker alone"; fi
if has "Docker left alone (--no-docker)"; then ok "and says so"; else fail "--no-docker not reported: $out"; fi
if has "would: cargo sweep"; then ok "--no-docker leaves the other steps"; else fail "--no-docker skipped the sweep: $out"; fi
out=$(DOCKER_DOWN=1 PATH="$WORK/bin:$PATH" CARGO_TARGET_DIR="$WORK/target" TMPDIR="$WORK/tmp" tools/disk-gc.sh --dry-run 2>&1) || true
if has "docker" || has "Docker"; then fail "Docker pruned or mentioned while it isn't running: $out"; else ok "Docker not running: left alone"; fi
if tools/disk-gc.sh --help | grep -q "every"; then ok "--help says the Docker prune is every project's"; else fail "--help does not say the prune is host-wide"; fi
if tools/disk-gc.sh --help | grep -q -- "--no-docker"; then ok "--help lists --no-docker"; else fail "--help does not list --no-docker"; fi

echo "# a build running: not now"
rc=0
out=$(FAKE_BUILD=cargo PATH="$WORK/bin:$PATH" CARGO_TARGET_DIR="$WORK/target" TMPDIR="$WORK/tmp" tools/disk-gc.sh --dry-run 2>&1) || rc=$?
if [ "$rc" = 3 ] && has "a build is running; not now" && ! has "would:"; then ok "a cargo build running: nothing done (exit 3)"; else fail "ran during a build (exit $rc): $out"; fi

echo "# the shared build lock"
if command -v flock >/dev/null 2>&1; then
	lock="$WORK/build.lock"
	: >"$lock"
	LYRION_BUILD_LOCK=$lock
	export LYRION_BUILD_LOCK
	gc "$WORK/target"
	if has "would: cargo sweep"; then ok "the lock free: the run goes ahead"; else fail "the run did not go ahead with the lock free: $out"; fi
	rc=0
	out=$(flock "$lock" env PATH="$WORK/bin:$PATH" CARGO_TARGET_DIR="$WORK/target" TMPDIR="$WORK/tmp" tools/disk-gc.sh --dry-run 2>&1) || rc=$?
	unset LYRION_BUILD_LOCK
	if [ "$rc" = 3 ] && has "build lock ($lock) is held"; then ok "the lock held: not now (exit 3)"; else fail "ran although the lock was held (exit $rc): $out"; fi
	if has "would:"; then fail "did something while the lock was held: $out"; else ok "and did nothing"; fi
else
	echo "skip - flock not installed"
fi

echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

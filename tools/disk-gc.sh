#!/usr/bin/env bash
# Reclaim build space for this checkout without touching sources or current artefacts.
#
#   tools/disk-gc.sh [--days N] [--dry-run] [--no-docker]
#
# Removes, under this repository:
#   - Rust build artefacts in target/ not used for N days (default 3), with cargo-sweep;
#   - incremental compilation caches older than N days (they are only a speed-up);
#   - target/ directories left inside agent worktrees (.claude/worktrees/*/target), which
#     must build into the shared target/ instead;
#   - Android module build/ output older than N days;
# and, on the host:
#   - Docker's build cache unused for N days and every dangling image. Docker keeps one
#     cache for the whole host and can't tell which project made what, so this is every
#     project's, not only this one's (the run says so); --no-docker leaves Docker alone;
#   - leftovers in the system temp folder older than N days: this project's test scratch
#     directories (anywhere-*), Robolectric's extracted runtimes, abandoned Playwright
#     downloads, and Vitest module caches made from this checkout (nothing else there).
#
# It never runs while a cargo, gradle or docker build is in progress, and it takes the
# shared build lock if one is given in LYRION_BUILD_LOCK. A daily timer on the dev machine
# runs it; run it by hand when `df` looks tight. Nothing here is specific to any install.
set -euo pipefail

days=3
dry=
docker_too=1
while [ $# -gt 0 ]; do
	case $1 in
		--days) days=${2:?}; shift 2 ;;
		--dry-run) dry=1; shift ;;
		--no-docker) docker_too=; shift ;;
		-h|--help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
		*) echo "unknown option: $1" >&2; exit 2 ;;
	esac
done
case $days in ''|*[!0-9]*) echo "--days wants a number" >&2; exit 2 ;; esac

cd "$(dirname "$0")/.."
root=$(pwd -P)
target=${CARGO_TARGET_DIR:-$root/target}

run() { if [ -n "$dry" ]; then echo "would: $*"; else "$@"; fi; }

# A build in progress must never lose files from under it. Gradle's daemon and Kotlin's
# compile daemon outlive a build, so only an active Gradle client counts. The `[G]` form
# keeps pgrep from matching this script's own command line.
if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null \
	|| pgrep -f '[G]radleWrapperMain' >/dev/null || pgrep -f '[d]ocker build' >/dev/null; then
	echo "disk-gc: a build is running; not now" >&2
	exit 3
fi
# The shared build lock, when there is one: held for the whole run, so a build that takes it
# waits for this (and this never starts while a build holds it).
if [ -n "${LYRION_BUILD_LOCK:-}" ]; then
	command -v flock >/dev/null 2>&1 || { echo "disk-gc: LYRION_BUILD_LOCK is set but flock is not installed" >&2; exit 2; }
	exec 9>>"$LYRION_BUILD_LOCK"
	flock -n 9 || { echo "disk-gc: the build lock ($LYRION_BUILD_LOCK) is held; not now" >&2; exit 3; }
fi

before=$(df -P "$root" | awk 'NR==2{print $4}')
echo "disk-gc: $(df -h "$root" | awk 'NR==2{print $4" free of "$2}') before"

# 1. Stale Rust artefacts (cargo-sweep: `cargo install --locked cargo-sweep`). Cargo finds its
#    subcommands in $CARGO_HOME/bin as well as on PATH, so ask cargo whether sweep is there
#    rather than the shell: a timer's PATH has no ~/.cargo/bin, and the shell test made every
#    timer run skip the sweep while reporting the tool missing.
if [ -d "$target" ]; then
	if cargo --list 2>/dev/null | grep -qE '^[[:space:]]+sweep([[:space:]]|$)'; then
		run cargo sweep --time "$days" -r "$root" || echo "disk-gc: cargo sweep failed" >&2
	else
		echo "disk-gc: cargo-sweep not installed; skipping artefact sweep" >&2
	fi
fi

# 2. Incremental caches: large, and only ever a speed-up.
if [ -d "$target" ]; then
	find "$target" -maxdepth 2 -type d -name incremental -print0 2>/dev/null \
		| while IFS= read -r -d '' d; do
			find "$d" -mindepth 1 -maxdepth 1 -type d -mtime +"$days" -print0 \
				| while IFS= read -r -d '' s; do run rm -rf "$s"; done
		done
fi

# 3. Worktree target dirs: agents must build into the shared target/.
for d in "$root"/.claude/worktrees/*/target; do
	[ -d "$d" ] && run rm -rf "$d"
done

# 4. Android module output older than N days (gradle recreates it).
for d in "$root"/apps/android/*/build; do
	[ -d "$d" ] && [ -n "$(find "$d" -maxdepth 0 -mtime +"$days" 2>/dev/null)" ] && run rm -rf "$d"
done

# 5. Docker's build cache and dangling images: the whole host's, every project's (Docker
#    can't tell which project made what), so the run says so, and --no-docker skips it. A
#    dry run prints the commands; a real one keeps their output out of the log.
if [ -z "$docker_too" ]; then
	echo "disk-gc: Docker left alone (--no-docker)"
elif command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
	echo "disk-gc: pruning Docker for the whole host (every project, not only this one): build cache unused for $days days, and dangling images; --no-docker skips this"
	if [ -n "$dry" ]; then
		run docker builder prune -f --filter "until=$((days * 24))h"
		run docker image prune -f
	else
		docker builder prune -f --filter "until=$((days * 24))h" >/dev/null 2>&1 || echo "disk-gc: docker builder prune failed" >&2
		docker image prune -f >/dev/null 2>&1 || echo "disk-gc: docker image prune failed" >&2
	fi
fi

# 6. The system temp folder. Only names this project makes (its tests' anywhere-* scratch
#    directories), the Android test runtime's and Playwright's leftovers, and Vitest module
#    caches whose contents name this checkout; everything else in there belongs to someone
#    else and is left alone. Permission errors (another user's files) are not fatal.
tmp=${TMPDIR:-/tmp}
for d in "$tmp"/anywhere-* "$tmp"/robolectric-* "$tmp"/playwright-download-*; do
	[ -e "$d" ] || continue
	[ -n "$(find "$d" -maxdepth 0 -mtime +"$days" 2>/dev/null)" ] && { run rm -rf "$d" || true; }
done
for c in "$tmp"/*/client; do
	d=${c%/client}
	[ -d "$c" ] || continue
	[ -n "$(find "$d" -maxdepth 0 -mtime +"$days" 2>/dev/null)" ] || continue
	grep -rqsF -m1 "$root" "$c" 2>/dev/null && { run rm -rf "$d" || true; }
done

after=$(df -P "$root" | awk 'NR==2{print $4}')
echo "disk-gc: $(df -h "$root" | awk 'NR==2{print $4" free of "$2}') after (freed $(( (after - before) / 1024 )) MiB)"
free_gib=$(( after / 1024 / 1024 ))
if [ "$free_gib" -lt 20 ]; then
	echo "disk-gc: WARNING only ${free_gib} GiB free; consider 'cargo clean' in $root" >&2
	exit 4
fi

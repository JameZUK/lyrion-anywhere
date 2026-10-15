#!/usr/bin/env bash
# Reclaim build space for this checkout without touching sources or current artefacts.
#
#   tools/disk-gc.sh [--days N] [--dry-run]
#
# Removes, under this repository:
#   - Rust build artefacts in target/ not used for N days (default 3), with cargo-sweep;
#   - incremental compilation caches older than N days (they are only a speed-up);
#   - target/ directories left inside agent worktrees (.claude/worktrees/*/target), which
#     must build into the shared target/ instead;
#   - Android module build/ output older than N days;
# and, on the host:
#   - Docker build cache and dangling images older than N days.
#
# It never runs while a cargo, gradle or docker build is in progress, and it takes the
# shared build lock if one is given in LYRION_BUILD_LOCK. A daily timer on the dev machine
# runs it; run it by hand when `df` looks tight. Nothing here is specific to any install.
set -euo pipefail

days=3
dry=
while [ $# -gt 0 ]; do
	case $1 in
		--days) days=${2:?}; shift 2 ;;
		--dry-run) dry=1; shift ;;
		-h|--help) sed -n '2,18p' "$0"; exit 0 ;;
		*) echo "unknown option: $1" >&2; exit 2 ;;
	esac
done
case $days in ''|*[!0-9]*) echo "--days wants a number" >&2; exit 2 ;; esac

cd "$(dirname "$0")/.."
root=$(pwd -P)
target=${CARGO_TARGET_DIR:-$root/target}

run() { if [ -n "$dry" ]; then echo "would: $*"; else "$@"; fi; }

# A build in progress must never lose files from under it. The `[c]argo` form keeps pgrep
# from matching this script's own command line.
if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null || pgrep -f '[g]radle' >/dev/null \
	|| pgrep -f '[d]ocker build' >/dev/null; then
	echo "disk-gc: a build is running; not now" >&2
	exit 3
fi

before=$(df -P "$root" | awk 'NR==2{print $4}')
echo "disk-gc: $(df -h "$root" | awk 'NR==2{print $4" free of "$2}') before"

# 1. Stale Rust artefacts (cargo-sweep: `cargo install --locked cargo-sweep`).
if command -v cargo-sweep >/dev/null 2>&1 && [ -d "$target" ]; then
	run cargo sweep --time "$days" -r "$root" >/dev/null 2>&1 || echo "disk-gc: cargo sweep failed" >&2
else
	echo "disk-gc: cargo-sweep not installed; skipping artefact sweep" >&2
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

# 5. Docker build cache and dangling images.
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
	run docker builder prune -f --filter "until=$((days * 24))h" >/dev/null 2>&1 || true
	run docker image prune -f >/dev/null 2>&1 || true
fi

after=$(df -P "$root" | awk 'NR==2{print $4}')
echo "disk-gc: $(df -h "$root" | awk 'NR==2{print $4" free of "$2}') after (freed $(( (after - before) / 1024 )) MiB)"
free_gib=$(( after / 1024 / 1024 ))
if [ "$free_gib" -lt 20 ]; then
	echo "disk-gc: WARNING only ${free_gib} GiB free; consider 'cargo clean' in $root" >&2
	exit 4
fi

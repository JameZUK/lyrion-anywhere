#!/usr/bin/env bash
# Wait for a GitHub Actions run to finish, and fail unless it succeeded.
#
#   tools/release/ci-wait.sh run <run-id>
#   tools/release/ci-wait.sh branch <branch> [<commit>]   # the branch's newest run (of that commit)
#   tools/release/ci-wait.sh tag <tag>                    # the run a tag push started
#
# Polls every CI_WAIT_POLL seconds (default 45). A run that does not exist yet (the push
# was a moment ago) is waited for up to CI_WAIT_APPEAR seconds (default 600). Uses the gh
# CLI in the current repository. Notes: `gh run list --commit <short sha>` returns nothing,
# so commits are matched by their full sha; a tag's runs share the commit's sha with main's,
# so a tag is found by its head branch. On failure the failed jobs are listed, with their
# ids for `gh api repos/<owner>/<repo>/actions/jobs/<id>/logs` (completed jobs only).
# A hung or flaky tag run: `gh run rerun <run-id> --failed` once it has completed, never a
# new tag (docs/RELEASING.md).
set -euo pipefail

poll=${CI_WAIT_POLL:-45}
appear=${CI_WAIT_APPEAR:-600}
mode=${1:-}
what=${2:-}
commit=${3:-}
if [ -z "$mode" ] || [ -z "$what" ]; then
	sed -n '2,8p' "$0" >&2
	exit 2
fi

find_run() {
	case "$mode" in
	run) echo "$what" ;;
	branch)
		if [ -n "$commit" ]; then
			local full
			full=$(git rev-parse --verify "${commit}^{commit}" 2>/dev/null || echo "$commit")
			gh run list --branch "$what" --limit 40 --json databaseId,headSha,createdAt \
				--jq "[.[] | select(.headSha == \"$full\")] | sort_by(.createdAt) | last | .databaseId // empty"
		else
			gh run list --branch "$what" --limit 1 --json databaseId --jq '.[0].databaseId // empty'
		fi
		;;
	tag)
		gh run list --limit 80 --json databaseId,headBranch,createdAt \
			--jq "[.[] | select(.headBranch == \"$what\")] | sort_by(.createdAt) | last | .databaseId // empty"
		;;
	*)
		echo "unknown mode: $mode (run | branch | tag)" >&2
		exit 2
		;;
	esac
}

stamp() { date -u +%H:%M:%S; }

id=""
waited=0
while [ -z "$id" ]; do
	id=$(find_run)
	if [ -z "$id" ]; then
		if [ "$waited" -ge "$appear" ]; then
			echo "$(stamp) no run found for $mode $what $commit after ${appear}s" >&2
			exit 1
		fi
		echo "$(stamp) no run yet for $mode $what $commit; waiting"
		sleep 30
		waited=$((waited + 30))
	fi
done

url=$(gh run view "$id" --json url --jq .url)
echo "$(stamp) waiting for run $id ($mode $what): $url"
start=$(date +%s)
while :; do
	status=$(gh run view "$id" --json status,conclusion --jq '"\(.status) \(.conclusion // "")"')
	st=${status%% *}
	concl=${status#* }
	if [ "$st" = "completed" ]; then
		mins=$(( ($(date +%s) - start) / 60 ))
		if [ "$concl" = "success" ]; then
			echo "$(stamp) run $id: success (after ${mins} min here)"
			exit 0
		fi
		echo "$(stamp) run $id: $concl (after ${mins} min here)" >&2
		gh run view "$id" --json jobs \
			--jq '.jobs[] | select(.conclusion != null and .conclusion != "success" and .conclusion != "skipped") | "  \(.name): \(.conclusion) (job \(.databaseId))"' >&2 || true
		exit 1
	fi
	echo "$(stamp) run $id: $st"
	sleep "$poll"
done

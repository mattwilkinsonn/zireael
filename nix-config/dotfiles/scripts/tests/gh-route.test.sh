#!/usr/bin/env bash
# Unit tests for gh-route — the GitHub API bucket router (SEA-1083).
#
# Pure bash — no bats. Each case runs the real gh-route as a subprocess
# with a fake `gh` first on PATH and a pre-seeded GH_ROUTE_CACHE, so the
# router never touches the network. Run directly:
#   bash nix-config/dotfiles/scripts/tests/gh-route.test.sh
#
# What is defended here:
#   - pick: routes to the bucket with the higher remaining/limit fraction;
#     ties and a REST lead resolve to REST.
#   - remaining: reports the right bucket's count.
#   - shape parity: reviews/comments/review-comments emit the identical
#     normalized REST JSON whether served by REST or GraphQL, including the
#     GraphQL-only `[bot]` login suffix for Bot authors.
#   - back-off: both buckets below FLOOR emits the floor warning and returns
#     within the capped wait; headroom present returns immediately, no warning.
#   - unknown command exits 2.
# shellcheck shell=bash

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${GH_ROUTE:-$SCRIPT_DIR/../gh-route}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIN="$WORK/bin"
CACHE="$WORK/cache.json"
mkdir -p "$BIN"

export PATH="$BIN:$PATH"
export GH_ROUTE_CACHE="$CACHE"

# Fake `gh`: only ever invoked as `gh api …` (callers always pass owner/repo,
# so `_repo` never shells out). Dispatches on `graphql` vs a `repos/…` REST
# path and, for `rate_limit`, returns a drained snapshot. When the router
# passes `--jq PROG` (the GraphQL branch), the fake applies it exactly as real
# gh would; the REST branch pipes to an external jq, so the fake emits raw JSON.
# REST and GraphQL fixtures carry the SAME logical rows in their native shapes
# (one human, one Bot) so the router's normalization must converge them.
cat >"$BIN/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = api ] || { echo "fake gh: unexpected invocation: $*" >&2; exit 1; }
shift

is_graphql=0 is_rl=0 restpath="" jqprog="" query="" prev=""
for a in "$@"; do
	case "$a" in
	graphql) is_graphql=1 ;;
	rate_limit) is_rl=1 ;;
	repos/*) restpath="$a" ;;
	query=*) query="${a#query=}" ;;
	esac
	[ "$prev" = "--jq" ] && jqprog="$a"
	prev="$a"
done

REVIEWS_REST='[{"user":{"login":"alice"},"state":"APPROVED","submitted_at":"2024-01-01T00:00:00Z","commit_id":"aaa111","body":"lgtm"},{"user":{"login":"seal-bot[bot]"},"state":"COMMENTED","submitted_at":"2024-01-02T00:00:00Z","commit_id":"bbb222","body":"nit: rename"}]'
REVIEWS_GQL='{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[{"author":{"login":"alice","__typename":"User"},"state":"APPROVED","submittedAt":"2024-01-01T00:00:00Z","commit":{"oid":"aaa111"},"body":"lgtm"},{"author":{"login":"seal-bot","__typename":"Bot"},"state":"COMMENTED","submittedAt":"2024-01-02T00:00:00Z","commit":{"oid":"bbb222"},"body":"nit: rename"}]}}}}}'
COMMENTS_REST='[{"user":{"login":"alice"},"body":"hi","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T01:00:00Z"},{"user":{"login":"seal-bot[bot]"},"body":"CI passed","created_at":"2024-01-02T00:00:00Z","updated_at":"2024-01-02T00:00:00Z"}]'
COMMENTS_GQL='{"data":{"repository":{"pullRequest":{"comments":{"nodes":[{"author":{"login":"alice","__typename":"User"},"body":"hi","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T01:00:00Z"},{"author":{"login":"seal-bot","__typename":"Bot"},"body":"CI passed","createdAt":"2024-01-02T00:00:00Z","updatedAt":"2024-01-02T00:00:00Z"}]}}}}}'
RC_REST='[{"user":{"login":"alice"},"body":"style","path":"src/a.ts","line":10,"commit_id":"ccc333"},{"user":{"login":"seal-bot[bot]"},"body":"unused var","path":"src/b.ts","line":20,"commit_id":"ddd444"}]'
RC_GQL='{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[{"comments":{"nodes":[{"author":{"login":"alice","__typename":"User"},"body":"style","path":"src/a.ts","line":10,"commit":{"oid":"ccc333"},"originalCommit":{"oid":"zzz000"}},{"author":{"login":"seal-bot","__typename":"Bot"},"body":"unused var","path":"src/b.ts","line":20,"commit":{"oid":"ddd444"},"originalCommit":{"oid":"zzz000"}}]}}]}}}}}'
HEAD_SHA_REST='{"head":{"sha":"c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00"},"number":1}'
HEAD_SHA_GQL='{"data":{"repository":{"pullRequest":{"headRefOid":"c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00"}}}}'
CHECK_RUNS_REST='{"check_runs":[{"name":"build","status":"completed","conclusion":"success"},{"name":"lint","status":"queued","conclusion":null}]}'
CHECK_RUNS_GQL='{"data":{"repository":{"object":{"checkSuites":{"nodes":[{"checkRuns":{"nodes":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]}},{"checkRuns":{"nodes":[{"name":"lint","status":"QUEUED","conclusion":null}]}}]}}}}}'
PR_LIST_REST='[{"number":7,"title":"Add feature","state":"open","head":{"sha":"abc123"},"user":{"login":"alice"}},{"number":8,"title":"Fix bug","state":"open","head":{"sha":"def456"},"user":{"login":"dependabot[bot]"}}]'
PR_LIST_GQL='{"data":{"repository":{"pullRequests":{"nodes":[{"number":7,"title":"Add feature","state":"OPEN","headRefOid":"abc123","author":{"login":"alice","__typename":"User"}},{"number":8,"title":"Fix bug","state":"OPEN","headRefOid":"def456","author":{"login":"dependabot","__typename":"Bot"}}]}}}}'
RL_DRAINED='{"resources":{"core":{"remaining":5,"limit":5000,"reset":9999999999},"graphql":{"remaining":5,"limit":5000,"reset":9999999999}}}'

emit() { if [ -n "$jqprog" ]; then jq -r "$jqprog"; else cat; fi; }

if [ "$is_rl" = 1 ]; then
	printf '%s' "$RL_DRAINED"
	exit 0
fi

if [ "$is_graphql" = 1 ]; then
	case "$query" in
	*reviewThreads*) printf '%s' "$RC_GQL" | emit ;;
	*reviews*) printf '%s' "$REVIEWS_GQL" | emit ;;
	*comments*) printf '%s' "$COMMENTS_GQL" | emit ;;
	*checkSuites*) printf '%s' "$CHECK_RUNS_GQL" | emit ;;
	*states:OPEN*) printf '%s' "$PR_LIST_GQL" | emit ;;
	*headRefOid*) printf '%s' "$HEAD_SHA_GQL" | emit ;;
	*) printf '{}' | emit ;;
	esac
	exit 0
fi

case "$restpath" in
*/issues/*/comments) printf '%s' "$COMMENTS_REST" | emit ;;
*/pulls/*/reviews) printf '%s' "$REVIEWS_REST" | emit ;;
*/pulls/*/comments) printf '%s' "$RC_REST" | emit ;;
*/commits/*/check-runs) printf '%s' "$CHECK_RUNS_REST" | emit ;;
*state=open*) printf '%s' "$PR_LIST_REST" | emit ;;
*/pulls/*) printf '%s' "$HEAD_SHA_REST" | emit ;;
*) printf '[]' | emit ;;
esac
GH
chmod +x "$BIN/gh"

fail=0
pass=0

# Write a fresh rate_limit cache (fetched now, so the router reads it and never
# calls `gh api rate_limit`). Args: core_rem core_lim gql_rem gql_lim [reset].
seed_cache() {
	local cr="$1" cl="$2" gr="$3" gl="$4" reset="${5:-}" now
	now="$(date +%s)"
	[ -n "$reset" ] || reset="$((now + 3600))"
	# shellcheck disable=SC2016
	jq -n \
		--argjson cr "$cr" --argjson cl "$cl" \
		--argjson gr "$gr" --argjson gl "$gl" \
		--argjson reset "$reset" --argjson t "$now" \
		'{_fetched_at: $t, resources: {core: {remaining: $cr, limit: $cl, reset: $reset}, graphql: {remaining: $gr, limit: $gl, reset: $reset}}}' \
		>"$CACHE"
}

# Run a gh-route read forced onto a bucket by seeding the cache so that bucket
# wins pick AND clears FLOOR (so _await_headroom returns at once). $1=rest|gql.
run_route() {
	local mode="$1"
	shift
	case "$mode" in
	rest) seed_cache 4000 5000 300 5000 ;;
	gql) seed_cache 300 5000 4000 5000 ;;
	esac
	bash "$SCRIPT" "$@" 2>/dev/null
}

# Canonicalize JSON on stdin (sorted keys, compact) for shape comparison.
canon() { jq -Sc .; }

check() {
	local desc="$1" got="$2" want="$3"
	if [ "$got" = "$want" ]; then
		pass=$((pass + 1))
		printf '  ok   %s\n' "$desc"
	else
		fail=$((fail + 1))
		printf '  FAIL %s\n     got:  %s\n     want: %s\n' "$desc" "$got" "$want"
	fi
}

check_contains() {
	local desc="$1" got="$2" want="$3"
	case "$got" in
	*"$want"*)
		pass=$((pass + 1))
		printf '  ok   %s\n' "$desc"
		;;
	*)
		fail=$((fail + 1))
		printf '  FAIL %s\n     got:      %s\n     want sub: %s\n' "$desc" "$got" "$want"
		;;
	esac
}

echo "pick (fractional-headroom routing):"

seed_cache 100 5000 4000 5000
check "graphql-dominant fraction → graphql" "$(bash "$SCRIPT" pick)" "graphql"

seed_cache 4000 5000 100 5000
check "rest-dominant fraction → rest" "$(bash "$SCRIPT" pick)" "rest"

seed_cache 2500 5000 2500 5000
check "equal fraction → rest (tie to idle bucket)" "$(bash "$SCRIPT" pick)" "rest"

seed_cache 500 5000 0 5000
check "graphql exhausted → rest" "$(bash "$SCRIPT" pick)" "rest"

# Floor guard (hardening): a bucket at/below FLOOR (200) must not be chosen when
# the other still has room, even if the near-empty bucket has a momentarily
# HIGHER remaining/limit fraction — else the very next call 403s. The guard runs
# BEFORE the fraction compare. These two make the near-empty bucket's fraction
# strictly the higher one (0.199 vs 0.06), so the guard is the ONLY thing that
# can produce the pick; the pre-hardening fraction logic would pick the opposite.
seed_cache 300 5000 199 1000
check "graphql at/below floor but higher fraction → rest (guard overrides fraction)" "$(bash "$SCRIPT" pick)" "rest"

seed_cache 199 1000 300 5000
check "rest at/below floor but higher fraction → graphql (guard overrides fraction)" "$(bash "$SCRIPT" pick)" "graphql"

# Boundary of the ≤ guard: remaining exactly AT the floor is guarded (a strict `<`
# would wrongly let 200 fall through to the fraction compare and pick the
# near-empty bucket); one request above the floor is healthy and competes on
# fraction as normal — the adjacent 200/201 pair straddles the boundary with
# opposite outcomes, so an off-by-one in the guard reddens exactly one of them.
seed_cache 300 5000 200 1000
check "graphql exactly at floor → rest (≤ boundary is guarded)" "$(bash "$SCRIPT" pick)" "rest"

seed_cache 300 5000 201 1000
check "graphql one above floor → graphql (healthy, wins on fraction)" "$(bash "$SCRIPT" pick)" "graphql"

# Fraction compare is reached only once BOTH buckets clear the floor: a strictly
# higher fraction wins (graphql here, 0.80 vs 0.60). The tie and rest-lead paths
# through this branch already live above.
seed_cache 3000 5000 4000 5000
check "both healthy, graphql higher fraction → graphql" "$(bash "$SCRIPT" pick)" "graphql"

echo
echo "remaining:"

seed_cache 4321 5000 1234 5000
check "remaining rest → core count" "$(bash "$SCRIPT" remaining rest)" "4321"
check "remaining graphql → graphql count" "$(bash "$SCRIPT" remaining graphql)" "1234"

echo
echo "shape parity (REST branch == GraphQL branch, [bot] normalized):"

EXP_REVIEWS='[{"user":{"login":"alice"},"state":"APPROVED","submitted_at":"2024-01-01T00:00:00Z","commit_id":"aaa111","body":"lgtm"},{"user":{"login":"seal-bot[bot]"},"state":"COMMENTED","submitted_at":"2024-01-02T00:00:00Z","commit_id":"bbb222","body":"nit: rename"}]'
EXP_COMMENTS='[{"user":{"login":"alice"},"body":"hi","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T01:00:00Z"},{"user":{"login":"seal-bot[bot]"},"body":"CI passed","created_at":"2024-01-02T00:00:00Z","updated_at":"2024-01-02T00:00:00Z"}]'
EXP_RC='[{"user":{"login":"alice"},"body":"style","path":"src/a.ts","line":10,"commit_id":"ccc333"},{"user":{"login":"seal-bot[bot]"},"body":"unused var","path":"src/b.ts","line":20,"commit_id":"ddd444"}]'

check "reviews via REST → normalized shape" \
	"$(run_route rest reviews 1 o/r | canon)" \
	"$(printf '%s' "$EXP_REVIEWS" | canon)"
check "reviews via GraphQL → same shape ([bot] appended)" \
	"$(run_route gql reviews 1 o/r | canon)" \
	"$(printf '%s' "$EXP_REVIEWS" | canon)"

check "comments via REST → normalized shape" \
	"$(run_route rest comments 1 o/r | canon)" \
	"$(printf '%s' "$EXP_COMMENTS" | canon)"
check "comments via GraphQL → same shape ([bot] appended)" \
	"$(run_route gql comments 1 o/r | canon)" \
	"$(printf '%s' "$EXP_COMMENTS" | canon)"

check "review-comments via REST → normalized shape" \
	"$(run_route rest review-comments 1 o/r | canon)" \
	"$(printf '%s' "$EXP_RC" | canon)"
check "review-comments via GraphQL → same shape ([bot] appended)" \
	"$(run_route gql review-comments 1 o/r | canon)" \
	"$(printf '%s' "$EXP_RC" | canon)"

echo
echo "shape parity — head-sha / check-runs / pr-list (REST branch == GraphQL branch):"

# head-sha: REST reads .head.sha off the pull object; GraphQL reads headRefOid.
# Both --jq to a bare SHA string, so the two routes must yield the identical SHA.
EXP_HEAD_SHA='c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00'
check "head-sha via REST → bare head SHA" \
	"$(run_route rest head-sha 1 o/r)" "$EXP_HEAD_SHA"
check "head-sha via GraphQL → same SHA (headRefOid)" \
	"$(run_route gql head-sha 1 o/r)" "$EXP_HEAD_SHA"

# check-runs: REST relies on the API already lowercasing status/conclusion and
# flattens check_runs across pages; GraphQL applies explicit ascii_downcase and
# flattens across checkSuites. Feed lowercase REST vs UPPERCASE GraphQL carrying
# the same logical runs (build in suite 1, lint in suite 2) and assert both
# normalize to the identical lowercase, flattened shape — the divergence a parity
# test is here to catch. null conclusion must survive as JSON null, not "null".
EXP_CHECK_RUNS='{"check_runs":[{"name":"build","status":"completed","conclusion":"success"},{"name":"lint","status":"queued","conclusion":null}]}'
check "check-runs via REST → normalized lowercase shape" \
	"$(run_route rest check-runs abc o/r | canon)" \
	"$(printf '%s' "$EXP_CHECK_RUNS" | canon)"
check "check-runs via GraphQL → UPPERCASE ascii_downcased to same shape" \
	"$(run_route gql check-runs abc o/r | canon)" \
	"$(printf '%s' "$EXP_CHECK_RUNS" | canon)"

# pr-list: REST state is already lowercase and Bot logins already carry [bot];
# GraphQL downcases state and appends [bot] from __typename. Same logical PRs
# (one human, one Bot author) must converge on the identical normalized array.
EXP_PR_LIST='[{"number":7,"title":"Add feature","state":"open","head":{"sha":"abc123"},"user":{"login":"alice"}},{"number":8,"title":"Fix bug","state":"open","head":{"sha":"def456"},"user":{"login":"dependabot[bot]"}}]'
check "pr-list via REST → normalized shape" \
	"$(run_route rest pr-list o/r | canon)" \
	"$(printf '%s' "$EXP_PR_LIST" | canon)"
check "pr-list via GraphQL → same shape (state downcased, [bot] appended)" \
	"$(run_route gql pr-list o/r | canon)" \
	"$(printf '%s' "$EXP_PR_LIST" | canon)"

echo
echo "back-off (both buckets < FLOOR):"

# Both drained → warns and, capped at MAX_WAIT=1, returns well inside timeout.
seed_cache 5 5000 5 5000 9999999999
drained_err="$WORK/drained.err"
drained_start="$(date +%s)"
GH_ROUTE_MAX_WAIT=1 timeout 5 bash "$SCRIPT" reviews 1 o/r >/dev/null 2>"$drained_err"
drained_rc=$?
drained_elapsed=$(($(date +%s) - drained_start))
check_contains "drained emits the floor warning" "$(cat "$drained_err")" "both buckets < 200"
check "drained returns (not killed by timeout)" \
	"$([ "$drained_rc" -ne 124 ] && echo ok || echo TIMEOUT)" "ok"
check "drained wait stays capped (≤3s)" \
	"$([ "$drained_elapsed" -le 3 ] && echo ok || echo "slow:$drained_elapsed")" "ok"

# Headroom present → no wait, no warning.
seed_cache 4000 5000 4000 5000
headroom_err="$WORK/headroom.err"
headroom_start="$(date +%s)"
GH_ROUTE_MAX_WAIT=1 timeout 5 bash "$SCRIPT" reviews 1 o/r >/dev/null 2>"$headroom_err"
headroom_elapsed=$(($(date +%s) - headroom_start))
headroom_warn=no
case "$(cat "$headroom_err")" in *"both buckets"*) headroom_warn=yes ;; esac
check "headroom emits no floor warning" "$headroom_warn" "no"
check "headroom returns immediately (≤2s)" \
	"$([ "$headroom_elapsed" -le 2 ] && echo ok || echo "slow:$headroom_elapsed")" "ok"

echo
echo "dispatch:"

seed_cache 4000 5000 4000 5000
bash "$SCRIPT" totally-bogus >/dev/null 2>/dev/null
check "unknown command exits 2" "$?" "2"

echo
echo "passed: $pass  failed: $fail"
[ "$fail" -eq 0 ]

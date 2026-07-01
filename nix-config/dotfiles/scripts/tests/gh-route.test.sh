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
RL_DRAINED='{"resources":{"core":{"remaining":5,"limit":5000,"reset":9999999999},"graphql":{"remaining":5,"limit":5000,"reset":9999999999}}}'

emit() { if [ -n "$jqprog" ]; then jq "$jqprog"; else cat; fi; }

if [ "$is_rl" = 1 ]; then
	printf '%s' "$RL_DRAINED"
	exit 0
fi

if [ "$is_graphql" = 1 ]; then
	case "$query" in
	*reviewThreads*) printf '%s' "$RC_GQL" | emit ;;
	*reviews*) printf '%s' "$REVIEWS_GQL" | emit ;;
	*comments*) printf '%s' "$COMMENTS_GQL" | emit ;;
	*) printf '{}' | emit ;;
	esac
	exit 0
fi

case "$restpath" in
*/issues/*/comments) printf '%s' "$COMMENTS_REST" | emit ;;
*/pulls/*/reviews) printf '%s' "$REVIEWS_REST" | emit ;;
*/pulls/*/comments) printf '%s' "$RC_REST" | emit ;;
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

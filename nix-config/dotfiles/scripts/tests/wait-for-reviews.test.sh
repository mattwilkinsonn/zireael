#!/usr/bin/env bash
# Regression tests for wait-for-reviews (tern routing, SEA-1083).
#
# Pure bash — no bats. Two seams are exercised, each in isolation:
#   1. The tern isError/JSON-RPC-error guard — the jq -e filter inside
#      tern_state, which is EXTRACTED live from the script so the test
#      defends the production filter, not a drifting copy. It is run
#      against canned SSE `data:` frames. This is the load-bearing case:
#      the bug was that a tern error (bad repo / GraphQL failure / rate
#      limit) had its error text emitted as if it were review-state JSON,
#      which then crashed the next jq under `set -e`. The fix makes the
#      filter exit NON-zero on a tool-level (result.isError) or JSON-RPC
#      (.error) error, so the caller's `|| true` leaves $state empty and
#      the gh-route fallback runs.
#   2. Arg validation — the real script run as a subprocess (bad args
#      exit before any polling) with fake gh/gh-route/curl on PATH and
#      tiny WAIT_* envs so the accepted-arg cases backstop out in ~1s
#      instead of polling forever. The bug was that an unknown leading-
#      dash flag (e.g. --bogus) was silently taken as the repo name.
#
# Run directly:
#   bash nix-config/dotfiles/scripts/tests/wait-for-reviews.test.sh
# shellcheck shell=bash

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${WAIT_FOR_REVIEWS:-$SCRIPT_DIR/../wait-for-reviews}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIN="$WORK/bin"
mkdir -p "$BIN"

# Fake binaries: the arg-validation cases must never touch the network.
# tern is forced OFF (LITELLM_MCP_URL/KEY unset below), so head-sha and the
# per-poll reviews/comments come from gh-route, all returning quickly.
export PATH="$BIN:$PATH"

# fake gh: `gh repo view …` yields a repo (default-repo path, no network);
# any `gh api …` yields an empty array. Never invoked by the accepted cases
# here (they pass --repo), but present so the default-repo path is safe too.
cat >"$BIN/gh" <<'GH'
#!/usr/bin/env bash
if [ "${1:-}" = repo ] && [ "${2:-}" = view ]; then
	echo "owner/repo"
	exit 0
fi
if [ "${1:-}" = api ]; then
	echo '[]'
	exit 0
fi
echo '[]'
GH
chmod +x "$BIN/gh"

# fake gh-route: head-sha → a fixed SHA; reviews/comments → empty arrays. With
# empty reviews/comments every bot classifies as "pending", which blocks until
# the (tiny) backstop, so the accepted-arg runs exit fast and deterministically.
cat >"$BIN/gh-route" <<'GHR'
#!/usr/bin/env bash
case "${1:-}" in
head-sha) echo "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" ;;
*) echo '[]' ;;
esac
GHR
chmod +x "$BIN/gh-route"

# fake curl: never reached (tern off), stubbed so a stray call can't hit the
# network — emits nothing, exits clean.
cat >"$BIN/curl" <<'CURL'
#!/usr/bin/env bash
exit 0
CURL
chmod +x "$BIN/curl"

# tern OFF so the script resolves head/reviews via gh-route and backstops out.
unset LITELLM_MCP_URL LITELLM_API_KEY
export WAIT_BACKSTOP_SECS=1 WAIT_GRACE_SECS=0 WAIT_POLL_SECS=1

# The exact production filter, lifted from the script so a drift (or the guard
# being dropped) is caught here rather than silently diverging from a copy.
FILTER="$(sed -n "s/.*| jq -e -r '\(.*\)' 2>.*/\1/p" "$SCRIPT")"

fail=0
pass=0

# Run the tern SSE-extraction pipe exactly as tern_state does — strip the
# `data: ` prefix, feed the frame through the extracted jq -e filter — and
# report "<zero|nonzero>|<stdout>". zero/nonzero is the contract the caller
# keys on (`state="$(tern_state || true)"`): nonzero → empty $state → fallback.
run_filter() {
	local resp="$1" out rc
	out="$(sed -n 's/^data: //p' <<<"$resp" | jq -e -r "$FILTER" 2>/dev/null)"
	rc=$?
	if [ "$rc" -eq 0 ]; then
		printf 'zero|%s' "$out"
	else
		printf 'nonzero|%s' "$out"
	fi
}

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

echo "tern isError/error guard (jq -e filter in tern_state):"

# The filter must have been lifted — an empty capture means the script's jq
# line was reformatted and every behavioral case below would misfire; fail loud.
check "filter extracted from the live script (non-empty)" \
	"$([ -n "$FILTER" ] && echo ok || echo EMPTY)" \
	"ok"

# Tool-level error: content text is present but isError:true. Pre-fix this text
# ("github REST 404 …") was emitted as $state and crashed the next jq. The guard
# must exit non-zero and emit NOTHING so $state stays empty → gh-route fallback.
TOOL_ERR='event: message
data: {"result":{"content":[{"text":"github REST 404: pull request not found"}],"isError":true}}'
check "tool isError:true → non-zero exit, empty stdout" \
	"$(run_filter "$TOOL_ERR")" \
	"nonzero|"

# JSON-RPC transport error: no result at all, just .error. Same contract.
RPC_ERR='event: message
data: {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"boom"}}'
check "JSON-RPC .error → non-zero exit, empty stdout" \
	"$(run_filter "$RPC_ERR")" \
	"nonzero|"

# Success: isError:false, content text is the review-state JSON. The filter must
# exit 0 and print exactly that inner text (which the caller then jq-parses for
# .head_sha) — the whole point of the guard is to let this through unchanged.
OK='event: message
data: {"result":{"content":[{"text":"{\"head_sha\":\"abc\"}"}],"isError":false}}'
check "success (isError:false) → exit 0, emits inner head_sha JSON" \
	"$(run_filter "$OK")" \
	'zero|{"head_sha":"abc"}'

echo
echo "arg validation (real script as subprocess, fake gh/gh-route on PATH):"

# Missing PR → usage() → exit 2 with a "usage" message on stderr.
noargs_err="$WORK/noargs.err"
timeout 5 bash "$SCRIPT" >/dev/null 2>"$noargs_err"
noargs_rc=$?
check "no PR arg → exit 2" "$noargs_rc" "2"
check_contains "no PR arg → 'usage' on stderr" "$(cat "$noargs_err")" "usage"

# Unknown leading-dash flag where the repo goes → the regression. Pre-fix this
# was silently taken as the repo name; now it must be rejected: exit 2 with an
# "unknown option" message, not swallowed.
bogus_err="$WORK/bogus.err"
timeout 5 bash "$SCRIPT" 305 --bogus >/dev/null 2>"$bogus_err"
bogus_rc=$?
check "'305 --bogus' → exit 2" "$bogus_rc" "2"
check_contains "'305 --bogus' → 'unknown option' on stderr" "$(cat "$bogus_err")" "unknown option"

# --repo with no following value (space form, end of args) → the value guard:
# reject rather than default-resolve a bogus repo. exit 2 + the specific message.
repo_noval_err="$WORK/repo_noval.err"
timeout 5 bash "$SCRIPT" 305 --repo >/dev/null 2>"$repo_noval_err"
repo_noval_rc=$?
check "'305 --repo' (no value) → exit 2" "$repo_noval_rc" "2"
check_contains "'305 --repo' → '--repo needs an owner/repo value' on stderr" \
	"$(cat "$repo_noval_err")" \
	"--repo needs an owner/repo value"

# --repo followed by a dash flag → must NOT be swallowed as REPO=--bogus; the
# guard rejects a leading-dash value the same as a missing one.
repo_dashval_err="$WORK/repo_dashval.err"
timeout 5 bash "$SCRIPT" 305 --repo --bogus >/dev/null 2>"$repo_dashval_err"
repo_dashval_rc=$?
check "'305 --repo --bogus' (dash value) → exit 2" "$repo_dashval_rc" "2"
check_contains "'305 --repo --bogus' → '--repo needs an owner/repo value' on stderr" \
	"$(cat "$repo_dashval_err")" \
	"--repo needs an owner/repo value"

# --repo= with an empty equals value → same guard on the equals form.
repo_empty_err="$WORK/repo_empty.err"
timeout 5 bash "$SCRIPT" 305 --repo= >/dev/null 2>"$repo_empty_err"
repo_empty_rc=$?
check "'305 --repo=' (empty equals) → exit 2" "$repo_empty_rc" "2"
check_contains "'305 --repo=' → '--repo needs an owner/repo value' on stderr" \
	"$(cat "$repo_empty_err")" \
	"--repo needs an owner/repo value"

# --repo owner/repo (space form) → accepted: NOT an arg exit (rc != 2) and the
# header line prints, proving the args parsed and the loop was entered.
accept1_out="$WORK/accept1.out"
timeout 5 bash "$SCRIPT" 305 --repo owner/repo >"$accept1_out" 2>/dev/null
accept1_rc=$?
check "'305 --repo owner/repo' → not an arg exit (rc != 2)" \
	"$([ "$accept1_rc" -ne 2 ] && echo ok || echo "argexit:$accept1_rc")" \
	"ok"
check_contains "'305 --repo owner/repo' → header prints" \
	"$(cat "$accept1_out")" \
	"wait-for-reviews: owner/repo#305"

# --repo=owner/repo (equals form) → same acceptance.
accept2_out="$WORK/accept2.out"
timeout 5 bash "$SCRIPT" 305 --repo=owner/repo >"$accept2_out" 2>/dev/null
accept2_rc=$?
check "'305 --repo=owner/repo' → not an arg exit (rc != 2)" \
	"$([ "$accept2_rc" -ne 2 ] && echo ok || echo "argexit:$accept2_rc")" \
	"ok"
check_contains "'305 --repo=owner/repo' → header prints" \
	"$(cat "$accept2_out")" \
	"wait-for-reviews: owner/repo#305"

echo
echo "passed: $pass  failed: $fail"
[ "$fail" -eq 0 ]

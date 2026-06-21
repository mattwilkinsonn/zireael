#!/usr/bin/env bash
# Regression tests for the bootstrap arg parsers in bootstrap-common.sh.
#
# Pure bash — no bats dependency. Each case runs the parser in a
# subshell (the parser calls `err`, which `exit`s, so isolation keeps
# one failing case from killing the run). Run directly:
#   bash shared/scripts/tests/bootstrap-args.test.sh
#
# Covers the awsmac regression: `macos-runner-bootstrap.sh awsmac
# ec2-user` used to die with "[err] unknown arg: awsmac" because the
# positional hostname was forwarded to the flag-only
# parse_tailscale_auth_key. parse_mac_runner_args is the single-pass
# replacement that tolerates positionals AND the --auth-key flag.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMMON="$SCRIPT_DIR/../bootstrap-common.sh"

fail=0
pass=0

# Run parse_mac_runner_args in a clean subshell with the given argv and
# print "EXIT|TARGET_HOSTNAME|ADMIN_USER|TAILSCALE_AUTH_KEY". On parser
# error (`err` calls exit) the subshell dies before printing, so the
# empty capture + non-zero status is rendered as the "1|||" marker.
run_parse() {
	local out
	if out=$(
		# shellcheck disable=SC1090
		source "$COMMON"
		parse_mac_runner_args "$@" 2>/dev/null || exit 1
		printf '0|%s|%s|%s' "$TARGET_HOSTNAME" "$ADMIN_USER" "$TAILSCALE_AUTH_KEY"
	); then
		printf '%s' "$out"
	else
		printf '1|||'
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

echo "parse_mac_runner_args:"

# The actual user invocation that regressed.
check "hostname + admin user positionals" \
	"$(run_parse awsmac ec2-user)" \
	"0|awsmac|ec2-user|"

# Hostname only — admin user defaults to mattw.
check "hostname only, admin defaults to mattw" \
	"$(run_parse mattmini)" \
	"0|mattmini|mattw|"

# --auth-key <value> form, after positionals.
check "positionals + --auth-key <value>" \
	"$(run_parse awsmac ec2-user --auth-key tskey-abc)" \
	"0|awsmac|ec2-user|tskey-abc"

# --auth-key=<value> form.
check "positionals + --auth-key=<value>" \
	"$(run_parse awsmac ec2-user --auth-key=tskey-xyz)" \
	"0|awsmac|ec2-user|tskey-xyz"

# Flag before positionals — order independence.
check "--auth-key before positionals" \
	"$(run_parse --auth-key tskey-q awsmac ec2-user)" \
	"0|awsmac|ec2-user|tskey-q"

# A key that doesn't start with tskey- must NOT clobber the admin user.
check "non-tskey --auth-key value doesn't leak into positionals" \
	"$(run_parse awsmac ec2-user --auth-key weirdkey)" \
	"0|awsmac|ec2-user|weirdkey"

# Missing hostname → error exit.
check "missing hostname errors" \
	"$(run_parse)" \
	"1|||"

# Env-var fallback: SEAL_MAC_HOSTNAME / SEAL_MAC_ADMIN_USER, no argv.
check "env-var hostname + admin, no argv" \
	"$(SEAL_MAC_HOSTNAME=awsmac SEAL_MAC_ADMIN_USER=ec2-user run_parse)" \
	"0|awsmac|ec2-user|"

echo
echo "passed: $pass  failed: $fail"
[ "$fail" -eq 0 ]

#!/usr/bin/env bash
# Run nix-switch (or darwin-rebuild switch for Mac) across the personal
# hosts this Mac manages (MBP + WSL). The sealedsecurity fleet (CI
# runners + mattfw) lives in the sealed repo now — use
# `sealed/infra/nix/scripts/nix-switch-all.sh` for those.
# Sequential, not parallel — easier to read output and abort on first
# failure.
#
# Doesn't rely on the per-host `nix-switch` zsh alias (SSH non-interactive
# shells don't expand zsh aliases); hardcodes the rebuild command per host.
#
# Usage:
#   nix-switch-all.sh                       # all hosts
#   nix-switch-all.sh --only mattpc         # one specific host
#   nix-switch-all.sh --except mattpc       # skip one host
#
# Mac is targeted by SSH-host name "mac" (special-cased to run locally).

set -euo pipefail

# SSH-host name → flake target name.
declare -a HOSTS=(
	"mac:Matts-MacBook-Pro"
	"mattpc:mattpc-wsl"
)

ONLY=""
declare -a EXCEPT=()

while [ $# -gt 0 ]; do
	case "$1" in
	--only)
		ONLY="$2"
		shift 2
		;;
	--except)
		EXCEPT+=("$2")
		shift 2
		;;
	-h | --help)
		sed -n '2,16p' "$0" | sed 's/^# //; s/^#//'
		exit 0
		;;
	*)
		echo "unknown arg: $1" >&2
		exit 2
		;;
	esac
done

is_excepted() {
	local host=$1
	for e in "${EXCEPT[@]}"; do
		[ "$e" = "$host" ] && return 0
	done
	return 1
}

declare -a FAILED=()

for entry in "${HOSTS[@]}"; do
	host="${entry%%:*}"
	target="${entry##*:}"
	if [ -n "$ONLY" ] && [ "$ONLY" != "$host" ]; then
		continue
	fi
	if is_excepted "$host"; then
		echo "==> $host (skipped)"
		continue
	fi

	echo ""
	echo "==> $host ($target)"
	if [ "$host" = "mac" ]; then
		# Determinate Nix on macOS requires sudo for darwin-rebuild (writes to
		# /etc, /Library/LaunchDaemons, etc). Match the local `nix-switch` alias
		# in darwin/home.nix which also uses `sudo HOME="$HOME"`.
		if ! sudo HOME="$HOME" darwin-rebuild switch --flake "$HOME/repos/zireael/nix-config#$target" --show-trace; then
			FAILED+=("$host")
		fi
	else
		# shellcheck disable=SC2029  # $target is a flake attr name ([a-z-]+); safe to expand client-side
		if ! ssh "$host" "sudo nixos-rebuild switch --flake \"\$HOME/repos/zireael/nix-config#$target\" --show-trace"; then
			FAILED+=("$host")
		fi
	fi
done

echo ""
if [ "${#FAILED[@]}" -eq 0 ]; then
	echo "✓ All hosts switched successfully."
else
	echo "✗ Failed: ${FAILED[*]}"
	exit 1
fi

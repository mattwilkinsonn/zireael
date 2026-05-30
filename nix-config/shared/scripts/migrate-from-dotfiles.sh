#!/usr/bin/env bash
# Migrate an already-bootstrapped host from the old dotfiles-at-$HOME
# layout to the new ~/repos/zireael + ~/repos/privatefiles split.
#
# Idempotent. Safe to re-run if any step fails mid-migration.
#
# What it does, in order:
#
#   1. Sanity-check the source layout: ~/.git (dotfiles repo) and
#      ~/nix-config (the subdir the old flake reads) must both exist.
#      If they don't, this host has already been migrated — exit
#      successfully with a no-op message.
#   2. Clone (or refresh) zireael into ~/repos/zireael/.
#   3. On dev boxes (--dev or auto-detected by hostname against
#      DEV_HOSTNAMES below), clone privatefiles into
#      ~/repos/privatefiles/. The symlinks back into $HOME
#      (~/.claude/CLAUDE.md, ~/.seal/SEAL.md, etc.) are authored
#      declaratively by home-manager — see
#      shared/privatefiles-symlinks.nix — and laid down by step 4's
#      nix-rebuild, not by this script.
#   4. Rebuild the system against the new flake path. Picks the right
#      build command (darwin-rebuild / nixos-rebuild) by uname.
#   5. After a successful rebuild, retire the dotfiles repo:
#      - Move ~/.git → ~/.git.archived-YYYYMMDD/ (don't delete; let the
#        user delete after a few weeks of confirmed-working state).
#      - Move ~/.jj similarly.
#      - Drop the (now stale) ~/nix-config symlink/dir if it's not
#        already the same path as ~/repos/zireael/nix-config.
#
# Usage:
#   bash <path>/migrate-from-dotfiles.sh             # auto-detect dev vs server
#   bash <path>/migrate-from-dotfiles.sh --dev       # force clone privatefiles
#   bash <path>/migrate-from-dotfiles.sh --no-dev    # skip privatefiles
#   bash <path>/migrate-from-dotfiles.sh --dry-run   # print steps without executing
#
# Env overrides (mostly for testing):
#   ZIREAEL_REPO=mattwilkinsonn/zireael
#   PRIVATEFILES_REPO=mattwilkinsonn/privatefiles
#   ZIREAEL_DIR=$HOME/repos/zireael
#   PRIVATEFILES_DIR=$HOME/repos/privatefiles

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./bootstrap-common.sh
source "$SCRIPT_DIR/bootstrap-common.sh"

ZIREAEL_REPO="${ZIREAEL_REPO:-mattwilkinsonn/zireael}"
PRIVATEFILES_REPO="${PRIVATEFILES_REPO:-mattwilkinsonn/privatefiles}"
ZIREAEL_DIR="${ZIREAEL_DIR:-$HOME/repos/zireael}"
PRIVATEFILES_DIR="${PRIVATEFILES_DIR:-$HOME/repos/privatefiles}"
NIX_CONFIG_DIR="$ZIREAEL_DIR/nix-config"

DEV=auto
DRY_RUN=false
while [ $# -gt 0 ]; do
	case "$1" in
	--dev)
		DEV=yes
		shift
		;;
	--no-dev)
		DEV=no
		shift
		;;
	-n | --dry-run)
		DRY_RUN=true
		shift
		;;
	-h | --help)
		sed -n '1,/^set -euo pipefail/p' "$0" | grep '^#' | sed 's/^# \?//'
		exit 0
		;;
	*) err "unknown flag: $1 (try --help)" ;;
	esac
done

run() {
	if $DRY_RUN; then
		echo "DRY-RUN: $*"
	else
		"$@"
	fi
}

# Hosts that have a sealedsecurity team OP token + the
# `~/repos/privatefiles/` clone. The team token alone isn't a usable
# signal — `mattserver` also has it for env-injecting container
# secrets — so we match on hostname directly.
#
# Keep in sync with the import set in nix-config/flake.nix that pulls
# in shared/privatefiles-symlinks.nix.
DEV_HOSTNAMES=(
	"Matts-MacBook-Pro"
	"mattfw"
	"mattpc-wsl"
)

# Resolve the bare hostname early — both the dev auto-detect AND the
# rebuild step need it, and macOS's `hostname` returns mDNS / Tailscale
# forms that don't match the flake attribute (`Matts-MacBook-Pro`).
# Order:
#   1. HOSTNAME_OVERRIDE (env, escape hatch for testing)
#   2. macOS: `scutil --get LocalHostName` (bare form, original case)
#   3. Linux/fallback: `hostname` with `.local` stripped
if [ -n "${HOSTNAME_OVERRIDE:-}" ]; then
	HOSTNAME="$HOSTNAME_OVERRIDE"
elif [ "$(uname -s)" = "Darwin" ] && command -v scutil >/dev/null 2>&1; then
	HOSTNAME="$(scutil --get LocalHostName)"
else
	HOSTNAME="$(hostname)"
	HOSTNAME="${HOSTNAME%.local}"
fi

# ---- Step 1: sanity check ----
step "Checking current dotfiles layout"
if [ ! -d "$HOME/.git" ] && [ -d "$ZIREAEL_DIR/.git" ]; then
	echo "Looks like this host is already migrated (no ~/.git, $ZIREAEL_DIR present). Exiting."
	exit 0
fi
if [ ! -d "$HOME/.git" ]; then
	err "Expected ~/.git (the dotfiles repo) to exist. Cannot determine current layout."
fi

# Auto-detect dev mode if not forced. Match against DEV_HOSTNAMES;
# everything else is a server host (no privatefiles clone).
if [ "$DEV" = "auto" ]; then
	DEV=no
	for h in "${DEV_HOSTNAMES[@]}"; do
		if [ "$HOSTNAME" = "$h" ]; then
			DEV=yes
			break
		fi
	done
	if [ "$DEV" = "yes" ]; then
		echo "Auto-detected dev host (hostname: $HOSTNAME)."
	else
		echo "Auto-detected server host (hostname: $HOSTNAME, not in DEV_HOSTNAMES)."
	fi
fi

# ---- Step 2: clone zireael ----
step "Setting up $ZIREAEL_DIR"
if [ -d "$ZIREAEL_DIR/.git" ]; then
	echo "$ZIREAEL_DIR already exists — pulling latest"
	run jj -R "$ZIREAEL_DIR" git fetch
	run jj -R "$ZIREAEL_DIR" bookmark set main -r "main@origin"
else
	run mkdir -p "$HOME/repos"
	ensure_gh
	if ! gh auth status &>/dev/null; then
		echo "gh auth required for the zireael clone."
		gh auth login
	fi
	run gh repo clone "$ZIREAEL_REPO" "$ZIREAEL_DIR"
	ensure_jj
	run jj git init --colocate "$ZIREAEL_DIR"
	run jj -R "$ZIREAEL_DIR" bookmark track main --remote=origin
fi

# ---- Step 3: clone privatefiles (dev hosts only) + author symlinks ----
if [ "$DEV" = "yes" ]; then
	step "Setting up $PRIVATEFILES_DIR"
	if [ -d "$PRIVATEFILES_DIR/.git" ]; then
		echo "$PRIVATEFILES_DIR already exists — pulling latest"
		run jj -R "$PRIVATEFILES_DIR" git fetch
		run jj -R "$PRIVATEFILES_DIR" bookmark set main -r "main@origin"
	else
		run mkdir -p "$HOME/repos"
		run gh repo clone "$PRIVATEFILES_REPO" "$PRIVATEFILES_DIR"
		ensure_jj
		run jj git init --colocate "$PRIVATEFILES_DIR"
		run jj -R "$PRIVATEFILES_DIR" bookmark track main --remote=origin
	fi

	# Note: symlinks from $HOME → privatefiles (~/.claude/CLAUDE.md,
	# ~/.seal/SEAL.md, ~/repos/sealedsecurity/*, ~/repos/repos.code-workspace)
	# are NOT authored here. home-manager creates them declaratively
	# on activation — see nix-config/shared/privatefiles-symlinks.nix.
	# Step 4's nix-rebuild lays them down; if you ever lose one, run
	# `nix-switch` to re-converge rather than re-running this script.
else
	echo "Skipping privatefiles (server host)."
fi

# ---- Step 4: rebuild ----
step "Rebuilding system against $NIX_CONFIG_DIR"
case "$(uname -s)" in
Darwin)
	FLAKE_TARGET="${FLAKE_TARGET:-$HOSTNAME}"
	run sudo HOME="$HOME" darwin-rebuild switch --flake "$NIX_CONFIG_DIR#$FLAKE_TARGET" --show-trace
	;;
Linux)
	FLAKE_TARGET="${FLAKE_TARGET:-$HOSTNAME}"
	run sudo nixos-rebuild switch --flake "$NIX_CONFIG_DIR#$FLAKE_TARGET" --show-trace
	;;
*)
	err "unsupported uname: $(uname -s)"
	;;
esac

# ---- Step 5: retire the dotfiles repo ----
step "Retiring the dotfiles repo"
STAMP="$(date +%Y%m%d-%H%M%S)"
if [ -d "$HOME/.git" ]; then
	# Refuse to retire if main is ahead of origin (unpushed work).
	upstream='main@{u}'
	if [ "$(git --git-dir="$HOME/.git" rev-list --count "main..$upstream" 2>/dev/null || echo 0)" != "0" ] ||
		[ "$(git --git-dir="$HOME/.git" rev-list --count "$upstream..main" 2>/dev/null || echo 0)" != "0" ]; then
		err "$HOME/.git main differs from origin/main — push or stash before retiring."
	fi
	run mv "$HOME/.git" "$HOME/.git.archived-$STAMP"
	echo "Moved ~/.git → ~/.git.archived-$STAMP (delete manually after a few weeks of confirmed-working state)."
fi
if [ -d "$HOME/.jj" ]; then
	run mv "$HOME/.jj" "$HOME/.jj.archived-$STAMP"
	echo "Moved ~/.jj → ~/.jj.archived-$STAMP"
fi

# Drop the old ~/nix-config path. Could be a regular dir (NixOS bootstrap
# laid it down from the zireael checkout) or a symlink (if a user set
# up a compat link). Both removed safely — the source of truth is now
# $NIX_CONFIG_DIR.
if [ -L "$HOME/nix-config" ] || [ -d "$HOME/nix-config" ]; then
	run rm -rf "$HOME/nix-config"
	echo "Removed stale ~/nix-config (source of truth is now $NIX_CONFIG_DIR)."
fi

step "Migration complete"
cat <<EOF

Done. Verify:
  - \`nix-switch\` runs and rebuilds against $NIX_CONFIG_DIR.
  - \`ls -la ~/.claude/CLAUDE.md ~/.seal/SEAL.md\` show symlinks into $PRIVATEFILES_DIR (dev hosts only).
  - \`cd $ZIREAEL_DIR && jj status\` runs cleanly.

After ~1-2 weeks of confirmed-working state, delete:
  - ~/.git.archived-$STAMP
  - ~/.jj.archived-$STAMP

EOF

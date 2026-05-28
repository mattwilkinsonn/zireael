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
#   3. On dev boxes (--dev or auto-detected by checking for
#      .config/op/team-service-account-token), clone privatefiles
#      into ~/repos/privatefiles/ and author the symlinks back into
#      $HOME (~/.claude/CLAUDE.md, ~/.seal/SEAL.md, etc.).
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

# ---- Step 1: sanity check ----
step "Checking current dotfiles layout"
if [ ! -d "$HOME/.git" ] && [ -d "$ZIREAEL_DIR/.git" ]; then
	echo "Looks like this host is already migrated (no ~/.git, $ZIREAEL_DIR present). Exiting."
	exit 0
fi
if [ ! -d "$HOME/.git" ]; then
	err "Expected ~/.git (the dotfiles repo) to exist. Cannot determine current layout."
fi

# Auto-detect dev mode if not forced. Dev boxes have a team OP token
# file (set by the bootstrap), servers don't.
if [ "$DEV" = "auto" ]; then
	if [ -f "$HOME/.config/op/team-service-account-token" ]; then
		DEV=yes
		echo "Auto-detected dev host (found team OP token)."
	else
		DEV=no
		echo "Auto-detected server host (no team OP token)."
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

	# shellcheck disable=SC2016  # literal $HOME in user-facing message
	step 'Authoring privatefiles symlinks back into $HOME'
	# Each symlink maps "where the app expects to find it" to
	# "canonical location in privatefiles/". Skipped silently if the
	# destination already exists (link or regular file — we don't
	# clobber).
	link_if_absent() {
		local target="$1" linkpath="$2"
		if [ -L "$linkpath" ]; then
			local current
			current="$(readlink "$linkpath")"
			if [ "$current" = "$target" ]; then
				echo "  $linkpath already correctly linked"
				return
			fi
			echo "  $linkpath linked to $current — backing up + relinking"
			run mv "$linkpath" "$linkpath.pre-zireael-migration"
		elif [ -e "$linkpath" ]; then
			echo "  $linkpath is a regular file — backing up + replacing with symlink"
			run mv "$linkpath" "$linkpath.pre-zireael-migration"
		fi
		run mkdir -p "$(dirname "$linkpath")"
		run ln -s "$target" "$linkpath"
	}

	link_if_absent "$PRIVATEFILES_DIR/home/.claude/CLAUDE.md" "$HOME/.claude/CLAUDE.md"
	link_if_absent "$PRIVATEFILES_DIR/home/.claude/RTK.md" "$HOME/.claude/RTK.md"
	link_if_absent "$PRIVATEFILES_DIR/home/.seal/SEAL.md" "$HOME/.seal/SEAL.md"

	# sealedsecurity workspace meta (Linux + WSL dev only — Mac dev
	# may also keep it). Skip if ~/repos/sealedsecurity/ isn't on
	# this host.
	if [ -d "$HOME/repos/sealedsecurity" ]; then
		link_if_absent "$PRIVATEFILES_DIR/repos/sealedsecurity/SEAL.md" "$HOME/repos/sealedsecurity/SEAL.md"
		link_if_absent "$PRIVATEFILES_DIR/repos/sealedsecurity/sealedsecurity.code-workspace" "$HOME/repos/sealedsecurity/sealedsecurity.code-workspace"
	fi

	# repos.code-workspace at ~/repos/
	[ -f "$PRIVATEFILES_DIR/repos/repos.code-workspace" ] &&
		link_if_absent "$PRIVATEFILES_DIR/repos/repos.code-workspace" "$HOME/repos/repos.code-workspace"
else
	echo "Skipping privatefiles (server host)."
fi

# ---- Step 4: rebuild ----
step "Rebuilding system against $NIX_CONFIG_DIR"
HOSTNAME="${HOSTNAME_OVERRIDE:-$(hostname)}"
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

#!/usr/bin/env bash
# Shared helpers for per-host bootstrap scripts.
#
# `source` this from each bootstrap to pick up the common chunks:
# logging, the EUID guard, --auth-key parsing, dotfiles clone, op token
# write, op-driven password rotation, and inter-server SSH key fetch.
# Each function exits non-zero (via `err`) on hard failure; warning-
# only paths use `warn` and continue so re-runs can self-heal.
#
# Idempotent: every step skips when its sentinel already exists. Safe
# to source multiple times — only function/var definitions, no side
# effects at source time.
#
# Per-host scripts still own host-specific bits: tailscale auth (some
# hosts skip), hostname assertion, sanity checks, and any platform
# quirks (mac keychain instead of token file, pi systemd-creds, WSL
# windows-side dotfiles copy).

# Don't `set -e` here — callers already have it. Sourcing a file with
# `set -e` would inherit it but also override caller-side error traps.

# ---- logging ---------------------------------------------------------
step() { printf "\n\033[1;34m==>\033[0m %s\n" "$*"; }
warn() { printf "\033[1;33m[warn]\033[0m %s\n" "$*"; }
err() {
	printf "\033[1;31m[err]\033[0m %s\n" "$*" >&2
	exit 1
}

# ---- guards ----------------------------------------------------------

# Refuse root. Under sudo, $HOME=/root silently misroutes the dotfiles
# checkout, op token write, ~/.ssh/ writes, and gh auth. Each script
# uses `sudo` internally for the few commands that need it.
require_non_root() {
	if [ "$EUID" = "0" ]; then
		err "Don't run as root/sudo. Run as your normal user — sudo is invoked internally where needed."
	fi
}

# Assert hostname matches expected value (or one of several). Pass any
# number of acceptable hostnames; bails out with a friendly message if
# the current hostname doesn't match.
#
# Comparison strips any DNS suffix from `hostname` output. macOS in
# particular returns `<name>.local` once mDNS has run, even when
# /etc/hostname (or networking.hostName on nix-darwin) is set to the
# bare short name. The bare label before the first dot is what we
# care about for the host-identity check.
require_hostname() {
	local current
	# `hostname -s` is the short-name form on both macOS and Linux,
	# but isn't universally available (e.g. busybox). Fall back to
	# stripping `.<rest>` from `hostname` if -s isn't supported.
	current="$(hostname -s 2>/dev/null || hostname | sed 's/\..*//')"
	for expected in "$@"; do
		if [ "$current" = "$expected" ]; then
			return 0
		fi
	done
	# Platform-aware hint — macOS uses `scutil`, Linux uses
	# `hostnamectl`. Print both so the user can pick.
	case "$(uname -s)" in
	Darwin)
		err "hostname '$current' not in allowed set: $*. Fix on macOS with:
       sudo scutil --set HostName <name>
       sudo scutil --set LocalHostName <name>
       sudo scutil --set ComputerName <name>
     Then either reboot or: sudo dscacheutil -flushcache"
		;;
	*)
		err "hostname '$current' not in allowed set: $*. Fix with: sudo hostnamectl set-hostname <name>"
		;;
	esac
}

# ---- arg parsing -----------------------------------------------------

# Parse `--auth-key <key>` / `--auth-key=<key>` from the script's argv.
# Reads/writes the global TAILSCALE_AUTH_KEY (so callers can preset via
# env before calling). Errors on unknown args. Pass "$@" from the
# caller.
parse_tailscale_auth_key() {
	TAILSCALE_AUTH_KEY="${TAILSCALE_AUTH_KEY:-}"
	while [ $# -gt 0 ]; do
		case "$1" in
		--auth-key)
			TAILSCALE_AUTH_KEY="$2"
			shift 2
			;;
		--auth-key=*)
			TAILSCALE_AUTH_KEY="${1#*=}"
			shift
			;;
		*) err "unknown arg: $1" ;;
		esac
	done
	export TAILSCALE_AUTH_KEY
}

# ---- tailscale -------------------------------------------------------

# Bring tailscale up if it isn't already. Optional second arg is extra
# flags (e.g. `--ssh` for hosts that should run tailscale-ssh too). The
# auth key comes from $TAILSCALE_AUTH_KEY (set via env or
# parse_tailscale_auth_key); prompts interactively if empty.
tailscale_up_if_needed() {
	local extra_flags="${1:-}"
	if tailscale status &>/dev/null; then
		step "Tailscale already authenticated ($(tailscale ip -4 2>/dev/null || echo 'no IP yet'))"
		return 0
	fi
	step "Joining tailnet"
	if [ -z "${TAILSCALE_AUTH_KEY:-}" ]; then
		echo "No auth key provided. Generate one at:"
		echo "  https://login.tailscale.com/admin/settings/keys"
		echo "Paste the tskey-auth-... string (input hidden):"
		read -r -s TAILSCALE_AUTH_KEY
		echo
	fi
	[ -n "${TAILSCALE_AUTH_KEY:-}" ] || err "no auth key provided"
	# shellcheck disable=SC2086 # extra_flags intentionally unquoted so
	# it can be empty or multi-flag; values are script-controlled.
	sudo tailscale up --auth-key="$TAILSCALE_AUTH_KEY" $extra_flags
	echo "Tailscale up: $(tailscale ip -4)"
}

# ---- platform tool installs ------------------------------------------

# Ensure gh is on PATH. On Darwin: brew install gh. On Linux: nix profile
# install. Both are bridge state — gh ends up in shared/home.nix's
# home.packages after the first home-manager activation, but the
# bootstrap needs gh BEFORE home-manager has run (to clone the private
# dotfiles repo in the first place).
ensure_gh() {
	command -v gh >/dev/null 2>&1 && return 0
	case "$(uname -s)" in
	Darwin)
		step "Installing gh via brew"
		brew install gh
		;;
	Linux)
		step "Installing gh into nix profile (one-time, ~30s)"
		nix --extra-experimental-features 'nix-command flakes' \
			profile install nixpkgs#gh
		export PATH="$HOME/.nix-profile/bin:$PATH"
		;;
	*) err "unsupported OS: $(uname -s)" ;;
	esac
	command -v gh >/dev/null 2>&1 || err "gh still not on PATH after install"
}

# Ensure jj is on PATH. Same bridge-state pattern as ensure_gh — jj is in
# home.packages too, but the bootstrap colocates jj on the dotfiles repo
# immediately after clone, before home-manager has run.
ensure_jj() {
	command -v jj >/dev/null 2>&1 && return 0
	case "$(uname -s)" in
	Darwin)
		step "Installing jj via brew"
		brew install jj
		;;
	Linux)
		step "Installing jj into nix profile (one-time, ~30s)"
		nix --extra-experimental-features 'nix-command flakes' \
			profile install nixpkgs#jujutsu
		export PATH="$HOME/.nix-profile/bin:$PATH"
		;;
	*) err "unsupported OS: $(uname -s)" ;;
	esac
	command -v jj >/dev/null 2>&1 || err "jj still not on PATH after install"
}

# ---- dotfiles --------------------------------------------------------

# Clone the dotfiles repo (private GitHub repo) into $HOME as a normal
# colocated git+jj repo at ~/.git, backing up any conflicting files
# already in $HOME into ~/.dotfiles-backup. Uses `gh repo clone` for
# HTTPS auth so it works before home-manager has installed gh (and
# without writing git's credential.helper into ~/.config/git/config,
# which is symlinked from the nix store on NixOS).
#
# The repo doesn't get cloned directly into $HOME because (a) $HOME is
# non-empty so plain `git clone` would refuse, and (b) `jj git clone`
# requires an empty destination too. So: clone to a tempdir, move just
# the .git/ dir into $HOME, conflict-backup pre-existing files,
# `git reset --hard` to lay down the tracked tree, then jj-init.
#
# Args: <repo-slug>
clone_zireael_via_gh() {
	local repo_slug="$1"
	local dest="$HOME/repos/zireael"

	if [ -d "$dest/.git" ]; then
		step "zireael already cloned at $dest"
		return 0
	fi

	ensure_gh
	if ! gh auth status &>/dev/null; then
		echo "Authenticating to GitHub for the zireael repo:"
		gh auth login
		# Skip `gh auth setup-git` — it tries to write
		# ~/.config/git/config which is symlinked from the nix
		# store (read-only) on NixOS. `gh repo clone` uses gh's
		# own stored auth directly.
	fi

	step "Cloning zireael (github.com/$repo_slug) into $dest"
	mkdir -p "$HOME/repos"
	gh repo clone "$repo_slug" "$dest"

	init_jj_on_zireael
}

# Colocate jj on the cloned zireael repo. Idempotent.
init_jj_on_zireael() {
	local dest="$HOME/repos/zireael"
	if [ -d "$dest/.jj" ]; then
		step "jj already colocated on $dest (.jj/ exists)"
		return 0
	fi

	ensure_jj
	step "Initializing jj on the zireael repo"
	jj git init --colocate "$dest"
	jj -R "$dest" bookmark track main --remote=origin
}

# Optional: clone the privatefiles repo too. Dev boxes need it (it
# carries CLAUDE.md / RTK.md / the user-level SEAL.md / the
# sealedsecurity workspace meta files + the Tailscale ACL); headless
# server boxes don't. Idempotent.
clone_privatefiles_via_gh() {
	local repo_slug="${1:-mattwilkinsonn/privatefiles}"
	local dest="$HOME/repos/privatefiles"

	if [ -d "$dest/.git" ]; then
		step "privatefiles already cloned at $dest"
		return 0
	fi

	ensure_gh
	if ! gh auth status &>/dev/null; then
		echo "Authenticating to GitHub for the privatefiles repo:"
		gh auth login
	fi

	step "Cloning privatefiles (github.com/$repo_slug) into $dest"
	mkdir -p "$HOME/repos"
	gh repo clone "$repo_slug" "$dest"

	# jj colocate (matches the zireael pattern).
	ensure_jj
	if [ ! -d "$dest/.jj" ]; then
		jj git init --colocate "$dest"
		jj -R "$dest" bookmark track main --remote=origin
	fi
}

# ---- linuxbrew -------------------------------------------------------

# Install Homebrew on Linux (linuxbrew) if it isn't already. Idempotent —
# skips if /home/linuxbrew/.linuxbrew/bin/brew already exists.
#
# Why this lives outside nix: NixOS doesn't have a first-class linuxbrew
# module (the nix-homebrew project is Darwin-only). For our dev hosts
# (mattfw, mattpc-wsl) brew is useful for a handful of cases where the
# nixpkgs version of a tool lags upstream by months or where we want
# parity with the macOS dev workflow (e.g. `brew style` for the zireael
# tap formulae, hk's pkl cross-compile tooling, etc.).
#
# NixOS LIMITATION: the upstream Homebrew installer hardcodes
# /usr/bin/install (and other FHS coreutils paths), which don't exist
# on NixOS. The installer fails with:
#
#   /tmp/brew-install.sh: line 275: /usr/bin/install: No such file or
#   directory
#
# Workarounds:
#
#   - buildFHSEnv: wrap brew in an FHS sandbox via nix-shell. Heavy
#     setup; the resulting brew can't see host paths cleanly so
#     `brew style` on monorepo files needs careful bind-mount config.
#
#   - System-wide symlinks: ln -s the coreutils binaries from
#     /run/current-system/sw/bin into /usr/bin. Pollutes the FHS
#     layout NixOS deliberately avoids; sketchy long-term.
#
#   - Distrobox: run linuxbrew inside an Ubuntu/Fedora container
#     where the installer works unmodified. Most idiomatic for
#     "I want brew on NixOS" — see https://distrobox.it.
#
# For now this helper is a NO-OP on NixOS — bails with a warning
# pointing at distrobox. Non-NixOS Linux dev hosts (Debian/Ubuntu/
# Fedora) can use the installer directly and this helper works as
# advertised.
#
# The shellenv hook in shared/linux.nix already handles persistent
# PATH wiring regardless of how brew got there, so a distrobox-
# installed brew with the right export still works seamlessly.
ensure_linuxbrew() {
	if [ -x /home/linuxbrew/.linuxbrew/bin/brew ]; then
		step "linuxbrew already installed at /home/linuxbrew/.linuxbrew"
		return 0
	fi
	if [ "$(uname -s)" != "Linux" ]; then
		warn "ensure_linuxbrew called on non-Linux host — skipping"
		return 0
	fi
	# NixOS detection: /etc/os-release sets ID=nixos. The Homebrew
	# installer assumes FHS-standard paths and breaks here.
	if grep -q '^ID=nixos$' /etc/os-release 2>/dev/null; then
		warn "linuxbrew install skipped on NixOS — installer hardcodes FHS paths that don't exist here."
		echo "  See https://distrobox.it to run brew inside an Ubuntu container,"
		echo "  or use 'brew style' on macOS / CI runners which both have brew natively."
		return 0
	fi
	step "Installing linuxbrew (one-time, ~2min)"
	# The installer writes /home/linuxbrew/.linuxbrew/ and adds
	# itself to PATH via shellenv. shared/linux.nix already has the
	# `eval $(brew shellenv)` hook in programs.zsh.initContent, so
	# the next shell picks brew up automatically — no per-host
	# .bashrc edit needed.
	NONINTERACTIVE=1 /bin/bash -c \
		"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
	[ -x /home/linuxbrew/.linuxbrew/bin/brew ] ||
		err "linuxbrew install completed but /home/linuxbrew/.linuxbrew/bin/brew is missing"
	# Eval shellenv into THIS script's PATH so any subsequent step
	# (brew bundle, etc.) can find `brew`. The persistent PATH
	# wiring is handled by the zsh hook in shared/linux.nix.
	eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"
	echo "  linuxbrew installed; new shells will pick it up via shared/linux.nix's shellenv hook"
}

# ---- 1Password: token file -------------------------------------------

# Store a 1P service-account token at the given path (mode 600).
# Prompts interactively if the file doesn't exist. Used by hosts that
# read the token from a plain file (mattfw / mattserver / mattpc-wsl —
# the file is exported into the named env var by their home-manager
# configs). Mac uses macOS keychain instead; rpi4 uses systemd-creds —
# both bypass this helper.
#
# Args: <token-file-path> [label] [scope-description]
#   label             — short name for the SA (e.g. "Personal" / "Team").
#                       Default: "1Password service account".
#   scope-description — what to scope it to in 1P (free-form prose).
#                       Default: "Dev + Server vaults".
op_token_file_write() {
	local token_file="$1"
	local label="${2:-1Password service account}"
	local scope="${3:-Dev + Server vaults}"
	if [ -f "$token_file" ]; then
		step "$label token already at $token_file"
		return 0
	fi
	step "Storing $label token at $token_file"
	echo ""
	echo "Paste your $label token (ops_...) for this host."
	echo "Create one at 1password.com → Integrations → Service Accounts → New."
	echo "Scope: $scope."
	local op_token=""
	read -rsp "$label token: " op_token
	echo
	[ -n "$op_token" ] || err "empty token. Re-run, or store manually: install -m 600 -D /dev/stdin '$token_file' <<< 'ops_...'"
	install -m 600 -D /dev/stdin "$token_file" <<<"$op_token"
	unset op_token
}

# Source a token file into OP_SERVICE_ACCOUNT_TOKEN and verify auth via
# `op whoami`. Errors out if the token is invalid.
#
# Args: <token-file-path>
op_export_and_verify() {
	local token_file="$1"
	[ -f "$token_file" ] || err "token file missing at $token_file — call op_token_file_write first"
	OP_SERVICE_ACCOUNT_TOKEN="$(cat "$token_file")"
	export OP_SERVICE_ACCOUNT_TOKEN
	step "Verifying op auth via service account token"
	op whoami >/dev/null || err "op whoami failed — token invalid?"
}

# ---- 1Password: password rotation ------------------------------------

# Rotate `mattw` and `root` passwords from a 1P op:// reference. Reads
# the password via `op read`, then pipes both lines through `chpasswd`.
# If `--soft` is the second arg, a missing 1P item warns instead of
# erroring (used by mattserver where the item may not be created yet).
#
# Args: <op-reference> [--soft]
op_rotate_user_root_password() {
	local op_ref="$1"
	local mode="${2:-}"
	step "Rotating user + root passwords from 1P"
	local password
	if [ "$mode" = "--soft" ]; then
		password="$(op read "$op_ref" 2>/dev/null || true)"
		if [ -z "$password" ]; then
			warn "$op_ref not found in 1P — create the item, then re-run."
			warn "Skipping password rotation — baked-in initialHashedPassword is still active."
			return 0
		fi
	else
		password="$(op read "$op_ref")"
		[ -n "$password" ] || err "password from $op_ref is empty — check the item exists and the service account has read access"
	fi
	sudo chpasswd <<EOF
mattw:$password
root:$password
EOF
	unset password
	echo "  mattw + root passwords rotated"
}

# ---- 1Password: inter-server SSH key ---------------------------------

# Fetch the host-to-host automation SSH key from 1P (op://Server/inter-server)
# into ~/.ssh/id_ed25519_inter_server. Skips with a warning if the 1P
# item isn't readable — re-run after creating it. The matching public
# key in nixos/common.nix's authorizedKeys is shared across hosts.
op_fetch_inter_server_key() {
	local key_path="$HOME/.ssh/id_ed25519_inter_server"
	local op_ref="op://Server/inter-server/private key?ssh-format=openssh"
	if [ -f "$key_path" ]; then
		step "Inter-server SSH key already at $key_path"
		return 0
	fi
	step "Fetching inter-server SSH key from 1Password"
	if ! op read "$op_ref" >/dev/null 2>&1; then
		warn "1P item 'inter-server' not readable (item missing or no service-account access)"
		echo "  Re-run this script after the item is set up."
		return 0
	fi
	mkdir -p "$HOME/.ssh"
	# umask-077 subshell so the file is mode 600 at creation rather
	# than after the fact (no readable-window race).
	(umask 077 && op read "$op_ref" >"$key_path")
	chmod 600 "$key_path"
	[ -s "$key_path" ] || err "inter-server private key file empty"
	echo "  $key_path written (mode 600)"
}

#!/bin/bash
set -e

echo "=== Mac Setup Script ==="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

# ─── Xcode Command Line Tools ────────────────────────────────────────────────
if ! xcode-select -p &>/dev/null; then
	echo "Installing Xcode Command Line Tools..."
	xcode-select --install
	echo "Press any key once Xcode CLT installation is complete..."
	read -r -n 1
fi

# ─── Homebrew ─────────────────────────────────────────────────────────────────
if ! command -v brew &>/dev/null; then
	echo "Installing Homebrew..."
	/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
	eval "$(/opt/homebrew/bin/brew shellenv zsh)"
fi

# ─── Dotfiles (colocated git+jj at ~/.git, work-tree at $HOME) ────────────────
# clone_zireael_via_gh handles brew-installing gh + jj, gh auth, the
# clone-into-tempdir-and-move-.git dance, conflict backup, and jj init.
clone_zireael_via_gh "mattwilkinsonn/zireael"

# ─── 1Password service account tokens (Keychain) ─────────────────────────────
# Stored as generic passwords in macOS Keychain so they're loaded into
# OP_SERVICE_ACCOUNT_TOKEN / OP_TEAM_SERVICE_ACCOUNT_TOKEN at shell start
# (see darwin/home.nix). With these in place, `op` uses token auth — no
# biometric prompts, no consent dialogs, no Group Container TCC. Required
# by load-secrets.
#
# Two accounts, two tokens. Personal SA reads op://Dev + op://Server;
# team SA reads op://Local Dev (sealedsecurity.1password.com).
if ! security find-generic-password -a "$USER" -s "OP_SERVICE_ACCOUNT_TOKEN" -w &>/dev/null; then
	echo ""
	echo "Paste your Personal 1Password service account token (ops_...) for this Mac."
	echo "Create one at 1password.com → Integrations → Service Accounts."
	echo "Scope: read access to personal Dev + Server vaults."
	read -rsp "Personal token: " OP_TOKEN
	echo
	if [ -n "$OP_TOKEN" ]; then
		security add-generic-password -a "$USER" -s "OP_SERVICE_ACCOUNT_TOKEN" -w "$OP_TOKEN"
		echo "Personal token stored in Keychain (service: OP_SERVICE_ACCOUNT_TOKEN)."
	else
		echo "Skipped — empty token. Re-run this script or use:"
		echo "  security add-generic-password -a \"\$USER\" -s \"OP_SERVICE_ACCOUNT_TOKEN\" -w 'ops_...'"
	fi
	unset OP_TOKEN
fi

if ! security find-generic-password -a "$USER" -s "OP_TEAM_SERVICE_ACCOUNT_TOKEN" -w &>/dev/null; then
	echo ""
	echo "Paste your Team 1Password service account token (ops_...) — sealedsecurity.1password.com."
	echo "Create one at sealedsecurity.1password.com → Integrations → Service Accounts."
	echo "Scope: read access to Local Dev vault."
	read -rsp "Team token: " OP_TEAM_TOKEN
	echo
	if [ -n "$OP_TEAM_TOKEN" ]; then
		security add-generic-password -a "$USER" -s "OP_TEAM_SERVICE_ACCOUNT_TOKEN" -w "$OP_TEAM_TOKEN"
		echo "Team token stored in Keychain (service: OP_TEAM_SERVICE_ACCOUNT_TOKEN)."
	else
		echo "Skipped — empty token. Re-run this script or use:"
		echo "  security add-generic-password -a \"\$USER\" -s \"OP_TEAM_SERVICE_ACCOUNT_TOKEN\" -w 'ops_...'"
	fi
	unset OP_TEAM_TOKEN
fi

# ─── Nix ──────────────────────────────────────────────────────────────────────
if ! command -v nix &>/dev/null; then
	echo "Installing Nix (Determinate Systems installer)..."
	curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
	# Source nix in current shell. shellcheck source=/dev/null because
	# the file is created by the installer above; doesn't exist before
	# install nor at lint time, so static path-following can't resolve it.
	# shellcheck source=/dev/null
	. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
fi

# ─── Nix custom config (trust + flake autoaccept) ─────────────────────────────
# Determinate Nix on Mac uses /etc/nix/nix.conf for installer-managed settings
# and /etc/nix/nix.custom.conf for user-tracked overrides. We need:
#   - trusted-users: client must be trusted to override substituters/keys
#     (otherwise flake-declared `extra-substituters` are silently ignored
#     with a "you are not a trusted user" warning on every nix invocation).
#   - accept-flake-config: auto-trust the flake's nixConfig block (the
#     dotfiles flake declares the nixos-raspberrypi cachix in its nixConfig).
# Same effect baked into NixOS hosts via nix.settings in nixos/common.nix;
# Mac is system-managed by Determinate so it lives here instead.
if ! grep -q '^trusted-users' /etc/nix/nix.custom.conf 2>/dev/null; then
	echo "Configuring /etc/nix/nix.custom.conf (trusted-users + accept-flake-config)..."
	sudo tee -a /etc/nix/nix.custom.conf >/dev/null <<EOF
trusted-users = root $USER
accept-flake-config = true
EOF
	sudo launchctl kickstart -k system/systems.determinate.nix-daemon
fi

# ─── nix-darwin (system config + home-manager + homebrew casks) ──────────────
echo "Building nix-darwin configuration..."
sudo HOME="$HOME" nix run nix-darwin -- switch --flake "$HOME/repos/zireael/nix-config#Matts-MacBook-Pro"

# ─── Xcode (full IDE, installed via xcodes) ──────────────────────────────────
if ! xcodes installed 2>/dev/null | grep -q "Xcode"; then
	echo "Installing latest Xcode (this will take a while)..."
	xcodes install --latest
	sudo xcodes select "$(xcodes installed | tail -1)"
fi

# Rustup, cargo tools (just-lsp, cargo-edit, cargo-nextest, cargo-binstall,
# cargo-update, sccache), Claude Code install, and `rtk init -g` are now
# handled declaratively by home-manager: rustup + cargo tools live in
# home.packages; Claude Code + rtk init are home.activation hooks; starship-jj
# runs via cargo-binstall in linux.nix's activation. Nothing imperative needed
# here once nix-darwin has switched.

# ─── Berkeley Mono (paid font; manual install) ───────────────────────────────
# Buy at https://berkeleygraphics.com/typefaces/berkeley-mono/
# Mac: install via Font Book (drag .otf into the app, persists in
#   ~/Library/Fonts/). With Google Drive desktop app installed, the .otfs
#   are at ~/Library/CloudStorage/GoogleDrive-<email>/My Drive/System Configurations/Fonts/Berkeley Mono/OTF/

echo ""
echo "=== Setup complete! ==="
echo "Open a new terminal to pick up shell changes."
echo "TODO: Enable Settings Sync in VS Code (sign in with GitHub)"
echo "TODO: Disable System Integrity Protection (SIP) for full yabai functionality (https://github.com/asmvik/yabai/wiki/Disabling-System-Integrity-Protection)"
code

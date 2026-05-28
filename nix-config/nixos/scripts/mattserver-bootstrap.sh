#!/usr/bin/env bash
# First-boot bootstrap for mattserver.
# Run AFTER `nixos-install --flake .#mattserver` completes and you've rebooted
# into the new system, ON mattserver itself (SSH or local TTY as mattw).
#
# Pre-reqs (see nixos/mattserver/INSTALL.md):
#   - btrfs+ZFS partition layout in place
#   - nixos-install --flake .#mattserver completed
#   - Rebooted into the freshly-installed system
#   - Logged in as `mattw`
#
# Handles:
#   1. Tailscale auth (pre-auth key — headless-friendly).
#   2. Dotfiles repo → colocated git+jj at ~/.git, work-tree at $HOME.
#   3. 1Password service-account token at ~/.config/op/service-account-token.
#   4. nixos-rebuild switch (home-manager activations need the user session).
#   5. Password rotation — replace initialHashedPassword from 1P.
#   6. Inter-server SSH key from 1Password.
#   7. Sanity checks — SSH, Tailscale, ZFS pool, runner services.
#
# GitHub runner token file is NOT written here — do that manually after
# this script completes (see INSTALL.md "GitHub runner token" section).
#
# Re-runnable: each step skips if already done.
#
# Usage:
#   bash mattserver-bootstrap.sh
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash mattserver-bootstrap.sh
#   bash mattserver-bootstrap.sh --auth-key tskey-auth-...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"
NIX_CONFIG_DIR="$HOME/repos/zireael/nix-config"

require_non_root
parse_tailscale_auth_key "$@"
require_hostname mattserver

echo "Bootstrapping host: $(hostname)"

# ---------------------------------------------------------------------
# 1. Tailscale
# ---------------------------------------------------------------------
tailscale_up_if_needed --ssh

# ---------------------------------------------------------------------
# 2. Dotfiles repo
# ---------------------------------------------------------------------
clone_zireael_via_gh "$ZIREAEL_REPO_SLUG"

if [ ! -d "$NIX_CONFIG_DIR" ]; then
	err "$NIX_CONFIG_DIR not found after zireael checkout — verify zireael contains nix-config/ at the root."
fi

# ---------------------------------------------------------------------
# 3. 1Password service-account tokens (two accounts: personal + team)
# ---------------------------------------------------------------------
# Personal account (mattserver-svc) — reads op://Dev + op://Server.
TOKEN_FILE="$HOME/.config/op/service-account-token"
op_token_file_write "$TOKEN_FILE" Personal "read access to personal Dev + Server vaults"
op_export_and_verify "$TOKEN_FILE"

# Team account (matt-dev-svc at sealedsecurity.1password.com) — reads
# op://Employee Dev. Used by the user shell only.
TEAM_TOKEN_FILE="$HOME/.config/op/team-service-account-token"
op_token_file_write "$TEAM_TOKEN_FILE" Team "read access to Employee Dev vault on sealedsecurity.1password.com"

# ---------------------------------------------------------------------
# 4. nixos-rebuild switch
# ---------------------------------------------------------------------
step "Running nixos-rebuild switch --flake .#mattserver"
sudo nixos-rebuild switch \
	--flake "$NIX_CONFIG_DIR#mattserver" \
	--show-trace

# ---------------------------------------------------------------------
# 5. Rotate passwords from 1Password
# ---------------------------------------------------------------------
# `--soft` so a missing 1P item warns instead of erroring — this host
# may be bootstrapped before the password item exists.
op_rotate_user_root_password 'op://Dev/mattserver Password/password' --soft

# ---------------------------------------------------------------------
# 6. Inter-server SSH key
# ---------------------------------------------------------------------
op_fetch_inter_server_key

# ---------------------------------------------------------------------
# 7. Sanity checks
# ---------------------------------------------------------------------
step "Sanity checks"

echo "[zfs]"
if command -v zpool >/dev/null 2>&1; then
	if sudo zpool list tank &>/dev/null; then
		echo "  tank pool: $(sudo zpool list -H -o health tank)"
		sudo zfs list -r tank | head -10
	else
		warn "  ZFS pool 'tank' not found — create it per INSTALL.md before using backup receives."
	fi
else
	warn "  zpool not on PATH — ZFS not enabled yet? Check boot.supportedFilesystems."
fi

echo "[runners]"
for svc in github-runner-sealed \
	github-runner-sealed-2 \
	github-runner-sealed-3 \
	github-runner-sealed-4; do
	if systemctl is-active --quiet "$svc" 2>/dev/null; then
		echo "  $svc: active"
	elif ! systemctl list-unit-files --no-legend "$svc" 2>/dev/null | grep -q "$svc"; then
		echo "  $svc: not configured (enableRunners=false in system.nix — flip + rebuild to enable)"
	else
		STATUS="$(systemctl is-active "$svc" 2>/dev/null || echo 'unknown')"
		warn "  $svc: $STATUS — write token file per INSTALL.md then: sudo systemctl restart $svc"
	fi
done

echo "[tailscale]"
tailscale status | head -5

echo "[ssh]"
if systemctl is-active --quiet sshd; then
	echo "  sshd: active"
else
	warn "  sshd not active"
fi

echo "[gpu]"
if command -v radeontop >/dev/null 2>&1; then
	lspci | grep -i "vga\|display\|3d" || warn "  no GPU in lspci output"
else
	warn "  radeontop not found (normal before nix-switch applies)"
fi

cat <<EOF

Bootstrap complete for $(hostname).

Tailscale: $(tailscale status --json 2>/dev/null | grep -o '"DNSName":"[^"]*"' | head -1 | sed 's/"DNSName":"\(.*\).$/\1/' || echo "<run 'tailscale status'>")

Next steps:
  1. Write GitHub runner token file (INSTALL.md "GitHub runner token").
  2. Create ZFS pool if not done yet (INSTALL.md "SATA SSHD — ZFS backup pool").
  3. Grant ZFS delegation to the backup user (INSTALL.md "ZFS backup receive setup").
  4. Add SSH config entry on your Mac (INSTALL.md "SSH from the Mac").

Gaming:
  sudo systemctl start sddm   # one-off KDE/Steam session
EOF

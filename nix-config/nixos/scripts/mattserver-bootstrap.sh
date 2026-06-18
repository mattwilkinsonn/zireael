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
#   3. nixos-rebuild switch (home-manager activations need the user session).
#   4. Password rotation — prompts the operator for new mattw + root
#      password. (Manual rather than 1P-driven: mattserver is the
#      CI agent host; we deliberately keep zero standing 1Password
#      service-account credentials on this box. See INSTALL.md
#      "Security posture" for the threat-model rationale.)
#   5. Sanity checks — SSH, Tailscale, ZFS pool, buildkite agents.
#
# Buildkite agent token file is NOT written here — do that manually
# after this script completes (see INSTALL.md "Buildkite agent token"
# section). The agent token is encrypted at rest via systemd-creds; the
# encrypt script is the one-time + per-rotation operator step.
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
# 3. nixos-rebuild switch
# ---------------------------------------------------------------------
step "Running nixos-rebuild switch --flake .#mattserver"
sudo nixos-rebuild switch \
	--flake "$NIX_CONFIG_DIR#mattserver" \
	--show-trace

# ---------------------------------------------------------------------
# 4. Rotate passwords (manual prompt — see header)
# ---------------------------------------------------------------------
# Deliberately not 1P-driven on mattserver: this host runs untrusted
# GHA workflows via the self-hosted runners, so we minimise standing
# capability the runner UID can ever reach (zero OP service-account
# tokens on disk).
#
# If the bootstrap is being re-run on an already-configured host and
# you don't want to rotate, just hit Enter at both prompts to skip.
step "Password rotation (mattw + root)"
read -r -s -p "New password for mattw + root (or Enter to skip): " new_password
echo
if [ -n "$new_password" ]; then
	read -r -s -p "Confirm: " confirm_password
	echo
	if [ "$new_password" != "$confirm_password" ]; then
		err "passwords don't match — re-run the script and try again"
	fi
	sudo chpasswd <<EOF
mattw:$new_password
root:$new_password
EOF
	unset new_password confirm_password
	echo "  mattw + root passwords rotated"
else
	echo "  skipped (blank input)"
fi

# ---------------------------------------------------------------------
# 5. Sanity checks
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

echo "[buildkite-agents]"
for svc in buildkite-agent-sealed \
	buildkite-agent-sealed-2; do
	if systemctl is-active --quiet "$svc" 2>/dev/null; then
		echo "  $svc: active"
	elif ! systemctl list-unit-files --no-legend "$svc" 2>/dev/null | grep -q "$svc"; then
		echo "  $svc: not configured — check services.buildkite-agents in system.nix"
	else
		STATUS="$(systemctl is-active "$svc" 2>/dev/null || echo 'unknown')"
		warn "  $svc: $STATUS — run mattserver-encrypt-agent-token.sh per INSTALL.md then: sudo systemctl restart $svc"
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
  1. Encrypt the Buildkite agent token into systemd-creds at
     /etc/buildkite-agent/agent-token.cred (INSTALL.md "Buildkite agent token"):
       sudo bash $HOME/repos/zireael/nix-config/nixos/scripts/mattserver-encrypt-agent-token.sh
     Then nix-switch; the buildkite-agent units register on next boot.
  2. Create ZFS pool if not done yet (INSTALL.md "SATA SSHD — ZFS backup pool").
  3. Grant ZFS delegation to the backup user (INSTALL.md "ZFS backup receive setup").
  4. Add SSH config entry on your Mac (INSTALL.md "SSH from the Mac").

Gaming:
  sudo systemctl start sddm   # one-off KDE/Steam session
EOF

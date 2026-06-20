#!/usr/bin/env bash
# First-boot bootstrap for mattlinuxpro (converted 2013 trashcan Mac Pro).
# Run AFTER `nixos-install --flake .#mattlinuxpro` completes and you've
# rebooted into the new system, ON mattlinuxpro itself (SSH or local TTY
# as mattw).
#
# Pre-reqs (see nixos/mattlinuxpro/INSTALL.md):
#   - btrfs partition layout in place
#   - nixos-install --flake .#mattlinuxpro completed
#   - Rebooted into the freshly-installed system
#   - Logged in as `mattw`
#
# Handles:
#   1. Tailscale auth (pre-auth key — headless-friendly).
#   2. zireael repo → ~/repos/zireael (gh clone).
#   3. nixos-rebuild switch (home-manager activations need the user session).
#   4. Password rotation — prompts the operator for new mattw + root
#      password. (Manual rather than 1P-driven: this is a CI agent host;
#      we deliberately keep zero standing 1Password service-account
#      credentials on this box. See INSTALL.md "Security posture".)
#   5. Sanity checks — SSH, Tailscale, buildkite agents.
#
# Buildkite agent token file is NOT written here — do that manually
# after this script completes (see INSTALL.md "Buildkite agent token"
# section). The agent token is encrypted at rest via systemd-creds; the
# encrypt script is the one-time + per-rotation operator step.
#
# Re-runnable: each step skips if already done.
#
# Usage:
#   bash mattlinuxpro-bootstrap.sh
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash mattlinuxpro-bootstrap.sh
#   bash mattlinuxpro-bootstrap.sh --auth-key tskey-auth-...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"
NIX_CONFIG_DIR="$HOME/repos/zireael/nix-config"

require_non_root
parse_tailscale_auth_key "$@"
require_hostname mattlinuxpro

echo "Bootstrapping host: $(hostname)"

# ---------------------------------------------------------------------
# 1. Tailscale
# ---------------------------------------------------------------------
tailscale_up_if_needed --ssh

# ---------------------------------------------------------------------
# 2. zireael repo
# ---------------------------------------------------------------------
clone_zireael_via_gh "$ZIREAEL_REPO_SLUG"

if [ ! -d "$NIX_CONFIG_DIR" ]; then
	err "$NIX_CONFIG_DIR not found after zireael checkout — verify zireael contains nix-config/ at the root."
fi

# ---------------------------------------------------------------------
# 3. nixos-rebuild switch
# ---------------------------------------------------------------------
step "Running nixos-rebuild switch --flake .#mattlinuxpro"
sudo nixos-rebuild switch \
	--flake "$NIX_CONFIG_DIR#mattlinuxpro" \
	--show-trace

# ---------------------------------------------------------------------
# 4. Rotate passwords (manual prompt — see header)
# ---------------------------------------------------------------------
# Deliberately not 1P-driven: this host runs untrusted PR-time CI
# workloads via the self-hosted runners, so we minimise standing
# capability the runner UID can ever reach (zero OP service-account
# tokens on disk).
#
# If the bootstrap is being re-run on an already-configured host and you
# don't want to rotate, just hit Enter at both prompts to skip.
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

echo "[buildkite-agents]"
for svc in buildkite-agent-sealed \
	buildkite-agent-sealed-2; do
	if systemctl is-active --quiet "$svc" 2>/dev/null; then
		echo "  $svc: active"
	elif ! systemctl list-unit-files --no-legend "$svc" 2>/dev/null | grep -q "$svc"; then
		echo "  $svc: not configured — check sealed.buildkiteAgent in system.nix"
	else
		STATUS="$(systemctl is-active "$svc" 2>/dev/null || echo 'unknown')"
		warn "  $svc: $STATUS — run mattlinuxpro-encrypt-agent-token.sh per INSTALL.md then: sudo systemctl restart $svc"
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

cat <<EOF

Bootstrap complete for $(hostname).

Tailscale: $(tailscale status --json 2>/dev/null | grep -o '"DNSName":"[^"]*"' | head -1 | sed 's/"DNSName":"\(.*\).$/\1/' || echo "<run 'tailscale status'>")

Next steps:
  1. Encrypt the Buildkite agent token + ci-app-key into systemd-creds
     (INSTALL.md "Buildkite agent token"):
       sudo bash $HOME/repos/zireael/nix-config/nixos/scripts/mattlinuxpro-encrypt-agent-token.sh
       sudo bash $HOME/repos/zireael/nix-config/nixos/scripts/mattlinuxpro-encrypt-ci-app-key.sh /path/to/sealedsecurity-ci.pem
     Then nix-switch; the decrypt-agent-token + buildkite-agent units
     start and the agents register (no reboot needed).
  2. Add SSH config entry on your Mac (INSTALL.md "SSH from the Mac").
EOF

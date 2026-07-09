#!/usr/bin/env bash
# Bare-metal bootstrap for the NixOS daily driver on mattpc.
#
# The bare-metal analogue of mattpc-wsl-bootstrap.sh. Where the WSL
# bootstrap is two-phase (copy dotfiles from the Windows side, then a
# `nixos`→`mattw` user migration across a `wsl --shutdown`), this host
# is installed straight from nixos/mattpc/INSTALL.md: nixos-install
# already created the `mattw` user and laid down the config, so there's
# no Windows copy and no user migration. This script is therefore
# SINGLE-PHASE — run once as `mattw` after the first boot to:
#   1. Ensure ~/repos/zireael (+ privatefiles) is cloned and jj-colocated.
#   2. Write both 1Password service-account tokens (mode 600) that
#      nixos/mattpc/home.nix exports into the env.
#   3. Bring up this host's own tailscaled (bare metal runs its own,
#      unlike WSL which borrowed the Windows daemon) with tailscale-ssh.
#   4. Re-converge with the op token in env so op-backed home-manager
#      activations fire (they no-op on a token-less first switch).
#   5. Set the `mattw` + `root` login/sudo passwords (prompted once,
#      stored nowhere — never in 1Password; see step 5's comment).
#
# Idempotent — every step skips when its sentinel already exists, so
# re-running picks up where you left off.
#
# Usage:
#   bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-bootstrap.sh [--auth-key <tskey>]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_DIR="$HOME/repos/zireael"
NIX_CONFIG_DIR="$ZIREAEL_DIR/nix-config"

require_non_root
require_hostname mattpc
parse_tailscale_auth_key "$@"

# ---------------------------------------------------------------------
# 1. Ensure the zireael (+ privatefiles) repo is present
# ---------------------------------------------------------------------
# INSTALL.md's nixos-install runs from a repo clone (e.g.
# /mnt/etc/nixos-repo), which doesn't survive into $HOME after reboot.
# If the daily-driver checkout at ~/repos/zireael isn't there yet, clone
# it over gh (HTTPS auth, before home-manager has installed gh). If a
# prior INSTALL step already placed it, skip — clone_zireael_via_gh and
# init_jj_on_zireael are both idempotent.
if [ -d "$ZIREAEL_DIR/.git" ]; then
	step "zireael already at $ZIREAEL_DIR — skipping clone"
	init_jj_on_zireael
else
	ensure_gh
	clone_zireael_via_gh mattwilkinsonn/zireael
fi

# privatefiles carries the sealedsecurity workspace meta this dev host
# needs. clone_privatefiles_via_gh skips silently if already present.
clone_privatefiles_via_gh mattwilkinsonn/privatefiles

[ -d "$NIX_CONFIG_DIR" ] || err "$NIX_CONFIG_DIR missing after clone"

# ---------------------------------------------------------------------
# 2. 1Password service-account tokens (two accounts)
# ---------------------------------------------------------------------
# Personal account (pc-svc) — reads op://Dev + op://Server.
TOKEN_FILE="$HOME/.config/op/service-account-token"
op_token_file_write "$TOKEN_FILE" Personal "read access to personal Dev + Server vaults"
op_export_and_verify "$TOKEN_FILE"

# Team account (matt-dev-svc at sealedsecurity.1password.com) —
# reads op://Local Dev. Used by the user shell only.
TEAM_TOKEN_FILE="$HOME/.config/op/team-service-account-token"
op_token_file_write "$TEAM_TOKEN_FILE" Team "read access to Local Dev vault on sealedsecurity.1password.com"

# ---------------------------------------------------------------------
# 3. Join the tailnet (bare metal runs its own tailscaled)
# ---------------------------------------------------------------------
# WSL borrowed the Windows host's tailscaled and skipped this entirely;
# bare metal enables services.tailscale itself (system.nix), so we bring
# the node up here with tailscale-ssh. Auth key comes from the parsed
# --auth-key flag; absent, the helper falls back to an interactive login
# URL to approve the node.
tailscale_up_if_needed --ssh

# ---------------------------------------------------------------------
# 4. Re-converge so home-manager activations see the op token
# ---------------------------------------------------------------------
# The install-time switch created /home/mattw with home-manager-rendered
# files, but op-backed activations (load-secrets warmup, anything reading
# op://) skipped because OP_SERVICE_ACCOUNT_TOKEN wasn't set yet. Re-run
# with the token in env so they actually fire. No
# --option extra-experimental-features here (WSL needed it on its very
# first switch): this system was installed with flakes enabled system-
# wide via nixos/common.nix's nix.settings.
step "Running nixos-rebuild switch (token-aware activations)"
sudo --preserve-env=OP_SERVICE_ACCOUNT_TOKEN \
	nixos-rebuild switch \
	--flake "$NIX_CONFIG_DIR#mattpc" \
	--show-trace

# ---------------------------------------------------------------------
# 5. Set user + root login/sudo passwords (interactive, one-shot)
# ---------------------------------------------------------------------
# Prompted once and stored nowhere — deliberately NOT sourced from 1P.
# The service-account token above is 600-perm, loaded into every shell's
# env, and preserved across sudo for the rebuild; its op://Dev/... scope
# would cover a password item in 1P, so a token leak could read the
# sudo/root password and trivially privesc to full root. The login
# password must never be reachable via that token.
set_user_root_password_interactive

# ---------------------------------------------------------------------
# 6. linuxbrew (one-time install, for tools NixOS doesn't ship)
# ---------------------------------------------------------------------
# No-op on NixOS (the installer hardcodes FHS paths that don't exist
# here) — the helper bails with a distrobox pointer. Kept for parity
# with the other dev hosts.
ensure_linuxbrew

# ---------------------------------------------------------------------
# 7. Sanity checks
# ---------------------------------------------------------------------
step "Sanity checks"

echo "[ssh]"
if systemctl is-active --quiet sshd; then
	if ss -tlnp 2>/dev/null | grep -q ':22 '; then
		echo "  sshd: active, listening on :22"
	else
		warn "  sshd active but not on :22 — config may not have applied"
	fi
else
	warn "  sshd not active"
fi

echo "[tailscale]"
if tailscale status &>/dev/null; then
	echo "  tailscale: up ($(tailscale ip -4 2>/dev/null || echo 'no IP yet'))"
else
	warn "  tailscale status unreachable — node may not have joined the tailnet"
fi

echo "[podman]"
if systemctl is-active --quiet podman.socket 2>/dev/null; then
	echo "  podman.socket: active"
else
	warn "  podman.socket not active (rootless socket comes up on first user login — try opening a fresh shell)"
fi

cat <<EOF

\033[1;32m✓ Bootstrap complete for $(hostname).\033[0m

Next steps:
  1. SSH into this host from any tailnet device:
       ssh mattw@mattpc                       # local net
       ssh mattw@mattpc.tail2be430.ts.net     # via tailnet
     (The personal SSH key is wired declaratively in nixos/common.nix —
     no manual authorized_keys step needed.)
  2. Remote OS-select for the Windows dual-boot (from any tailnet host):
       sudo bootctl set-oneshot windows && sudo reboot
     A normal Windows restart afterward returns to NixOS (INSTALL.md §6).
  3. (One-time per host) rclone config — set up the 'gdrive' remote so
     Berkeley Mono fonts auto-sync. Without it, the syncBerkeleyMono
     activation prints a warning every nix-switch and Berkeley Mono
     falls back to JetBrains Mono.
EOF

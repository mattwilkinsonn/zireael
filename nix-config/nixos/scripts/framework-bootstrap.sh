#!/usr/bin/env bash
# First-boot bootstrap for the Framework Desktop (mattfw).
# Run AFTER `nixos-install` completes and you've rebooted into the new
# system, ON the Framework itself.
#
# Pre-reqs (handled out of band — see nixos/mattfw/INSTALL.md):
#   - LUKS+btrfs partition layout in place
#   - nixos-install --flake .#mattfw completed
#   - Rebooted into the freshly-installed system
#   - Logged in as `mattw` over SSH (or local TTY)
#
# Handles:
#   1. Tailscale auth (pre-auth key — headless-friendly).
#   2. Dotfiles repo → colocated git+jj at ~/.git, work-tree at $HOME.
#   3. 1Password service-account token at ~/.config/op/service-account-
#      token (mode 600). Same pattern as the other dev hosts;
#      nixos/mattfw/home.nix's mkBefore block reads this into
#      OP_SERVICE_ACCOUNT_TOKEN at every shell start.
#   4. nixos-rebuild switch — apply the host config (now that the user's
#      home and tailnet are wired up, home-manager activations have
#      everything they need).
#   5. Password rotation — replace baked-in initialHashedPassword for
#      `mattw` and `root` with the value from op://Dev/Framework Password.
#   6. linuxbrew — one-time install, idempotent; for tools nixpkgs lags
#      or where Mac↔Linux dev parity matters.
#   7. Sanity checks — services up, GPU detected, ROCm present.
#
# Re-runnable: each step skips if already done. Safe to re-run if
# something fails partway.
#
# Usage:
#   bash framework-bootstrap.sh                              # interactive prompts
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash framework-bootstrap.sh
#   bash framework-bootstrap.sh --auth-key tskey-auth-...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"
NIX_CONFIG_DIR="$HOME/repos/zireael/nix-config"
# nix-config is a subdirectory inside the dotfiles tree — the dotfiles
# repo clones into $HOME as a normal colocated git+jj repo at ~/.git,
# and $NIX_CONFIG_DIR is just a regular directory in that tree. There
# is no separate GitHub repo for nix-config.

require_non_root
parse_tailscale_auth_key "$@"
require_hostname mattfw

echo "Bootstrapping host: $(hostname)"

# ---------------------------------------------------------------------
# 1. Tailscale
# ---------------------------------------------------------------------
tailscale_up_if_needed --ssh

# ---------------------------------------------------------------------
# 2. Dotfiles repo (also brings nix-config along)
# ---------------------------------------------------------------------
# Single repo at github.com/$ZIREAEL_REPO_SLUG. Cloned into $HOME as
# a colocated git+jj repo at ~/.git — the nix-config subdirectory
# ($HOME/repos/zireael/nix-config) falls out of that worktree automatically. There
# is NO separate `mattwilkinsonn/nix-config` repo on GitHub.
clone_zireael_via_gh "$ZIREAEL_REPO_SLUG"

if [ ! -d "$NIX_CONFIG_DIR" ]; then
	err "$NIX_CONFIG_DIR not found after zireael checkout — the dotfiles tree should provide it. Verify the dotfiles repo still contains nix-config/ at the root."
fi

# ---------------------------------------------------------------------
# 3. 1Password service-account tokens (two accounts: personal + team)
# ---------------------------------------------------------------------
# Personal account (framework-svc) — reads op://Dev + op://Server.
# Used by the user shell AND root-side systemd fetchers (openclaw-env-refresh).
TOKEN_FILE="$HOME/.config/op/service-account-token"
op_token_file_write "$TOKEN_FILE" Personal "read access to personal Dev + Server vaults"
op_export_and_verify "$TOKEN_FILE"

# Team account (matt-dev-svc at sealedsecurity.1password.com) — reads
# op://Employee Dev. Used by the user shell only (load-secrets pulls
# Sealed Claude OAuth + Linear API key from here).
TEAM_TOKEN_FILE="$HOME/.config/op/team-service-account-token"
op_token_file_write "$TEAM_TOKEN_FILE" Team "read access to Employee Dev vault on sealedsecurity.1password.com"

# ---------------------------------------------------------------------
# 4. nixos-rebuild switch
# ---------------------------------------------------------------------
# nixos-install used the flake at install time, but home-manager
# activations didn't fully run (no user session). Now that we've got a
# user session + dotfiles + op token, run a real switch so all the
# user-side activations (cargo-binstall, fnm install, claude install,
# etc.) actually fire.
step "Running nixos-rebuild switch --flake .#mattfw"
sudo nixos-rebuild switch \
	--flake "$NIX_CONFIG_DIR#mattfw" \
	--show-trace

# ---------------------------------------------------------------------
# 5. Rotate user + root passwords from 1Password
# ---------------------------------------------------------------------
# Replace the baked-in initialHashedPassword from nixos/common.nix with
# the actual rotated password from 1P. Framework gets its own item
# rather than reusing the Pi password — different blast radius (dev
# box with sudo + tailnet access vs. headless server).
op_rotate_user_root_password 'op://Dev/Framework Password/password'

# ---------------------------------------------------------------------
# 6. linuxbrew (one-time install, for tools NixOS doesn't ship)
# ---------------------------------------------------------------------
# Parity with the macOS dev workflow — `brew style` on the zireael
# tap formulae, hk's pkl tooling, or any future tool where nixpkgs
# lags upstream. shared/linux.nix has the shellenv hook ready.
ensure_linuxbrew

# ---------------------------------------------------------------------
# 7. Sanity checks
# ---------------------------------------------------------------------
step "Sanity checks"

echo "[gpu]"
if command -v rocminfo >/dev/null 2>&1; then
	if rocminfo 2>/dev/null | grep -qi "gfx1151"; then
		echo "  rocminfo: gfx1151 detected"
	else
		warn "  rocminfo ran but gfx1151 not in output — kernel may not have loaded amdgpu yet (reboot?)"
	fi
else
	warn "  rocminfo not on PATH — system.nix's rocmPackages may not have applied yet"
fi

echo "[memory]"
TOTAL_MEM_GB=$(awk '/MemTotal/ { printf "%.0f", $2/1024/1024 }' /proc/meminfo)
echo "  MemTotal: ${TOTAL_MEM_GB} GiB visible to the OS"
# At gttsize=126976 (124 GiB) + UMA=512MB the OS sees ~127 GiB total
# (128 GiB physical minus the 512 MB UMA frame buffer). MemTotal is
# the post-UMA value — GTT is shared with the OS so it doesn't subtract.
# A reading well below 127 GiB means BIOS UMA is still set high.
if [ "$TOTAL_MEM_GB" -lt 120 ]; then
	warn "  Less than 120 GiB visible — BIOS UMA Frame Buffer is set higher than 512 MB."
	warn "  Drop UMA to 512MB in BIOS so the iGPU pulls from GTT instead."
fi
if grep -q "amdgpu.gttsize" /proc/cmdline; then
	echo "  amdgpu.gttsize present in /proc/cmdline"
else
	warn "  amdgpu.gttsize NOT in /proc/cmdline — kernel param didn't apply (rebuild + reboot?)"
fi
if grep -q "ttm.pages_limit" /proc/cmdline; then
	echo "  ttm.pages_limit present in /proc/cmdline"
else
	warn "  ttm.pages_limit NOT in /proc/cmdline — without this, gttsize alone doesn't"
	warn "    actually let the kernel pin the full GTT range. Rebuild + reboot."
fi
GTT_TOTAL_BYTES=$(cat /sys/class/drm/card*/device/mem_info_gtt_total 2>/dev/null | head -1 || echo 0)
if [ "$GTT_TOTAL_BYTES" -gt 0 ]; then
	GTT_TOTAL_GB=$(awk -v b="$GTT_TOTAL_BYTES" 'BEGIN { printf "%.1f", b/1024/1024/1024 }')
	echo "  GTT total: ${GTT_TOTAL_GB} GiB (mem_info_gtt_total)"
fi

echo "[tailscale]"
tailscale status | head -5

echo "[ssh]"
if systemctl is-active --quiet sshd; then
	echo "  sshd: active"
else
	warn "  sshd not active"
fi

cat <<EOF

\033[1;32m✓ Bootstrap complete for $(hostname).\033[0m

Tailscale URL: $(tailscale status --json 2>/dev/null | grep -o '"DNSName":"[^"]*"' | head -1 | sed 's/"DNSName":"\(.*\).$/\1/' || echo "<run 'tailscale status' to see>")

VSCode Remote-SSH from your Mac:
  ssh mattw@mattfw                                    # LAN
  ssh mattw@mattfw.tail08a5c5.ts.net                  # Tailscale

Local desktop (only when needed):
  sudo systemctl start sddm                           # one-off Plasma session
  # or
  sudo systemctl isolate graphical.target             # this boot only
  sudo systemctl isolate multi-user.target            # back to headless

Local LLM tuning:
  rocm-smi                                            # confirm GPU present
  rocminfo | grep -A1 'Marketing Name'                # check it's gfx1151
  cat /proc/meminfo | grep -E '^Mem'                  # check available RAM
  # Then install your inference stack (llama.cpp / Ollama / vLLM with ROCm).
EOF

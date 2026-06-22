#!/usr/bin/env bash
# Two-phase bootstrap for the NixOS-WSL2 distro on mattpc.
#
# Phase 1 (you are `nixos`, fresh tarball):
#   1. Copy the Windows-side dotfiles repo into ~/.git and reset the
#      worktree into /home/nixos.
#   2. nixos-rebuild switch --flake ~/repos/zireael/nix-config#mattpc-wsl  (creates
#      `mattw` user, sets sshd on 2222, runs every home-manager
#      activation for mattw — Berkeley Mono rclone, claude install,
#      starship-jj, fnm + LTS, obsidian-headless, RTK).
#   3. Tell you to `wsl --shutdown` Windows-side, re-enter as `mattw`,
#      re-run this script.
#
# Phase 2 (you are `mattw`, post-rebuild + WSL restart):
#   1. Migrate /home/nixos/.git → /home/mattw/.git, lay down the
#      worktree in /home/mattw, colocate jj on top.
#   2. Store the 1Password service-account token at
#      ~/.config/op/service-account-token (mode 600).
#   3. nixos-rebuild switch again so home-manager activations re-run
#      with op token in env (op-backed activations were skipped on the
#      phase-1 rebuild because the token wasn't there yet).
#   4. Rotate `mattw` and `root` passwords from op://Dev/mattpc-wsl
#      Password.
#   5. Sanity checks (sshd on 2222, podman socket, resolv.conf).
#
# Both phases are idempotent — re-running picks up where you left off.
#
# Usage:
#   # Phase 1, as `nixos`:
#   bash /mnt/c/Users/<WIN_USER>/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
#
#   # Then `wsl --shutdown` Windows-side, `wsl -d NixOS`, you're now `mattw`:
#   bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_DIR="$HOME/repos/zireael"
PRIVATEFILES_DIR="$HOME/repos/privatefiles"
NIX_CONFIG_DIR="$ZIREAEL_DIR/nix-config"

require_non_root

# ---------------------------------------------------------------------
# Phase router
# ---------------------------------------------------------------------
case "$USER" in
nixos) PHASE=1 ;;
mattw) PHASE=2 ;;
*) err "unexpected user '$USER' (expected 'nixos' for phase 1 or 'mattw' for phase 2)" ;;
esac
echo "Detected phase $PHASE (user: $USER, hostname: $(hostname))"

# =====================================================================
# Phase 1: nixos user, fresh tarball
# =====================================================================
if [ "$PHASE" = "1" ]; then
	# Hostname check — nixos is the tarball default. If hostname is
	# already mattpc-wsl, the rebuild already ran but you're still on
	# the nixos session (didn't wsl --shutdown). Skip ahead.
	if [ "$(hostname)" != "nixos" ] && [ "$(hostname)" != "mattpc-wsl" ]; then
		err "hostname '$(hostname)' is neither 'nixos' (tarball default) nor 'mattpc-wsl' (post-rebuild). Aborting."
	fi

	# The NixOS-WSL tarball ships with `nix` on PATH but no `git`,
	# AND with experimental features (nix-command, flakes) disabled
	# by default. Install git into the `nixos` user's profile up
	# front so subsequent `git` calls are plain invocations — using
	# `nix shell nixpkgs#git --command git ...` everywhere instead
	# leaks nix's download-progress lines into git's stderr capture
	# and breaks the conflict-file parser in step 1.2.
	#
	# After the first nixos-rebuild, mattw gets git via
	# shared/home.nix's home.packages and this profile install
	# becomes irrelevant — gets cleaned up on the next nix GC since
	# the nixos user is abandoned.
	if ! command -v git >/dev/null 2>&1; then
		step "Installing git into the nixos profile (one-time, ~30s)"
		nix --extra-experimental-features 'nix-command flakes' \
			profile install nixpkgs#git
		# nix profile install drops symlinks into ~/.nix-profile/bin
		# but PATH on the tarball default shell may not include it.
		export PATH="$HOME/.nix-profile/bin:$PATH"
		command -v git >/dev/null 2>&1 || err "git still not on PATH after profile install — manual debug needed"
	fi

	# -------------------------------------------------------------
	# 1.1 Copy zireael repo from the Windows side
	# -------------------------------------------------------------
	# windows/windows-setup.ps1 has already cloned the zireael repo
	# (+ optionally privatefiles) to %USERPROFILE%\repos\zireael\ on
	# Windows. Mount that via /mnt/c instead of re-authing gh inside
	# WSL.
	if [ -d "$ZIREAEL_DIR/.git" ]; then
		step "zireael already at $ZIREAEL_DIR — skipping copy"
	else
		step "Locating Windows-side zireael via /mnt/c"

		# WSL exposes the Windows username via cmd.exe; tr strips the
		# Windows CRLF terminator. Override with WIN_USER=... to skip
		# the cmd.exe call.
		WIN_USER="${WIN_USER:-$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r\n')}"
		[ -n "$WIN_USER" ] || err "could not detect Windows username (cmd.exe interop broken?). Re-run with: WIN_USER=<name> bash $0"

		WIN_ZIREAEL="/mnt/c/Users/$WIN_USER/repos/zireael"
		if [ ! -d "$WIN_ZIREAEL/.git" ]; then
			err "$WIN_ZIREAEL not found. Run windows/windows-setup.ps1 on the Windows side first — it clones zireael there."
		fi

		step "Copying $WIN_ZIREAEL → $ZIREAEL_DIR"
		mkdir -p "$HOME/repos"
		cp -r "$WIN_ZIREAEL" "$ZIREAEL_DIR"

		# Strip Windows-side filemode + executable bits the cp
		# preserved. Git-on-Windows leaves filemode=true with a
		# different bit interpretation than git-on-Linux.
		git --git-dir="$ZIREAEL_DIR/.git" config core.filemode false
	fi

	# Optionally copy privatefiles too. Dev hosts need it (CLAUDE.md,
	# user-level SEAL.md, sealedsecurity workspace meta). Skip
	# silently when the Windows side didn't clone it.
	WIN_PRIVATEFILES="/mnt/c/Users/${WIN_USER:-$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r\n')}/repos/privatefiles"
	if [ -d "$WIN_PRIVATEFILES/.git" ] && [ ! -d "$PRIVATEFILES_DIR/.git" ]; then
		step "Copying $WIN_PRIVATEFILES → $PRIVATEFILES_DIR"
		cp -r "$WIN_PRIVATEFILES" "$PRIVATEFILES_DIR"
		git --git-dir="$PRIVATEFILES_DIR/.git" config core.filemode false
	fi

	# -------------------------------------------------------------
	# 1.2 Normalize line endings (LF) in the zireael worktree
	# -------------------------------------------------------------
	# Strip CRLF from text files that the Windows-side git clone
	# converted on checkout (default `core.autocrlf=true`). Without
	# this, every shebang line in our .sh files gets a trailing \r
	# that bash chokes on with "$'\r': command not found", and
	# .nix files with CRLF break the nix parser on multi-line
	# strings. .gitattributes at the repo root fixes future
	# clones, but the existing Windows checkout we just copied
	# predates it. Always run — cheap, idempotent.
	step "Normalizing line endings (LF) in $ZIREAEL_DIR"
	for ROOT in "$ZIREAEL_DIR" "$PRIVATEFILES_DIR"; do
		[ -d "$ROOT" ] || continue
		find "$ROOT" -type f \
			\( -name '*.sh' -o -name '*.nix' -o -name '*.md' \
			-o -name '*.toml' -o -name '*.yaml' -o -name '*.yml' \
			-o -name '*.lock' -o -name '*.conf' -o -name '*.cil' \
			-o -name '.gitignore' -o -name '.gitattributes' \
			-o -name 'Containerfile' -o -name 'Brewfile' \) \
			-exec sed -i 's/\r$//' {} +
	done

	# -------------------------------------------------------------
	# 1.3 First nixos-rebuild — creates mattw, sets hostname,
	#     opens sshd on 2222
	# -------------------------------------------------------------
	if [ "$(hostname)" = "mattpc-wsl" ]; then
		step "Rebuild already applied (hostname=mattpc-wsl). Skipping."
	else
		step "Running first nixos-rebuild switch --flake .#mattpc-wsl"
		# nixos-rebuild doesn't accept --extra-experimental-features
		# directly; it's a nix.conf knob, surfaced via --option. The
		# rebuild itself ENABLES these features system-wide via
		# nixos/common.nix's nix.settings, but the rebuild has to
		# run with them flagged on first since /etc/nix/nix.conf
		# isn't patched until after this call succeeds.
		sudo nixos-rebuild switch \
			--flake "$NIX_CONFIG_DIR#mattpc-wsl" \
			--option extra-experimental-features 'nix-command flakes' \
			--show-trace
	fi

	cat <<'EOF'

================================================================
Phase 1 complete.

Next: from an elevated PowerShell on the Windows side:
  wsl --shutdown
  wsl -d NixOS

You'll come back as the `mattw` user. Then re-run this script:
  bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
================================================================
EOF
	exit 0
fi

# =====================================================================
# Phase 2: mattw user, post-rebuild
# =====================================================================
if [ "$PHASE" = "2" ]; then
	if [ "$(hostname)" != "mattpc-wsl" ]; then
		err "hostname '$(hostname)' is not 'mattpc-wsl' — first rebuild didn't apply? Re-run phase 1 as the 'nixos' user."
	fi

	# -------------------------------------------------------------
	# 2.1 Migrate zireael (+ privatefiles) from /home/nixos to /home/mattw
	# -------------------------------------------------------------
	if [ -d "$ZIREAEL_DIR/.git" ]; then
		step "zireael already at $ZIREAEL_DIR — skipping migration"
	else
		step "Migrating /home/nixos/repos/zireael → $ZIREAEL_DIR"
		if [ ! -d /home/nixos/repos/zireael/.git ]; then
			err "/home/nixos/repos/zireael not found. Did phase 1 run? Re-enter WSL as 'nixos' and run this script."
		fi

		mkdir -p "$HOME/repos"
		sudo cp -a /home/nixos/repos/zireael "$ZIREAEL_DIR"
		sudo chown -R "$USER:$(id -gn)" "$ZIREAEL_DIR"
	fi

	# Same for privatefiles (skip silently if phase 1 didn't have it).
	if [ -d /home/nixos/repos/privatefiles/.git ] && [ ! -d "$PRIVATEFILES_DIR/.git" ]; then
		step "Migrating /home/nixos/repos/privatefiles → $PRIVATEFILES_DIR"
		sudo cp -a /home/nixos/repos/privatefiles "$PRIVATEFILES_DIR"
		sudo chown -R "$USER:$(id -gn)" "$PRIVATEFILES_DIR"
	fi

	[ -d "$NIX_CONFIG_DIR" ] || err "$NIX_CONFIG_DIR missing after migration"

	# Colocate jj on zireael (+ privatefiles, if present). Idempotent.
	init_jj_on_zireael
	[ -d "$PRIVATEFILES_DIR/.git" ] && {
		ensure_jj
		[ -d "$PRIVATEFILES_DIR/.jj" ] || {
			jj git init --colocate "$PRIVATEFILES_DIR"
			jj -R "$PRIVATEFILES_DIR" bookmark track main --remote=origin
		}
	}

	# -------------------------------------------------------------
	# 2.2 1Password service-account tokens (two accounts)
	# -------------------------------------------------------------
	# Personal account (pc-svc) — reads op://Dev + op://Server.
	TOKEN_FILE="$HOME/.config/op/service-account-token"
	op_token_file_write "$TOKEN_FILE" Personal "read access to personal Dev + Server vaults"
	op_export_and_verify "$TOKEN_FILE"

	# Team account (matt-dev-svc at sealedsecurity.1password.com) —
	# reads op://Local Dev. Used by the user shell only.
	TEAM_TOKEN_FILE="$HOME/.config/op/team-service-account-token"
	op_token_file_write "$TEAM_TOKEN_FILE" Team "read access to Local Dev vault on sealedsecurity.1password.com"

	# -------------------------------------------------------------
	# 2.3 Re-rebuild so home-manager activations see the op token
	# -------------------------------------------------------------
	# Phase-1 rebuild created /home/mattw with home-manager-rendered
	# files, but op-backed activations (load-secrets warmup, anything
	# reading op://) skipped because OP_SERVICE_ACCOUNT_TOKEN wasn't
	# set yet. Re-run with the token in env so they actually fire.
	step "Running nixos-rebuild switch (token-aware activations)"
	sudo --preserve-env=OP_SERVICE_ACCOUNT_TOKEN \
		nixos-rebuild switch \
		--flake "$NIX_CONFIG_DIR#mattpc-wsl" \
		--show-trace

	# -------------------------------------------------------------
	# 2.4 Rotate user + root passwords from 1Password
	# -------------------------------------------------------------
	op_rotate_user_root_password 'op://Dev/mattpc-wsl Password/password'

	# -------------------------------------------------------------
	# 2.5 linuxbrew (one-time install, for tools NixOS doesn't ship)
	# -------------------------------------------------------------
	# Useful for parity with the macOS dev workflow — e.g.
	# `brew style` on the zireael tap formulae, hk's pkl tooling,
	# or any future tool where nixpkgs lags upstream. The shellenv
	# hook in shared/linux.nix handles persistent PATH wiring.
	ensure_linuxbrew

	# -------------------------------------------------------------
	# 2.6 Sanity checks
	# -------------------------------------------------------------
	step "Sanity checks"

	echo "[ssh]"
	if systemctl is-active --quiet sshd; then
		if ss -tlnp 2>/dev/null | grep -q ':2222 '; then
			echo "  sshd: active, listening on :2222"
		else
			warn "  sshd active but not on :2222 — config may not have applied"
		fi
	else
		warn "  sshd not active"
	fi

	echo "[podman]"
	if systemctl is-active --quiet podman.socket 2>/dev/null; then
		echo "  podman.socket: active"
	else
		warn "  podman.socket not active (rootless socket comes up on first user login — try opening a fresh shell)"
	fi

	echo "[network]"
	if [ -r /etc/resolv.conf ]; then
		echo "  resolv.conf nameserver: $(awk '/^nameserver/ {print $2; exit}' /etc/resolv.conf)"
	else
		warn "  /etc/resolv.conf unreadable"
	fi

	cat <<EOF

\033[1;32m✓ Bootstrap complete for $(hostname).\033[0m

Next steps (Windows-side):
  1. SSH into the WSL distro from any tailnet host:
       ssh -p 2222 mattw@mattpc                     # Windows mirrored net
       ssh -p 2222 mattw@mattpc.tail08a5c5.ts.net   # via tailnet
     (The Tailnet SSH public key from op://Personal is wired
     declaratively in nixos/common.nix — no manual authorized_keys
     step needed.)
  2. Add a Podman Desktop remote connection at
     ssh://mattw@localhost:2222 to manage containers in this WSL
     engine from the Windows GUI.
  3. VSCode Remote-WSL launches via wsl.exe — no SSH config needed.
  4. (One-time per host) rclone config — set up the 'gdrive' remote
     so Berkeley Mono fonts auto-sync. Without it, the
     syncBerkeleyMono activation prints a warning every nix-switch
     and Berkeley Mono falls back to JetBrains Mono.
EOF
	exit 0
fi

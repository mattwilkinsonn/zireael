#!/usr/bin/env bash
# First-boot bootstrap for awsmac (AWS EC2 mac2-m2.metal, Apple M2 —
# STOPGAP macOS CI runner on a $100 AWS-credit budget).
#
# Pre-reqs (see darwin/awsmac/INSTALL.md for the full procedure):
#   - The mac2-m2.metal instance is launched on a dedicated host (24h
#     min allocation) from a current macOS AMI, and you can SSH in as
#     `ec2-user` with your EC2 keypair (the AMI wires it up). No OCLP,
#     no Setup Assistant — the AMI is pre-provisioned.
#   - The account is `ec2-user` (the AMI default admin) — no account
#     creation step, unlike the owned-mini hosts. system.nix +
#     home.nix are pinned to ec2-user.
#   - This script is reachable on disk (`gh repo clone` from /tmp once
#     you're SSH'd in, or scp it over).
#
# Step order is deliberately front-loaded with the "get me to a state
# where I can reach this over Tailscale" steps, so the rest can be
# debugged over the tailnet rather than the EC2-keypair SSH path:
#
#   1. Xcode CLT (needed for brew + git)
#   2. macOS hostname set to `awsmac` (must precede tailscale up
#      or your tailnet hostname will be `awsmac-local` or similar)
#   3. Homebrew (needed for tailscale, gh)
#   4. Tailscale install + auth (you can now SSH in and copy-paste)
#   5. gh install + auth + dotfiles clone
#   6. Nix
#   7. Buildkite agent token → /etc/buildkite-agent/agent-token
#   8. darwin-rebuild switch (agents start immediately)
#   9. Sanity checks
#
# Security posture: zero standing 1Password service-account tokens on
# this host. awsmac runs untrusted CI workflows via the
# self-hosted Buildkite agent pool, so any SA token on disk is a
# credential a compromised agent could exfiltrate. The agent token in
# step 7 is the only secret here, and it's mode-600 root-wheel
# (consider Keychain-with-restricted-ACL encryption later; macOS has
# no systemd-creds counterpart).
#
# Re-runnable: every step skips if already done.
#
# Usage:
#   bash awsmac-bootstrap.sh
#   bash awsmac-bootstrap.sh --auth-key tskey-auth-...
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash awsmac-bootstrap.sh

set -euo pipefail

# Guarantee the standard macOS system paths + the Apple Silicon Homebrew
# prefix are on PATH before anything else. This script shells out to
# system tools in /usr/sbin (scutil) and /sbin (shutdown, reboot) and to
# /opt/homebrew/bin (brew, tailscale). On a fresh macOS account — or a box
# whose nix-darwin /etc/static symlinks are dangling — the login shell's
# path_helper never runs, so these dirs are missing and the script dies
# with "scutil: command not found". Setting PATH explicitly here makes
# bootstrap robust to that state. nix bits get prepended later once the
# nix profile exists.
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/opt/homebrew/sbin:$PATH"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"
NIX_CONFIG_DIR="$HOME/repos/zireael/nix-config"
SEALED_TOKEN_FILE="/etc/buildkite-agent/agent-token"
CI_APP_KEY_FILE="/etc/buildkite-agent/ci-app-key.pem"
TARGET_HOSTNAME="awsmac"
TS_HOSTNAME="${TARGET_HOSTNAME}.tail08a5c5.ts.net"

require_non_root
parse_tailscale_auth_key "$@"
require_hostname awsmac

echo "Bootstrapping host: $(hostname)"
echo

# ---------------------------------------------------------------------
# 1. Xcode Command Line Tools
# ---------------------------------------------------------------------
# CLT installs clang, git, make, and the system headers Homebrew + nix
# both need. `xcode-select --install` is the GUI-prompt path; we trigger
# it programmatically and wait for completion. Idempotent: a working
# CLT install returns the path on stderr, so we probe with `-p` first.
step "Xcode Command Line Tools"
if xcode-select -p >/dev/null 2>&1; then
	echo "  already installed at $(xcode-select -p)"
else
	echo "  triggering install (a GUI prompt will appear — accept it)"
	# The empty placeholder file is the Apple-documented way to make
	# `softwareupdate` show CLT in its list non-interactively.
	sudo touch /tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress
	xcode-select --install 2>/dev/null || true
	echo "  waiting for CLT install to complete (re-check every 30s)..."
	# shellcheck disable=SC2034
	for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
		if xcode-select -p >/dev/null 2>&1; then break; fi
		sleep 30
	done
	sudo rm -f /tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress
	xcode-select -p >/dev/null 2>&1 ||
		err "Xcode CLT install did not complete within 10 min — re-run after the GUI installer finishes"
	echo "  done"
fi

# ---------------------------------------------------------------------
# 2. macOS hostname
# ---------------------------------------------------------------------
# Set all three name surfaces before Tailscale registers — otherwise
# `tailscale up` picks up `awsmac.local` (the LocalHostName /
# Bonjour name) or worse, and the tailnet ends up with a wrong name.
# nix-darwin's networking.hostName/computerName/localHostName will
# re-apply these declaratively later; doing it now is just so the
# values are right at Tailscale-auth time.
#
# Three names macOS tracks separately:
#   HostName       — network/SSH identity (what `hostname` returns)
#   LocalHostName  — Bonjour/mDNS (`<name>.local`)
#   ComputerName   — "About this Mac" display name
step "macOS hostname"
current_short="$(hostname -s 2>/dev/null || hostname | sed 's/\..*//')"
if [ "$current_short" = "$TARGET_HOSTNAME" ] &&
	[ "$(scutil --get LocalHostName 2>/dev/null)" = "$TARGET_HOSTNAME" ] &&
	[ "$(scutil --get ComputerName 2>/dev/null)" = "$TARGET_HOSTNAME" ]; then
	echo "  already set to '$TARGET_HOSTNAME'"
else
	echo "  setting HostName / LocalHostName / ComputerName to '$TARGET_HOSTNAME'"
	sudo scutil --set HostName "$TARGET_HOSTNAME"
	sudo scutil --set LocalHostName "$TARGET_HOSTNAME"
	sudo scutil --set ComputerName "$TARGET_HOSTNAME"
	sudo dscacheutil -flushcache
	echo "  done"
fi

# ---------------------------------------------------------------------
# 3. Homebrew (Apple Silicon prefix /opt/homebrew)
# ---------------------------------------------------------------------
# Moved before Tailscale + gh because both come from brew. The cask
# install in step 4 needs `brew` on PATH; the formula installs in
# steps 4+5 too.
step "Homebrew"
if [ -x /opt/homebrew/bin/brew ]; then
	echo "  already installed"
else
	echo "  installing — you'll be prompted for sudo"
	/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
	[ -x /opt/homebrew/bin/brew ] || err "brew not at /opt/homebrew/bin/brew after install"
fi
eval "$(/opt/homebrew/bin/brew shellenv)"

# The EC2 Mac AMI ships a pre-installed `aws/aws` tap (awscli). Homebrew
# 4.6+ refuses to auto-update third-party taps until they're explicitly
# trusted, so every `brew install` below prints a noisy "Skipping
# aws/aws because it is not trusted" warning. We don't use that tap (the
# agent's awscli comes from nixpkgs), but trusting it silences the
# warning so real brew errors aren't buried. Guarded on the tap being
# present (only on the AMI) + idempotent (`brew trust` is a no-op once
# trusted). `|| true` so a Homebrew that predates the `trust` subcommand
# doesn't abort the bootstrap.
if brew tap 2>/dev/null | grep -q '^aws/aws$'; then
	echo "  trusting pre-installed AMI tap aws/aws (silences untrusted-tap warning)"
	brew trust aws/aws 2>/dev/null || true
fi

# ---------------------------------------------------------------------
# 4. Tailscale (formula) — install + auth
# ---------------------------------------------------------------------
# Front-loaded so the rest of the script is debuggable over SSH.
#
# Why the formula and not the cask: the `tailscale-app` cask runs
# tailscaled as a per-user LaunchAgent in the Aqua (GUI) launchd
# session. SSH sessions live in a different launchd session domain
# and can't see Aqua's XPC services, so `tailscale status` hangs
# forever from SSH even when the daemon is alive. Same problem for
# any root LaunchDaemon (e.g. our Glances tailscale-serve hook).
# The formula installs tailscaled as a launchd SYSTEM daemon via
# brew services, which is visible from every session domain.
#
# Migration: if a previous bootstrap run installed the cask, we need
# to uninstall it before installing the formula — and crucially that
# drops your current tailnet IP. If you're running this script over
# SSH via Tailscale, the cask uninstall will kill the SSH session
# mid-script. To avoid that, run from a local console session (or
# from native sshd via the box's LAN IP, NOT via Tailscale).
step "Tailscale formula"
if brew list --cask tailscale-app >/dev/null 2>&1; then
	# Cask present — we need to migrate.
	if [ -n "${SSH_CONNECTION:-}" ]; then
		# Detect whether SSH came in via Tailscale (in which case
		# the cask uninstall will kill us) or via native sshd over
		# LAN (which is fine — survives the Tailscale daemon swap).
		# Heuristic: SSH_CONNECTION's source IP starts with 100. for
		# the Tailscale 100.64/10 CGNAT range.
		ssh_src="${SSH_CONNECTION%% *}"
		case "$ssh_src" in
		100.*)
			warn "Tailscale migration: cask → formula will drop your tailnet IP,"
			warn "which will kill THIS SSH session (you came in over Tailscale,"
			warn "source IP $ssh_src). Re-run from a local console or via native"
			warn "sshd on the LAN IP instead."
			err "Refusing to break the only SSH path mid-migration."
			;;
		*)
			echo "  SSH src $ssh_src looks like a LAN connection — proceeding."
			;;
		esac
	fi
	echo "  uninstalling tailscale-app cask (migration to formula)"
	brew uninstall --cask tailscale-app || warn "  cask uninstall failed; continuing"
	# Best-effort cleanup of the bootstrap-script's old shim and any
	# /Library/Tailscale state the cask left behind. The formula
	# uses /var/lib/tailscale / /var/run/tailscaled.socket, so the
	# old state is just clutter.
	sudo rm -f /opt/homebrew/bin/tailscale 2>/dev/null || true
fi

if [ -x /opt/homebrew/sbin/tailscaled ] && [ -x /opt/homebrew/bin/tailscale ]; then
	echo "  tailscale formula already installed ($(/opt/homebrew/bin/tailscale version | head -1))"
else
	brew install tailscale
fi

step "tailscaled launchd system daemon"
if sudo launchctl print system/homebrew.mxcl.tailscale >/dev/null 2>&1; then
	echo "  homebrew.mxcl.tailscale already loaded as a system daemon"
else
	sudo brew services start tailscale
	# Give launchd + tailscaled a few seconds to come up before we
	# hit it with `tailscale up`.
	sleep 3
fi

step "tailscale up"
# sudo here is correct — formula's tailscaled is root-owned, and
# `tailscale up` needs root for the initial wireguard setup.
if sudo /opt/homebrew/bin/tailscale status >/dev/null 2>&1; then
	echo "  Already authenticated: $(sudo /opt/homebrew/bin/tailscale ip -4 2>/dev/null | head -1)"
else
	if [ -z "${TAILSCALE_AUTH_KEY:-}" ]; then
		echo "  Generate a pre-auth key at:"
		echo "    https://login.tailscale.com/admin/settings/keys"
		echo "  Single-use, tagged 'tag:server'. Paste below (input hidden):"
		read -r -s TAILSCALE_AUTH_KEY
		echo
	fi
	[ -n "${TAILSCALE_AUTH_KEY:-}" ] || err "no auth key provided"
	sudo /opt/homebrew/bin/tailscale up \
		--auth-key="$TAILSCALE_AUTH_KEY" \
		--ssh \
		--hostname="$TARGET_HOSTNAME"
	unset TAILSCALE_AUTH_KEY
	echo "  Tailnet IP: $(sudo /opt/homebrew/bin/tailscale ip -4 2>/dev/null | head -1)"
fi

cat <<EOF

================================================================
Tailscale is up.

From your laptop you can now SSH in instead of using the console:
    ssh ec2-user@${TS_HOSTNAME}

Continuing with the rest of the bootstrap. If anything below fails,
you can re-run this script over SSH from your laptop — every step
above this is idempotent and will skip cleanly.
================================================================

EOF

# ---------------------------------------------------------------------
# 5. gh + dotfiles clone
# ---------------------------------------------------------------------
# `ensure_gh` (from bootstrap-common.sh) installs gh via brew on
# Darwin. `clone_zireael_via_gh` triggers `gh auth login` if not
# already authed, then clones into $HOME with conflict backup.
clone_zireael_via_gh "$ZIREAEL_REPO_SLUG"

if [ ! -d "$NIX_CONFIG_DIR" ]; then
	err "$NIX_CONFIG_DIR not found after zireael checkout — verify zireael contains nix-config/ at the root."
fi

# ---------------------------------------------------------------------
# 6. Nix (Determinate Systems installer)
# ---------------------------------------------------------------------
# We use the Determinate Systems installer (same as the MBP), NOT the
# upstream nixos.org one. On the EC2 macOS AMI the upstream installer's
# launchd nix-daemon crash-loops: dyld's library-validation refuses to
# load /nix/store dylibs into a hardened launchd-spawned process when
# /nix isn't a firmlink-blessed mount (the AMI ships /nix as a plain
# APFS volume with no /etc/synthetic.conf — a manual `nix-daemon` works
# but the launchd one fails with "file system sandbox blocked open()" /
# OS_REASON_DYLD). Determinate's installer sets the volume + firmlink +
# daemon up as one coherent unit that doesn't trip that validation.
# Determinate manages its own daemon, so system.nix sets
# `nix.enable = false`.
step "Nix (Determinate)"
if command -v nix >/dev/null 2>&1; then
	echo "  already installed ($(nix --version | head -1))"
else
	echo "  installing Determinate Nix — you'll be prompted to confirm"
	curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix |
		sh -s -- install --no-confirm
	# Source the daemon profile so `nix` is on PATH in this shell.
	# shellcheck disable=SC1091
	. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
	command -v nix >/dev/null 2>&1 || err "nix still not on PATH after install"
	echo "  done ($(nix --version | head -1))"
fi

# Nix custom config (trust + caches). Determinate Nix owns
# /etc/nix/nix.conf (installer-managed) and reads
# /etc/nix/nix.custom.conf for user overrides. Since system.nix runs
# `nix.enable = false`, nix-darwin does NOT write these — so they live
# here, mirroring flake.nix's nixConfig (garnix + nixos-raspberrypi
# caches) + the MBP's mac-setup.sh pattern:
#   - trusted-users: ec2-user must be trusted or flake-declared
#     `extra-substituters` are silently ignored ("not a trusted user").
#   - the substituter + trusted-key pair so the binary caches are
#     honored outside a flake context too.
#   - accept-flake-config: auto-trust the flake's nixConfig block.
if ! grep -q '^trusted-users' /etc/nix/nix.custom.conf 2>/dev/null; then
	echo "  configuring /etc/nix/nix.custom.conf (trust + caches)"
	sudo tee -a /etc/nix/nix.custom.conf >/dev/null <<'EOF'
trusted-users = root ec2-user
accept-flake-config = true
extra-substituters = https://nixos-raspberrypi.cachix.org https://cache.garnix.io
extra-trusted-public-keys = nixos-raspberrypi.cachix.org-1:4iMO9LXa8BqhU+Rpg6LQKiGa2lsNh/j2oiYLNOQ5sPI= cache.garnix.io:CTFPyKSLcx5RMJKfLo5EEPUObbA78b0YQ2DTCJXqr9g=
EOF
	# Reload the daemon so it picks up the new custom.conf.
	sudo launchctl kickstart -k system/systems.determinate.nix-daemon 2>/dev/null || true
fi

# ---------------------------------------------------------------------
# 7. Buildkite agent token → /etc/buildkite-agent/agent-token
# ---------------------------------------------------------------------
# Must exist BEFORE darwin-rebuild because the agent launchd daemons
# read it at launch (see darwin/awsmac/system.nix). The decrypt-
# agent-token daemon stages it from this root-owned source into
# /var/run/buildkite-agent/agent-token on each boot. Without the file,
# `darwin-rebuild` lays down the daemons but they fail to register.
step "Buildkite agent token"
if sudo test -s "$SEALED_TOKEN_FILE"; then
	echo "  $SEALED_TOKEN_FILE already present."
else
	echo "Paste the Buildkite AGENT token (org Agents page → Reveal"
	echo "Agent Token). This is the org-wide agent registration token,"
	echo "NOT the BUILDKITE_API_TOKEN used by the bk CLI."
	echo "Get it at: https://buildkite.com/organizations/sealedsecurity/agents"
	echo
	read -rsp "Agent token: " BK_TOKEN
	echo
	[ -n "$BK_TOKEN" ] || err "empty token"
	sudo mkdir -p /etc/buildkite-agent
	# Create the file 600 root:wheel first, then write the secret into
	# it via tee. macOS's BSD `install` can't read /dev/stdin
	# ("Inappropriate file type or format" — that's a GNU-coreutils
	# extension), so we can't pipe the token straight through install
	# like the Linux side does. `install /dev/null` lays down an empty
	# file with the right mode/owner; `tee` (which handles stdin fine
	# on BSD) then fills it. The token stays off argv either way.
	sudo install -m 600 -o root -g wheel /dev/null "$SEALED_TOKEN_FILE"
	printf '%s' "$BK_TOKEN" | sudo tee "$SEALED_TOKEN_FILE" >/dev/null
	unset BK_TOKEN
	echo "  $SEALED_TOKEN_FILE written."
fi

# Source file stays root:wheel; the decrypt-agent-token launchd daemon
# re-stages a group-readable copy under /var/run on every boot, so the
# agent never reads this root-only original directly. chmod is enforced
# unconditionally so a rerun over a pre-existing over-permissive file
# still ends up mode 600.
sudo chown root:wheel "$SEALED_TOKEN_FILE"
sudo chmod 600 "$SEALED_TOKEN_FILE"
echo "  owner: root:wheel mode 600 (re-staged to /var/run/buildkite-agent by decrypt-agent-token)"

# ---------------------------------------------------------------------
# 7b. sealedsecurity-ci App key → /etc/buildkite-agent/ci-app-key.pem
# ---------------------------------------------------------------------
# The git credential helper signs a JWT with this App private key to
# mint installation tokens for cloning (the Buildkite pipeline's
# git@github.com: URL is rewritten to HTTPS). The decrypt-ci-app-key
# daemon re-stages it group-readable under /var/run each boot. Without
# it, checkout fails "Permission denied (publickey)".
step "sealedsecurity-ci App key"
if sudo test -s "$CI_APP_KEY_FILE"; then
	echo "  $CI_APP_KEY_FILE already present."
else
	echo "Path to the sealedsecurity-ci App private key (.pem),"
	echo "downloaded from GitHub → org → Developer settings →"
	echo "GitHub Apps → sealedsecurity-ci → Private keys → Generate."
	echo
	read -rp "Path to .pem: " PEM_PATH
	[ -n "$PEM_PATH" ] && [ -r "$PEM_PATH" ] || err "unreadable .pem path: $PEM_PATH"
	sudo mkdir -p /etc/buildkite-agent
	sudo install -m 600 -o root -g wheel "$PEM_PATH" "$CI_APP_KEY_FILE"
	echo "  $CI_APP_KEY_FILE written. Remember to shred the source:"
	echo "    shred -u $PEM_PATH   # (or 'rm -P' on macOS)"
fi
sudo chown root:wheel "$CI_APP_KEY_FILE"
sudo chmod 600 "$CI_APP_KEY_FILE"
echo "  owner: root:wheel mode 600 (re-staged by decrypt-ci-app-key)"

# ---------------------------------------------------------------------
# 8. darwin-rebuild switch
# ---------------------------------------------------------------------
# Lays down:
#   - Tailscale cask (re-confirmation; already installed in step 4)
#   - Tailscale CLI symlink at /opt/homebrew/bin/tailscale (re-confirmation)
#   - Native sshd intentionally unloaded (see system.nix postActivation)
#   - Strict pmset (sleep 0, autorestart, etc.)
#   - pf egress filter for the runner UID (see system.nix
#     pf-runner-egress launchd daemon)
#   - GitHub runner LaunchDaemons (start immediately — token in place)
#   - Glances + tailscale-serve-glances LaunchDaemons
#
# nix-darwin needs `darwin-rebuild` on PATH; first-time install needs
# `nix run nix-darwin -- switch`, subsequent runs use the symlinked
# darwin-rebuild from the user nix profile.
if ! command -v darwin-rebuild >/dev/null 2>&1; then
	step "First-time nix-darwin bootstrap"
	echo "  nix run nix-darwin -- switch --flake .#awsmac"
	sudo HOME="$HOME" nix run --extra-experimental-features 'nix-command flakes' \
		nix-darwin -- switch --flake "$NIX_CONFIG_DIR#awsmac" --show-trace
else
	step "darwin-rebuild switch --flake .#awsmac"
	sudo HOME="$HOME" darwin-rebuild switch \
		--flake "$NIX_CONFIG_DIR#awsmac" \
		--show-trace
fi

# ---------------------------------------------------------------------
# 9. Sanity checks
# ---------------------------------------------------------------------
step "Sanity checks"

echo "[ssh]"
# Native sshd is intentionally unloaded post-bootstrap (see
# system.nix postActivation). The probe below uses
# `launchctl print system/com.openssh.sshd` to confirm the unit
# isn't registered — exit 0 means it IS registered (bad here),
# non-zero means it isn't (good).
if sudo launchctl print system/com.openssh.sshd >/dev/null 2>&1; then
	warn "  Remote Login (sshd): unexpectedly enabled — re-run nix-switch to unload"
else
	echo "  Remote Login (sshd): off (Tailscale SSH is the access path)"
fi

echo "[pf]"
# Egress filter for the _buildkite-agent UID — see system.nix
# `launchd.daemons.pf-agent-egress`. We load our ruleset top-level
# (no anchor), so `pfctl -sr` enumerates the active rules; counting
# "user _buildkite-agent" lines confirms our rules made it in. Apple-
# default anchors at the top of the ruleset pass through; the agent-
# scoped rules are the ones we care about for this check.
if sudo pfctl -s info 2>/dev/null | grep -q "Status: Enabled"; then
	rule_count=$(sudo pfctl -sr 2>/dev/null | grep -c "user _buildkite-agent")
	if [ "$rule_count" -gt 0 ]; then
		echo "  pf enabled, $rule_count agent-egress rules loaded"
	else
		warn "  pf enabled but no agent-egress rules found — re-run nix-switch"
	fi
else
	warn "  pf disabled — agent-egress filter not active"
fi

echo "[tailscale]"
if [ -x /opt/homebrew/bin/tailscale ]; then
	/opt/homebrew/bin/tailscale status 2>&1 | head -5
else
	warn "  /opt/homebrew/bin/tailscale missing — brew install tailscale failed?"
fi

echo "[buildkite-agents]"
# Single agent on this box (system.nix declares only sealed-macos).
svc="com.sealedsecurity.buildkite-agent-sealed-macos"
if sudo launchctl list 2>/dev/null | grep -q "$svc"; then
	echo "  $svc: loaded"
else
	warn "  $svc: not loaded"
fi

echo "[glances]"
if sudo launchctl list 2>/dev/null | grep -q com.sealedsecurity.glances; then
	echo "  glances: loaded (reachable at https://awsmac.tail08a5c5.ts.net:9443/)"
else
	warn "  glances launchd daemon not loaded"
fi

echo "[secrets-hygiene]"
# Verify no OP service-account credential is reachable from the
# agent UID. This is a defense-in-depth check on top of the
# config-side guarantee — the system.nix postActivation +
# bootstrap script don't write any SA token to disk, but a
# previous version of either might have left one behind, OR
# something added later might re-introduce one.
#
# Three places an agent-reachable SA could lurk:
#   1. ec2-user's Keychain entry "OP_SERVICE_ACCOUNT_TOKEN" — still
#      under ec2-user's UID-scoped ACL by default, so the _buildkite-agent
#      process can't read it, BUT if something ever called
#      `security add-generic-password -T ""` (empty trusted-apps
#      list) it'd be world-readable on the user keychain. Probe.
#   2. /var/lib/buildkite-agents/*/.config/op/ — if some legacy
#      bootstrap layered an SA token into the agent's HOME.
#   3. launchctl setenv OP_SERVICE_ACCOUNT_TOKEN — global env
#      seen by every launchd job including the agent.
hits=0
if security find-generic-password -a "$USER" -s OP_SERVICE_ACCOUNT_TOKEN \
	-w >/dev/null 2>&1; then
	warn "  ec2-user keychain still has OP_SERVICE_ACCOUNT_TOKEN — delete with:"
	warn "    security delete-generic-password -a \"$USER\" -s OP_SERVICE_ACCOUNT_TOKEN"
	hits=$((hits + 1))
fi
if sudo find /var/lib/buildkite-agents -name 'service-account-token*' \
	-o -name 'team-service-account-token*' 2>/dev/null | grep -q .; then
	warn "  found leftover OP SA token files under /var/lib/buildkite-agents — remove manually"
	hits=$((hits + 1))
fi
if sudo launchctl getenv OP_SERVICE_ACCOUNT_TOKEN 2>/dev/null | grep -q .; then
	warn "  global launchctl OP_SERVICE_ACCOUNT_TOKEN set — clear with:"
	warn "    sudo launchctl unsetenv OP_SERVICE_ACCOUNT_TOKEN"
	hits=$((hits + 1))
fi
if [ "$hits" -eq 0 ]; then
	echo "  no OP SA credentials reachable from runner UID — good"
fi

cat <<EOF

Bootstrap complete for $(hostname).

Tailscale: $(/opt/homebrew/bin/tailscale status --json 2>/dev/null | grep -o '"DNSName":"[^"]*"' | head -1 | sed 's/"DNSName":"\(.*\).$/\1/' || echo "$TS_HOSTNAME")

Verify the agents registered at:
  https://buildkite.com/organizations/sealedsecurity/agents

They should appear with the tag:
  queue=macos-arm64-selfhosted
EOF

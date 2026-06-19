#!/usr/bin/env bash
# First-boot bootstrap for mattmacpro (Mac Pro 2013 CI host).
#
# Pre-reqs (see darwin/mattmacpro/INSTALL.md for the full procedure):
#   - macOS Sonoma installed via OCLP, root-patched.
#   - mattw admin user created during macOS setup.
#   - Energy Saver tuned per the hardening section of INSTALL.md.
#     (Remote Login is intentionally left OFF — Tailscale SSH is the
#     only access path; see system.nix "SSH" block.)
#   - This script is reachable on disk (USB containing nix-config, or
#     `gh repo clone` from /tmp once you have an SSH path).
#
# Step order is deliberately front-loaded with the "get me to a state
# where I can SSH in" steps, so the rest can be debugged remotely
# instead of with USB stick + console keyboard:
#
#   1. Xcode CLT (needed for brew + git)
#   2. macOS hostname set to `mattmacpro` (must precede tailscale up
#      or your tailnet hostname will be `mattmacpro-local` or similar)
#   3. Homebrew (needed for tailscale, gh)
#   4. Tailscale install + auth (you can now SSH in and copy-paste)
#   5. gh install + auth + dotfiles clone
#   6. Nix
#   7. Buildkite agent token → /etc/buildkite-agent/agent-token
#   8. darwin-rebuild switch (agents start immediately)
#   9. Sanity checks
#
# Security posture: zero standing 1Password service-account tokens on
# this host. mattmacpro runs untrusted CI workflows via the
# self-hosted Buildkite agent pool, so any SA token on disk is a
# credential a compromised agent could exfiltrate. The agent token in
# step 7 is the only secret here, and it's mode-600 root-wheel
# (consider Keychain-with-restricted-ACL encryption later; macOS has
# no systemd-creds counterpart).
#
# Re-runnable: every step skips if already done.
#
# Usage:
#   bash mattmacpro-bootstrap.sh
#   bash mattmacpro-bootstrap.sh --auth-key tskey-auth-...
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash mattmacpro-bootstrap.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"
NIX_CONFIG_DIR="$HOME/repos/zireael/nix-config"
SEALED_TOKEN_FILE="/etc/buildkite-agent/agent-token"
TARGET_HOSTNAME="mattmacpro"
TS_HOSTNAME="${TARGET_HOSTNAME}.tail08a5c5.ts.net"

require_non_root
parse_tailscale_auth_key "$@"
require_hostname mattmacpro

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
# `tailscale up` picks up `mattmacpro.local` (the LocalHostName /
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
# 3. Homebrew (x86_64 prefix /usr/local)
# ---------------------------------------------------------------------
# Moved before Tailscale + gh because both come from brew. The cask
# install in step 4 needs `brew` on PATH; the formula installs in
# steps 4+5 too.
step "Homebrew"
if [ -x /usr/local/bin/brew ]; then
	echo "  already installed"
else
	echo "  installing — you'll be prompted for sudo"
	/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
	[ -x /usr/local/bin/brew ] || err "brew not at /usr/local/bin/brew after install"
fi
eval "$(/usr/local/bin/brew shellenv)"

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
	sudo rm -f /usr/local/bin/tailscale 2>/dev/null || true
fi

if [ -x /usr/local/sbin/tailscaled ] && [ -x /usr/local/bin/tailscale ]; then
	echo "  tailscale formula already installed ($(/usr/local/bin/tailscale version | head -1))"
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
if sudo /usr/local/bin/tailscale status >/dev/null 2>&1; then
	echo "  Already authenticated: $(sudo /usr/local/bin/tailscale ip -4 2>/dev/null | head -1)"
else
	if [ -z "${TAILSCALE_AUTH_KEY:-}" ]; then
		echo "  Generate a pre-auth key at:"
		echo "    https://login.tailscale.com/admin/settings/keys"
		echo "  Single-use, tagged 'tag:ci-runner'. Paste below (input hidden):"
		read -r -s TAILSCALE_AUTH_KEY
		echo
	fi
	[ -n "${TAILSCALE_AUTH_KEY:-}" ] || err "no auth key provided"
	sudo /usr/local/bin/tailscale up \
		--auth-key="$TAILSCALE_AUTH_KEY" \
		--ssh \
		--hostname="$TARGET_HOSTNAME"
	unset TAILSCALE_AUTH_KEY
	echo "  Tailnet IP: $(sudo /usr/local/bin/tailscale ip -4 2>/dev/null | head -1)"
fi

cat <<EOF

================================================================
Tailscale is up.

From your laptop you can now SSH in instead of using the console:
    ssh mattw@${TS_HOSTNAME}

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
# 6. Nix (upstream installer from nixos.org)
# ---------------------------------------------------------------------
# We use the official upstream installer from nixos.org rather than
# Determinate Systems' installer. The Determinate installer is the
# default on the MBP, but for x86_64-darwin (Mac Pro 2013's arch)
# it errors with "x86_64-darwin not supported" — Determinate has
# narrowed their platform support. Upstream Nix supports
# x86_64-darwin officially.
#
# `--daemon` selects the multi-user install (per-user is deprecated
# on macOS). The installer creates the nixbld* build users, the
# /nix synthetic volume, and the launchd plist for nix-daemon.
step "Nix"
if command -v nix >/dev/null 2>&1; then
	echo "  already installed ($(nix --version | head -1))"
else
	echo "  installing — you'll be prompted for sudo and asked to confirm"
	curl --proto '=https' --tlsv1.2 -sSf -L https://nixos.org/nix/install |
		sh -s -- --daemon
	# Installer adds /nix/var/nix/profiles/default/bin to PATH via
	# /etc/zshrc, but the current shell hasn't sourced it.
	# shellcheck disable=SC1091
	[ -f /etc/zshrc ] && . /etc/zshrc 2>/dev/null || true
	export PATH="/nix/var/nix/profiles/default/bin:$PATH"
	command -v nix >/dev/null 2>&1 || err "nix still not on PATH after install"
	echo "  done ($(nix --version | head -1))"
fi

# ---------------------------------------------------------------------
# 7. Buildkite agent token → /etc/buildkite-agent/agent-token
# ---------------------------------------------------------------------
# Must exist BEFORE darwin-rebuild because the agent launchd daemons
# read it at launch (see darwin/mattmacpro/system.nix). The decrypt-
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
	sudo install -m 600 /dev/stdin "$SEALED_TOKEN_FILE" <<<"$BK_TOKEN"
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
# 8. darwin-rebuild switch
# ---------------------------------------------------------------------
# Lays down:
#   - Tailscale cask (re-confirmation; already installed in step 4)
#   - Tailscale CLI symlink at /usr/local/bin/tailscale (re-confirmation)
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
	echo "  nix run nix-darwin -- switch --flake .#mattmacpro"
	sudo HOME="$HOME" nix run --extra-experimental-features 'nix-command flakes' \
		nix-darwin -- switch --flake "$NIX_CONFIG_DIR#mattmacpro" --show-trace
else
	step "darwin-rebuild switch --flake .#mattmacpro"
	sudo HOME="$HOME" darwin-rebuild switch \
		--flake "$NIX_CONFIG_DIR#mattmacpro" \
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
if [ -x /usr/local/bin/tailscale ]; then
	/usr/local/bin/tailscale status 2>&1 | head -5
else
	warn "  /usr/local/bin/tailscale missing — brew install tailscale failed?"
fi

echo "[buildkite-agents]"
for svc in com.sealedsecurity.buildkite-agent-sealed-macos \
	com.sealedsecurity.buildkite-agent-sealed-macos-2; do
	if sudo launchctl list 2>/dev/null | grep -q "$svc"; then
		echo "  $svc: loaded"
	else
		warn "  $svc: not loaded"
	fi
done

echo "[glances]"
if sudo launchctl list 2>/dev/null | grep -q com.sealedsecurity.glances; then
	echo "  glances: loaded (reachable at https://mattmacpro.tail08a5c5.ts.net:9443/)"
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
#   1. mattw's Keychain entry "OP_SERVICE_ACCOUNT_TOKEN" — still
#      under mattw's UID-scoped ACL by default, so the _buildkite-agent
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
	warn "  mattw keychain still has OP_SERVICE_ACCOUNT_TOKEN — delete with:"
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

Tailscale: $(/usr/local/bin/tailscale status --json 2>/dev/null | grep -o '"DNSName":"[^"]*"' | head -1 | sed 's/"DNSName":"\(.*\).$/\1/' || echo "$TS_HOSTNAME")

Verify the agents registered at:
  https://buildkite.com/organizations/sealedsecurity/agents

They should appear with the tag:
  queue=macos-x64-selfhosted
EOF

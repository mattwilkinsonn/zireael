#!/usr/bin/env bash
# First-boot bootstrap for a fresh Raspberry Pi (rpi4 or rpi5).
# Run AFTER booting the SD image / NixOS install, ON the Pi itself.
#
# Handles:
#   1. System clock — Pi 4 has no RTC; first boot's clock is months old.
#      Set it manually so TLS handshakes work for everything that follows.
#      (rpi4's nix config also pins NTP servers as IPs, but that doesn't
#      help the very first boot before nix-switch runs.)
#   2. Tailscale — join the tailnet using a pre-auth key (no browser needed
#      since this Pi is headless). Pass --auth-key=tskey-... or set
#      TAILSCALE_AUTH_KEY env var; otherwise the script prompts.
#   3. Dotfiles repo — colocated git+jj at ~/.git, work-tree at $HOME.
#      Pre-existing files that conflict get backed up to ~/.dotfiles-backup.
#   4. op-pi-svc service-account token — encrypt via systemd-creds at
#      /etc/op-pi-svc-token.cred so the env-refresh services can decrypt it
#      to fetch container env from 1P. Intentionally NOT exposed to the
#      user shell — Pi shells skip auto-loading 1P secrets (see
#      nixos/home.nix). Container services consume this credential via
#      LoadCredentialEncrypted= in their unit definitions.
#   5. /boot/firmware mount — rpi4-only quirk for uboot-builder.
#   6. nixos-rebuild switch — apply the host config (auto-detected from
#      `hostname`).
#   7. Password rotation — replace baked-in initialHashedPassword for
#      `mattw` and `root` with the value from op://Server/Pi-password.
#   8. Inter-server SSH key — fetch the dedicated `inter-server`
#      private key from 1P "Server" vault to
#      ~/.ssh/id_ed25519_inter_server (mode 600) for host-to-host SSH
#      automation between managed NixOS hosts (ad-hoc scp/ssh, podman
#      remote between hosts, future automation). Same key reused on
#      every NixOS host this script bootstraps; the public key in
#      nixos/common.nix's authorizedKeys is shared across hosts. Skips
#      cleanly if the 1P item doesn't exist yet — re-run the script
#      after creating it.
#   9. Sanity checks — services up, DNS resolves, secrets in env file.
#
# Re-runnable: each step skips if already done. Safe to re-run if
# something fails partway.
#
# Usage:
#   bash rpi-bootstrap.sh                              # interactive prompts
#   TAILSCALE_AUTH_KEY=tskey-auth-... bash rpi-bootstrap.sh
#   bash rpi-bootstrap.sh --auth-key tskey-auth-...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR/../../shared/scripts
# shellcheck source=../../shared/scripts/bootstrap-common.sh
source "$SCRIPT_DIR/../../shared/scripts/bootstrap-common.sh"

ZIREAEL_REPO_SLUG="mattwilkinsonn/zireael"

# Refuse to run as root. The script uses `sudo` internally for the
# specific commands that need root (date set, tailscale up, nixos-rebuild,
# chpasswd, systemd-creds decrypt). Running the whole script under sudo
# sets $HOME=/root, which makes the zireael checkout, ~/.ssh/ writes,
# and gh auth all target /root instead of /home/mattw — silently
# duplicating work and re-prompting for credentials.
if [ "$EUID" = "0" ]; then
	err "Don't run this script as root/sudo. Run as your normal user — sudo is invoked internally where needed."
fi

# Parse --auth-key flag
TAILSCALE_AUTH_KEY="${TAILSCALE_AUTH_KEY:-}"
while [[ $# -gt 0 ]]; do
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

HOST="$(hostname)"
case "$HOST" in
rpi4 | rpi5) ;;
*) err "hostname '$HOST' not recognized — expected rpi4 or rpi5. Set it via systemd-firstboot or the SD image config." ;;
esac

echo "Bootstrapping host: $HOST"

# ---------------------------------------------------------------------
# 1. System clock
# ---------------------------------------------------------------------
# Sanity-check: if year is < 2026, clock is definitely wrong (these Pis
# were first deployed in 2026). Pi 4 routinely boots with stale clock
# because no RTC battery in the Canakit case. Clock-wrong → TLS fails →
# NTP can't resolve servers → Tailscale auth fails → everything cascades.
#
# Auto-recover by setting clock to a known-good recent date. Doesn't need
# to be exact — NTP fine-tunes after Tailscale brings up DNS. The
# hardcoded date below just needs to be after all relevant TLS certs were
# issued (Tailscale certs from late April 2026, etc.). Update this when
# editing the script.
LAST_KNOWN_GOOD_DATE='2026-05-04 12:00:00'
CURRENT_YEAR=$(date +%Y)
if [ "$CURRENT_YEAR" -lt 2026 ]; then
	step "Clock is wrong (year $CURRENT_YEAR). Setting to last-known-good ($LAST_KNOWN_GOOD_DATE)."
	sudo date -s "$LAST_KNOWN_GOOD_DATE"
	echo "Clock set to: $(date) (NTP will fine-tune once DNS is up)"
	sudo systemctl restart systemd-timesyncd 2>/dev/null || true
else
	step "Clock looks sane ($(date -Iseconds))"
fi

# ---------------------------------------------------------------------
# 2. Tailscale
# ---------------------------------------------------------------------
if tailscale status &>/dev/null; then
	step "Tailscale already authenticated ($(tailscale ip -4 2>/dev/null || echo 'no IP yet'))"
else
	step "Joining tailnet"
	if [ -z "$TAILSCALE_AUTH_KEY" ]; then
		echo "No auth key provided. Generate one at:"
		echo "  https://login.tailscale.com/admin/settings/keys"
		echo "Paste the tskey-auth-... string (input hidden):"
		read -r -s TAILSCALE_AUTH_KEY
		echo
	fi
	[ -n "$TAILSCALE_AUTH_KEY" ] || err "no auth key provided"
	sudo tailscale up --auth-key="$TAILSCALE_AUTH_KEY"
	echo "Tailscale up: $(tailscale ip -4)"
fi

# ---------------------------------------------------------------------
# 3. Dotfiles repo
# ---------------------------------------------------------------------
clone_zireael_via_gh "$ZIREAEL_REPO_SLUG"

# ---------------------------------------------------------------------
# 4. op-pi-svc service-account token
# ---------------------------------------------------------------------
if [ -f /etc/op-pi-svc-token.cred ]; then
	step "op-pi-svc token already encrypted at /etc/op-pi-svc-token.cred"
else
	step "Encrypting op-pi-svc service-account token"
	echo "Generate the token (or fetch existing) at:"
	echo "  https://my.1password.com/developer-tools/serviceaccounts"
	echo "Service account should have read access to the 'Pi' vault only."
	sudo bash "$HOME/repos/zireael/nix-config/nixos/scripts/rpi-encrypt-op-token.sh"
fi

# ---------------------------------------------------------------------
# 5. /boot/firmware mount (rpi4-specific quirk)
# ---------------------------------------------------------------------
# The standard sd-image-aarch64 module boots via extlinux (writes to
# /boot/extlinux/) and doesn't pre-create /boot/firmware as a mount
# point. raspberry-pi-4.base's uboot-builder writes vendor firmware to
# /boot/firmware though, so we need that mount in place BEFORE
# nixos-rebuild runs (chicken-and-egg: the rebuild itself sets up the
# fileSystems config, but the bootloader install needs the mount during
# the rebuild). New SD images built from the post-fix config (with
# fileSystems."/boot/firmware" declared and sdImage.firmwareSize=256)
# will mount this automatically on first boot — but this is defensive
# in case an older image is being bootstrapped.
if [ "$HOST" = "rpi4" ]; then
	if ! mountpoint -q /boot/firmware; then
		step "Mounting FIRMWARE partition at /boot/firmware (uboot-builder needs it)"
		sudo mkdir -p /boot/firmware
		sudo mount -L FIRMWARE /boot/firmware || err "couldn't mount FIRMWARE partition"
	else
		step "/boot/firmware already mounted"
	fi
fi

# ---------------------------------------------------------------------
# 6. First nixos-rebuild switch
# ---------------------------------------------------------------------
step "Running nixos-rebuild switch --flake .#$HOST"
sudo nixos-rebuild switch \
	--flake "$HOME/repos/zireael/nix-config#$HOST" \
	--show-trace

# ---------------------------------------------------------------------
# 7. Rotate user + root passwords from 1Password
# ---------------------------------------------------------------------
# Replace the initialHashedPassword from nixos/common.nix (which is the
# same baked-in value across every fresh SD image) with the actual
# password stored in 1P. Same value for `mattw` and `root` on both Pis —
# memorable, hand-typeable, single 1P item.
#
# `initialHashedPassword` (vs `hashedPassword`) means nix only applies it
# at user *creation*, not on every rebuild — so chpasswd rotations stick.
#
# Always runs: chpasswd is naturally idempotent, and re-running bootstrap
# will pick up any rotation done in 1P. The op-pi-svc service account
# must have read access to op://Server/Pi-password.
step "Rotating user + root passwords from 1P"

if [ ! -f /etc/op-pi-svc-token.cred ]; then
	err "op-pi-svc token missing at /etc/op-pi-svc-token.cred — step 4 should have created it"
fi

# Decrypt the systemd-creds-encrypted token. systemd-creds decrypt works
# standalone as root (host-bound encryption via machine ID). --name= must
# match the value passed to `systemd-creds encrypt --name=` (see
# rpi-encrypt-op-token.sh) — without it, decrypt falls back to filename
# inference (op-pi-svc-token), which mismatches the embedded op-pi-svc.
OP_TOKEN="$(sudo systemd-creds decrypt --name=op-pi-svc /etc/op-pi-svc-token.cred -)"
[ -n "$OP_TOKEN" ] || err "decrypted op-pi-svc token is empty"

PI_PASSWORD="$(OP_SERVICE_ACCOUNT_TOKEN="$OP_TOKEN" op read 'op://Server/Pi Root and User Password/password')"
[ -n "$PI_PASSWORD" ] || err "Pi-password from 1P is empty — check the 'Pi Root and User Password' item exists in the Server vault and op-pi-svc has read access"

sudo chpasswd <<EOF
mattw:$PI_PASSWORD
root:$PI_PASSWORD
EOF
unset PI_PASSWORD OP_TOKEN
echo "  mattw + root passwords rotated"

# ---------------------------------------------------------------------
# 8. Inter-server SSH key
# ---------------------------------------------------------------------
# Generic host-to-host SSH key for automation between managed NixOS
# hosts (ad-hoc scp/ssh, podman remote between hosts, future tooling).
# Dedicated ed25519 keypair in 1P, separate from matt's personal
# mattw@1password key — smaller blast radius, no shared trust between
# "personal Mac access" and "server-internal automation". Same key
# reused on every NixOS host bootstrapped by this script; the matching
# public key is in nixos/common.nix's authorizedKeys, so every host
# that imports common.nix trusts it.
INTER_SERVER_KEY="$HOME/.ssh/id_ed25519_inter_server"
if [ -f "$INTER_SERVER_KEY" ]; then
	step "Inter-server SSH key already at $INTER_SERVER_KEY"
else
	step "Fetching inter-server SSH key from 1Password"

	OP_TOKEN="$(sudo systemd-creds decrypt --name=op-pi-svc /etc/op-pi-svc-token.cred -)"
	[ -n "$OP_TOKEN" ] || err "decrypted op-pi-svc token is empty"

	# Probe-read first so a missing 1P item produces a clean skip rather
	# than a half-written empty key file. Re-run the script after creating
	# the item.
	if ! OP_SERVICE_ACCOUNT_TOKEN="$OP_TOKEN" op read \
		"op://Server/inter-server/private key?ssh-format=openssh" >/dev/null 2>&1; then
		warn "1P item 'inter-server' not readable (item missing or no service-account access)"
		echo "  Create an SSH key item named 'inter-server' in the 'Pi' vault,"
		echo "  ensure op-pi-svc has read access, then re-run this script."
		unset OP_TOKEN
	else
		mkdir -p "$HOME/.ssh"
		# umask-077 subshell so the file is mode 600 at creation rather
		# than after the fact (no readable-window race). Redirection
		# happens as $USER, so the file lands owned by mattw.
		(umask 077 && OP_SERVICE_ACCOUNT_TOKEN="$OP_TOKEN" op read \
			"op://Server/inter-server/private key?ssh-format=openssh" \
			>"$INTER_SERVER_KEY")
		chmod 600 "$INTER_SERVER_KEY"
		[ -s "$INTER_SERVER_KEY" ] || err "inter-server private key file empty"
		echo "  $INTER_SERVER_KEY written (mode 600)"
		unset OP_TOKEN
	fi
fi

# ---------------------------------------------------------------------
# 9. Sanity checks
# ---------------------------------------------------------------------
step "Sanity checks"

if [ "$HOST" = "rpi4" ]; then
	echo "[technitium-dns-server]"
	if systemctl is-active technitium-dns-server >/dev/null; then
		echo "  technitium-dns-server: active"
	else
		warn "  technitium-dns-server not active"
	fi
	if systemctl is-active technitium-seed >/dev/null; then
		echo "  technitium-seed: active"
	else
		warn "  technitium-seed not active (may still be running)"
	fi

	echo "[DNS test]"
	# getent uses the system's actual resolver (nsswitch → /etc/resolv.conf
	# → Technitium on 127.0.0.1), so this exercises the real path. Avoids
	# depending on `dig` being on $PATH (it's not in systemPackages).
	if getent hosts example.com >/dev/null 2>&1; then
		echo "  DNS resolution works (via system resolver → Technitium)"
	else
		warn "  DNS resolution failed"
	fi

elif [ "$HOST" = "rpi5" ]; then
	echo "[openclaw]"
	if systemctl is-active podman-openclaw >/dev/null; then
		echo "  podman-openclaw: active"
	else
		warn "  podman-openclaw not active"
	fi

	# openclaw.env is mode 640 1000:100 — mattw IS in group 100 so direct
	# read works, but be defensive in case perms drift. `sudo cat | wc -l`
	# (not `sudo wc -l <file`) because sudo doesn't elevate the redirect.
	OPENCLAW_LINES=$(sudo cat /srv/openclaw/openclaw.env 2>/dev/null | wc -l || echo 0)
	if [ "$OPENCLAW_LINES" -gt 0 ]; then
		echo "  openclaw.env: populated ($OPENCLAW_LINES lines)"
	else
		warn "  openclaw.env empty — openclaw-env-refresh.service may have failed"
	fi
fi

echo "[cockpit]"
if systemctl is-active cockpit.socket >/dev/null; then
	echo "  cockpit.socket: active"
else
	warn "  cockpit.socket not active"
fi

echo "[tailscale]"
tailscale status | head -5

cat <<EOF

\033[1;32m✓ Bootstrap complete for $HOST.\033[0m

URLs (tailnet only):
  https://$HOST.tail08a5c5.ts.net:9443/   Cockpit web UI
EOF

if [ "$HOST" = "rpi4" ]; then
	cat <<EOF
  https://$HOST.tail08a5c5.ts.net/        Technitium admin UI

First-time setup:
  - Admin password is set from 1Password (op://Server/Technitium Admin Password).
    Default admin/admin is rotated automatically on first boot.
  - Currently tailnet-only DNS. To make it LAN-wide, point your router's
    DHCP DNS server at 192.168.1.50.
EOF
elif [ "$HOST" = "rpi5" ]; then
	cat <<EOF
  https://rpi.tail08a5c5.ts.net/                OpenClaw Control UI
EOF
fi

cat <<EOF

For Cockpit federation: log into the OTHER Pi's Cockpit and add this
host via top-left dropdown → "Add new host" → $HOST.tail08a5c5.ts.net
EOF

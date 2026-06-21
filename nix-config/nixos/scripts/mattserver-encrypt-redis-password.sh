#!/usr/bin/env bash
# One-time bootstrap on mattserver: encrypt the sccache Redis password
# via systemd-creds so the decrypt-redis-password.service oneshot can
# stage it at boot without a plaintext password on disk.
#
# Output is host-bound (machine-ID-keyed encryption) — only this host
# can decrypt. Same posture as the agent token: no hardware TPM, but the
# machine ID gates against "rsync /etc/buildkite-agent off the box, read
# elsewhere" attacks.
#
# The password is the sccache Redis `requirepass` (defense-in-depth on
# top of the tailnet ACL — see the services.redis block in
# nixos/mattserver/system.nix). The SAME value must be set as the
# SCCACHE_REDIS_PASSWORD Buildkite secret so the CI pipelines can
# authenticate. Generate a strong random value once (e.g.
# `openssl rand -base64 32`), paste it here, and store the identical
# value in Buildkite secrets.
#
# Run once when first enabling the password, OR whenever it is rotated
# (rotate here AND in Buildkite secrets, then restart — see the end).
#
# Usage:  sudo bash nixos/scripts/mattserver-encrypt-redis-password.sh

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

OUT=/etc/buildkite-agent/redis-password.cred

mkdir -p "$(dirname "$OUT")"

if [ -f "$OUT" ]; then
	read -r -p "$OUT already exists. Overwrite? [y/N] " yn
	case "$yn" in
	[Yy]*) ;;
	*)
		echo "Aborted."
		exit 0
		;;
	esac
fi

echo
echo "Paste the sccache Redis password. This MUST match the"
echo "SCCACHE_REDIS_PASSWORD Buildkite secret the CI pipelines use."
echo "Tip: generate one with: openssl rand -base64 32"
echo "(Input is hidden. Press Enter when done.)"
echo
read -r -s PASSWORD

if [ -z "$PASSWORD" ]; then
	echo "ERROR: no password provided"
	exit 1
fi

# --name= must match the value decrypt-redis-password.service passes to
# `systemd-creds decrypt --name=<id>` (see nixos/mattserver/system.nix).
echo -n "$PASSWORD" | systemd-creds encrypt --name=redis-sccache-password - "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

systemd-creds decrypt --name=redis-sccache-password "$OUT" - >/dev/null
echo "Decrypt OK."

echo
echo "Done. decrypt-redis-password.service stages this at boot to"
echo "/run/redis-password/requirepass, which redis-sccache reads via"
echo "requirePassFile. After a rotation (here AND in Buildkite secrets):"
echo "  sudo systemctl restart decrypt-redis-password.service"
echo "  sudo systemctl restart redis-sccache.service"

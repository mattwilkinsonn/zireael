#!/usr/bin/env bash
# One-time bootstrap on mattserver: encrypt the sealedsecurity-ci
# GitHub App private key (.pem) via systemd-creds so the agent's git
# credential helper can decrypt it at boot without a plaintext key on
# disk.
#
# Output is host-bound (machine-ID-keyed encryption) — only this host
# can decrypt. Good enough to gate against "rsync /etc/buildkite-agent
# off the box, read the key elsewhere".
#
# The key is the App's downloaded .pem (GitHub → org → Developer
# settings → GitHub Apps → sealedsecurity-ci → Private keys → Generate
# a private key). It's what the credential helper signs the JWT with to
# mint installation tokens for cloning. Run once at setup, or whenever
# the App key is rotated.
#
# Usage:
#   sudo bash nixos/scripts/mattserver-encrypt-ci-app-key.sh /path/to/sealedsecurity-ci.pem

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

PEM="${1:-}"
if [ -z "$PEM" ] || [ ! -r "$PEM" ]; then
	echo "ERROR: pass the path to the sealedsecurity-ci .pem as arg 1"
	echo "Usage: sudo bash $0 /path/to/sealedsecurity-ci.pem"
	exit 1
fi

OUT=/etc/buildkite-agent/ci-app-key.pem.cred

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

# --name= must match the value decrypt-ci-app-key.service passes to
# `systemd-creds decrypt --name=<id>` (see nixos/mattserver/system.nix).
systemd-creds encrypt --name=buildkite-ci-app-key "$PEM" "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

systemd-creds decrypt --name=buildkite-ci-app-key "$OUT" - >/dev/null
echo "Decrypt OK."

echo
echo "Done. The decrypt-agent-token.service oneshot (which stages both"
echo "the agent token and this App key) decrypts this at boot to"
echo "/run/buildkite-agent/ci-app-key.pem for the git credential helper."
echo "Restart to pick up a rotation:"
echo "  sudo systemctl restart decrypt-agent-token.service"
echo "  sudo systemctl restart 'buildkite-agent-sealed*.service'"
echo
echo "Remember to shred the plaintext .pem you passed in once this"
echo "succeeds: shred -u $PEM"

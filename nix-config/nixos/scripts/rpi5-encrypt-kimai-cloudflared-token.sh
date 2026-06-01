#!/usr/bin/env bash
# One-time bootstrap on rpi5: encrypt the Cloudflare Tunnel token for
# hours.sealedsecurity.com via systemd-creds so cloudflared-kimai.service
# can decrypt it at runtime without storing the token in Nix or plaintext
# on disk.
#
# The encrypted file is host-bound — only this rpi5 install can decrypt it.
# Pi 5 has no hardware TPM, so encryption uses the host machine ID as the key.
# Still meaningfully better than plaintext on disk.
#
# Cloudflare setup expected before running this:
#   1. Cloudflare Zero Trust → Networks → Tunnels → Create tunnel.
#   2. Choose Cloudflared and copy the connector token.
#   3. Add Public Hostname:
#        hours.sealedsecurity.com → http://localhost:8080
#   4. Recommended: put Cloudflare Access in front of the hostname and allow
#      only Matt + the contractor.
#
# Usage on rpi5:
#   sudo bash nixos/scripts/rpi5-encrypt-kimai-cloudflared-token.sh

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

OUT_DIR=/etc/cloudflared
OUT="$OUT_DIR/hours-sealedsecurity-com-token.cred"
CRED_NAME=tunnel-token

mkdir -p "$OUT_DIR"
chmod 700 "$OUT_DIR"
chown root:root "$OUT_DIR"

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
echo "Paste the Cloudflare Tunnel connector token for hours.sealedsecurity.com:"
echo "(Input is hidden. Press Enter when done.)"
echo
read -r -s TOKEN

if [ -z "$TOKEN" ]; then
	echo "ERROR: no token provided"
	exit 1
fi

# Remotely-managed Cloudflare Tunnel tokens are JWT-like strings. Keep this as
# a warning, not a hard failure, in case Cloudflare changes the format.
if [[ ! $TOKEN =~ ^eyJ ]]; then
	echo "WARNING: token doesn't look like a Cloudflare connector JWT"
	read -r -p "Continue anyway? [y/N] " yn
	case "$yn" in [Yy]*) ;; *) exit 1 ;; esac
fi

echo -n "$TOKEN" | systemd-creds encrypt --name="$CRED_NAME" - "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

DECRYPTED_TOKEN=$(systemd-creds decrypt --name="$CRED_NAME" "$OUT" -)
if [ "$DECRYPTED_TOKEN" != "$TOKEN" ]; then
	echo "ERROR: decrypt verification failed (content mismatch)"
	exit 1
fi
echo "Decrypt OK."
unset DECRYPTED_TOKEN

echo
echo "Done. Deploy/restart with:"
echo "  sudo nixos-rebuild switch --flake ~/repos/zireael/nix-config#rpi5 --show-trace"
echo "  sudo systemctl restart cloudflared-kimai.service"
echo

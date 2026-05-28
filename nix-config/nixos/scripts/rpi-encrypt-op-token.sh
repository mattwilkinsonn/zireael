#!/usr/bin/env bash
# One-time bootstrap on the Pi: encrypt the Pi-svc 1Password service
# account token via systemd-creds so the openclaw-env-refresh.service
# can decrypt it at runtime to fetch container env vars from 1Password.
#
# The encrypted file is host-bound — only this Pi can decrypt. Pi 5
# has no hardware TPM, so encryption uses the host machine ID as the
# key. Still meaningfully better than plaintext on disk.
#
# Run once when first setting up Pi-svc, OR if the service account
# token is rotated in 1Password.
#
# Usage:  sudo bash nixos/scripts/rpi-encrypt-op-token.sh

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

OUT=/etc/op-pi-svc-token.cred

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
echo "Paste the Pi-svc service account token (starts with 'ops_eyJ...'):"
echo "(Input is hidden. Press Enter when done.)"
echo
read -r -s TOKEN

if [ -z "$TOKEN" ]; then
	echo "ERROR: no token provided"
	exit 1
fi

if [[ ! $TOKEN =~ ^ops_ ]]; then
	echo "WARNING: token doesn't start with 'ops_' — may not be a service account token"
	read -r -p "Continue anyway? [y/N] " yn
	case "$yn" in [Yy]*) ;; *) exit 1 ;; esac
fi

echo -n "$TOKEN" | systemd-creds encrypt --name=op-pi-svc - "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

DECRYPTED_PREFIX=$(systemd-creds decrypt --name=op-pi-svc "$OUT" - | head -c 12)
echo "Decrypt OK. Token starts with: ${DECRYPTED_PREFIX}..."

echo
echo "Done. The openclaw-env-refresh.service can now decrypt this credential"
echo "via LoadCredentialEncrypted= in its unit definition."
echo
echo "Next: nix-switch to deploy the env-refresh service, then:"
echo "  sudo systemctl restart openclaw-env-refresh.service"
echo "  sudo systemctl restart podman-openclaw.service"

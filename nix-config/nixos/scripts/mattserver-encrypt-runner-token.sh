#!/usr/bin/env bash
# One-time bootstrap on mattserver: encrypt the GitHub Actions runner
# PAT via systemd-creds so the runner units can decrypt it at boot
# without a plaintext token on disk.
#
# Output is host-bound (machine-ID-keyed encryption) — only this host
# can decrypt. Mattserver has no hardware TPM, but the machine ID is
# good enough to gate against "rsync /etc/github-runner off the box,
# read elsewhere" attacks.
#
# Run once when first setting up the runners, OR whenever the PAT is
# rotated in GitHub.
#
# Usage:  sudo bash nixos/scripts/mattserver-encrypt-runner-token.sh

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

OUT=/etc/github-runner/sealed-token.cred
OLD_PLAINTEXT=/etc/github-runner/sealed-token

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
echo "Paste the GitHub PAT for the sealed runner pool (ghp_... or"
echo "github_pat_...). Required scope: manage_runners:org (fine-"
echo "grained) or admin:org (classic)."
echo "(Input is hidden. Press Enter when done.)"
echo
read -r -s TOKEN

if [ -z "$TOKEN" ]; then
	echo "ERROR: no token provided"
	exit 1
fi

# --name= must match the value the consuming unit passes to
# LoadCredentialEncrypted=<id>:<path>. The systemd-creds docs require
# the encrypt-time name and decrypt-time name to match.
echo -n "$TOKEN" | systemd-creds encrypt --name=sealed-runner-token - "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

DECRYPTED_PREFIX=$(systemd-creds decrypt --name=sealed-runner-token "$OUT" - | head -c 12)
echo "Decrypt OK. Token starts with: ${DECRYPTED_PREFIX}..."

# Refuse to leave plaintext on disk after a successful encrypt.
if [ -f "$OLD_PLAINTEXT" ]; then
	echo
	echo "Found pre-existing plaintext at $OLD_PLAINTEXT"
	read -r -p "Remove it now? [Y/n] " yn
	case "$yn" in
	[Nn]*)
		echo "Left $OLD_PLAINTEXT in place. Remember to remove it manually."
		;;
	*)
		shred -u "$OLD_PLAINTEXT"
		echo "Removed (shredded) $OLD_PLAINTEXT"
		;;
	esac
fi

echo
echo "Done. The runner units will decrypt this credential at boot via"
echo "the decrypt-runner-token.service oneshot (declared in"
echo "nixos/mattserver/system.nix). Restart the runners to pick up:"
echo "  sudo systemctl restart decrypt-runner-token.service"
echo "  sudo systemctl restart 'github-runner-sealed*.service'"

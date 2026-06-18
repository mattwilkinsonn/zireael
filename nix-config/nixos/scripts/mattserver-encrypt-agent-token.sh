#!/usr/bin/env bash
# One-time bootstrap on mattserver: encrypt the Buildkite agent token
# via systemd-creds so the buildkite-agent units can decrypt it at boot
# without a plaintext token on disk.
#
# Output is host-bound (machine-ID-keyed encryption) — only this host
# can decrypt. Mattserver has no hardware TPM, but the machine ID is
# good enough to gate against "rsync /etc/buildkite-agent off the box,
# read elsewhere" attacks.
#
# Run once when first setting up the agents, OR whenever the agent token
# is rotated in the Buildkite org settings.
#
# The token is the Buildkite AGENT token from the org's Agents page
# (Agents → Reveal Agent Token), NOT the BUILDKITE_API_TOKEN the `bk`
# CLI uses. See the buildkite-agents handoff doc.
#
# Usage:  sudo bash nixos/scripts/mattserver-encrypt-agent-token.sh

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
	echo "ERROR: must run as root (sudo)"
	exit 1
fi

if ! command -v systemd-creds >/dev/null; then
	echo "ERROR: systemd-creds not found (need systemd 250+)"
	exit 1
fi

OUT=/etc/buildkite-agent/agent-token.cred

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
echo "Paste the Buildkite AGENT token (org Agents page → Reveal Agent"
echo "Token). This is the org-wide agent registration token, NOT the"
echo "BUILDKITE_API_TOKEN used by the bk CLI."
echo "(Input is hidden. Press Enter when done.)"
echo
read -r -s TOKEN

if [ -z "$TOKEN" ]; then
	echo "ERROR: no token provided"
	exit 1
fi

# --name= must match the value the consuming unit passes to
# `systemd-creds decrypt --name=<id>`. The encrypt-time name and
# decrypt-time name have to match (see decrypt-agent-token.service in
# nixos/mattserver/system.nix).
echo -n "$TOKEN" | systemd-creds encrypt --name=buildkite-agent-token - "$OUT"
chmod 600 "$OUT"
chown root:root "$OUT"

echo
echo "Encrypted to $OUT"
echo "Verifying decrypt..."

DECRYPTED_PREFIX=$(systemd-creds decrypt --name=buildkite-agent-token "$OUT" - | head -c 8)
echo "Decrypt OK. Token starts with: ${DECRYPTED_PREFIX}..."

echo
echo "Done. The buildkite-agent units decrypt this credential at boot"
echo "via the decrypt-agent-token.service oneshot (declared in"
echo "nixos/mattserver/system.nix). Restart to pick up a rotation:"
echo "  sudo systemctl restart decrypt-agent-token.service"
echo "  sudo systemctl restart 'buildkite-agent-sealed*.service'"

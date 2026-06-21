#!/usr/bin/env bash
# Sync the home-manager tree to a mounted Ventoy drive so a fresh
# install can run the bootstrap scripts directly from USB.
#
# Usage:
#   bash darwin/scripts/sync-ventoy.sh                       # default mount
#   bash darwin/scripts/sync-ventoy.sh /Volumes/MyVentoy     # custom mount
#
# Bootstrap entry points after sync, run from the freshly-installed
# target machine with Ventoy plugged in:
#   bash /Volumes/Ventoy/home-manager/darwin/scripts/mac-setup.sh
#   # Windows: \\?\Volume{...}\home-manager\windows\windows-setup.ps1
#
# This syncs the personal-host tree (MBP + WSL). The sealedsecurity
# fleet (CI runners + inference box) bootstraps from the sealed repo —
# see sealed/infra/nix/nixos/scripts/ + darwin/scripts/.
#
# Mac-only — Ventoy's authoring side is where the tree gets edited;
# Linux hosts are read-only consumers. Run from Linux too if/when it
# makes sense (the rsync command itself is portable; the default mount
# path isn't).
set -euo pipefail

VENTOY_MOUNT="${1:-/Volumes/Ventoy}"
SRC="$HOME/repos/zireael/nix-config/"
DEST="$VENTOY_MOUNT/nix-config/"

if [ ! -d "$VENTOY_MOUNT" ]; then
	echo "Ventoy not mounted at $VENTOY_MOUNT" >&2
	echo "Pass a different path as the first arg if it's mounted elsewhere." >&2
	exit 1
fi

# Excludes:
#   .DS_Store          — macOS Finder metadata; meaningless on the drive
rsync -ahv --delete \
	--exclude '.DS_Store' \
	"$SRC" "$DEST"

echo ""
echo "Synced $SRC → $DEST"

#!/usr/bin/env bash
# Pi 5 image build / remote deploy helper.
#
# Modes:
#   image          Build the nixos-raspberrypi installer SD/SSD image.
#                  Output is a symlink at ./result; flash result/sd-image/*.img
#                  with `dd` or Raspberry Pi Imager.
#   deploy [host]  Run nixos-rebuild against a Pi already on the network,
#                  building remotely (avoids cross-compiling). `host` defaults
#                  to mattw@rpi5.
#
# This script must run from a machine with Nix + flakes (Mac, NixOS).
set -euo pipefail

usage() {
	cat <<EOF
Usage: $0 <image|deploy> [host]

  image          Build installer image via nixos-raspberrypi#installerImages.rpi5
  deploy [host]  Remote nixos-rebuild against the Pi (default: mattw@rpi5)
EOF
	exit 2
}

MODE="${1:-}"
HOST="${2:-mattw@rpi5}"

case "$MODE" in
image)
	echo "Building Pi 5 installer image (will take a while; first run hits cachix)"
	nix build github:nvmd/nixos-raspberrypi#installerImages.rpi5 --print-out-paths
	echo
	echo "Image is at ./result. Flash the .img file inside:"
	ls -la result/sd-image/ 2>/dev/null || ls -la result/
	;;
deploy)
	REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)/nix-config"
	echo "Deploying to $HOST from $REPO_DIR"
	nixos-rebuild switch \
		--flake "$REPO_DIR#rpi5" \
		--target-host "$HOST" \
		--build-host "$HOST" \
		--use-remote-sudo \
		--show-trace
	;;
*)
	usage
	;;
esac

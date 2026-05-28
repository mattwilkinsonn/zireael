{ lib, ... }:

# Shared home-manager config for both rpi4 and rpi5. Per-host extras
# (just the nix-switch flake target) live in nixos/rpi{4,5}/home.nix.
#
# Note: this config does NOT import shared/load-secrets.nix — the
# Pis are headless servers and we don't want every interactive shell
# pulling the full 1P secret bundle into env. The op-pi-svc token
# still exists encrypted at /etc/op-pi-svc-token.cred for systemd
# container env-refresh services (see containers.nix); shells just
# don't see it. For one-off shell needs on a Pi, copy-paste the env
# var directly. `load-secrets` is not defined in Pi shells.
{
  # Pin DOCKER_HOST to the root podman socket — production containers
  # (OpenClaw on rpi5) run as root via oci-containers in containers.nix,
  # so user tools (lazydocker, podman client over SSH) need to talk to
  # the root socket. lib.mkAfter overrides shared/linux.nix's
  # auto-detect even if a user-mode socket later shows up.
  programs.zsh.initContent = lib.mkAfter ''
    export DOCKER_HOST="unix:///run/podman/podman.sock"
  '';

  # rtk: not on crates.io and brew tap doesn't ship aarch64, so it has
  # to be built from git via `cargo install`. That compile takes ~10min
  # on aarch64 and exceeds the home-manager systemd activation timeout —
  # so we don't run it inline. Just nudge the user to install once,
  # manually, outside activation. After that lands at ~/.cargo/bin/rtk
  # it sticks across rebuilds. To update later, re-run the cargo install
  # manually.
  home.activation.checkRtk = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    if [ ! -x "$HOME/.cargo/bin/rtk" ]; then
      echo ""
      echo "  rtk is not installed. Run this in a regular shell (not activation):"
      echo "    cargo install --git https://github.com/rtk-ai/rtk"
      echo "  First build takes ~10 minutes on aarch64; subsequent rebuilds will skip."
      echo ""
    fi
  '';
}

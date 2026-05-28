{ inputs, ... }:

# Server-tier overlay — picks a small handful of packages from
# nixpkgs-unstable where stable's lag actually hurts day-to-day. Used
# by rpi4, rpi5, mattserver: hosts that stay on stable for cache
# efficiency but want fresh jj.
#
# Dev hosts (Mac, mattfw, mattpc-wsl) do NOT import this — they get
# `shared/unstable-wholesale.nix` instead, which routes every
# package through unstable. Anything that was historically pinned
# here (pulumi-bin, awscli2, starship-jj) only matters on dev hosts,
# so it doesn't need a per-package entry — wholesale unstable covers
# it.
#
# When 26.05 lands and the stable channel catches up enough that jj's
# version isn't painful, drop entries from this overlay (or delete the
# file entirely if empty).
#
# Imported at the system level (flake.nix) so the override applies to
# both `environment.systemPackages` and home-manager packages
# uniformly.
{
  nixpkgs.overlays = [
    (
      _: prev:
      let
        unstable = inputs.nixpkgs-unstable.legacyPackages.${prev.stdenv.hostPlatform.system};
      in
      {
        inherit (unstable) jujutsu jjui;
      }
    )
  ];
}

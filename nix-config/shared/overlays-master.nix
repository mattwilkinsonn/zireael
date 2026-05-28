{ inputs, lib, ... }:

# Master-channel overlay. Layered ON TOP of `shared/overlays.nix`.
#
# Master skips the full Hydra build set, so cache hits are worse and
# the host may compile some packages from source. Only opt in for
# hosts where:
#
#   1. The package genuinely needs to track upstream more aggressively
#      than nixos-unstable's release cadence (jj's release cycle is
#      faster than nixos-unstable's promotion cycle).
#   2. The host has enough CPU/disk that the occasional from-source
#      build doesn't ruin a switch (mattpc-wsl: yes; rpi4/rpi5: no).
#
# `lib.mkAfter` forces this overlay to the END of the merged
# nixpkgs.overlays list. Overlays apply in order, so the last one
# wins — without mkAfter, module-merge ordering can put the unstable
# overlay later and silently undo the master pick.
#
# When a package's master version reaches nixos-unstable, remove it
# from this overlay — the unstable overlay already routes it.
{
  nixpkgs.overlays = lib.mkAfter [
    (
      _: prev:
      let
        master = inputs.nixpkgs-master.legacyPackages.${prev.stdenv.hostPlatform.system};
      in
      {
        inherit (master) jujutsu biome;
      }
    )
  ];
}

{
  inputs,
  lib,
  system,
  ...
}:

# Wholesale unstable: replace the resolved package set with one
# evaluated directly from nixpkgs-unstable. Used by dev hosts (Mac,
# mattfw, mattpc-wsl) so every CLI tool lands at its latest packaged
# version without per-package overlay entries.
#
# Server / Pi hosts (rpi4, rpi5, mattserver) intentionally do NOT
# import this — they stay on stable + a tiny targeted overlay
# (`shared/overlays.nix`) for the one or two packages where lag
# matters (jj, jjui). That keeps their build paths small and
# cache-aligned with Hydra's stable release schedule.
#
# Mechanism: `_module.args.pkgs = ...` replaces the resolved package
# set wholesale, instead of swapping individual attrs via an overlay.
# Overlay-based replacement breaks stdenv bootstrap on darwin because
# the host channel's bootstrap-files don't match unstable's
# bootstrap-files. `nixpkgs.pkgs = ...` conflicts with `nixpkgs.config
# = ...` declarations in other modules (nixos/common.nix sets
# allowUnfree). `_module.args.pkgs = ...` sidesteps both, but
# nix-darwin (and nixos's nixpkgs module) already bind
# `_module.args.pkgs` at default priority — hence `lib.mkForce` to
# win the merge.
#
# `system` comes via `specialArgs` from flake.nix (the host's
# constructor declares its system string).
#
# Tradeoff: more cache misses than the targeted overlay since every
# binary now flows through unstable's Hydra builds. For dev hosts
# this is fine; for the Pis it's not.
#
# Drop this module entirely once the root nixpkgs input bumps to a
# release that's close enough to upstream (26.05 will probably
# narrow the gap enough for the dev/server split to disappear).
{
  _module.args.pkgs = lib.mkForce (
    import inputs.nixpkgs-unstable {
      inherit system;
      config = {
        allowUnfree = true;
        # nixpkgs ships its own `pkgs.openclaw` package (separate from
        # the nix-openclaw flake input we use on mattfw) and has
        # marked it insecure due to prompt-injection concerns. We
        # don't actually build nixpkgs's openclaw — mattfw consumes
        # nix-openclaw's pre-built derivation via
        # `programs.openclaw.package` — but the home-manager module
        # does a `pkgs.openclaw or null` lookup at eval time, and the
        # insecure-package throw isn't caught by `or`. Permitting
        # the version unblocks evaluation; nothing on the build path
        # depends on it. No-op for Mac / mattpc-wsl which never
        # reference `pkgs.openclaw`.
        permittedInsecurePackages = [
          "openclaw-2026.4.22"
        ];
      };
    }
  );
}

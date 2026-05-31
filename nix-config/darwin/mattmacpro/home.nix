{
  lib,
  pkgs,
  ...
}:

# mattw home config for mattmacpro — Mac Pro 2013 CI host.
#
# Imports shared/home.nix (the universal CLI tier — eza, bat, btm,
# ripgrep, fd, jj, zellij, lazydocker, etc.) so when you SSH in
# everything you expect on a "real" box is on PATH. Does NOT import
# shared/dev.nix (that's the toolchains tier — rustup, fnm, uv,
# pulumi, akiflow, etc. — which is heavyweight and wrong for a CI
# host where runners get tooling via system.nix extraPackages).
#
# Username is `mattw` (not `mattwilkinson` like the MBP) — matches
# every other server-class host (mattserver, mattfw, mattpc-wsl,
# rpis).
#
# Homebrew prefix is /usr/local on x86_64 (not /opt/homebrew on
# Apple Silicon). `brew shellenv` picks the right one at runtime.
#
# Note: this file is invoked with `pkgs` passed via
# home-manager.extraSpecialArgs (set in flake.nix). That's
# structurally different from how the other hosts wire HM, and is
# what allows useGlobalPkgs to NOT trigger the
# k9s/aerospace/etc. type-eval recursion on x86_64-darwin.

{
  imports = [
    ../../shared/home.nix
  ];

  home.username = lib.mkForce "mattw";
  home.homeDirectory = lib.mkForce "/Users/mattw";

  programs.zsh = {
    shellAliases = {
      nix-switch = "sudo HOME=\"$HOME\" darwin-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattmacpro\" --show-trace";
    };

    initContent = lib.mkBefore ''
      # macOS x86_64 brew prefix is /usr/local (Intel), not /opt/homebrew.
      if [ -x /usr/local/bin/brew ]; then
        eval "$(/usr/local/bin/brew shellenv)"
      fi

      # No 1Password service-account token exported here. mattmacpro
      # is a runner host (untrusted GHA workloads); per the bootstrap
      # script's "Security posture" section we keep zero standing OP
      # SA credentials accessible to processes on this box.
    '';
  };

  # Apple cctools / system PATH fallback for home-manager activations.
  # PATH is *appended* (not prepended) so GNU coreutils from the nix
  # store still win where they exist.
  home.activation.augmentActivationPath = lib.hm.dag.entryBefore [ "writeBoundary" ] ''
    export PATH="$PATH:/usr/bin:/Library/Developer/CommandLineTools/usr/bin"
  '';

  _module.args = { inherit pkgs; };
}

{
  lib,
  pkgs,
  ...
}:

# home config for awsmac — AWS EC2 mac2-m2.metal stopgap CI host.
#
# Imports shared/home.nix (the universal CLI tier — eza, bat, btm,
# ripgrep, fd, jj, zellij, lazydocker, etc.) so when you SSH in
# everything you expect on a "real" box is on PATH. Does NOT import
# shared/dev.nix (that's the toolchains tier — rustup, fnm, uv,
# pulumi, akiflow, etc. — which is heavyweight and wrong for a CI
# host where the runner gets tooling via system.nix agentPackages).
#
# Username is `ec2-user` here — the AWS macOS AMI's default admin
# account. Unlike the rest of the fleet (which standardizes on
# `mattw`), we apply home-manager to the AMI-provided account rather
# than creating a new one, since this box is the most disposable
# runner (terminated within days). See system.nix top-of-file.
#
# Homebrew prefix is /opt/homebrew on Apple Silicon. `brew shellenv`
# picks the right one at runtime.
#
# Note: this file is invoked with `pkgs` passed via
# home-manager.extraSpecialArgs (set in flake.nix). That's
# structurally different from how the other hosts wire HM, and is
# what allows useGlobalPkgs to NOT trigger the
# k9s/aerospace/etc. type-eval recursion.

{
  imports = [
    ../../shared/home.nix
  ];

  home.username = lib.mkForce "ec2-user";
  home.homeDirectory = lib.mkForce "/Users/ec2-user";

  programs.zsh = {
    shellAliases = {
      nix-switch = "sudo HOME=\"$HOME\" darwin-rebuild switch --flake \"$HOME/repos/zireael/nix-config#awsmac\" --show-trace";
    };

    initContent = lib.mkBefore ''
      # Apple Silicon brew prefix is /opt/homebrew.
      if [ -x /opt/homebrew/bin/brew ]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
      fi

      # No 1Password service-account token exported here. awsmac is a
      # runner host (untrusted CI workloads); per the bootstrap
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

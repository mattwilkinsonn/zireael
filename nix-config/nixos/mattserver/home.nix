{ lib, ... }:

# mattserver user config.
#
# Security posture: this host runs untrusted GitHub Actions workflows
# via the self-hosted runner pool. To minimise the blast radius of a
# runner-side code-exec, mattserver deliberately keeps zero standing
# 1Password service-account tokens on disk — no
# ~/.config/op/service-account-token, no team token. See
# nixos/mattserver/INSTALL.md "Security posture" for the full
# rationale.
#
# Practical effect: no `op inject`-driven secret loading at shell
# start (shared/load-secrets.nix intentionally not imported), no 1P
# reads from interactive sessions here. If you need a 1P-stored
# value on this host, copy-paste it from the unlocked 1P app on your
# Mac.

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattserver\" --show-trace";
}

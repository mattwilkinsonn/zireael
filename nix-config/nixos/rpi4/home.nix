{ lib, ... }:

# rpi4 user config — minimal headless. Shared rpi config (DOCKER_HOST,
# OP token loader, rtk reminder) lives in nixos/home.nix.

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#rpi4\" --show-trace";
}

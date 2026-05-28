{ lib, ... }:

# rpi5 user config — server-only. Shared rpi config (DOCKER_HOST,
# OP token loader, rtk reminder) lives in nixos/home.nix. Plasma + GUI
# apps live in rpi5/desktop.nix (not imported by default).

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#rpi5\" --show-trace";
}

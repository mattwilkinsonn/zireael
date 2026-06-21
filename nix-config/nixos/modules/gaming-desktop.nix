# Gaming desktop module — KDE Plasma 6 + Steam. Boots straight to SDDM
# by default; no manual `systemctl start sddm` needed.
#
# Generic + reusable: import this from any host's module list to turn it
# into a KDE gaming station. Hardware-specific bits (GPU driver,
# 32-bit graphics, firmware) belong in the host's own system.nix — this
# module assumes hardware.graphics is already configured there.
#
# To temporarily go headless without rebuilding:
#
#   sudo systemctl isolate multi-user.target  # this boot only
#   sudo systemctl isolate graphical.target   # back to DE
#
# To make headless the default again, flip `bootToDesktop` below to false.
#
# Requires the host's flake entry to include the nix-flatpak NixOS module
# (the services.flatpak block below depends on it) and the
# plasma-manager home-manager module (the home-manager layer below).

{
  pkgs,
  lib,
  inputs,
  ...
}:

let
  bootToDesktop = true;
in
{
  # ============================================================
  # Boot target
  # ============================================================

  # plasma6 pulls in graphical-desktop.nix which sets defaultUnit to
  # graphical.target — that's already what we want when bootToDesktop is
  # true, so no override needed in that case. Override to multi-user.target
  # only when running headless.
  systemd.defaultUnit = lib.mkIf (!bootToDesktop) (lib.mkForce "multi-user.target");

  # ============================================================
  # System layer
  # ============================================================

  services.desktopManager.plasma6.enable = true;
  services.displayManager.sddm = {
    enable = true;
    wayland.enable = true;
  };

  # When headless, remove SDDM from the graphical.target wants chain so it
  # doesn't start automatically. `systemctl start sddm` still works on demand.
  systemd.services.display-manager.wantedBy = lib.mkIf (!bootToDesktop) (lib.mkForce [ ]);

  hardware.bluetooth = {
    enable = true;
    powerOnBoot = true;
  };
  services.blueman.enable = true;

  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };

  # Steam — system-level so Proton + 32-bit libraries land in the right
  # paths. 32-bit graphics support comes from hardware.graphics.enable32Bit
  # in system.nix.
  programs.steam = {
    enable = true;
    remotePlay.openFirewall = false;
    dedicatedServer.openFirewall = false;
  };

  # GameMode daemon — lets games request a temporary performance profile
  # (CPU governor → performance, scheduler hints, etc.) via D-Bus.
  programs.gamemode.enable = true;

  # Declarative Flatpak for GUI apps that move faster than nixpkgs.
  services.flatpak = {
    enable = true;
    remotes = [
      {
        name = "flathub";
        location = "https://dl.flathub.org/repo/flathub.flatpakrepo";
      }
    ];
    packages = [
      "com.visualstudio.code"
      "md.obsidian.Obsidian"
      "org.chromium.Chromium"
    ];
    update.onActivation = true;
    uninstallUnmanaged = true;
  };

  # ============================================================
  # Home-manager layer
  # ============================================================

  home-manager.users.mattw = {
    imports = [
      inputs.plasma-manager.homeModules.plasma-manager
    ];

    programs.plasma = {
      enable = true;
      shortcuts = {
        "org.kde.krunner.desktop"."_launch" = "Meta+Space";
      };

      # Display off after 10min, never suspend the box. Pairs with any
      # host-level systemd sleep-target masking (e.g. an always-on
      # server/runner host) — Plasma stops asking, systemd stops obeying.
      powerdevil.AC = {
        powerButtonAction = "shutDown";
        autoSuspend.action = "nothing";
        turnOffDisplay.idleTimeout = 600;
      };
    };

    home.packages = with pkgs; [
      foot
      kdePackages.yakuake
      _1password-gui
      speedcrunch
      trayscale
      mangohud # FPS + GPU stats overlay
      lutris # Wine/Proton game manager
    ];
  };
}

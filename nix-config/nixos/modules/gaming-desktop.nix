# Gaming desktop module — KDE Plasma 6 + Steam. Boots straight to SDDM
# by default; no manual `systemctl start sddm` needed.
#
# Generic + reusable: import this from any host's module list to turn it
# into a KDE gaming station. Hardware-specific bits (GPU driver,
# 32-bit graphics, firmware) belong in the host's own system.nix — this
# module assumes hardware.graphics is already configured there.
#
# Per-host knobs (all have defaults; set them from the importing host's
# config, no need to edit this shared file):
#
#   gamingDesktop.bootToDesktop              = true;     # false → headless default
#   gamingDesktop.username                   = "mattw";  # the desktop user
#   gamingDesktop.flatpakUpdateOnActivation  = true;     # false → offline-safe rebuilds
#
# To temporarily go headless without rebuilding:
#
#   sudo systemctl isolate multi-user.target  # this boot only
#   sudo systemctl isolate graphical.target   # back to DE
#
# To make headless the default, set `gamingDesktop.bootToDesktop = false`
# in the host config.
#
# Requires the host's flake entry to include the nix-flatpak NixOS module
# (the services.flatpak block below depends on it) and the
# plasma-manager home-manager module (the home-manager layer below).
#
# DORMANT: nothing in this flake imports this module today (it's kept
# for reuse if a gaming box comes back). zireael's flake no longer
# carries the `nix-flatpak` or `plasma-manager` inputs — they were
# dropped when the last desktop host left. Re-add both inputs (and pass
# plasma-manager through to home-manager.extraSpecialArgs) before
# importing this on a host, or the eval fails on the missing
# `inputs.plasma-manager` reference below.

{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  cfg = config.gamingDesktop;
in
{
  options.gamingDesktop = {
    bootToDesktop = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Boot straight to the SDDM display manager (graphical.target).
        Set false for a headless default — SDDM stays off the
        graphical.target wants chain and `systemctl start sddm` brings
        it up on demand.
      '';
    };

    username = lib.mkOption {
      type = lib.types.str;
      default = "mattw";
      description = ''
        The desktop user that gets the plasma-manager home-manager
        layer (Plasma config, gaming packages). Defaults to the
        personal-host primary user.
      '';
    };

    flatpakUpdateOnActivation = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Update + prune Flatpak apps from Flathub on every
        `nixos-rebuild switch`. Set false for offline-safe activations
        (a host that may rebuild while disconnected) and handle Flatpak
        updates via a separate timer or manual invocation instead.
      '';
    };
  };

  config = {
    # ============================================================
    # Boot target
    # ============================================================

    # plasma6 pulls in graphical-desktop.nix which sets defaultUnit to
    # graphical.target — that's already what we want when bootToDesktop is
    # true, so no override needed in that case. Override to multi-user.target
    # only when running headless.
    systemd.defaultUnit = lib.mkIf (!cfg.bootToDesktop) (lib.mkForce "multi-user.target");

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
    systemd.services.display-manager.wantedBy = lib.mkIf (!cfg.bootToDesktop) (lib.mkForce [ ]);

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
      update.onActivation = cfg.flatpakUpdateOnActivation;
      uninstallUnmanaged = true;
    };

    # ============================================================
    # Home-manager layer
    # ============================================================

    home-manager.users.${cfg.username} = {
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
  };
}

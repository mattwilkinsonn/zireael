{
  pkgs,
  lib,
  inputs,
  ...
}:

# mattfw desktop module — KDE Plasma 6 + Flatpak GUI apps. Imported into the
# flake module list by default, BUT the display manager is not autostarted.
# Default systemd target stays multi-user, so a normal boot drops to a TTY
# and SSH stays the primary access path. To bring up Plasma locally for
# debug:
#
#   sudo systemctl isolate graphical.target   # this boot only
#   # or for a one-off login session:
#   sudo systemctl start sddm
#
# To go back headless:
#
#   sudo systemctl isolate multi-user.target  # stops sddm + plasma session
#
# Default boot target is pinned to multi-user.target below so a reboot
# always returns to headless without manual intervention.

{
  # ============================================================
  # Boot target
  # ============================================================

  # Headless by default. KDE bits below are installed and ready to start
  # on demand — they just don't run unless explicitly invoked.
  # mkForce because graphical-desktop.nix (pulled in by plasma6) sets
  # systemd.defaultUnit = "graphical.target" with normal priority, and
  # the whole point of this module is to override that.
  systemd.defaultUnit = lib.mkForce "multi-user.target";

  # ============================================================
  # System layer
  # ============================================================

  # KDE Plasma 6 + SDDM. enable=true installs the packages and writes the
  # service unit. The display manager is then explicitly disabled below
  # so it doesn't start on boot — `systemctl start sddm` brings it up
  # manually.
  services.desktopManager.plasma6.enable = true;
  services.displayManager.sddm = {
    enable = true;
    wayland.enable = true;
  };

  # Force SDDM off at boot. The .enable above gets the unit installed; this
  # WantedBy override removes it from the graphical.target wants chain so
  # nothing pulls it in automatically.
  systemd.services.display-manager.wantedBy = pkgs.lib.mkForce [ ];

  # Bluetooth peripherals (KB/mouse) for the rare case we plug in a screen
  # and need to use it locally.
  hardware.bluetooth = {
    enable = true;
    powerOnBoot = true;
  };
  services.blueman.enable = true;

  # Pipewire audio — KDE expects it. Headless services don't care, the
  # module is dormant when no session is active.
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };

  # Declarative Flatpak — fast-moving GUI apps (parity with rpi5/desktop.nix).
  # Only relevant when desktop is up.
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
  # Home-manager layer (injected into mattw's home-manager config)
  # ============================================================

  home-manager.users.mattw = {
    imports = [
      inputs.plasma-manager.homeModules.plasma-manager
    ];

    # Plasma 6 declarative settings — same shape as rpi5/desktop.nix.
    programs.plasma = {
      enable = true;

      # KRunner on Meta+Space — Mac-style Spotlight ergonomics.
      shortcuts = {
        "org.kde.krunner.desktop"."_launch" = "Meta+Space";
      };

      # Display off after 10min, never suspend the box. Defense-in-depth
      # alongside the systemd target masks in system.nix — Plasma stops
      # asking, systemd stops obeying.
      powerdevil.AC = {
        powerButtonAction = "shutDown";
        autoSuspend.action = "nothing";
        turnOffDisplay.idleTimeout = 600;
      };
    };

    # GUI apps for the rare debug session. Web apps (Spotify, Slack, etc.)
    # install as PWAs in Chromium when needed.
    home.packages = with pkgs; [
      foot # Wayland-native terminal
      kdePackages.yakuake # quake-style drop-down terminal
      _1password-gui
      speedcrunch
      pods # podman GUI (GTK4 native)
      trayscale # Tailscale tray
    ];
  };
}

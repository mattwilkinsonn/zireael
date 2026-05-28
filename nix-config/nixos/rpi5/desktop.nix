{
  pkgs,
  inputs,
  ...
}:

# Optional desktop module for the Pi 5 — KDE Plasma 6 + Flatpak GUI apps +
# Bluetooth peripherals + plasma-manager declarative config. NOT imported
# by default; the Pi 5 runs as a headless server primarily (Cockpit +
# Tailscale).
#
# To re-enable for occasional debug/desktop use:
#   1. Add `./nixos/rpi5/desktop.nix` to the rpi5 modules list in flake.nix
#   2. nix-switch
#   3. Reboot — SDDM starts, login as mattw, KDE Plasma session
#
# To disable again:
#   1. Remove the import from flake.nix
#   2. nix-switch (uninstalls Flatpak apps via uninstallUnmanaged, stops SDDM)
#   3. Reboot — back to headless TTY

{
  # ============================================================
  # System layer
  # ============================================================

  # KDE-related overlays. Klassy comes from unstable for current decoration
  # sets; ksystemstats CPU plugin is patched out to avoid the aarch64 hang
  # in /proc/cpuinfo polling (KDE Bug 493093). Both are pure desktop concerns.
  nixpkgs.overlays = [
    (
      _: prev:
      let
        unstable = inputs.nixpkgs-unstable.legacyPackages.${prev.stdenv.hostPlatform.system};
      in
      {
        inherit (unstable) klassy;

        kdePackages = prev.kdePackages.overrideScope (
          _kfinal: kprev: {
            ksystemstats = kprev.ksystemstats.overrideAttrs (oldAttrs: {
              postInstall = (oldAttrs.postInstall or "") + ''
                # CPU plugin hangs on aarch64-linux: infinite loop parsing
                # /proc/cpuinfo on Raspberry Pi. See KDE Bug 493093 (REOPENED)
                # and pending fix in MR 118:
                # https://bugs.kde.org/show_bug.cgi?id=493093
                # https://invent.kde.org/plasma/ksystemstats/-/merge_requests/118
                # Drop this overlay once the fix lands in a release we're on.
                rm -f $out/lib/qt-6/plugins/ksystemstats/ksystemstats_plugin_cpu.so
              '';
            });
          }
        );
      }
    )
  ];

  # KDE Plasma 6 (Wayland session via SDDM). Originally tried COSMIC but
  # cosmic-comp hits a known V3D7-driver GL state-tracking bug on Pi 5.
  services.desktopManager.plasma6.enable = true;
  services.displayManager.sddm = {
    enable = true;
    wayland.enable = true;
  };

  # Plain US keyboard layout — only relevant when desktop is up.
  services.xserver.xkb.layout = "us";

  # Bluetooth peripherals (KB/mouse) — useful for desktop debug, useless
  # headless. raspberry-pi-5.bluetooth module is imported in flake.nix for
  # the hardware base; this just powers on the adapter at boot.
  hardware.bluetooth.powerOnBoot = true;

  # Declarative Flatpak — fast-moving GUI apps. Only relevant when desktop
  # is up; uninstallUnmanaged = true means disabling this module purges
  # all listed flatpaks on next nix-switch.
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
      # Betterbird (Thunderbird fork) instead of Thunderbird — Mozilla's
      # official Thunderbird flatpak is x86_64-only on Flathub.
      # "eu.betterbird.Betterbird"
      # "dev.vencord.Vesktop"
      # "com.github.IsmaelMartinez.teams_for_linux"
    ];
    update.onActivation = true;
    uninstallUnmanaged = true;
  };

  # ============================================================
  # Home-manager layer (injected into mattw's home-manager config)
  # ============================================================

  home-manager.users.mattw = {
    imports = [
      # Declarative Plasma 6 config (default apps, panel layout, shortcuts).
      inputs.plasma-manager.homeModules.plasma-manager
    ];

    # Plasma 6 declarative settings (replaces ad-hoc kwriteconfig6 hook).
    programs.plasma = {
      enable = true;

      # Default terminal: Foot (KDE reads this from kdeglobals[General]).
      # configFile."kdeglobals"."General"."TerminalApplication" = "foot";
      # configFile."kdeglobals"."General"."TerminalService" = "foot.desktop";

      # Force fonts DPI down from the 96 default. The HDMI output runs at
      # 3840x1080 (Pi 5 can't drive the 49" panel's native 5120x1440 @ 60Hz
      # reliably), and at that low pixel density the 96-DPI default makes
      # everything look oversized.
      configFile."kcmfonts"."General"."forceFontDPI" = 80;

      # KRunner on Meta+Space — feels like Cmd+Space ergonomically.
      shortcuts = {
        "org.kde.krunner.desktop"."_launch" = "Meta+Space";
      };
    };

    # GUI apps. Mac side gets equivalents via brew casks. Apps that don't
    # ship aarch64-linux binaries (Spotify, Discord, Zoom, Sunsama, Slack,
    # Linear, Circleback, Claude Desktop) need a web app instead — install
    # as PWAs in Chromium via the three-dot menu → Install.
    home.packages = with pkgs; [
      # ghostty: dropped on Pi — Pi 5 V3D + GTK4 GL stack can't acquire a
      # context. Konsole (bundled with Plasma 6) is the daily-driver terminal
      # here. Mac side keeps ghostty via brew cask.
      foot # Wayland-native, GLES — works on Pi 5 V3D where Ghostty doesn't
      kdePackages.yakuake # quake-style drop-down terminal (Konsole-based)
      _1password-gui # no flathub release; official .deb only
      speedcrunch
      pods # podman GUI (GTK4 native — lighter than podman-desktop)
      trayscale # Tailscale tray icon
      # zapzap # WhatsApp Web wrapper (Qt-native — whatsapp-for-linux not on aarch64)
      transmission_4-qt # BitTorrent (Qt build, matches Plasma 6)
    ];
  };
}

{ pkgs, lib, ... }:
# mattpc — bare-metal NixOS (i9-14900KS + RTX 4080 + 64 GB), the daily driver.
# Dual-boots Windows (Disk 1, games only) via systemd-boot; NixOS owns Disk 0.
# Shares nixos/common.nix with the other personal hosts (users, openssh:22,
# podman, zsh, nix-ld, stateVersion) — this file adds only the bare-metal
# pieces WSL didn't need: bootloader, GPU, desktop, networking, tailscale.
{
  networking.hostName = "mattpc";

  # ── Bootloader: systemd-boot + a chainloaded Windows entry ──────────
  # NixOS owns Disk 0's ESP; systemd-boot is the default (boots NixOS).
  # Windows' bootloader lives on Disk 1's 200 MB ESP — a *different* disk,
  # and systemd-boot can't load a binary from another ESP directly. So we
  # ship edk2-uefi-shell and register a "Windows" entry that chainloads
  # Disk 1's Windows Boot Manager, giving one unified boot menu.
  #
  # SSH boot-select (the requirement): default stays NixOS; a one-shot
  # reboot into Windows that auto-returns afterward:
  #
  #     bootctl list                              # confirm the "Windows" entry
  #     sudo bootctl set-oneshot windows && sudo reboot
  #
  # set-oneshot applies to the next boot only, so after the gaming session
  # the default (NixOS) is restored automatically — the "game, then back to
  # NixOS" flow.
  #
  # AT INSTALL: efiDeviceHandle is a placeholder. Boot the edk2 shell entry
  # once, run `map -c` to list filesystem handles, find the one whose
  # `\EFI\Microsoft\Boot\bootmgfw.efi` exists (Disk 1's ESP), set
  # efiDeviceHandle to it (e.g. "FS1"), and rebuild. See INSTALL.md.
  boot.loader = {
    systemd-boot = {
      enable = true;
      configurationLimit = 10; # cap kept generations (2 GiB ESP; NVIDIA kernels ~150 MB each)
      windows."windows" = {
        title = "Windows";
        # REPLACE at install with the handle from edk2's `map -c` (see above).
        efiDeviceHandle = "REPLACE-WITH-EDK2-FS-HANDLE";
        sortKey = "y_windows"; # below NixOS entries, above the edk2 shell
      };
      edk2-uefi-shell = {
        enable = true;
        sortKey = "z_edk2";
      };
    };
    efi.canTouchEfiVariables = true;
  };

  # Hibernate resumes from the disko-declared swap (resumeDevice = true).
  boot.kernelParams = [ ];

  # ── GPU: RTX 4080 (proprietary nvidia) ──────────────────────────────
  hardware.graphics = {
    enable = true;
    enable32Bit = true; # Steam / 32-bit games
  };
  services.xserver.videoDrivers = [ "nvidia" ];
  hardware.nvidia = {
    modesetting.enable = true; # required for Wayland
    open = false; # RTX 4080 (Ada) works on either; proprietary is safest
    nvidiaSettings = true;
    powerManagement.enable = false;
    package = lib.mkDefault pkgs.linuxPackages.nvidiaPackages.stable;
  };

  # ── Desktop: Hyprland (Wayland compositor) ──────────────────────────
  # System side: the compositor + portals + a display manager. The user
  # session config (keybinds, monitors, bar) lives in nixos/mattpc/home.nix.
  programs.hyprland = {
    enable = true;
    withUWSM = true; # session manager — cleaner env + systemd integration
  };
  # tuigreet on greetd: minimal, Wayland-friendly login that launches the
  # UWSM-wrapped Hyprland session.
  services.greetd = {
    enable = true;
    settings.default_session = {
      command = "${pkgs.tuigreet}/bin/tuigreet --time --remember --cmd 'uwsm start hyprland-uwsm.desktop'";
      user = "greeter";
    };
  };
  # Wayland-native portals for screen-share / file pickers under Hyprland.
  xdg.portal = {
    enable = true;
    extraPortals = [
      pkgs.xdg-desktop-portal-gtk # file pickers
      pkgs.xdg-desktop-portal-hyprland # wlr-screencopy (screen share) — GTK alone can't
    ];
  };
  # NVIDIA + Wayland: let GBM back the EGL platform.
  environment.sessionVariables.NIXOS_OZONE_WL = "1";

  # Audio (PipeWire).
  services.pulseaudio.enable = false;
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  # ── Networking + Tailscale ──────────────────────────────────────────
  # Bare metal: NetworkManager for the desktop; its own tailscaled (WSL
  # borrowed the Windows host's daemon — bare metal runs its own).
  networking.networkmanager.enable = true;
  services.tailscale.enable = true;

  # Firewall on (WSL left it to the Windows firewall; bare metal owns it).
  # SSH (22, from common.nix) + let Tailscale punch its own holes.
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [ 22 ];
    trustedInterfaces = [ "tailscale0" ];
    checkReversePath = "loose"; # Tailscale UDP
  };
  networking.firewall.allowedUDPPorts = [ 41641 ]; # tailscale

  # Desktop niceties.
  services.fwupd.enable = true; # firmware updates
  fonts.enableDefaultPackages = true;
}

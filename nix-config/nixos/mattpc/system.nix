{ pkgs, lib, ... }:
# mattpc — bare-metal NixOS (i9-14900KS + RTX 4080 + 64 GB), the daily driver.
# Dual-boots Windows (Disk 1, games only) via systemd-boot; NixOS owns Disk 0.
# Shares nixos/common.nix with the other personal hosts (users, openssh:22,
# podman, zsh, nix-ld, stateVersion) — this file adds only the bare-metal
# pieces WSL didn't need: bootloader, GPU, desktop, networking, tailscale.
{
  networking.hostName = "mattpc";

  # ── Bootloader: lanzaboote (signed Secure Boot chain) ───────────────
  # NixOS owns Disk 0's ESP. lanzaboote keeps systemd-boot as the boot
  # manager but signs systemd-bootx64.efi and installs per-generation
  # signed UKIs (EFI/Linux/nixos-generation-*.efi) with the machine-owner
  # keys under pkiBundle, so the firmware accepts the chain under Secure
  # Boot. Enabling it sets boot.loader.external.enable internally, which
  # is mutually exclusive with the in-tree systemd-boot installer — hence
  # systemd-boot.enable = mkForce false. Keys are provisioned manually via
  # sbctl at install time (INSTALL.md §7); autoGenerateKeys/autoEnrollKeys
  # stay off.
  #
  # Windows dual-boot: selected via its own firmware-native "Windows Boot
  # Manager" UEFI entry on Disk 1's ESP (bootmgfw.efi is Microsoft-signed,
  # so it boots under Secure Boot once Microsoft keys are enrolled). Pick
  # it over SSH with a genuine UEFI one-shot that reverts to NixOS after:
  #
  #     efibootmgr -v                          # one-time: find its entry number
  #     sudo efibootmgr --bootnext <N> && sudo reboot
  #
  # The firmware consumes --bootnext on the next boot, then reverts to
  # BootOrder (NixOS) — the "game, then back to NixOS" flow. See INSTALL.md.
  boot.loader.systemd-boot.enable = lib.mkForce false;
  boot.lanzaboote = {
    enable = true;
    pkiBundle = "/var/lib/sbctl";
    configurationLimit = 10; # cap kept generations (2 GiB ESP; NVIDIA kernels ~150 MB each)
  };
  boot.loader.efi.canTouchEfiVariables = true;

  # Secure Boot key enrollment (sbctl) + Windows boot-select (efibootmgr).
  # Neither is on PATH by default; the manual key flow (INSTALL.md §7) and
  # the --bootnext Windows selection above both need them on the host.
  environment.systemPackages = [
    pkgs.sbctl
    pkgs.efibootmgr
  ];

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

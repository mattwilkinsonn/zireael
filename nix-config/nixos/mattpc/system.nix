{ pkgs, lib, ... }:
# mattpc — bare-metal NixOS (i9-14900KS + RTX 4080 + 64 GB), the daily driver.
# Dual-boots Windows (Disk 1, games only) via systemd-boot; NixOS owns Disk 0.
# Shares nixos/common.nix with the other personal hosts (users, openssh:22,
# podman, zsh, nix-ld, stateVersion) — this file adds only the bare-metal
# pieces WSL didn't need: bootloader, GPU, desktop, networking, tailscale.
{
  networking.hostName = "mattpc";

  # ── Bootloader: systemd-boot + firmware-level Windows selection ─────
  # NixOS owns Disk 0's ESP; systemd-boot is the default (boots NixOS).
  # Windows has its own UEFI boot entry (Windows Boot Manager on Disk 1's
  # ESP), created by the Windows install — the firmware sees both disks'
  # entries.
  #
  # SSH boot-select (the requirement): a one-shot reboot into Windows uses
  # the UEFI `BootNext` variable via efibootmgr — firmware-level, so it
  # works across disks with no chainloader:
  #
  #     sudo efibootmgr                          # list; note Windows' Boot####
  #     sudo efibootmgr -n <nnnn> && sudo reboot # boot Windows ONCE, then
  #                                              # auto-return to NixOS
  #
  # BootNext is consumed by the firmware on the next boot only, so the
  # default (systemd-boot → NixOS) is restored automatically afterward —
  # exactly the "game, then back to NixOS" flow. `canTouchEfiVariables`
  # lets efibootmgr write the variable. (Windows appears in the firmware
  # boot menu, not the systemd-boot menu — cross-disk entries can't show
  # there without an edk2 chainloader, which this nixpkgs pin predates;
  # the SSH switch above is the controllable path and needs no chainloader.)
  boot.loader = {
    systemd-boot = {
      enable = true;
      configurationLimit = 20; # cap kept generations on the 1 GiB ESP
    };
    efi.canTouchEfiVariables = true;
  };
  # efibootmgr for the BootNext SSH-switch above (+ inspecting entries).
  environment.systemPackages = [ pkgs.efibootmgr ];

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
    extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
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

{
  pkgs,
  ...
}:

# mattfw — Framework Desktop (Ryzen AI Max+ 395 / Strix Halo, 128GB LPDDR5X).
# Primary use: local LLM inference target accessed over SSH. mattpc-wsl is the
# primary dev box; this one is tuned aggressively for "biggest model + biggest
# context window the hardware can take." KDE is installed (./desktop.nix)
# but the display manager is NOT enabled at boot — start SDDM manually if
# a local screen is ever needed.
#
# Memory split for local models: BIOS UMA Frame Buffer set to 512 MB
# (display framebuffer only — UMA is GPU-exclusive memory the OS can
# never reclaim, so we keep it tiny) and the iGPU pulls the rest from
# system RAM via GTT. Kernel params `amdgpu.gttsize=126976` +
# `ttm.pages_limit=32505856` (paired) cap GTT at 124 GiB, leaving ~4 GiB
# for the OS. Tune down if the OS feels memory-starved — see INSTALL.md
# for the table mapping gttsize values → ttm.pages_limit values → OS
# headroom.

{
  # ============================================================
  # Boot
  # ============================================================

  boot = {
    # systemd-boot on UEFI — the Framework Desktop is a modern AMD platform
    # with no BIOS-fallback need. EFI partition mounted at /boot.
    loader = {
      systemd-boot.enable = true;
      efi.canTouchEfiVariables = true;
    };

    # Latest kernel — Strix Halo (gfx1151 / RDNA 3.5) needs recent amdgpu.
    # 6.11+ is when the Strix Halo IDs landed; 6.13+ has the GTT size knob
    # respected reliably. Pinning `latest` keeps us on whatever's current
    # in nixos-25.11 without a per-rebuild bump.
    kernelPackages = pkgs.linuxPackages_latest;

    # Kernel params for the iGPU.
    #   amdgpu.gttsize=126976 + ttm.pages_limit=32505856  — paired knobs
    #     that cap the GPU's GTT (Graphics Translation Table) at 124 GiB
    #     and the kernel's pinned-memory ceiling at the same value
    #     (32505856 × 4 KiB = 126976 MiB = 124 GiB). Strix Halo's iGPU
    #     can DMA into normal system RAM via GTT; this is what lets
    #     local-LLM workloads address >>96 GB without reserving UMA.
    #     Both knobs are required — gttsize alone lets the GPU *ask*
    #     for that much memory, but ttm.pages_limit is the actual ceiling
    #     on pinned pages, so without it the kernel can refuse to back
    #     the full allocation. ~4 GiB left to the OS at this setting,
    #     which is fine for a headless inference box accessed over SSH.
    #     Drop both values together if you need more OS headroom (see
    #     the GTT Memory Reference table in nixos/mattfw/INSTALL.md).
    #     BIOS UMA Frame Buffer should be set to 512MB (display
    #     framebuffer only) so it doesn't fight the GTT pool.
    #   amd_iommu=on iommu=pt          — IOMMU on, passthrough mode. Needed
    #     for the GPU to DMA across the full memory pool cleanly. `pt`
    #     avoids the perf hit of identity-mapping every kernel allocation.
    #   amdgpu.ppfeaturemask=0xffffffff — unlocks all PowerPlay features
    #     so undervolt/clock tooling works if you want to tune later.
    #
    # Firmware regression note: linux-firmware-20251125 broke ROCm on
    # gfx1151 (instability/crashes). 20251111 is the last good version.
    # NixOS sources firmware from `linux-firmware` in nixpkgs; if ROCm
    # workloads start crashing arbitrarily, check the version with
    # `cat /sys/module/firmware_class/parameters/path` + the linux-
    # firmware derivation in your current generation.
    kernelParams = [
      "amdgpu.gttsize=126976"
      "ttm.pages_limit=32505856"
      "amd_iommu=on"
      "iommu=pt"
      "amdgpu.ppfeaturemask=0xffffffff"
    ];

    # No plymouth, no `quiet splash` — this is a headless inference box
    # accessed over SSH. Verbose boot is the right default: the next time
    # something hangs at boot, the actual error is on screen instead of
    # hidden behind a frozen splash. (We hit exactly that on first install:
    # plymouth got stuck on the framebuffer at "Starting systemd-udevd"
    # while the rest of boot proceeded fine.)

    # Larger /tmp on tmpfs — model files and build artifacts can be huge.
    # Default 50% of RAM is fine on 128GB.
    tmp.useTmpfs = true;

    # Bias the kernel against swapping under memory pressure — spinning model
    # weights out of GTT and back into RAM is way more painful than letting
    # cold pages stay resident.
    kernel.sysctl = {
      "vm.swappiness" = 10;
    };
  };

  # ============================================================
  # Hardware
  # ============================================================

  hardware = {
    # CPU microcode updates at boot.
    cpu.amd.updateMicrocode = true;

    # Modern graphics stack — replaces the old hardware.opengl options.
    # 32-bit support for Steam/Proton/wine if you ever want to run games
    # locally; harmless overhead otherwise. RADV (Mesa's Vulkan) +
    # radeonsi (Mesa's OpenGL) come by default — that's what we want.
    # AMD's proprietary `amdvlk` is removed in 25.11 and not coming back.
    graphics = {
      enable = true;
      enable32Bit = true;
      extraPackages = with pkgs; [
        rocmPackages.clr.icd # OpenCL ICD loader for ROCm compute workloads
      ];
    };

    # Firmware blobs for AMD GPU (necessary for amdgpu init).
    enableRedistributableFirmware = true;
    enableAllFirmware = true;
  };

  # ROCm / HIP runtime. Installed system-wide so any user (and llama.cpp /
  # Ollama / vLLM running as a service) can find it. ROCM_PATH is what most
  # toolchains look at.
  environment.systemPackages = with pkgs; [
    rocmPackages.rocm-smi # GPU monitoring (`rocm-smi`)
    rocmPackages.rocminfo # `rocminfo` — confirms gfx1151 is detected
    radeontop # CLI GPU usage TUI
    pciutils # `lspci` for hardware inspection
    usbutils # `lsusb`
    lm_sensors # `sensors` — temps, fan speeds
    btrfs-progs # btrfs admin tools
  ];

  environment.variables = {
    ROCM_PATH = "${pkgs.rocmPackages.clr}";
    HIP_PATH = "${pkgs.rocmPackages.clr}";
  };

  # ============================================================
  # Filesystem
  # ============================================================

  # btrfs root with subvolumes. Set up by the installer (see INSTALL.md):
  #   @       → /
  #   @home   → /home
  #   @nix    → /nix
  #   @log    → /var/log
  # The label "nixos" is set during mkfs.btrfs.
  fileSystems."/" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "btrfs";
    options = [
      "subvol=@"
      "compress=zstd:1"
      "noatime"
      "ssd"
      "discard=async"
    ];
  };
  fileSystems."/home" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "btrfs";
    options = [
      "subvol=@home"
      "compress=zstd:1"
      "noatime"
      "ssd"
      "discard=async"
    ];
  };
  fileSystems."/nix" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "btrfs";
    options = [
      "subvol=@nix"
      "compress=zstd:1"
      "noatime"
      "ssd"
      "discard=async"
    ];
  };
  fileSystems."/var/log" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "btrfs";
    options = [
      "subvol=@log"
      "compress=zstd:1"
      "noatime"
      "ssd"
      "discard=async"
    ];
  };
  fileSystems."/boot" = {
    device = "/dev/disk/by-label/BOOT";
    fsType = "vfat";
    options = [
      "fmask=0077"
      "dmask=0077"
    ];
  };

  # Periodic TRIM for the NVMe (better than continuous discard on btrfs;
  # `discard=async` above handles immediate frees, fstrim catches stragglers).
  services.fstrim.enable = true;

  # btrfs scrub once a month — checks on-disk integrity. Single-disk system
  # so no auto-repair, but scrub will report any silent corruption so we know
  # to act on it.
  services.btrfs.autoScrub = {
    enable = true;
    interval = "monthly";
    fileSystems = [ "/" ];
  };

  # 32GB swap partition — large enough for OOM-avoidance and to match
  # GPU-evicted weight pages if a model briefly exceeds budget.
  # Hibernate-to-disk not enabled (and not advised on a box that mostly
  # sits running inference jobs).
  swapDevices = [
    {
      device = "/dev/disk/by-label/swap";
    }
  ];

  # ============================================================
  # Networking
  # ============================================================

  networking = {
    hostName = "mattfw";
    # NetworkManager — easier for a desktop-class machine where Wi-Fi /
    # different ethernet adapters / VPN profiles come and go. The Pis use
    # static-ip systemd-networkd because they're fixed-role servers.
    networkmanager.enable = true;
  };

  networking.firewall = {
    enable = true;
    # 22 = SSH (LAN + Tailscale). VSCode Remote uses this.
    allowedTCPPorts = [ 22 ];
    trustedInterfaces = [ "tailscale0" ];
  };

  services.tailscale = {
    enable = true;
    openFirewall = true;
    extraUpFlags = [ "--ssh" ];
  };

  # ============================================================
  # Services
  # ============================================================

  # NTP — desktop has an RTC battery so first-boot clock is fine, but keep
  # timesyncd on for ongoing accuracy.
  services.timesyncd.enable = true;

  # Power profiles — modern desktop-friendly power management. KDE's power
  # widget toggles balanced/performance via this when the desktop is up.
  services.power-profiles-daemon.enable = true;

  # Never let the box sleep. mattfw is a remote-SSH dev box + local LLM
  # inference target — a suspended host means in-flight builds/inferences
  # die and SSH access drops until someone walks over and wakes it.
  #
  # Masks the systemd sleep targets so `systemctl suspend`, idle timers,
  # and any session-level "sleep now" call all become no-ops. KDE PowerDevil
  # still controls screen DPMS (monitors can still sleep), only the box-level
  # sleep paths are blocked.
  systemd.targets = {
    sleep.enable = false;
    suspend.enable = false;
    hibernate.enable = false;
    hybrid-sleep.enable = false;
  };
  services.logind.settings.Login = {
    IdleAction = "ignore";
    HandlePowerKey = "poweroff";
    HandleSuspendKey = "ignore";
    HandleHibernateKey = "ignore";
  };

  # X11 keyboard layout — only consumed when the desktop module is loaded
  # and SDDM is started, but harmless headless.
  services.xserver.xkb.layout = "us";
}

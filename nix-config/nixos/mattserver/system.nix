{
  pkgs,
  lib,
  ...
}:

# mattserver — Old gaming PC (AMD Ryzen 3600 + RX 5700 XT, 64 GB DDR4).
# Roles:
#   1. ZFS backup receive target (btrfs root on 1TB NVMe; ZFS pool on 2TB SATA SSHD)
#   2. Self-hosted Buildkite CI agents (sealedsecurity org)
#   3. KDE Plasma gaming station (boots to SDDM by default; flip
#      `bootToDesktop` in desktop.nix to go headless)

{
  imports = [ ../modules/buildkite-agent.nix ];

  # ============================================================
  # Boot
  # ============================================================

  boot = {
    loader = {
      systemd-boot.enable = true;
      efi.canTouchEfiVariables = true;
    };

    # Pinned to the newest kernel the in-tree OpenZFS release supports.
    # `linuxPackages_latest` outpaces OpenZFS by 1–2 kernel releases and
    # trips `zfs-kernel … is marked as broken` during nixos-install.
    # This expression self-heals as OpenZFS catches up.
    kernelPackages = pkgs.linuxPackages;

    # ZFS kernel module support. Root is btrfs; ZFS lives on the SATA SSHD.
    supportedFilesystems = [ "zfs" ];

    # Don't force-import the ZFS pool at boot — if the SSHD is missing or
    # sick, we still want the system to come up on the NVMe root.
    zfs.forceImportRoot = false;

    kernel.sysctl = {
      "vm.swappiness" = 10;
    };

    # 24 GB tmpfs for /tmp — large enough for Rust incremental build
    # artifacts and wasm cross-compile intermediates without eating into
    # the NVMe's persistent space.
    tmp = {
      useTmpfs = true;
      tmpfsSize = "24G";
    };
  };

  # ============================================================
  # Hardware
  # ============================================================

  hardware = {
    cpu.amd.updateMicrocode = true;

    # RDNA 1 (gfx1010 / Navi 10) — Mesa RADV handles Vulkan; radeonsi for
    # OpenGL. 32-bit support required for Steam/Proton.
    graphics = {
      enable = true;
      enable32Bit = true;
    };

    enableRedistributableFirmware = true;
    enableAllFirmware = true;
  };

  # Performance CPU governor — avoids frequency ramp-up delay at the start
  # of a CI job or game session. The 3600 idles at ~2.2 GHz under schedutil;
  # `performance` holds it at boost clocks from the first instruction.
  powerManagement.cpuFreqGovernor = "performance";

  # Never let the box sleep. It's a backup target + runner host first; gaming
  # is a sometimes role. A suspended host means missed snapshots and CI jobs
  # falling back to GitHub-hosted runners.
  #
  # Masks at the systemd target level so `systemctl suspend`, idle timers,
  # and any session-level "sleep now" call all become no-ops. KDE PowerDevil
  # still controls screen DPMS (monitors can still go to sleep), only the
  # box-level sleep paths are blocked.
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

  environment.systemPackages = with pkgs; [
    # Mattserver-specific extras. The system-profiling toolkit
    # (pciutils, usbutils, lm_sensors, dmidecode, lshw, …) lives in
    # nixos/common.nix and applies here automatically.
    btrfs-progs
    radeontop
  ];

  # ============================================================
  # Filesystem — btrfs on NVMe
  # ============================================================

  # 1 TB HP EX920 NVMe (fast OS + build layer). Subvolume layout matches
  # mattfw: @=/, @home, @nix, @log. See INSTALL.md for partition steps.
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

  services.fstrim.enable = true;

  services.btrfs.autoScrub = {
    enable = true;
    interval = "monthly";
    fileSystems = [ "/" ];
  };

  # 16 GB swap — enough for OOM headroom alongside gaming + runner workloads.
  swapDevices = [
    {
      device = "/dev/disk/by-label/swap";
    }
  ];

  # ============================================================
  # ZFS — backup receive pool on SATA SSHD
  # ============================================================

  # hostId is required; ZFS uses it to determine pool ownership and refuse
  # imports from foreign hosts. Generated from:
  #   printf '%s' 'mattserver' | sha256sum | head -c 8
  networking.hostId = "0bf374c7";

  services.zfs.autoScrub = {
    enable = true;
    interval = "monthly";
  };

  # Dedicated backup user. Source hosts SSH in as this user and pipe
  # `zfs send` into `zfs receive`. ZFS delegation (zfs allow, one-time
  # setup in INSTALL.md) grants receive permission without a root shell.
  #
  # authorizedKeys intentionally empty: the previous shared
  # `inter-server` keypair was retired (see `nixos/common.nix` history
  # for context). The backup path documented in INSTALL.md isn't
  # currently wired up; if we ever set up syncoid/sanoid pushes we'll
  # mint a fresh dedicated keypair scoped to the backup user only.
  users.users.backup = {
    isNormalUser = true;
    openssh.authorizedKeys.keys = [ ];
  };

  # ============================================================
  # Buildkite agents (self-hosted PR-time CI)
  # ============================================================
  #
  # SEA-830: the box runs self-hosted Buildkite agents (was self-hosted
  # GitHub Actions runners pre-SEA-587 migration). PR-time Linux
  # pipelines (lints, test-linux, live-tests, deploy-docs) route to the
  # `linux-x64-selfhosted` queue these agents register. The whole agent
  # setup — services.buildkite-agents, the systemd-creds decrypt unit,
  # docker/envfs, the token + cache groups, the per-agent unit env, and
  # the build-dir cleanup timer — lives in the shared
  # nixos/modules/buildkite-agent.nix module (SEA-839), consumed here
  # and by the trashcan Linux runner. Only the per-host knobs live here.
  #
  # Two agent instances, 1:1 with the old runner count. Concurrency is
  # "one job per agent" (the agent claims a job, runs the container,
  # releases); job-internal parallelism (CARGO_BUILD_JOBS) lives in the
  # seal-ci container. The 3600 is 6c/12t — CPU is the bottleneck under a
  # double build (cores saturate while RAM stays <10 GB), so N=2 matches
  # the core budget. queue/arch take the module defaults
  # (linux-x64-selfhosted / x64).
  #
  # Host-bound secret staging stays per-host: encrypt the agent token +
  # ci-app-key once via `sudo bash
  # nixos/scripts/mattserver-encrypt-agent-token.sh` (writes
  # /etc/buildkite-agent/*.cred, host-bound via systemd-creds machine-ID
  # encryption); the module's decrypt-agent-token.service stages them
  # into tmpfs at boot.
  sealed.buildkiteAgent = {
    enable = true;
    agentNames = [
      "sealed"
      "sealed-2"
    ];
  };

  # ============================================================
  # Nix
  # ============================================================

  # Use all cores for parallel Nix builds (e.g. nix-switch rebuilding
  # the system itself). Separate from CARGO_BUILD_JOBS which governs
  # cargo parallelism within runner jobs.
  nix.settings = {
    max-jobs = "auto";
    cores = 0;
  };

  # ============================================================
  # Networking
  # ============================================================

  networking = {
    hostName = "mattserver";
    networkmanager.enable = true;
  };

  # nftables backend (replaces the iptables-based default).
  # `networking.firewall` works on either backend; we keep nftables
  # for the inbound firewall below.
  networking.nftables.enable = true;

  networking.firewall = {
    enable = true;
    # No inbound TCP from the LAN — SSH is Tailscale-only. The
    # bootstrap-time native sshd is unloaded post-bootstrap via
    # `disableSshd` activation below. Tailscale SSH (enabled via
    # `extraUpFlags = [ "--ssh" ]`) handles every interactive
    # session; the tailscale0 trust below covers tailnet inbound.
    allowedTCPPorts = [ ];
    trustedInterfaces = [ "tailscale0" ];
  };

  # NOTE: no host-level egress lockdown on the CI agent here.
  #
  # The pre-SEA-830 self-hosted-runner setup carried an nftables
  # `output`-hook rule that dropped the runner UID's egress to RFC1918
  # / link-local / CGNAT (LAN-pivot defense-in-depth). That rule was
  # valid when job code ran as forks of the runner UID in the host
  # network namespace. SEA-830 moved PR jobs INTO a podman container
  # (the Buildkite docker plugin), which breaks UID/GID-by-`output`
  # filtering two ways: (1) `meta skgid` matches only the primary GID,
  # and the agents join `ci-egress` as a supplementary group, so the
  # match never fires; (2) container egress traverses
  # `forward`/`postrouting`, not `output`, so the rule never sees the
  # job's packets at all. The control would have constrained only the
  # trusted agent daemon while silently failing open for the untrusted
  # workload — worse than no rule. Dropped here deliberately.
  #
  # Container-aware LAN egress isolation (a `forward`/`postrouting`
  # rule matching the podman network, or a CNI egress policy) is
  # tracked in SEA-835. The threat model (malicious CI job pivoting to
  # the home LAN; upstream PR-approval + dep cooldowns are the primary
  # defense) is unchanged — only the enforcement point needs redoing
  # for the container model. mattmini still runs jobs natively, so
  # its pf egress rule remains correctly scoped.

  # Unload the bootstrap-time native sshd. The mattserver-bootstrap
  # script enables it so the operator can SSH in over the LAN to
  # finish the first nixos-rebuild, but post-bootstrap Tailscale
  # SSH is the only access path we need.
  #
  # `lib.mkForce false` because `nixos/common.nix` sets
  # `services.openssh.enable = true` across every NixOS host — the
  # Pis + mattfw + mattpc-wsl rely on that default for their own
  # LAN-fallback access. Mattserver opts out per-host.
  services.openssh.enable = lib.mkForce false;

  # Drop Cockpit. Common.nix turns it on (with `openFirewall = true`
  # → 9090 inbound) so the Pis + mattfw have a tailnet-served
  # management UI; mattserver never wired up the matching
  # tailscale-serve unit, so the only path that ever reached it was
  # http://<lan-ip>:9090 — i.e. LAN-inbound, defeating the same
  # lockdown the sshd drop above is closing. Replaced operationally
  # by Tailscale SSH + `btm` for live monitoring. Same per-host
  # override pattern as mattpc-wsl. SEA-672 follow-up.
  services.cockpit.enable = lib.mkForce false;

  services.tailscale = {
    enable = true;
    openFirewall = true;
    extraUpFlags = [ "--ssh" ];
  };

  # ============================================================
  # Services
  # ============================================================

  services.timesyncd.enable = true;

  services.xserver.xkb.layout = "us";
}

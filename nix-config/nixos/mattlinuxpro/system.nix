{
  pkgs,
  lib,
  ...
}:

# mattlinuxpro — 2013 "trashcan" Mac Pro (Intel Xeon E5, 64 GB DDR3),
# converted from a retired macOS Buildkite runner to a headless NixOS
# Linux runner (SEA-839).
#
# Role: self-hosted Buildkite CI agents (sealedsecurity org), joining
# mattserver on the `linux-x64-selfhosted` queue to add x64 Linux
# capacity (the bottleneck queue). No desktop, no gaming, no ZFS — this
# box does one job. The whole agent setup lives in the shared
# nixos/modules/buildkite-agent.nix module; only the per-host knobs are
# here.
#
# Why this hardware is clean for Linux: a 2013 trashcan has NO T2 chip
# (that's 2018+), so none of the T2 storage / secure-boot headaches.
# Standard EFI x86_64 box — NVMe/PCIe SSD, ethernet, Thunderbolt all
# Linux-supported. The dual FirePro GPUs are irrelevant headless.

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

    kernelPackages = pkgs.linuxPackages;

    kernel.sysctl = {
      "vm.swappiness" = 10;
    };

    # tmpfs /tmp for Rust incremental build artifacts + wasm
    # cross-compile intermediates. Consumer NVMe fsync is slow under the
    # O_DSYNC the test harness does, so keeping /tmp in RAM avoids the
    # per-write amplification (SEA-841 documents the audit-store variant
    # of this on mattserver). 64 GB of DDR3 leaves ample headroom for a
    # generous tmpfs even with two concurrent jobs (~20 GB RSS).
    tmp = {
      useTmpfs = true;
      tmpfsSize = "32G";
    };
  };

  # ============================================================
  # Hardware
  # ============================================================

  hardware = {
    cpu.intel.updateMicrocode = true;
    enableRedistributableFirmware = true;
    enableAllFirmware = true;
  };

  # Performance CPU governor — avoids frequency ramp-up delay at the
  # start of a CI job. CI throughput is the only thing this box does, so
  # hold boost clocks from the first instruction.
  powerManagement.cpuFreqGovernor = "performance";

  # Never let the box sleep — a suspended runner means CI jobs falling
  # back to GitHub-hosted runners. Mask at the systemd target level so
  # `systemctl suspend`, idle timers, and session-level "sleep now"
  # calls all become no-ops.
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
    # Host-specific extras. The system-profiling toolkit (pciutils,
    # usbutils, lm_sensors, dmidecode, lshw, …) lives in
    # nixos/common.nix and applies here automatically.
    btrfs-progs
  ];

  # ============================================================
  # Filesystem — btrfs on the internal SSD
  # ============================================================

  # Subvolume layout matches mattserver/mattfw: @=/, @home, @nix, @log.
  # See INSTALL.md for partition steps.
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

  # 16 GB swap — OOM headroom alongside the runner workload.
  swapDevices = [
    {
      device = "/dev/disk/by-label/swap";
    }
  ];

  # ============================================================
  # Buildkite agents (self-hosted PR-time CI)
  # ============================================================
  #
  # SEA-839: the converted trashcan joins mattserver on the
  # `linux-x64-selfhosted` queue (PR-time Linux pipelines: lints,
  # test-linux, live-tests, deploy-docs). The whole agent setup —
  # services.buildkite-agents, the systemd-creds decrypt unit,
  # docker/envfs, the token + cache groups, the per-agent unit env, and
  # the build-dir cleanup timer — lives in the shared
  # nixos/modules/buildkite-agent.nix module. Only the per-host knobs
  # live here.
  #
  # Two agent instances. Concurrency is "one job per agent" (the agent
  # claims a job, runs the container, releases); job-internal parallelism
  # lives in the seal-ci container. Each job sees all visible cores (no
  # CARGO_BUILD_JOBS / nextest test-threads pin in the pipeline), so N
  # agents = N× CPU oversubscription regardless of core count. N=2 is the
  # proven-green ratio on mattserver and was the ceiling the native macOS
  # runner on this same box settled at — past 2× oversub, the
  # starvation-sensitive cancel-path tests (serialized in
  # .config/nextest.toml) hang past the kill switch. The Xeon's 12c/24t
  # could feed more agents, but only with per-container `--cpus` pinning
  # so each job is confined to a core slice instead of grabbing all 24;
  # that pinning touches the shared pipeline (which also runs on
  # ephemeral hosted runners that must NOT be sliced), so it's deliberately
  # a separate follow-up (SEA-844). Until then N=2 doubles Linux capacity
  # vs. the single mattserver host with zero pipeline risk. queue/arch
  # take the module defaults (linux-x64-selfhosted / x64).
  #
  # Host-bound secret staging stays per-host: encrypt the agent token +
  # ci-app-key once via `sudo bash
  # nixos/scripts/mattlinuxpro-encrypt-agent-token.sh` (writes
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
  # the system itself). Separate from the per-job cargo parallelism
  # inside runner containers.
  nix.settings = {
    max-jobs = "auto";
    cores = 0;
  };

  # ============================================================
  # Networking
  # ============================================================

  networking = {
    hostName = "mattlinuxpro";
    networkmanager.enable = true;
  };

  # hostId is required by some stateful services and cheap to set.
  # Generated from:
  #   printf '%s' 'mattlinuxpro' | sha256sum | head -c 8
  networking.hostId = "cfe5e83a";

  # nftables backend (replaces the iptables-based default).
  networking.nftables.enable = true;

  networking.firewall = {
    enable = true;
    # No inbound TCP from the LAN — SSH is Tailscale-only. The
    # bootstrap-time native sshd is unloaded post-bootstrap (below).
    # Tailscale SSH (`extraUpFlags = [ "--ssh" ]`) handles every
    # interactive session; the tailscale0 trust below covers tailnet
    # inbound.
    allowedTCPPorts = [ ];
    trustedInterfaces = [ "tailscale0" ];
  };

  # NOTE: no host-level egress lockdown on the CI agent here. PR jobs run
  # inside the seal-ci container (the Buildkite docker plugin), and
  # container egress traverses `forward`/`postrouting`, not `output`, so
  # a UID/GID-by-`output` nftables rule never sees the job's packets.
  # Container-aware LAN egress isolation is tracked in SEA-835; the
  # threat model (malicious CI job pivoting to the home LAN; upstream
  # PR-approval + dep cooldowns are the primary defense) is shared with
  # mattserver. Same deliberate omission as mattserver's networking NOTE.

  # Unload the bootstrap-time native sshd. The bootstrap script enables
  # it so the operator can SSH in over the LAN to finish the first
  # nixos-rebuild, but post-bootstrap Tailscale SSH is the only access
  # path we need.
  #
  # `lib.mkForce false` because `nixos/common.nix` sets
  # `services.openssh.enable = true` across every NixOS host — the Pis +
  # mattfw + mattpc-wsl rely on that default for their own LAN-fallback
  # access. This host opts out per-host (same pattern as mattserver).
  services.openssh.enable = lib.mkForce false;

  # Drop Cockpit. Common.nix turns it on (with `openFirewall = true` →
  # 9090 inbound) for the Pis + mattfw; this host never wired up a
  # matching tailscale-serve unit, so the only path that ever reached it
  # was LAN-inbound, defeating the same lockdown the sshd drop above is
  # closing. Live monitoring is via Tailscale SSH + `btm`. Same per-host
  # override pattern as mattserver / mattpc-wsl.
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

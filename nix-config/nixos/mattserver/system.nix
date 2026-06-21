{
  pkgs,
  lib,
  ...
}:

# mattserver — Old gaming PC (AMD Ryzen 3600 + RX 5700 XT, 64 GB DDR4),
# now a headless server. Roles:
#   1. ZFS backup receive target (btrfs root on 1TB NVMe; ZFS pool on 2TB SATA SSHD)
#   2. Self-hosted Buildkite CI agents (sealedsecurity org) + the
#      sccache Redis L1 tier
#
# The KDE Plasma / Steam gaming desktop was removed (it's a server now);
# that setup lives on as the reusable nixos/modules/gaming-desktop.nix
# for another box if wanted.

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

    # No hardware.graphics — this is a headless server. The RX 5700 XT
    # is unused (no GL/Vulkan rendering, no 32-bit Steam stack); only
    # the firmware below is kept so the GPU + other devices initialise
    # cleanly at the console.
    enableRedistributableFirmware = true;
    enableAllFirmware = true;
  };

  # Performance CPU governor — avoids frequency ramp-up delay at the start
  # of a CI job. The 3600 idles at ~2.2 GHz under schedutil;
  # `performance` holds it at boost clocks from the first instruction.
  powerManagement.cpuFreqGovernor = "performance";

  # Never let the box sleep. It's a backup target + CI runner host, so a
  # suspended host means missed snapshots and CI jobs falling back to
  # Buildkite-hosted runners.
  #
  # Masks at the systemd target level so `systemctl suspend`, idle timers,
  # and any session-level "sleep now" call all become no-ops.
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

  # 16 GB swap — OOM headroom alongside the CI runner + Redis workloads.
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

  # Cap the ZFS ARC hard at 1 GiB. ARC defaults to ~50% of RAM (here that
  # would be ~32 GB), which competes directly with the things this box
  # actually cares about now: the sccache Redis L1 (up to maxmemory 40 GB)
  # and the two Buildkite agents' build jobs. The ZFS pool is a
  # backup-receive target on a slow SATA SSHD that isn't even wired up
  # currently — its caching matters far less than keeping RAM free for
  # Redis + CI, so give it the smallest practical window. In bytes:
  # 1 GiB = 1073741824. SEA-843.
  boot.kernelParams = [ "zfs.zfs_arc_max=1073741824" ];

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

  # Redis as the shared L1 tier of the CI sccache chain
  # (disk → redis → R2). sccache's on-disk LRU index is single-writer,
  # so the per-agent disk L0 can't be shared between agents/boxes; R2
  # is durable but ~80ms/object over the network. Redis is the missing
  # concurrency-safe fast tier: when one agent compiles a crate, any
  # other agent on the tailnet (mattserver's own + mattlinuxpro) reuses
  # the object at ~1ms instead of fetching from R2. The seal CI
  # pipelines point `SCCACHE_REDIS_ENDPOINT` here; `write_error_policy`
  # stays `l0` so a Redis outage just falls back to disk+R2. SEA-843.
  #
  # Reachability: jobs run inside the seal-ci container (Buildkite
  # docker plugin, default bridge network), so `localhost` from inside
  # the container is the container, not this host. The CI containers
  # reach this host over the tailnet (mattserver-local containers NAT
  # out to this host's own tailscale0; mattlinuxpro's go over the
  # tailnet), so Redis has to listen on the tailscale interface.
  #
  # Bind 0.0.0.0 and let the firewall + tailnet ACL enforce access
  # rather than binding the tailscale IP explicitly. Two reasons:
  #   1. Boot robustness — the tailscale IP (100.91.42.76) only exists
  #      once tailscaled has connected, which happens async after boot.
  #      Binding it explicitly would crash-loop redis until the IP
  #      appears (and again on every tailscale restart). 0.0.0.0 is
  #      always bindable.
  #   2. The firewall is already the enforcement boundary for every
  #      service on this host — SSH is tailscale-only via the same
  #      `trustedInterfaces = [ "tailscale0" ]` + `allowedTCPPorts = []`
  #      mechanism (networking block above), not via per-service binds.
  #      Binding 0.0.0.0 here is consistent with that model: inbound
  #      from anything but tailscale0 is dropped.
  #
  # Access is narrowed at the tailnet ACL layer: mattserver
  # carries a `tag:redis`, and the grant
  # `[tag:ci-runner, tag:dev, mattwilki17@gmail.com] -> tag:redis:6379`
  # (privatefiles/tailscale/tailscale.jsonc) means only the CI runners
  # (tag:ci-runner — Linux + macOS), the dev boxes (tag:dev), and Matt's
  # own machines reach Redis — not the rest of the tailnet, and not
  # non-runner servers (rpi4 DNS, rpi5 exit node, mattmacpro). The ACL
  # is the PRIMARY boundary; a `requirepass` (requirePassFile below) is
  # layered on top as defense-in-depth, so a compromised tag:ci-runner /
  # tag:dev peer still can't read or poison the cache without the
  # password. (sccache keys embed the target triple, so only same-arch
  # boxes actually share objects — Linux-x64 boxes share for real;
  # macOS-arm64 grantees always-miss against this Linux cache,
  # harmlessly.)
  services.redis.servers.sccache = {
    enable = true;
    port = 6379;
    # Listen on all interfaces; the firewall (trustedInterfaces =
    # [ "tailscale0" ], allowedTCPPorts = []) is what restricts inbound
    # to the tailnet. See the block comment above for why this is bound
    # 0.0.0.0 rather than the explicit tailscale IP.
    bind = "0.0.0.0";
    # Defense-in-depth password (the tailnet ACL is the primary
    # boundary). Read from a file outside the nix store — staged at boot
    # by decrypt-redis-password.service below from a host-bound
    # systemd-creds blob, same pattern as the agent token. The seal CI
    # pipelines pass the matching value via the SCCACHE_REDIS_PASSWORD
    # Buildkite secret. SEA-843.
    requirePassFile = "/run/redis-password/requirepass";
    # It's a pure cache: no RDB snapshots, no AOF. Losing the dataset
    # on restart just re-warms from R2 (the durable L2), so persistence
    # would be wasted fsync churn on the backup-target's disks.
    save = [ ];
    appendOnly = false;
    settings = {
      # Bias HIGH: the R2 working set is ~25 GB and growing; size the
      # cap above it so the cache stays fully resident with ~zero
      # eviction. 40 GB holds today's cache + headroom; even if a
      # concurrent ~10 GB build coexists, that's ~50/64 GB — comfortable
      # on this box. allkeys-lru is a backstop that should rarely fire
      # at this cap. Revisit if the R2 bucket grows materially.
      maxmemory = "40gb";
      maxmemory-policy = "allkeys-lru";
      # Disable Redis protected-mode. Protected-mode (the default)
      # refuses every non-loopback connection UNLESS a password is set.
      # We now set requirePassFile, so protected-mode would technically
      # allow remote auth'd connections — but leave it explicitly off:
      # the access model is firewall + tailnet ACL + requirepass, and
      # protected-mode's loopback-only heuristic is orthogonal to that
      # and only adds confusion (it was the cause of the earlier
      # "remote service unreachable" failures before requirepass
      # existed). SEA-843.
      "protected-mode" = false;
    };
  };

  # Stage the Redis password (defense-in-depth for the sccache L1, see
  # the services.redis block above). Decrypts a host-bound systemd-creds
  # blob to /run/redis-password/requirepass before redis starts; the
  # redis module's prep-conf ExecStartPre (runs as root) cats that file
  # into `requirepass`. Same machine-ID-bound encryption as the
  # buildkite agent token — create the blob once on the box with
  # nixos/scripts/mattserver-encrypt-redis-password.sh (writes
  # /etc/buildkite-agent/redis-password.cred). Plaintext never touches
  # persistent storage after that. SEA-843.
  systemd.services.decrypt-redis-password = {
    description = "Decrypt the sccache Redis password into /run";
    wantedBy = [ "multi-user.target" ];
    # Ordered before the redis instance so the password file exists when
    # redis-sccache's prep-conf reads requirePassFile.
    before = [ "redis-sccache.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      # Own RuntimeDirectory (not redis's) so this unit can create the
      # file before redis-sccache starts and owns its own /run dir.
      # 0750 dir + 0640 file: the redis prep-conf runs as root (`+`
      # prefix in the module) and only needs read; the value's
      # confidentiality is the .cred blob, not the runtime file perms,
      # but keep the file 0640 root:root to be tidy.
      RuntimeDirectory = "redis-password";
      RuntimeDirectoryMode = "0750";
      RuntimeDirectoryPreserve = true;
      # `+` runs ExecStart as root for systemd-creds decrypt against the
      # host machine-ID key.
      ExecStart = [
        ''
          +${pkgs.writeShellScript "decrypt-redis-password" ''
            set -euo pipefail
            ${pkgs.systemd}/bin/systemd-creds decrypt \
              --name=redis-sccache-password \
              /etc/buildkite-agent/redis-password.cred \
              /run/redis-password/requirepass
            chmod 0640 /run/redis-password/requirepass
          ''}
        ''
      ];
    };
  };

  # Make the redis instance wait for the password to be staged. The
  # buildkite-agents module owns redis-sccache.service; declaring the
  # ordering here merges into it.
  systemd.services.redis-sccache = {
    after = [ "decrypt-redis-password.service" ];
    requires = [ "decrypt-redis-password.service" ];
  };

  services.xserver.xkb.layout = "us";
}

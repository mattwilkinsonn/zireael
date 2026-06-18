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

let
  # Shared supplementary group the egress filter keys on. The nixpkgs
  # services.buildkite-agents module creates a per-agent system user
  # (buildkite-agent-sealed, buildkite-agent-sealed-2, …), so there's
  # no single static UID to match in the nftables rule like the old
  # github-runner setup had. Instead every agent joins this group and
  # the egress lockdown matches `meta skgid "ci-egress"` — one rule
  # covers any number of agent instances. SEA-830.
  egressGroup = "ci-egress";

  # Per-PR-pipeline jobs run INSIDE the seal-ci container (the
  # Buildkite docker plugin launches it), not natively on the agent.
  # So the agent host only needs: the buildkite-agent binary, a base
  # userland, and a docker-compatible CLI to talk to the container
  # runtime. The Rust toolchain, sccache, dbus, protobuf, etc. all
  # live in the seal-ci image now — none of it belongs on the agent.
  #
  # `docker-client` is the CLI only (no dockerd); it talks to podman's
  # docker-compat socket at /run/docker.sock (virtualisation.podman
  # .dockerSocket.enable in nixos/common.nix symlinks it there). The
  # agent user joins the `podman` group (relaxed to 0660 in common.nix)
  # so the socket is reachable.
  agentRuntimePackages = with pkgs; [
    bash
    coreutils
    gnutar
    gzip
    git
    nix
    # Buildkite plugins + hook scripts reach for these directly on the
    # agent (outside the job container): jq for plugin JSON, curl for
    # the gh-app-token mint, gnugrep/gnused for shell glue.
    jq
    curl
    gnugrep
    gnused
    # Container CLI — the docker plugin shells out to `docker`.
    docker-client
  ];

  # Two agent instances, 1:1 with the old runner count. Concurrency is
  # now "one job per agent" (the agent claims a job, runs the container,
  # releases) — N=2 matches the historical SEA-680 round-4 shape so a
  # 4-PR stack serialises as 2 batches of 2. Job-internal parallelism
  # (CARGO_BUILD_JOBS) lives in the seal-ci container now, not here.
  # Shared agent definition. Both instances are identical (the module
  # derives each agent's name + dataDir from its attrset key in
  # services.buildkite-agents below), so this is a plain value, not a
  # `name:`-parameterised function.
  agentConfig = {
    enable = true;

    # Registration token — the Buildkite *Agent* token (org Agents
    # page), NOT the BUILDKITE_API_TOKEN used by the `bk` CLI. Decrypted
    # at boot from the host-bound systemd-creds blob into tmpfs by
    # decrypt-agent-token.service below; the module reads it via
    # tokenPath at preStart. See the buildkite-agents handoff doc.
    tokenPath = "/run/buildkite-agent/agent-token";

    # Tags the pipelines match on. `queue` is the routing key the
    # repointed .buildkite/pipelines/*.yml steps target; the os/arch
    # tags are informational + available for future pinning.
    tags = {
      queue = "linux-x64-selfhosted";
      os = "linux";
      arch = "x64";
      host = "mattserver";
    };

    runtimePackages = agentRuntimePackages;

    # Join the egress-filter group (kernel nftables lockdown below) and
    # the podman group (docker-compat socket access).
    extraGroups = [
      egressGroup
      "podman"
    ];

    # Don't garbage-collect the agent's nix deps mid-build, and give the
    # build directory a stable home on the NVMe-backed state dir (the
    # module defaults dataDir to /var/lib/buildkite-agent-<name>, which
    # is already on btrfs root here — fine as-is).
  };
in

{
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
  # `linux-x64-selfhosted` queue these agents register.
  #
  # Execution model: each PR job runs INSIDE the seal-ci container,
  # launched by the Buildkite docker plugin. The agent host only
  # provides the agent binary + a docker-compatible CLI (podman's
  # docker-compat socket — virtualisation.podman.dockerSocket in
  # nixos/common.nix). The Rust toolchain, sccache, dbus, etc. live in
  # the seal-ci image, NOT on the agent — which is why the old native
  # toolchain `extraPackages`, the shared sccache server, and the
  # bwrap-sandbox-unwrap serviceOverrides are all gone: bwrap now runs
  # inside the container the plugin launches, configured by the
  # pipeline's own cap/userns flags.
  #
  # Compile-result caching is sccache → Cloudflare R2 configured inside
  # the container (cross-agent, works hosted or self-hosted). Warm
  # cargo-registry / target-incremental bind-mounts into the container
  # are a follow-up (B1-full) — the cutover lands cold-registry first.
  services.buildkite-agents = {
    sealed = agentConfig;
    sealed-2 = agentConfig;
  };

  # Egress-filter group every agent joins (see the nftables lockdown
  # below). The buildkite-agents module creates per-agent users
  # (buildkite-agent-sealed, -sealed-2); this shared supplementary
  # group is what the kernel egress rule matches so one rule covers all
  # instances regardless of count.
  users.groups.${egressGroup} = { };

  # Token provisioning. The agents register with the Buildkite *Agent*
  # token (org Agents page) — distinct from the BUILDKITE_API_TOKEN the
  # `bk` CLI uses. Encrypt it once on the host (host-bound via
  # systemd-creds machine-ID encryption):
  #
  #   sudo bash nixos/scripts/mattserver-encrypt-agent-token.sh
  #
  # That writes /etc/buildkite-agent/agent-token.cred. The
  # decrypt-agent-token.service oneshot below decrypts it to
  # /run/buildkite-agent/agent-token (tmpfs, mode 640, readable by the
  # agent group) before any agent unit starts. Plaintext never touches
  # persistent storage after the bootstrap step.
  systemd.services.decrypt-agent-token = {
    description = "Decrypt the Buildkite agent token into /run";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      RuntimeDirectory = "buildkite-agent";
      RuntimeDirectoryMode = "0750";
      RuntimeDirectoryPreserve = true;
      # `+` prefix runs ExecStart as root (needed for systemd-creds
      # decrypt against the host machine-ID key). The token is then
      # made group-readable by the egress group so every per-agent
      # user (all members of the group) can read it via tokenPath.
      ExecStart = [
        ''
          +${pkgs.writeShellScript "decrypt-agent-token" ''
            set -euo pipefail
            ${pkgs.systemd}/bin/systemd-creds decrypt \
              --name=buildkite-agent-token \
              /etc/buildkite-agent/agent-token.cred \
              /run/buildkite-agent/agent-token
            chgrp ${egressGroup} /run/buildkite-agent/agent-token
            chmod 640 /run/buildkite-agent/agent-token
          ''}
        ''
      ];
    };
  };

  # Order each agent unit behind the decrypt. The module names units
  # `buildkite-agent-<name>`; keep this list in sync with
  # services.buildkite-agents above. Declared by single-key path so the
  # module system MERGES with the module-generated units rather than
  # replacing them.
  systemd.services.buildkite-agent-sealed = {
    after = [ "decrypt-agent-token.service" ];
    requires = [ "decrypt-agent-token.service" ];
  };
  systemd.services.buildkite-agent-sealed-2 = {
    after = [ "decrypt-agent-token.service" ];
    requires = [ "decrypt-agent-token.service" ];
  };

  # ============================================================
  # Agent build-dir cleanup
  # ============================================================

  # Weekly prune of Rust target/ directories left in the agents' build
  # checkouts. The module puts each agent's checkouts under
  # /var/lib/buildkite-agent-<name>/builds/; a single workspace target/
  # can reach 10-20 GB. 30-day window keeps warm-ish trees for
  # infrequent-push branches while bounding disk. (target/ inside the
  # *container* is separate; this is just the host-side checkout state.)
  systemd.services.agent-cache-cleanup = {
    description = "Prune stale Rust target/ directories from agent build dirs";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = pkgs.writeShellScript "agent-cache-cleanup" ''
        find /var/lib/buildkite-agent-sealed /var/lib/buildkite-agent-sealed-2 \
          -maxdepth 6 \
          -name "target" -type d \
          -mtime +30 \
          -print0 2>/dev/null | xargs -0 -r rm -rf
      '';
    };
  };

  systemd.timers.agent-cache-cleanup = {
    description = "Weekly cleanup of stale agent Rust build artifacts";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar = "weekly";
      Persistent = true;
    };
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

  # nftables backend (replaces the iptables-based default). Required
  # for the agent-egress filter below — `networking.firewall` still
  # works on either backend; nftables gives us the group-matched
  # output chain we want for the lockdown.
  networking.nftables.enable = true;
  # Skip the rule-check at build time. The check would invoke `nft -c`
  # inside the build sandbox, where only nixbld* users exist —
  # `meta skgid != "ci-egress"` fails name resolution and the
  # build aborts with "Group does not exist". The check still runs at
  # activation time on the host, where ci-egress is present.
  networking.nftables.checkRuleset = false;

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

  # Egress lockdown for the self-hosted CI agent pool.
  #
  # Threat model: a malicious workflow lands code-exec as a Buildkite
  # agent process. Already mitigated upstream by external-PR workflow
  # approval + dep cooldowns + Tailscale ACLs gating tailnet peer
  # reachability. This rule is defense-in-depth: cut the agent off from
  # the LAN (router admin, NAS, other workstations) and from
  # CGNAT-routed paths that bypass tailscaled.
  #
  # Why nftables (kernel, by GID) and not systemd's per-unit
  # `IPAddressDeny=` / `PrivateNetwork=`: PR jobs run inside a container
  # the agent launches via the docker plugin, and a per-unit network
  # filter wouldn't follow the job into the container's network path
  # cleanly. Filtering by GID at the kernel survives every fork inside
  # the unit, the container launch, and any in-process privilege
  # escalation short of root.
  #
  # `meta skgid` is the source GID for outbound packets. The
  # buildkite-agents module gives each agent its own per-name user
  # (buildkite-agent-sealed, -sealed-2), so there's no single UID to
  # match like the old shared-runner setup — instead every agent joins
  # the `ci-egress` group and we match the GID. nft resolves
  # `"ci-egress"` via getgrnam at ruleset load, robust against the
  # auto-allocated GID.
  #
  # Allowed egress for the agent group:
  #   - loopback (localhost services)
  #   - DNS (53/udp+tcp, any destination — needed for
  #     github.com, crates.io, registries, buildkite.com)
  #   - HTTP/HTTPS (80, 443) to public destinations (registries,
  #     GitHub API, Buildkite agent API, R2 sccache, artifacts)
  #   - git+ssh egress (22) for `cargo install --git` etc.
  #   - tailscale0 interface (tailnet ACLs govern peers)
  #
  # Dropped egress for the agent group:
  #   - any TCP/UDP/ICMP to RFC1918 (10/8, 172.16/12, 192.168/16)
  #   - link-local (169.254/16)
  #   - CGNAT (100.64/10) on non-tailscale0 paths — tailscale0
  #     egress was already passed above; this catches anyone
  #     trying to route around tailscaled
  #   - IPv6 ULA + link-local
  #
  # The `log prefix "agent-egress-drop: "` action surfaces blocked
  # attempts in the journal so we can see what legitimate egress (if
  # any) got caught and tighten the allowlist. After a week of clean
  # logs, the prefix can be demoted to bare `drop`.
  networking.nftables.tables.agent_egress = {
    family = "inet";
    content = ''
      chain output {
        type filter hook output priority filter; policy accept;

        # Skip the rule for any process not in the agent egress group.
        # The buildkite-agents module gives each agent its own per-name
        # user, all members of `ci-egress`; matching the GID covers
        # every agent instance with one rule. `meta skgid` resolves the
        # group name via getgrnam at ruleset load.
        meta skgid != "ci-egress" accept

        # Allow loopback — localhost-only services.
        oifname "lo" accept

        # Allow tailscale0 egress — tailnet ACLs are the policy layer
        # for peer reachability.
        oifname "tailscale0" accept

        # Allow DNS to any destination.
        udp dport 53 accept
        tcp dport 53 accept

        # Drop RFC1918 / link-local / CGNAT (non-tailscale0) before
        # the public-internet allow so the agent can't fall through
        # the 443-allow into a LAN host listening on 443.
        ip daddr { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16,
                   169.254.0.0/16, 100.64.0.0/10 } \
          log prefix "agent-egress-drop: " level warn drop
        ip6 daddr { fc00::/7, fe80::/10 } \
          log prefix "agent-egress-drop: " level warn drop

        # Force v4 fallback for outbound TCP from this group.
        #
        # Strace evidence (2026-06-01 round-9 diagnostic) showed
        # `bun install` opening ~30 parallel non-blocking SYNs to
        # 2606:4700::6810:XXX:443 (Cloudflare /120, npmjs.org).
        # Every connect returned EINPROGRESS and then ETIMEDOUT
        # ~10s later. ZERO agent-egress drops fired during the
        # hang window — the SYNs leave the box and never get a
        # SYN-ACK. Sequential v4 (curl, single connection) to the
        # same Cloudflare service works in <300ms. Likely an
        # upstream anti-flood / WAF on Cloudflare's edge that
        # rate-limits a SYN burst from one source v6, OR a
        # SLAAC-allocated /64 we're not propagating cleanly.
        #
        # The fix is surgical: reject outbound v6 TCP for this group
        # so getaddrinfo's v4 results (also returned in the resolver
        # answer) are tried instead.
        #
        # Why `reject` (not `drop`):
        #   `drop` would silently swallow the SYN and the local
        #   socket would still sit in EINPROGRESS until the kernel's
        #   tcp_syn_retries exhausts (~127s) — the exact symptom we
        #   already see from the upstream silent drop. `reject with
        #   icmpx admin-prohibited` synthesizes an ICMPv6 admin-
        #   prohibited back into the local stack so the socket fails
        #   immediately with EACCES/EHOSTUNREACH; bun blows through
        #   its v6 candidates in microseconds and lands on v4 inside
        #   one app-level timeout instead of fifty.
        #
        # Why `meta nfproto ipv6 meta l4proto tcp` (not
        # `ip6 nexthdr tcp`):
        #   `ip6 nexthdr` inspects only the base header's Next
        #   Header field; if there's any extension header (Hop-by-
        #   Hop, Routing, Destination, Fragment) the match misses
        #   and the packet leaks through to the `tcp dport 443
        #   accept` below. `meta l4proto` walks the nexthdr chain.
        #   The `nfproto ipv6` gate is required because in this
        #   `inet`-family table `meta l4proto tcp` alone would
        #   match v4 TCP too and kill the v4 path.
        #
        # Distinct log prefix so future debugging can tell "v6
        # force-downgrade" from "actual policy block."
        meta nfproto ipv6 meta l4proto tcp \
          log prefix "agent-egress-drop-v6tcp: " level warn \
          reject with icmpx type admin-prohibited

        # Allow public HTTP/HTTPS + git+ssh.
        tcp dport { 80, 443, 22 } accept

        # Default deny for anything else this group tries.
        log prefix "agent-egress-drop: " level warn drop
      }
    '';
  };

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

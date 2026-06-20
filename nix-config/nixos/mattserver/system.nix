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
  # Shared supplementary group every agent joins so they can all read
  # the one decrypted agent-token file. The nixpkgs
  # services.buildkite-agents module creates a per-agent system user
  # (buildkite-agent-sealed, buildkite-agent-sealed-2, …) with its own
  # primary group, so there's no shared group to chgrp the token to.
  # This group provides one: decrypt-agent-token.service writes the
  # token mode 0640 owned by this group, and each agent (a member)
  # reads it via tokenPath. SEA-830.
  #
  # (Pre-SEA-835 this group also keyed an nftables egress filter; that
  # rule was dropped because it couldn't cover the container workload
  # — see the NOTE in the networking section. The group's remaining
  # job is shared token-read.)
  agentTokenGroup = "buildkite-token";

  # Shared group owning the Buildkite cache dir (/cache/bkcache). The
  # Buildkite cache plugin (used by test-linux/lints/live-tests for the
  # cargo target/ + bun-install caches) symlinks each job's cache paths
  # under /cache/bkcache and the docker plugin mounts that root into the
  # container. Pre-propagate-uid-gid the container started as root and
  # chowned the cache mount, so the host dir's owner didn't matter; now
  # the container (and the cache plugin) run as the agent uid, which must
  # be able to create + write entries under /cache/bkcache. The two agents
  # have distinct primary uids, so a single owner can't satisfy both — a
  # shared group + setgid dir (2775, declared via tmpfiles below) lets both
  # write and makes new entries inherit the group. SEA-830.
  agentCacheGroup = "buildkite-cache";

  # Git credential helper for checkout-time clone auth — mints a
  # sealedsecurity-ci App installation token over HTTPS. Shared with
  # mattmini (shared/buildkite-git-credential-app.nix); this host
  # decrypts the App key to /run/buildkite-agent/ci-app-key.pem via
  # decrypt-agent-token.service below (which stages both secrets). App
  # ID 4045728 is a public identifier, not a secret.
  ciGitCredentialHelper = import ../../shared/buildkite-git-credential-app.nix {
    inherit pkgs;
    appId = "4045728";
    keyPath = "/run/buildkite-agent/ci-app-key.pem";
  };

  # Git config for the AGENT processes only — pointed at via
  # GIT_CONFIG_GLOBAL in each agent unit's environment (below), NOT
  # written to /etc/gitconfig. A system-wide insteadOf rewrite is
  # additive and can't be cancelled at a lower config level, so putting
  # it in /etc/gitconfig would silently redirect mattw's own
  # git@github.com: SSH clones to HTTPS+App-token too. Scoping it to the
  # agents' GIT_CONFIG_GLOBAL keeps it off every other user's git.
  #
  #   - rewrite the SSH-form GitHub URL the Buildkite pipeline uses
  #     (git@github.com:owner/repo) to HTTPS so the helper applies;
  #   - register the App-token credential helper for github.com HTTPS.
  agentGitConfig = pkgs.writeText "buildkite-agent-gitconfig" ''
    [url "https://github.com/"]
        insteadOf = git@github.com:
    [credential "https://github.com"]
        helper = ${ciGitCredentialHelper}/bin/buildkite-git-credential-app
  '';

  # Per-PR-pipeline jobs run INSIDE the seal-ci container (the
  # Buildkite docker plugin launches it), not natively on the agent.
  # So the agent host only needs: the buildkite-agent binary, a base
  # userland, and a docker-compatible CLI to talk to the container
  # runtime. The Rust toolchain, sccache, dbus, protobuf, etc. all
  # live in the seal-ci image now — none of it belongs on the agent.
  #
  # `docker` is the real CLI + talks to the dockerd this host enables
  # (virtualisation.docker below). mattserver runs real dockerd for CI
  # rather than podman's docker-compat socket: the bwrap-in-container
  # sandbox tests need the same container-exec behavior the hosted
  # Buildkite agents (docker) and future GCP VMs (docker) provide, and
  # podman's rootless-userns nesting diverges from docker for the
  # in-container `bwrap --proc` mount. Standardizing on dockerd keeps
  # ONE portable pipeline across hosted + self-hosted + cloud. The agent
  # user joins the `docker` group to reach /run/docker.sock. (Shared
  # nixos/common.nix still enables podman for the rpis' oci-containers;
  # this host overrides its docker-compat socket off — see below — so
  # dockerd owns /run/docker.sock.)
  agentRuntimePackages = with pkgs; [
    bash
    coreutils
    gnutar
    gzip
    git
    nix
    # Buildkite plugins + hook scripts reach for these directly on the
    # agent (outside the job container): jq for plugin JSON, curl +
    # openssl for the gh-app-token JWT mint (the ci-image GHCR login
    # runs gh-app-token.sh on the agent), gnugrep/gnused for shell glue.
    jq
    curl
    openssl
    gnugrep
    gnused
    # Container CLI — the docker plugin shells out to `docker`, talking
    # to the real dockerd enabled on this host.
    docker
  ];

  # Two agent instances, 1:1 with the old runner count. Concurrency is
  # now "one job per agent" (the agent claims a job, runs the container,
  # releases) — N=2 matches the historical SEA-680 round-4 shape so a
  # 4-PR stack serialises as 2 batches of 2. Job-internal parallelism
  # (CARGO_BUILD_JOBS) lives in the seal-ci container now, not here.
  # Per-instance agent config. The buildkite-agents module derives
  # dataDir = /var/lib/buildkite-agent-<name> from the attrset key, so
  # each agent gets its own plugins dir under it (set via extraConfig
  # below) — keyed by `name`.
  mkAgent = name: {
    enable = true;

    # pre-checkout poisoning cleanup. Under docker + ci-entrypoint the
    # job container starts as root and chowns the bind-mounted checkout
    # to uid 1000 (the in-image workload uid). On a PERSISTENT agent the
    # next job's checkout/cleanup runs as the agent user (uid != 1000)
    # and can't unlink those root/1000-owned files ("dubious ownership"
    # + permission denied on .git/*). Hosted agents never hit this —
    # they're ephemeral. Before each checkout, chown the build tree back
    # to the agent uid via a throwaway ROOT container (the agent can
    # launch containers but isn't root itself, so a bare chown can't
    # cross the uid). Uses busybox — tiny + cached, decoupled from the
    # seal-ci image so a poisoned dir gets cleaned even if the seal-ci
    # pull is what failed. Idempotent + self-heals a job killed before
    # its own cleanup. SEA-830.
    hooks.pre-checkout = ''
      if [ -n "''${BUILDKITE_BUILD_CHECKOUT_PATH:-}" ] && [ -d "''${BUILDKITE_BUILD_CHECKOUT_PATH}" ]; then
        docker run --rm \
          -v "''${BUILDKITE_BUILD_CHECKOUT_PATH}:/reown" \
          --entrypoint chown \
          busybox:latest \
          -R "$(id -u):$(id -g)" /reown || true
      fi
    '';

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

    # The module's generated buildkite-agent.cfg sets build-path +
    # hooks-path but NOT plugins-path, so any step using a plugin
    # (every PR pipeline uses secret-env + docker) fails at checkout
    # with "Can't checkout plugin without a `plugins-path`". Point it
    # at a per-instance dir under the agent's dataDir
    # (/var/lib/buildkite-agent-<name>, createHome'd + owned by the
    # agent user) so the two instances don't race a shared plugin
    # checkout. The agent mkdir's it on first use.
    extraConfig = ''plugins-path="/var/lib/buildkite-agent-${name}/plugins"'';

    # Join the shared token-read group (so every agent can read the one
    # decrypted agent-token), the cache group (so both agents can write
    # the shared /cache/bkcache dir under propagate-uid-gid), and the
    # docker group (so the docker plugin can reach /run/docker.sock).
    extraGroups = [
      agentTokenGroup
      agentCacheGroup
      "docker"
    ];
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
    sealed = mkAgent "sealed";
    sealed-2 = mkAgent "sealed-2";
  };

  # FHS-path shebang support for CI job scripts. Buildkite plugin hooks
  # (secret-env, docker, …) and upstream tool scripts ship `#!/bin/bash`
  # / `#!/usr/bin/python3` shebangs that bare NixOS can't exec — it
  # provides only /bin/sh + /usr/bin/env, so a hook failed with
  # `perhaps the script interpreter "/bin/bash" is missing`. envfs (the
  # nixpkgs built-in module) is a FUSE filesystem that makes /bin/* and
  # /usr/bin/* resolve any binary on PATH, so those shebangs work
  # without patching every plugin. The idiomatic NixOS fix for
  # "third-party scripts assume FHS", and a self-hosted CI agent runs a
  # lot of such scripts. SEA-830.
  services.envfs.enable = true;

  # Real dockerd as the CI container engine (see agentRuntimePackages
  # comment for the why: bwrap-in-container parity with hosted Buildkite
  # + future GCP VMs, which both run docker). The shared nixos/common.nix
  # enables podman with dockerCompat + dockerSocket (it symlinks
  # /run/docker.sock at podman for the rpis' oci-containers). On this
  # host dockerd must own /run/docker.sock instead, so force podman's
  # docker-compat socket off here. Podman stays installed (the shared
  # config still enables the engine) but no longer claims the socket.
  virtualisation.docker.enable = true;
  virtualisation.podman.dockerCompat = lib.mkForce false;
  virtualisation.podman.dockerSocket.enable = lib.mkForce false;

  # Shared token-read group every agent joins. The buildkite-agents
  # module creates per-agent users (buildkite-agent-sealed, -sealed-2)
  # each with its own primary group; this shared group is what the one
  # decrypted agent-token file is chgrp'd to so every agent can read
  # it regardless of instance count.
  users.groups.${agentTokenGroup} = { };

  # Shared cache group + the setgid cache dir. Both agents are members
  # (extraGroups above); the dir is group-owned mode 2775 so either
  # agent can create entries and the setgid bit makes those entries
  # inherit the group (so the OTHER agent can then read/evict them on a
  # later job). Replaces the ad-hoc backup-owned /cache/bkcache that the
  # agent uid couldn't write under propagate-uid-gid. The `d` rule
  # adjusts the existing dir's owner+mode in place; stale backup-owned
  # contents underneath are cleared once by hand (they predate this).
  users.groups.${agentCacheGroup} = { };
  systemd.tmpfiles.rules = [
    "d /cache 0755 root root -"
    "d /cache/bkcache 2775 root ${agentCacheGroup} -"
  ];

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
      # decrypt against the host machine-ID key). One service stages
      # BOTH secrets — the agent token (needed at agent start) and the
      # sealedsecurity-ci App key (needed at checkout by the git
      # credential helper) — because they share /run/buildkite-agent and
      # a separate unit declaring the same RuntimeDirectory makes systemd
      # re-chown the whole tree to root:root on its activation, clobbering
      # the chgrp the other unit did (the regression that broke
      # agent-token reads). Both files AND the parent dir are group-owned
      # by the shared token group so every per-agent user (a member) can
      # traverse the 0750 dir and read the 0640 files. systemd creates
      # the RuntimeDirectory root:root by default, so the chgrp on the
      # dir is load-bearing, not just the files.
      ExecStart = [
        ''
          +${pkgs.writeShellScript "decrypt-agent-secrets" ''
            set -euo pipefail
            ${pkgs.systemd}/bin/systemd-creds decrypt \
              --name=buildkite-agent-token \
              /etc/buildkite-agent/agent-token.cred \
              /run/buildkite-agent/agent-token
            ${pkgs.systemd}/bin/systemd-creds decrypt \
              --name=buildkite-ci-app-key \
              /etc/buildkite-agent/ci-app-key.pem.cred \
              /run/buildkite-agent/ci-app-key.pem
            chgrp ${agentTokenGroup} \
              /run/buildkite-agent \
              /run/buildkite-agent/agent-token \
              /run/buildkite-agent/ci-app-key.pem
            chmod 0750 /run/buildkite-agent
            chmod 0640 /run/buildkite-agent/agent-token /run/buildkite-agent/ci-app-key.pem
          ''}
        ''
      ];
    };
  };

  # Order each agent unit behind the secrets decrypt, and point its git
  # at the agent-scoped gitconfig (GIT_CONFIG_GLOBAL — the agent users
  # have no ~/.gitconfig, so this layers the insteadOf rewrite +
  # credential helper onto just the agent processes, leaving
  # /etc/gitconfig and mattw's own git untouched). The module names
  # units `buildkite-agent-<name>`; keep this list in sync with
  # services.buildkite-agents above. Declared by single-key path so the
  # module system MERGES with the module-generated units rather than
  # replacing them.
  systemd.services.buildkite-agent-sealed = {
    after = [ "decrypt-agent-token.service" ];
    requires = [ "decrypt-agent-token.service" ];
    environment.GIT_CONFIG_GLOBAL = "${agentGitConfig}";
    # Pin the docker CLI at the real dockerd socket. shared/linux.nix's
    # shell init exports DOCKER_HOST at the rootless podman socket for
    # interactive user tooling, and that value leaks into the agent's
    # service environment — so the docker plugin's `docker` invocations
    # were silently hitting podman (5.7.0) instead of dockerd, which
    # reintroduced podman's netavark networking (a dead 169.254.1.1
    # first nameserver -> ~5s DNS timeout per daemon lookup -> 60-70x
    # slower e2e tests). An explicit service-env DOCKER_HOST overrides
    # the leak unconditionally. /run/docker.sock is dockerd's (podman's
    # docker-compat socket is forced off on this host).
    environment.DOCKER_HOST = "unix:///run/docker.sock";
  };
  systemd.services.buildkite-agent-sealed-2 = {
    after = [ "decrypt-agent-token.service" ];
    requires = [ "decrypt-agent-token.service" ];
    environment.GIT_CONFIG_GLOBAL = "${agentGitConfig}";
    environment.DOCKER_HOST = "unix:///run/docker.sock";
  };

  # (git config for the agents lives in agentGitConfig, pointed at via
  # GIT_CONFIG_GLOBAL in each agent unit above — NOT system-wide, so it
  # doesn't rewrite mattw's own git. See the agentGitConfig comment.)

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

{
  pkgs,
  lib,
  ...
}:

# mattserver — Old gaming PC (AMD Ryzen 3600 + RX 5700 XT, 64 GB DDR4).
# Roles:
#   1. ZFS backup receive target (btrfs root on 1TB NVMe; ZFS pool on 2TB SATA SSHD)
#   2. Self-hosted GitHub Actions runners (sealedsecurity org)
#   3. KDE Plasma gaming station (boots to SDDM by default; flip
#      `bootToDesktop` in desktop.nix to go headless)

let
  # Flip to true once GitHub PAT is written to /etc/github-runner/sealed-token
  # (see INSTALL.md "GitHub runner token"). Disabled during initial install
  # so nixos-rebuild doesn't fail on a missing token file.
  enableRunners = true;

  # Static service user for all four runner instances. The systemd
  # github-runner module defaults to DynamicUser=true which gives each
  # service start a fresh ephemeral UID — that's fine for isolation but
  # makes shared state on a shared filesystem location awkward (no
  # stable owner for the shared dir). Pinning all four instances to one
  # static user trades dynamic-UID isolation (we don't need it on a CI-
  # only box) for the ability to declaratively own /var/lib/github-runners/
  # and its subdirs. SEA-640.
  sealedRunnerUser = "sealed-runner";

  # Four sealed runner instances. Lints + tests run concurrently on one
  # push; the extra slots absorb additional pushes arriving before the
  # first finishes. Each gets 4 CARGO_BUILD_JOBS → 4 × 4 = 16 parallel
  # rustc clients. That oversubscribes the Ryzen 3600's 12 logical
  # cores by ~33%, which is fine in practice because rustc is link-
  # and IO-bound enough that strict 1:1 mapping leaves cores idle.
  #
  # Pre-SEA-672 was 4 instances; dropped to 3 to ease memory pressure
  # on the 32 GB config (4 × 4 × ~2.5 GB peak rustc = ~40 GB on a 32
  # GB box → sccache OOM kills). 64 GB upgrade restores the 4th slot
  # with comfortable headroom (see the per-runner MemoryMax math
  # below).
  #
  # State dirs are per-instance under /var/lib/github-runners/<name>/ so
  # cargo/rustup don't race across instances. The runner services run as
  # the shared `sealed-runner` user (see above) so they all own + can read
  # /var/lib/github-runners/shared/ where SCCACHE_DIR lives (each
  # runner's CARGO_HOME is per-instance — see SEA-672 write-up below).
  #
  # CI workflows target the canonical pool label `seal-linux-x64`. The
  # `mattserver` label is kept for explicit pinning when needed
  # (`runs-on: [seal-linux-x64, mattserver]`).
  mkSealedRunner = name: {
    enable = enableRunners;
    url = "https://github.com/sealedsecurity";
    tokenFile = "/etc/github-runner/sealed-token";

    # Pin to a static user so the shared dirs in
    # /var/lib/github-runners/shared/ have a stable owner. The
    # github-runner module defaults to DynamicUser=true which would
    # rotate the UID on every service start, breaking the shared
    # SCCACHE_DIR layout (the per-instance CARGO_HOME would also
    # break, but for the same reason).
    user = sealedRunnerUser;
    group = sealedRunnerUser;

    # Re-register on every service start. Without this, changing any
    # runner registration option (workDir, extraLabels, name) leaves
    # the old registration on GitHub and the configure script exits
    # 1 with "A runner exists with the same name" — the service then
    # sits in failed state and never picks up jobs. With `replace =
    # true` the configure step passes `--replace` to `config.sh`
    # which atomically replaces the previous registration. SEA-640.
    replace = true;

    # Move the runner's working directory off the systemd default
    # `RuntimeDirectory` (a tmpfs under /run) onto the NVMe-backed
    # state dir. Rust builds easily run a multi-GB target/ which
    # blows the tmpfs cap (typically 50% of RAM = ~32 GB shared
    # across all 4 instances). The state dir lives on btrfs root
    # with TB of headroom. SEA-640.
    workDir = "/var/lib/github-runners/${name}/work";

    # Custom labels for workflow targeting. Auto-labels (self-hosted, Linux,
    # X64) are added by the runner agent; these are the seal-specific ones.
    #
    # `seal-linux-x64` is the canonical pool label — seal workflows target
    #   it via `runs-on: seal-linux-x64`. Any runner registered with this
    #   label is eligible. A future second Linux x64 self-hosted box gets
    #   the same label and the two boxes load-balance via GitHub's scheduler.
    #
    # `mattserver` is a host-specific override for pinning a job to this
    #   specific box. Rare in practice; useful for debugging a runner-side
    #   issue without disabling the host.
    extraLabels = [
      "seal-linux-x64"
      "mattserver"
    ];
    extraPackages = with pkgs; [
      # Rust toolchain — CI installs the pinned nightly via `rustup show
      # active-toolchain` which reads rust-toolchain.toml. We pre-install
      # rustup so the CI step doesn't have to curl-install it.
      rustup

      # System build deps matching seal repo CI (replaces the apt install
      # steps — those don't work on NixOS). All jobs need these.
      protobuf # protobuf-compiler
      dbus # libdbus runtime
      dbus.dev # libdbus-1-dev: headers + pkg-config file
      pkg-config # required for dbus + openssl crates to resolve at build
      mold # fast linker (seal's .cargo/config.toml sets -fuse-ld=mold)
      clang # C compiler — linker driver for mold + bindgen
      gcc # fallback C linker; also needed by some proc-macro crates

      # Test runner — pre-installed so taiki-e/install-action detects it
      # in PATH and skips the GitHub Releases download step.
      cargo-nextest

      # Compiler cache — wired via RUSTC_WRAPPER below. Shared across all
      # four instances; sccache handles concurrent access via flock.
      sccache

      # Other sealed repo deps
      bun
      awscli2

      # oven-sh/setup-bun@v2 shells out to `unzip` to extract the
      # downloaded archive even when `bun` is already installed (it
      # always runs the download path on self-hosted runners that aren't
      # the GitHub-managed images). Without this the bun-setup step
      # fails with "Unable to locate executable file: unzip" on Even
      # Terminal + Deploy Docs Site jobs. SEA-640.
      unzip

      # Community GitHub Actions assume a standard *nix userland on
      # PATH. nix-darwin / NixOS's github-runner module curates the
      # runner PATH down to bash + coreutils + git + gnutar + gzip +
      # extraPackages, which excludes the rest of GNU coreutils. The
      # following additions cover the tools we've observed actions
      # reach for. SEA-640.
      curl # taiki-e/install-action: downloads binaries
      gawk # tecolicom/actions-use-homebrew-tools: awk pipeline
      gnused # taiki-e/install-action: sed -E for input parsing
      gnugrep # actions in general: grep -E
      findutils # find + xargs
      glibc.bin # ldd — taiki-e/install-action probes for musl/gnu

      # General CI utilities
      jq
      gnumake

      # actionlint — workflow YAML linter. The seal repo's `Lints` job
      # has a curl-install step gated on `runner.environment !=
      # 'self-hosted'` (it writes to /usr/local/bin which doesn't
      # exist on NixOS), so the binary needs to be pre-installed via
      # nixpkgs here. Version tracks nixpkgs.
      actionlint

      # bubblewrap — seal's sandbox spawn path is
      # `Command::new("bwrap")`. Without this on the runner's curated
      # PATH every test that exercises the sandbox path (rpc_lifecycle,
      # cancel, dev_server_ports, ...) fails with `failed to run
      # command: No such file or directory (os error 2)` because
      # `execvp("bwrap", ...)` returns ENOENT. The system-wide
      # /run/current-system/sw/bin/bwrap is on the login user's PATH
      # but the github-runner systemd unit builds its own PATH from
      # `extraPackages` only. SEA-640.
      bubblewrap

      python3
    ];
    extraEnvironment = {
      # Per-instance cargo home. Pre-SEA-672 this pointed at a
      # /shared subdir to keep one warm registry across all
      # runners — but concurrent cargo invocations on different
      # runners race during crate-source extraction: cargo creates
      # `registry/src/<crate>-<ver>/` before unpacking, sccache
      # tries to hash the source files mid-unpack, and gets ENOENT
      # on files that haven't been extracted yet. Symptoms range
      # from `sccache: failed to open file for hashing`, to rustc
      # ICEs (sccache falls back to direct rustc on partial
      # sources), to `could not parse/generate dep info`. The
      # index-lockfile that cargo uses serializes registry *index*
      # updates, NOT source unpacks — so a shared registry is only
      # safe when at most one cargo runs at a time, which doesn't
      # hold on a 4-runner box. Per-instance CARGO_HOME costs
      # ~1-2 GB extra per runner for the warm registry; trivial
      # given the TB-class NVMe.
      CARGO_HOME = "/var/lib/github-runners/${name}/.cargo";
      # Isolated per instance — rustup has no concurrent-write protection
      # and will corrupt toolchain state if two instances install
      # simultaneously.
      RUSTUP_HOME = "/var/lib/github-runners/${name}/.rustup";
      # 4 build jobs per instance × 4 instances = 16 parallel rustc
      # clients. Oversubscribes the Ryzen 3600's 12 logical cores by
      # ~33%, which is fine in practice because rustc's link + IO
      # latency leaves cores idle under strict 1:1 mapping.
      CARGO_BUILD_JOBS = "4";

      # pkg-config search path. On NixOS, `pkg-config` outside a Nix dev
      # shell can't auto-discover .pc files — there's no /usr/lib/pkgconfig
      # to fall back to. We point at dbus's `.pc` directory explicitly so
      # crates like `libdbus-sys` find `dbus-1.pc` at build time. If we
      # add another C-linked dep later (libssl, libudev, etc.), colon-
      # extend this with `${pkgs.<pkg>.dev}/lib/pkgconfig`. SEA-640.
      PKG_CONFIG_PATH = "${pkgs.dbus.dev}/lib/pkgconfig";

      # sccache wiring. Runner-level RUSTC_WRAPPER means cargo uses sccache
      # automatically, no per-workflow opt-in. The runners connect to a
      # *shared* sccache server running as its own systemd unit (see
      # `systemd.services.sccache-server` below). Pre-SEA-672 each runner
      # auto-spawned its own sccache server on first invocation; with N
      # concurrent runners that meant N daemons fighting for the same
      # SCCACHE_DIR via flock + N first-job races to bind the default
      # port 4226. Result: random `Connection reset by peer` (server
      # OOMed by sibling load) and `ENOENT on rustc` (server crashed
      # mid-spawn) failures during PR-time CI. A single supervised
      # server centralises cache state and survives sibling runner
      # restarts.
      RUSTC_WRAPPER = "sccache";
      SCCACHE_DIR = "/var/lib/github-runners/shared/.sccache";
      SCCACHE_CACHE_SIZE = "500G";
      # Point the sccache client at the dedicated server unit. Default
      # is 4226 anyway, but pin it explicitly so a future port change
      # only needs one edit.
      SCCACHE_SERVER_PORT = "4226";

      # Per-instance bun + uv install caches. Bun is used by docs/site
      # + linear-auto-done bundles; uv isn't called by today's seal CI
      # but is staged so a future Python tool gets a warm cache for
      # free. Pre-warm cost is one bun-install per instance (~3-5s on
      # cold cache, then the runner-local path is reused across that
      # instance's subsequent jobs). Per-instance (not /shared) for
      # the same reason CARGO_HOME is per-instance — concurrent writes
      # to a pool-wide install dir were the SEA-672 failure shape, and
      # the cross-instance warm-up payoff is small enough that the
      # extra GB of disk per runner is the right tradeoff.
      BUN_INSTALL_CACHE_DIR = "/var/lib/github-runners/${name}/.bun-install";
      UV_CACHE_DIR = "/var/lib/github-runners/${name}/.uv-cache";
    };

    # The systemd github-runner module's StateDirectory is `github-runner/<name>`
    # (singular — see nixpkgs nixos/modules/services/continuous-integration/
    # github-runner/service.nix). That gives the unit write access to
    # /var/lib/github-runner/<name>/ only; everything else under /var/lib
    # is read-only because of `ProtectSystem = "strict"`. Our env vars
    # above point at /var/lib/github-runners/... (plural) so we have to
    # whitelist it explicitly. The path is created with the right
    # ownership by systemd.tmpfiles.rules below. SEA-640.
    serviceOverrides = {
      ReadWritePaths = [ "/var/lib/github-runners" ];

      # SEA-672: per-runner memory ceiling. With 4 sibling runners each
      # running 4 parallel rustc processes (~2-3 GB peak each on
      # monomorphization-heavy crates) plus a shared sccache server
      # (now 12 GB cap), the box's 64 GB ceiling has comfortable
      # headroom even under worst-case load.
      #
      # Box-wide math under load:
      #   4 runners × 12 GB MemoryMax = 48 GB
      #   sccache server MemoryMax    = 12 GB
      #   kernel + page cache headroom ≈  4 GB
      #   ────────────────────────────────────
      #   Total                       = 64 GB physical
      #
      # `MemoryHigh` is a soft cap — the kernel tries to keep the
      # unit under 10 GB by throttling allocations and aggressively
      # reclaiming page cache. `MemoryMax` is the hard ceiling —
      # exceeding it gets the unit's processes OOM-killed instead
      # of dragging the whole host into swap thrash (and taking
      # the sccache server down with it, which is the specific
      # failure mode we're closing here).
      MemoryHigh = "10G";
      MemoryMax = "12G";

      # Sandbox unwrap: seal's daemon spawns its own bwrap-based
      # sandbox for every `command_run`. The github-runner module's
      # default systemd hardening was written for self-contained
      # build-tool runners; it's hostile to bwrap-as-payload:
      #
      #   * `SystemCallFilter = ["~@mount" ...]` denies
      #     mount/umount2/pivot_root — bwrap calls all of these to
      #     build its target namespace.
      #   * `RestrictNamespaces = true` denies
      #     unshare(CLONE_NEW{USER,NS,PID,...}) — bwrap relies on
      #     user-namespace creation to drop privs.
      #   * `PrivateUsers = true` runs the unit in a user-namespace
      #     of its own; nested user-ns creation requires the kernel's
      #     unprivileged-userns path, which the outer ns interferes
      #     with on some kernels.
      #   * `ProtectKernelTunables = true` makes /proc/sys/... read-
      #     only; bwrap's `--unshare-user-try-fallback` writes to
      #     /proc/self/setgroups + /proc/self/uid_map + .../gid_map,
      #     which (inside the new ns) sometimes triggers a write to
      #     /proc/sys/kernel/* during fallback.
      #
      # We're recursing one sandbox inside another. The runner
      # machine is dedicated to this org's CI — not a multi-tenant
      # build farm — so the outer isolation we're loosening is
      # already at "trust everything the workflow ran" level. The
      # inner bwrap is the actual security boundary the daemon
      # exercises in tests. SEA-640.
      SystemCallFilter = lib.mkForce [ ];
      RestrictNamespaces = false;
      PrivateUsers = false;
      ProtectKernelTunables = false;
      ProtectKernelModules = false;
      ProtectKernelLogs = false;
      ProtectControlGroups = false;
      ProtectClock = false;
      ProtectHome = false;
      ProtectHostname = false;
      ProtectSystem = false;
      ProtectProc = "default";
      ProcSubset = "all";
      PrivateDevices = false;
      PrivateMounts = false;
      PrivateTmp = false;
      NoNewPrivileges = false;
    };
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
  users.users.backup = {
    isNormalUser = true;
    openssh.authorizedKeys.keys = [
      # Inter-server keypair from common.nix — same key that mattw uses for
      # host-to-host automation. Kept in sync so rpi5/rpi4 can push backups
      # without a separate key.
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAJ6mvnQYv4B9V7KWo4xt6k5+2fx0Z1e9J384hSkg9Ec inter-server"
    ];
  };

  # ============================================================
  # GitHub Actions runners
  # ============================================================

  # Static service user for all four runner instances. The github-runner
  # module would otherwise allocate a fresh DynamicUser per service start;
  # we pin to a static UID/GID so /var/lib/github-runners/shared/ has a
  # stable owner across activations. SEA-640.
  users.users.${sealedRunnerUser} = {
    isSystemUser = true;
    group = sealedRunnerUser;
    description = "GitHub Actions runner service user (shared across sealed-*)";
    # /var/lib/github-runners is the runners' shared state root. Setting
    # home here is mostly documentation — each runner unit gets HOME
    # overridden to its workDir (a tmpfiles-managed runtime dir) by the
    # github-runner module.
    home = "/var/lib/github-runners";
  };
  users.groups.${sealedRunnerUser} = { };

  # Pre-create the runner state dirs with the right ownership. systemd
  # only auto-creates the per-instance StateDirectory at
  # /var/lib/github-runner/<name>/ (singular), which is not where our
  # extraEnvironment paths live. tmpfiles handles the plural-form layout.
  systemd.tmpfiles.rules = [
    "d /var/lib/github-runners                 0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/shared          0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/shared/.sccache 0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed          0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed/.cargo   0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed/work     0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-2        0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-2/.cargo 0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-2/work   0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-3        0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-3/.cargo 0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    "d /var/lib/github-runners/sealed-3/work   0755 ${sealedRunnerUser} ${sealedRunnerUser} -"
    # BUN_INSTALL_CACHE_DIR + UV_CACHE_DIR are intentionally NOT listed
    # here — both `bun install` and `uv` create their cache dirs with
    # `mkdir -p` on first use, and the per-instance parent
    # /var/lib/github-runners/<name>/ already has the right ownership
    # from the entries above. SEA-640.
  ];

  # Token file format: GitHub fine-grained PAT (or classic PAT) on one line.
  # Required scope: `manage_runners:org` (fine-grained) or `admin:org`
  # (classic). The org-level URL below means runners pick up jobs from any
  # sealedsecurity repo (seal, sealed, etc.) without per-repo registration.
  #
  # Write token file after first boot (see INSTALL.md):
  #   sudo install -m 600 -o root -g root \
  #     /dev/stdin /etc/github-runner/sealed-token <<< 'ghp_...'
  services.github-runners = {
    sealed = mkSealedRunner "sealed";
    sealed-2 = mkSealedRunner "sealed-2";
    sealed-3 = mkSealedRunner "sealed-3";
    sealed-4 = mkSealedRunner "sealed-4";
  };

  # ============================================================
  # Shared sccache server
  # ============================================================

  # SEA-672: one supervised sccache server shared across all runner
  # instances, replacing the pre-SEA-672 layout where each runner's
  # `RUSTC_WRAPPER=sccache` invocation auto-spawned its own server
  # on first use. Why the consolidation:
  #
  # 1. Concurrent first-jobs raced to bind the default port 4226.
  #    The loser printed `Server startup failed: Address in use`
  #    (the SEA-501 supervisor tests' exact symptom on CI).
  # 2. Each per-runner server held an open file lock on
  #    SCCACHE_DIR's lockfile; under heavy parallel rustc load
  #    one server would OOM and another would inherit a half-
  #    written cache slot, surfacing as `Connection reset by
  #    peer (os error 104)` mid-compile.
  # 3. Restarting any runner unit (e.g. nixos-rebuild switch)
  #    silently killed its local sccache server too, blowing
  #    the in-memory compile-result cache for that runner.
  #
  # The dedicated server is independent of any runner unit's
  # lifecycle. Restart-on-OOM via systemd's `Restart=on-failure`
  # makes the worst case "one minute of cache misses" instead of
  # "rest of the CI run fails".
  #
  # Memory accounting: the server's hot working set scales with
  # SCCACHE_CACHE_SIZE (500 GiB on-disk, ~1-2 GB resident for the
  # LRU index at that scale) plus inflight compile bytes. 12 GiB
  # hard cap keeps a runaway from squeezing the runner pools while
  # leaving the server room to handle 4 × 4 = 16 concurrent rustc
  # clients post-64 GB upgrade.
  systemd.services.sccache-server = {
    description = "Shared sccache server for self-hosted GitHub runners";
    wantedBy = [ "multi-user.target" ];
    after = [ "network.target" ];
    environment = {
      SCCACHE_DIR = "/var/lib/github-runners/shared/.sccache";
      SCCACHE_CACHE_SIZE = "500G";
      SCCACHE_SERVER_PORT = "4226";
      # Server-side log level. `warn` keeps the journal quiet on
      # the happy path; failures still surface.
      SCCACHE_LOG = "warn";
      # Foreground mode — keep the process attached so systemd
      # supervises it directly. Without this `sccache --start-server`
      # double-forks and the unit immediately marks itself
      # `Type=simple, finished`.
      SCCACHE_START_SERVER = "1";
      SCCACHE_NO_DAEMON = "1";
      # SEA-680: disable the default 10-minute idle timeout. With
      # the timeout active sccache exits CLEANLY (status=0) after
      # 600s without a client connect, and `Restart=on-failure`
      # below does NOT restart on a clean exit. The first lull in
      # PR-time CI traffic killed the server; the next wave of
      # builds saw `Connection reset by peer (os error 104)` /
      # `Failed to read response header` on every cc/rustc
      # invocation because nothing was listening on 4226 anymore.
      # `0` means "never idle-exit", which is what we want for a
      # supervised always-on server. SEA-680 surfaced the dead-
      # server cascade across all four loadtest PRs.
      SCCACHE_IDLE_TIMEOUT = "0";
    };
    serviceConfig = {
      Type = "simple";
      ExecStart = "${pkgs.sccache}/bin/sccache";
      User = sealedRunnerUser;
      Group = sealedRunnerUser;
      Restart = "always";
      RestartSec = "2s";
      # Per-unit memory accounting. Bumped from 4G → 8G after
      # SEA-672 round-3 surfaced sccache-server OOM kills under
      # 3-runner concurrent load (`sccache: warning: The server
      # looks like it shut down unexpectedly... Connection reset by
      # peer (os error 104)`). Each in-flight compile holds ~10-50 MB
      # of staging state, and with 3 runners × 4 jobs each (12
      # parallel rustc clients) + the LRU index + cache lookup
      # state, the 4G ceiling was hitting MemoryMax mid-PR.
      #
      # 64 GB upgrade + restored 4th runner: bumped 8G → 12G to
      # cover 4 × 4 = 16 concurrent rustc clients. Box-wide math
      # in serviceOverrides comment above: 4×12G runners + 12G
      # sccache + ~4G kernel/page-cache ≈ 64 GB physical. A
      # genuinely-leaked compile result still can't squeeze the
      # runner pools because the hard ceiling kicks in at 12G.
      MemoryHigh = "10G";
      MemoryMax = "12G";
      # The server reads + writes SCCACHE_DIR (cache state),
      # the requester runners' workspaces (rustc emits compiled
      # artifacts directly to the client's output paths through
      # the server), AND /tmp (sccache stages every preprocess
      # + compile result in a /tmp/sccacheXXXX temp dir before
      # moving it into either the cache or the client's out dir).
      # Without the workspace allow, sccache fails with `Read-only
      # file system (os error 30)` when rustc tries to write a
      # temp file under the runner's target/ dir — the server
      # inherits ProtectSystem=strict and the workspace is outside
      # its writable set. Without the /tmp allow, sccache fails
      # with the same EROFS shape on its own staging dir creation
      # (SEA-672 round 3 incident: every compile job blew up at
      # `sccache: Failed to create temp dir at path /tmp/sccacheXXXX`
      # the moment the first rustc invocation hit the server).
      #
      # Allowing the whole /var/lib/github-runners tree (instead
      # of enumerating per-runner work dirs) keeps the override
      # robust to runner-count changes — same shape as the
      # runner units' own ReadWritePaths.
      ReadWritePaths = [
        "/var/lib/github-runners"
        "/tmp"
      ];
      # Standard hardening that doesn't conflict with sccache's
      # workload (no bwrap-like namespace acrobatics here, unlike
      # the runner units).
      ProtectSystem = "strict";
      ProtectHome = true;
      # PrivateTmp left off — sccache and rustc both stage temp
      # files under /tmp during compile. With PrivateTmp the
      # server's view of /tmp is independent of the host's, which
      # is invisible to itself but can confuse tooling that
      # passes /tmp paths back and forth via env or argv.
      NoNewPrivileges = true;
      RestrictNamespaces = true;
      RestrictSUIDSGID = true;
    };
  };

  # ============================================================
  # Runner cache cleanup
  # ============================================================

  # Weekly prune of Rust target/ directories not modified in 30+ days.
  # These are the primary disk hog — a single workspace target/ can reach
  # 10–20 GB for large Rust projects. .cargo/ registry and rustup toolchains
  # are intentionally excluded: they're shared across projects and are much
  # smaller relative to the benefit of keeping them warm.
  #
  # 30-day window (was 14) post-64 GB / 1 TB NVMe context: the box is a
  # CI host first, with most of the disk dedicated to runner state.
  # Keeping warm target/ dirs for 4+ weeks catches infrequent-push
  # branches (release prep, security backports, slow feature stacks)
  # that would otherwise cold-rebuild from scratch on every visit.
  #
  # noatime is set on the btrfs mount so -atime would never update;
  # -mtime (directory entry mtime) is reliable here because cargo updates
  # it whenever it writes a new artifact.
  systemd.services.runner-cache-cleanup = {
    description = "Prune stale Rust target/ directories from runner workspaces";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = pkgs.writeShellScript "runner-cache-cleanup" ''
        find /var/lib/github-runners -maxdepth 6 \
          -name "target" -type d \
          -not -path "*/.cargo/*" \
          -mtime +30 \
          -print0 | xargs -0 -r rm -rf
      '';
    };
  };

  systemd.timers.runner-cache-cleanup = {
    description = "Weekly cleanup of stale runner Rust build artifacts";
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
  # for the runner-egress filter below — `networking.firewall` still
  # works on either backend; nftables gives us the user-matched
  # output chain we want for the lockdown.
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

  # Egress lockdown for the GitHub Actions runner pool.
  #
  # Threat model: a malicious workflow lands code-exec as
  # `sealed-runner`. Already mitigated upstream by external-PR
  # workflow approval + dep cooldowns + Tailscale ACLs gating
  # tailnet peer reachability. This rule is defense-in-depth:
  # cut the runner UID off from the LAN (router admin, NAS,
  # other workstations) and from CGNAT-routed paths that bypass
  # tailscaled.
  #
  # Why nftables (kernel, by UID) and not systemd's per-unit
  # `IPAddressDeny=` / `PrivateNetwork=`: the runner units
  # deliberately weaken systemd hardening
  # (`SystemCallFilter = lib.mkForce []`,
  # `RestrictNamespaces = false`, `PrivateUsers = false`)
  # because seal's daemon spawns bwrap sandboxes inside the
  # workflow — those primitives need to remain available. A
  # per-unit network filter would either need the same unwrap
  # (defeating the point) or break bwrap. Filtering by UID at
  # the kernel survives every fork inside the unit, the bwrap
  # unwrap, and any in-process privilege escalation short of
  # root.
  #
  # `meta skuid` is the source UID for outbound packets. We
  # match by username — nft resolves `"sealed-runner"` via
  # `getpwnam` at ruleset load, which is robust against the
  # auto-allocated system UID (the `users.users.sealed-runner`
  # block above doesn't pin a numeric uid).
  #
  # Allowed egress for the runner UID:
  #   - loopback (sccache server lives here)
  #   - DNS (53/udp+tcp, any destination — needed for
  #     github.com, crates.io, registries)
  #   - HTTP/HTTPS (80, 443) to public destinations (registries,
  #     GitHub API, action artifact endpoints)
  #   - git+ssh egress (22) for `cargo install --git` etc.
  #   - tailscale0 interface (tailnet ACLs govern peers)
  #
  # Dropped egress for the runner UID:
  #   - any TCP/UDP/ICMP to RFC1918 (10/8, 172.16/12, 192.168/16)
  #   - link-local (169.254/16)
  #   - CGNAT (100.64/10) on non-tailscale0 paths — tailscale0
  #     egress was already passed above; this catches anyone
  #     trying to route around tailscaled
  #   - IPv6 ULA + link-local
  #
  # The `log prefix "runner-egress-drop: "` action surfaces
  # blocked attempts in the journal so we can see what
  # legitimate egress (if any) got caught and tighten the
  # allowlist. After a week of clean logs, the prefix can be
  # demoted to bare `drop`.
  networking.nftables.tables.runner_egress = {
    family = "inet";
    content = ''
      chain output {
        type filter hook output priority filter; policy accept;

        # Skip the rule for any non-runner UID.
        meta skuid != "sealed-runner" accept

        # Allow loopback — sccache server, localhost-only services.
        oifname "lo" accept

        # Allow tailscale0 egress — tailnet ACLs are the policy layer
        # for peer reachability.
        oifname "tailscale0" accept

        # Allow DNS to any destination.
        udp dport 53 accept
        tcp dport 53 accept

        # Drop RFC1918 / link-local / CGNAT (non-tailscale0) before
        # the public-internet allow so the runner can't fall through
        # the 443-allow into a LAN host listening on 443.
        ip daddr { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16,
                   169.254.0.0/16, 100.64.0.0/10 } \
          log prefix "runner-egress-drop: " level warn drop
        ip6 daddr { fc00::/7, fe80::/10 } \
          log prefix "runner-egress-drop: " level warn drop

        # Allow public HTTP/HTTPS + git+ssh.
        tcp dport { 80, 443, 22 } accept

        # Default deny for anything else this UID tries.
        log prefix "runner-egress-drop: " level warn drop
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

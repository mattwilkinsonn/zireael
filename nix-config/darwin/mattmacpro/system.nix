{ pkgs, ... }:

# mattmacpro — Mac Pro 2013 trashcan (Intel Xeon E5 + 64 GB DDR3 ECC,
# FirePro D300/500/700 dual GPUs, ~512 GB Apple SSD).
#
# Roles:
#   1. Self-hosted GitHub Actions runners for the sealedsecurity org
#      (macOS x64 primary pool, label `seal-macos-x64`).
#
# OS strategy: macOS Sonoma 14.x via OpenCore Legacy Patcher.
# Mac Pro 2013 is natively stuck on Monterey (Apple dropped it from
# Ventura+); Monterey is already past security-update EOL and modern
# Homebrew formulae are starting to drop it. OCLP forward to Sonoma
# keeps the box inside a security-update window through late 2026.
# Re-run the patcher after every macOS point release (~quarterly) per
# darwin/mattmacpro/INSTALL.md. Box is headless so the FirePro GPU
# root-patch quality doesn't matter much; we only need the desktop
# to render at all for occasional VNC/screen sharing debug.
#
# Headless / always-on. Lives in a closet on wired ethernet; SSH
# (both Tailscale SSH and native sshd as a LAN fallback) is the only
# user-facing interface. pmset is locked into "never sleep, never
# hibernate, autorestart on power loss" — see the pmset activation
# block below.

let
  # Two runner instances. Xeon E5 in the Mac Pro 2013 is typically
  # 6 physical / 12 logical cores (E5-1650v2 / 2697v2 variants); 2
  # instances × 6 CARGO_BUILD_JOBS = 12 threads matches that.
  # Pre-SEA-672 this was 3 instances × 4 jobs; round-3 load test
  # showed 3 concurrent cold-cache builds saturated the box 1:1 on
  # cores and stretched per-compile times ~3×, blowing past a
  # 40min step cap. Dropping to 2 runners gives each ~6 cores avg
  # and cuts cold-cache build duration roughly in half. If your
  # specific SKU is a 4-core (E5-1620v2), drop further (1 runner
  # × 4 jobs) or hold at 2 instances × 4 jobs.
  #
  # `enable = true` unconditionally — the bootstrap script guarantees
  # /etc/github-runner/sealed-token exists before the first
  # darwin-rebuild runs (it prompts for the GitHub PAT and writes the
  # file as part of step 5 of mattmacpro-bootstrap.sh). If you ever
  # need to rebuild WITHOUT runners (debug, removing the box from the
  # pool), comment out the services.github-runners block at the
  # bottom of this file instead of toggling a flag here.
  #
  # State lives at /var/lib/github-runners/<name>/ — nix-darwin's
  # github-runner module on macOS uses the same layout as the NixOS
  # module, with launchd LaunchDaemons in place of systemd units.

  # Homebrew wrappers for the runner PATH. nix-darwin's github-runner
  # module curates the runner service's PATH to a fixed set (bash +
  # coreutils + git + gnutar + gzip + extraPackages). `/usr/local/bin`
  # (Intel Homebrew prefix on this box) is NOT on that PATH, so
  # workflow actions like `tecolicom/actions-use-homebrew-tools` that
  # shell out to `brew` fail with "command not found". Shim them into
  # the runner's PATH by writing thin wrappers as Nix-managed bin/
  # scripts. SEA-640.
  #
  # Wrapping (vs. directly prepending /usr/local/bin to the runner
  # PATH) keeps the shim list explicit: only `brew` and `brew`-
  # installed binaries we actually depend on get a wrapper. If a
  # workflow ever needs a different Homebrew-managed tool, add it to
  # this list rather than blanket-mounting /usr/local/bin.
  brewShims = pkgs.symlinkJoin {
    name = "brew-shims";
    paths = [
      (pkgs.writeShellScriptBin "brew" ''
        exec /usr/local/bin/brew "$@"
      '')
    ];
  };

  # Xcode Command Line Tools wrappers. cargo's build scripts shell
  # out to `cc` / `clang` / `ld` / `ar` for linking and library
  # discovery; on macOS the Xcode CLI ships them at /usr/bin/*. The
  # github-runner module's curated PATH excludes /usr/bin, so Rust
  # builds fail with `linker cc not found`. Same shim pattern as
  # brewShims above: explicit per-tool wrappers, no blanket
  # /usr/bin mount. SEA-640.
  #
  # Using the system tools (vs. nixpkgs LLVM) is deliberate — Rust
  # crates that build C deps on macOS expect Apple's clang for
  # framework/linker compatibility with the SDK. Pulling a Nix LLVM
  # here would cross-link two compiler toolchains and produce subtle
  # ABI breakage.
  xcodeShims = pkgs.symlinkJoin {
    name = "xcode-shims";
    paths =
      map
        (
          bin:
          pkgs.writeShellScriptBin bin ''
            exec /usr/bin/${bin} "$@"
          ''
        )
        [
          "cc"
          "c++"
          "clang"
          "clang++"
          "ld"
          "ar"
          "ranlib"
          "strip"
          "nm"
          "as"
          "lipo"
          "install_name_tool"
          "otool"
          "dsymutil"
          "xcrun"
          "codesign"
          "python3"
        ];
  };

  mkSealedRunner = name: {
    enable = true;
    url = "https://github.com/sealedsecurity";
    tokenFile = "/etc/github-runner/sealed-token";
    # Re-register on every service start. Without this, changing any
    # runner registration option (workDir, extraLabels, name) leaves
    # the old registration on GitHub and the configure script exits
    # with "A runner exists with the same name" — the launchd daemon
    # then sits in failed state and never picks up jobs. With
    # `replace = true` the configure step passes `--replace` to
    # `config.sh` which atomically replaces the previous
    # registration. SEA-640.
    replace = true;
    extraLabels = [
      # Canonical pool label — seal workflows target this via
      # `runs-on: seal-macos-x64`. Any macOS x64 runner registered
      # with this label is eligible; future second macOS host would
      # share the label and load-balance via GitHub's scheduler.
      "seal-macos-x64"
      # Host-specific override label for pinning: `runs-on:
      # [seal-macos-x64, mattmacpro]`. Rarely needed; useful for
      # debugging a runner-specific issue without disabling the box.
      "mattmacpro"
    ];
    extraPackages = with pkgs; [
      # Rust toolchain — CI installs the pinned nightly via `rustup
      # show active-toolchain`. Pre-install rustup so the CI step
      # doesn't have to curl-install it on every job.
      rustup

      # protobuf-compiler — equivalent of the Linux apt install step.
      protobuf

      # GNU coreutils — macOS ships BSD coreutils; some CI scripts
      # expect GNU `timeout`, `realpath`, etc. PATH-prefix-installed
      # under the runner service's environment so `coreutils-prefixed`
      # tools (gtimeout, grealpath) are also available.
      coreutils

      # pkg-config — required for openssl-sys and similar crates to
      # resolve at build.
      pkg-config

      # Test runner — pre-installed so taiki-e/install-action detects
      # it in PATH and skips the GitHub Releases download step.
      cargo-nextest

      # Compiler cache — wired via RUSTC_WRAPPER below.
      sccache

      # oven-sh/setup-bun@v2 shells out to `unzip` to extract the
      # downloaded archive even when `bun` is already installed. macOS
      # ships unzip in /usr/bin but the runner's curated PATH doesn't
      # include /usr/bin — so we add the Nix package explicitly. Mirror
      # of mattserver's fix. SEA-640.
      unzip

      # Community GitHub Actions assume a standard *nix userland on
      # PATH. nix-darwin's github-runner module curates the runner PATH
      # down to bash + coreutils + git + gnutar + gzip + extraPackages,
      # which excludes the rest of GNU coreutils + the macOS BSD utils
      # in /usr/bin. The following additions cover the tools we've
      # observed actions reach for. SEA-640.
      curl # taiki-e/install-action: downloads binaries
      gawk # tecolicom/actions-use-homebrew-tools: awk pipeline
      gnused # taiki-e/install-action: sed -E for input parsing
      gnugrep # actions in general: grep -E
      findutils # find + xargs

      # Brew wrapper — see top-of-file brewShims comment.
      brewShims

      # Xcode CLI wrappers (cc, clang, ld, ar, etc.) — see
      # top-of-file xcodeShims comment. Required for cargo's build
      # scripts and rustc's linker invocation.
      xcodeShims

      # Other sealed repo deps + general CI utilities
      bun
      awscli2
      jq
      gnumake
      # NOTE: deliberately no dbus / mold / clang / gcc here — macOS's
      # clang from the Xcode Command Line Tools is what cargo uses on
      # this platform; Linux-only deps would just be dead weight.
    ];
    extraEnvironment = {
      # Per-instance cargo home. Pre-SEA-672 this pointed at a
      # /shared subdir to keep one warm registry across all
      # runners — but concurrent cargo invocations on different
      # runners race during crate-source extraction: cargo creates
      # `registry/src/<crate>-<ver>/` before unpacking, sccache
      # tries to hash the source files mid-unpack, and gets ENOENT
      # on files that haven't been extracted yet. The
      # index-lockfile that cargo uses serializes registry *index*
      # updates, NOT source unpacks — so a shared registry is only
      # safe when at most one cargo runs at a time, which doesn't
      # hold on a 3-runner box. See mattserver/system.nix for the
      # full incident write-up. Per-instance CARGO_HOME costs
      # ~1-2 GB extra per runner for the warm registry.
      CARGO_HOME = "/var/lib/github-runners/${name}/.cargo";

      # Isolated per instance — rustup has no concurrent-write
      # protection and will corrupt toolchain state if two instances
      # install simultaneously.
      RUSTUP_HOME = "/var/lib/github-runners/${name}/.rustup";

      # 6 build jobs per instance × 2 instances = 12 threads = Xeon E5
      # logical core count. Bumped from 4 → 6 when sealed-macos-3
      # was retired (SEA-672 round 3 load-test fallout).
      CARGO_BUILD_JOBS = "6";

      # sccache wiring. Runner-level RUSTC_WRAPPER means cargo uses
      # sccache automatically without per-workflow opt-in. The runners
      # connect to a *shared* sccache server running as its own
      # launchd daemon (see `launchd.daemons.sccache-server` below).
      # Pre-SEA-672 each runner's first `RUSTC_WRAPPER=sccache` call
      # auto-spawned its own server, which on Linux turned into a
      # concurrent-bind race + per-server cache-state divergence. The
      # Mac side hasn't surfaced the failure yet (lower contention on
      # this 64 GB box) but the consolidation is the right hygiene
      # shape on both runner hosts. SEA-672.
      RUSTC_WRAPPER = "sccache";
      SCCACHE_DIR = "/var/lib/github-runners/shared/.sccache";
      SCCACHE_CACHE_SIZE = "30G";
      # Point the sccache client at the dedicated server unit.
      # Default is 4226; pin it explicitly so a future port change
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
      # extra GB of disk per runner is the right tradeoff. Both tools
      # `mkdir -p` their cache dir on first use, so no activation-
      # script entry needed.
      BUN_INSTALL_CACHE_DIR = "/var/lib/github-runners/${name}/.bun-install";
      UV_CACHE_DIR = "/var/lib/github-runners/${name}/.uv-cache";
    };
  };
in

{
  # ============================================================
  # Hostname + user
  # ============================================================

  networking.hostName = "mattmacpro";
  networking.computerName = "mattmacpro";
  networking.localHostName = "mattmacpro";

  # User registration. nix-darwin needs `users.users.<name>` declared
  # so home-manager.users.<name> resolves. The actual macOS account
  # ('mattw', UID >= 501) is created during the OS install per
  # INSTALL.md — this block just tells nix-darwin which user to apply
  # home-manager to.
  users.users.mattw = {
    name = "mattw";
    home = "/Users/mattw";
  };

  system.primaryUser = "mattw";

  # ============================================================
  # Nix
  # ============================================================

  # The upstream nixos.org installer drops org.nixos.nix-daemon as a
  # launchd daemon. Setting `nix.enable = true` lets nix-darwin
  # manage that same plist declaratively — it'll regenerate it on
  # every darwin-rebuild and the installer-provisioned version gets
  # replaced.
  #
  # This diverges from the MBP (`nix.enable = false`) for two
  # reasons: (1) the MBP runs Determinate Nix which has its own
  # daemon-management model where letting nix-darwin take over would
  # conflict, and (2) `services.github-runners` has an assertion
  # requiring `nix.enable = true` — without this the rebuild fails
  # before any runner LaunchDaemon gets laid down.
  nix.enable = true;

  # Trusted users — needed for the cachix substituters declared in
  # flake.nix's `nixConfig.extra-substituters` (cache.garnix.io,
  # nixos-raspberrypi.cachix.org) to be honored. The system root and
  # any admin-group member can also build privileged derivations
  # (e.g. nix-darwin activation). Without `mattw` in this list, the
  # nix-daemon ignores the flake's substituter declarations and
  # treats every `nix build` as if those caches don't exist,
  # forcing source builds.
  nix.settings.trusted-users = [
    "root"
    "@admin"
    "mattw"
  ];

  # Flake-level binary caches. Declared here in addition to
  # flake.nix's nixConfig so they survive being invoked outside a
  # flake context (e.g. `nix-build`, plain `nix-shell`). Matches the
  # contents of nixConfig.extra-substituters / extra-trusted-public-keys
  # so substituter trust is uniform across invocation paths.
  nix.settings.substituters = [
    "https://cache.nixos.org"
    "https://nixos-raspberrypi.cachix.org"
    "https://cache.garnix.io"
  ];
  nix.settings.trusted-public-keys = [
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
    "nixos-raspberrypi.cachix.org-1:4iMO9LXa8BqhU+Rpg6LQKiGa2lsNh/j2oiYLNOQ5sPI="
    "cache.garnix.io:CTFPyKSLcx5RMJKfLo5EEPUObbA78b0YQ2DTCJXqr9g="
  ];

  # Same direnv check-phase skip the MBP needs on aarch64-darwin's
  # 25.11. Carrying it forward to x86_64-darwin too because the fish
  # test runner SIGKILL inside the build sandbox isn't arch-specific.
  nixpkgs.overlays = [
    (_: prev: {
      direnv = prev.direnv.overrideAttrs (_: {
        doCheck = false;
      });
    })
  ];

  # ============================================================
  # System packages
  # ============================================================

  environment.systemPackages = with pkgs; [
    nixfmt-rfc-style
  ];

  # ============================================================
  # Homebrew (managed by nix-darwin)
  # ============================================================
  #
  # CI-focused subset only — this box is headless, no GUI casks
  # (with one exception: 1password-cli ships exclusively as a cask
  # now; the homebrew-core formula was retired). The cask is a tiny
  # zip with just the `op` binary, no .app bundle to worry about.

  homebrew = {
    enable = true;
    onActivation = {
      autoUpdate = true;
      upgrade = true;
      cleanup = "zap";
    };
    brews = [
      "coreutils"
      "swiftly"
      "xcodes"
      # zstd: same rationale as the MBP — podman links against
      # libzstd at runtime, so brew can't autoremove it. Declaring
      # it explicitly keeps the situation visible.
      "zstd"
      "podman"
      # Glances — TUI + web monitoring agent. Used in headless mode
      # exposed via Tailscale Serve (see services.glances below).
      "glances"
      # Tailscale CLI + daemon (formula, NOT the tailscale-app cask).
      # Cask history: the `tailscale-app` cask works fine on a
      # desktop Mac but is broken for headless CI in two ways:
      #   1. Its CLI shim is only created by the GUI's "Add to PATH"
      #      menu item — no headless install path.
      #   2. The cask's tailscaled runs as a per-USER LaunchAgent in
      #      the Aqua (GUI) launchd session domain, talking to its
      #      CLI via XPC mach services scoped to that session. SSH
      #      sessions live in a different launchd domain and can't
      #      see those services — `tailscale status` hangs forever
      #      from SSH. Same problem for any root LaunchDaemon trying
      #      to call `tailscale serve` (e.g. our Glances serve hook).
      # The formula sidesteps both:
      #   /usr/local/sbin/tailscaled — root-owned system daemon
      #   /usr/local/bin/tailscale   — CLI that talks to it via
      #                                /var/run/tailscaled.socket
      # The daemon runs as a launchd SYSTEM daemon via
      #   sudo brew services start tailscale
      # which writes /Library/LaunchDaemons/homebrew.mxcl.tailscale.plist.
      # System daemons are visible from every session domain (Aqua,
      # SSH, other LaunchDaemons), so the CLI works from anywhere.
      "tailscale"
    ];
    casks = [
      # 1Password CLI. Used to be a formula in homebrew-core; now
      # ships only as a cask. The cask is just the `op` binary —
      # no .app bundle, no GUI integration — so it's still fine
      # for a headless host.
      "1password-cli"
    ];
  };

  # ============================================================
  # Sleep + power management
  # ============================================================
  #
  # Strict always-on policy. Mac Pro 2013 is a headless CI host with
  # no other role — every sleep mode is disabled so SSH stays
  # reachable and CI jobs land instantly. Wake-on-magic-packet and
  # autorestart-on-power-loss as safety nets in case the box ever
  # does power-cycle (closet circuit breaker trip, etc.).
  #
  # pmset is per-power-source on laptops; the Mac Pro is AC-only so
  # `-a` (all sources) applies uniformly. Setting all of these
  # idempotently every activation matches how darwin/system.nix on
  # the MBP handles its displaysleep tuning.

  system.activationScripts.postActivation.text = ''
    # Sleep / power management — see system.nix top-of-file comment
    # for rationale. -a = all power sources.
    /usr/bin/pmset -a sleep 0          # system: never sleep
    /usr/bin/pmset -a disksleep 0      # disks: never spin down
    /usr/bin/pmset -a displaysleep 30  # display CAN sleep (harmless;
                                       # no monitor attached most of the time)
    /usr/bin/pmset -a powernap 0       # no trickle-wake; either fully on or off
    /usr/bin/pmset -a womp 1           # wake-on-magic-packet — recovery hatch
    /usr/bin/pmset -a autorestart 1    # auto-reboot after power loss
    /usr/bin/pmset -a tcpkeepalive 1   # keep TCP alive across low-power transitions
    /usr/bin/pmset -a standby 0        # disable deep-idle standby
    /usr/bin/pmset -a hibernatemode 0  # no hibernation

    # SSH (Remote Login). macOS ships sshd disabled by default. We
    # enable com.openssh.sshd via launchctl directly — using
    # `systemsetup -setremotelogin on` requires the calling process
    # to have Full Disk Access in TCC, which `darwin-rebuild` running
    # over SSH or from a non-Aqua shell doesn't have, so it silently
    # no-ops. launchctl + the system-wide plist path works in any
    # session.
    #
    # Tailscale SSH (enabled via `tailscale up --ssh` in the
    # bootstrap script) is the primary entry point. Native sshd is
    # the LAN fallback for cases where Tailscale is itself the
    # problem (auth issue, control-plane outage, etc.).
    if [ -f /System/Library/LaunchDaemons/ssh.plist ]; then
      /bin/launchctl enable system/com.openssh.sshd 2>/dev/null || true
      /bin/launchctl bootstrap system /System/Library/LaunchDaemons/ssh.plist 2>/dev/null || true
    fi
  '';

  # ============================================================
  # Tailscale — formula daemon (via brew services)
  # ============================================================
  #
  # tailscaled runs as a launchd SYSTEM daemon, started by
  # `sudo brew services start tailscale` in the bootstrap script.
  # brew services persists this via
  # /Library/LaunchDaemons/homebrew.mxcl.tailscale.plist, which
  # survives reboots without further intervention.
  #
  # nix-darwin's homebrew module doesn't model brew-services state
  # (only package presence) — so the "is the service started"
  # invariant is enforced one-shot by the bootstrap script, not
  # declaratively. If you ever need to restart the daemon:
  #   sudo brew services restart tailscale
  #
  # Auth happens once via `tailscale up --auth-key=... --ssh
  # --hostname=mattmacpro` (bootstrap step 4). After that the
  # daemon keeps state in /Library/Tailscale and stays joined
  # across reboots.

  # ============================================================
  # Glances — system metrics dashboard, exposed via Tailscale Serve
  # ============================================================
  #
  # Glances runs in web-server mode on localhost; Tailscale Serve
  # fronts it with TLS on the tailnet hostname. Reachable from any
  # tailnet device at:
  #     https://mattmacpro.tail08a5c5.ts.net:9443/
  #
  # No HTTP basic auth on Glances itself — the tailnet boundary
  # provides authentication (only tailnet members can resolve the
  # hostname / reach the IP). Tailscale ACLs already gate which
  # devices can talk to mattmacpro; we trust that as the auth layer.
  # If we ever want a second factor on the dashboard specifically,
  # Glances supports `--username/--password` for HTTP basic auth.
  #
  # Both launchd daemons run as root in the system launchd domain,
  # which means tailscale-serve-glances CAN reach the tailscaled
  # socket (formula tailscaled is also a system daemon — same
  # session domain). This is the reason we use the `tailscale`
  # formula on this host instead of the cask: the cask's tailscaled
  # is a per-USER LaunchAgent in the Aqua session, so root
  # LaunchDaemons can't talk to it.
  #
  # Disabled plugins: `docker` (no Docker daemon on this box; podman
  # via brew but we don't bother with the docker-compat probe) and
  # `smart` (smartctl needs sudo + the disk's S.M.A.R.T. data isn't
  # interesting for an Apple SSD).

  launchd.daemons.glances = {
    serviceConfig = {
      Label = "com.sealedsecurity.glances";
      ProgramArguments = [
        "/usr/local/bin/glances"
        "-w"
        "--bind"
        "127.0.0.1"
        "--port"
        "61208"
        "--disable-plugin"
        "docker,smart"
      ];
      RunAtLoad = true;
      KeepAlive = true;
      StandardOutPath = "/var/log/glances.log";
      StandardErrorPath = "/var/log/glances.log";
    };
  };

  # tailscale serve: persistent HTTPS reverse proxy on port 9443
  # backed by localhost:61208 (Glances). The serve config persists
  # in /var/lib/tailscale/serve.json once set, so this oneshot
  # re-runs idempotently on every boot — `tailscale serve --bg`
  # just confirms the existing mapping.
  #
  # `--bg` detaches so the launchd process exits 0 quickly (the
  # serve state lives in tailscaled, not this process). Without
  # --bg, the script would block forever waiting for serve to
  # "complete," and launchd's KeepAlive would respawn it in a loop.
  launchd.daemons.tailscale-serve-glances = {
    serviceConfig = {
      Label = "net.tailscale.serve.glances";
      ProgramArguments = [
        "/bin/sh"
        "-c"
        ''
          # Wait for tailscaled to be running + authed before we
          # try to configure serve. First boot post-install can
          # race: the formula's tailscaled launchd daemon and this
          # one are both RunAtLoad=true with no explicit ordering.
          for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
            if /usr/local/bin/tailscale status --json 2>/dev/null \
               | grep -q '"BackendState":"Running"'; then
              break
            fi
            sleep 5
          done

          # `tailscale serve` is persistent across reboots once set;
          # re-running it is idempotent. Output is captured to the
          # log file below for debugging.
          /usr/local/bin/tailscale serve --bg --https=9443 \
            http://127.0.0.1:61208 || true
        ''
      ];
      RunAtLoad = true;
      StandardOutPath = "/var/log/tailscale-serve-glances.log";
      StandardErrorPath = "/var/log/tailscale-serve-glances.log";
    };
  };

  # ============================================================
  # GitHub Actions runners
  # ============================================================
  #
  # Token file format: GitHub fine-grained PAT (or classic PAT) on
  # one line, mode 600 root:wheel. Required scope:
  #   `manage_runners:org` (fine-grained) or `admin:org` (classic).
  #
  # Provisioned by the bootstrap script BEFORE the first darwin-rebuild
  # — that's why the runner `enable = true` is unconditional. See
  # mattmacpro-bootstrap.sh step 5.

  # Pre-create the runner state dirs with the right ownership. The
  # darwin module's activation script creates per-instance dirs at
  # /var/lib/github-runners/<name>/ owned by the _github-runner user,
  # but the shared SCCACHE_DIR path it doesn't know about, and the
  # per-runner .cargo dirs need to exist with the right owner before
  # cargo's first registry write. Without this, cargo fails with
  # `Permission denied (os error 13)` on registry creation. SEA-640
  # + SEA-672 (per-runner .cargo replaces the old shared layout).
  #
  # Equivalent of mattserver's systemd.tmpfiles.rules — nix-darwin
  # doesn't have a tmpfiles abstraction, and custom-named activation
  # script slots are silently ignored (the activation runner only
  # invokes the slot names baked into modules/system/activation-
  # scripts.nix). Hook into the existing `extraActivation` slot —
  # one of the few slots nix-darwin explicitly leaves open for user
  # customisation.
  system.activationScripts.extraActivation.text = ''
    echo >&2 "setting up GitHub Actions runner dirs..."
    /bin/mkdir -p /var/lib/github-runners/shared/.sccache
    /bin/mkdir -p /var/lib/github-runners/sealed-macos/.cargo
    /bin/mkdir -p /var/lib/github-runners/sealed-macos-2/.cargo
    /usr/sbin/chown -R _github-runner:_github-runner /var/lib/github-runners
    /bin/chmod 0755 /var/lib/github-runners/shared
  '';

  services.github-runners = {
    sealed-macos = mkSealedRunner "sealed-macos";
    sealed-macos-2 = mkSealedRunner "sealed-macos-2";
    # SEA-672: dropped from 3 → 2 runners. The Mac Pro 2013 has 12
    # logical cores and 64 GB RAM; 3 concurrent cold-cache cargo
    # builds (each with `CARGO_BUILD_JOBS=4`) put 12 parallel rustc
    # processes on 12 cores 1:1, stretching individual compiles to
    # ~3× the solo-runner duration. Round-3 load test showed 3/4
    # concurrent macOS builds blew past a 40min `Build tests + bins`
    # cap mid-compile while a solo run on the same commit finished
    # in 17 min. The macOS path filter only fires when sandbox /
    # process-spawn / TLS code changes (cheaper-to-skip layer than
    # Linux), so dropping throughput to 2 here is fine — the
    # warm-cache steady state hits ~1m per macOS job anyway.
  };

  # ============================================================
  # Shared sccache server
  # ============================================================
  #
  # SEA-672: one supervised sccache server shared across all
  # runner instances, replacing the pre-SEA-672 layout where each
  # runner's `RUSTC_WRAPPER=sccache` invocation auto-spawned its
  # own server. Mirrors `systemd.services.sccache-server` on
  # mattserver — same shape, launchd-adapted. KeepAlive=true
  # makes launchd restart the server on crash (the launchd
  # equivalent of systemd's `Restart=on-failure`).
  #
  # The 64 GB on this box has more headroom than mattserver's
  # 32 GB, so we don't surface the OOM-the-sccache failure mode
  # here. The consolidation is still right on hygiene grounds:
  # one supervised server, one cache-state owner, restart-on-
  # crash by the OS-level supervisor.

  launchd.daemons.sccache-server = {
    serviceConfig = {
      Label = "com.sealedsecurity.sccache-server";
      ProgramArguments = [
        "${pkgs.sccache}/bin/sccache"
      ];
      EnvironmentVariables = {
        SCCACHE_DIR = "/var/lib/github-runners/shared/.sccache";
        SCCACHE_CACHE_SIZE = "30G";
        SCCACHE_SERVER_PORT = "4226";
        SCCACHE_LOG = "warn";
        # Foreground mode — keep the process attached so launchd
        # supervises it directly. Without this `sccache` double-
        # forks and launchd immediately marks the job exited.
        SCCACHE_START_SERVER = "1";
        SCCACHE_NO_DAEMON = "1";
        # SEA-680: disable the default 10-minute idle timeout.
        # launchd's `KeepAlive = true` re-spawns on clean exit
        # too (unlike systemd's `Restart=on-failure`) so the macOS
        # side doesn't see the dead-server cascade mattserver hit,
        # but letting the server idle-exit + re-spawn every 10
        # minutes still costs ~1-2s per cycle to re-read the LRU
        # index off disk. `0` keeps the server permanently up,
        # which is the right shape for a supervised always-on
        # cache server. SEA-680.
        SCCACHE_IDLE_TIMEOUT = "0";
      };
      RunAtLoad = true;
      KeepAlive = true;
      # Run as the github-runner service user so the cache dir
      # ownership matches what the runner-side reads + writes.
      # nix-darwin's `services.github-runners` defaults to the
      # `_github-runner` system user.
      UserName = "_github-runner";
      GroupName = "_github-runner";
      StandardOutPath = "/var/log/sccache-server.log";
      StandardErrorPath = "/var/log/sccache-server.log";
    };
  };

  # ============================================================
  # Runner cache cleanup — weekly launchd job
  # ============================================================
  #
  # mattserver has a systemd timer for this; nix-darwin uses launchd.
  # Same find-and-rm logic as nixos/mattserver/system.nix lines 321-342.

  launchd.daemons.runner-cache-cleanup = {
    serviceConfig = {
      Label = "com.sealedsecurity.runner-cache-cleanup";
      ProgramArguments = [
        "/bin/sh"
        "-c"
        ''
          find /var/lib/github-runners -maxdepth 6 \
            -name "target" -type d \
            -not -path "*/.cargo/*" \
            -mtime +14 \
            -print0 | xargs -0 -r rm -rf
        ''
      ];
      StartCalendarInterval = [
        {
          Weekday = 0; # Sunday
          Hour = 3;
          Minute = 0;
        }
      ];
      StandardOutPath = "/var/log/runner-cache-cleanup.log";
      StandardErrorPath = "/var/log/runner-cache-cleanup.log";
    };
  };

  # Used for backwards compat; don't change.
  system.stateVersion = 6;
}

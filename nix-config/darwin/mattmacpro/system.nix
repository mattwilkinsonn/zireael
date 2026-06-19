{ pkgs, lib, ... }:

# mattmacpro — Mac Pro 2013 trashcan (Intel Xeon E5 + 64 GB DDR3 ECC,
# FirePro D300/500/700 dual GPUs, ~512 GB Apple SSD).
#
# Roles:
#   1. Self-hosted Buildkite CI agents for the sealedsecurity org
#      (macOS x64 primary pool, queue `macos-x64-selfhosted`).
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
# Headless / always-on. Lives in a closet on wired ethernet;
# Tailscale SSH (LAN-side native sshd is intentionally unloaded)
# is the only user-facing interface. pmset is locked into "never
# sleep, never hibernate, autorestart on power loss" — see the
# pmset activation block below.

let
  # Service user for the Buildkite agents. nix-darwin's github-runner
  # module used to create `_github-runner` (uid/gid 533); we now hand-
  # roll the agent launchd daemons (nix-darwin has no buildkite-agents
  # module), so we declare the user ourselves. Renamed to drop the
  # github reference; uid/gid 533 is preserved so the existing
  # /var/lib state-dir ownership carries over without a chown sweep.
  agentUser = "_buildkite-agent";
  agentUid = 533;

  # Two agent instances. Xeon E5 in the Mac Pro 2013 is typically
  # 6 physical / 12 logical cores (E5-1650v2 / 2697v2 variants); 2
  # instances × 6 CARGO_BUILD_JOBS = 12 threads matches that.
  # Pre-SEA-672 this was 3 instances × 4 jobs; round-3 load test
  # showed 3 concurrent cold-cache builds saturated the box 1:1 on
  # cores and stretched per-compile times ~3×, blowing past a
  # 40min step cap. Dropping to 2 gives each ~6 cores avg and cuts
  # cold-cache build duration roughly in half.
  #
  # Unlike mattserver (Linux), macOS PR jobs run NATIVELY on the
  # agent — test-macos.yml has no docker plugin (`just
  # nextest-macos-pr` runs directly, using the macOS-built
  # sandbox-exec path). So the agent keeps the full toolchain
  # (rustup, sccache, the brew/Xcode shims) and the shared sccache
  # server, exactly as the old runner did. The only swap is the
  # service: a hand-rolled launchd buildkite-agent daemon in place
  # of the github-runner LaunchDaemon.
  #
  # State lives at /var/lib/buildkite-agents/<name>/ (kept as-is to
  # preserve the warm cargo/sccache caches across the cutover).

  # Homebrew wrappers for the agent PATH. The hand-rolled launchd
  # daemon (below) sets PATH explicitly from the package list; macOS
  # GUI/CI tools like `brew` live under /usr/local/bin which isn't on
  # that curated PATH, so we shim them in as Nix-managed bin scripts.
  # SEA-640 (carried forward from the github-runner setup).
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

  # Packages on the agent's PATH. The hand-rolled launchd daemon
  # builds PATH from this list via lib.makeBinPath. macOS jobs run
  # natively, so the full toolchain lives here (unlike mattserver,
  # where it's in the seal-ci container).
  agentPackages = with pkgs; [
    # The agent binary itself + base userland the agent + plugins +
    # hook scripts reach for outside any job sandbox.
    buildkite-agent
    bash
    # GNU coreutils — macOS ships BSD coreutils; some CI scripts expect
    # GNU `timeout`, `realpath`, etc. (also exposes the g-prefixed
    # gtimeout / grealpath).
    coreutils
    gnutar
    gzip
    git
    # Rust toolchain — CI installs the pinned nightly via `rustup
    # show active-toolchain`. Pre-install rustup so the CI step
    # doesn't have to curl-install it on every job.
    rustup

    # protobuf-compiler — equivalent of the Linux apt install step.
    protobuf

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

  # Per-instance environment (cargo/rustup/sccache/bun caches). The
  # launchd daemon merges this into EnvironmentVariables. Same shape
  # the github-runner module's `extraEnvironment` carried.
  mkAgentEnv = name: {
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
    CARGO_HOME = "/var/lib/buildkite-agents/${name}/.cargo";

    # Isolated per instance — rustup has no concurrent-write
    # protection and will corrupt toolchain state if two instances
    # install simultaneously.
    RUSTUP_HOME = "/var/lib/buildkite-agents/${name}/.rustup";

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
    SCCACHE_DIR = "/var/lib/buildkite-agents/shared/.sccache";
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
    BUN_INSTALL_CACHE_DIR = "/var/lib/buildkite-agents/${name}/.bun-install";
    UV_CACHE_DIR = "/var/lib/buildkite-agents/${name}/.uv-cache";
  };

  # Hand-rolled launchd daemon for one Buildkite agent instance.
  # nix-darwin has no services.buildkite-agents module, so we run
  # `buildkite-agent start` directly under launchd, configuring it via
  # BUILDKITE_AGENT_* env vars (the agent reads these in lieu of a cfg
  # file). KeepAlive restarts on crash; the agent's own graceful-stop
  # handles in-flight jobs on shutdown.
  #
  # The agent token is read at launch from the tmpfs path the
  # decrypt-agent-token daemon writes (see below) via
  # BUILDKITE_AGENT_TOKEN_PATH — the token never lands in the launchd
  # plist (which is world-readable in the Nix store).
  mkAgentDaemon = name: {
    serviceConfig = {
      Label = "com.sealedsecurity.buildkite-agent-${name}";
      ProgramArguments = [
        "${pkgs.buildkite-agent}/bin/buildkite-agent"
        "start"
      ];
      EnvironmentVariables = {
        PATH = "${lib.makeBinPath agentPackages}:/usr/bin:/bin:/usr/sbin:/sbin";
        HOME = "/var/lib/buildkite-agents/${name}";
        # Agent identity + routing. The token is supplied out-of-band
        # via the token file (next line) so it stays out of the plist.
        BUILDKITE_AGENT_NAME = "mattmacpro-${name}";
        BUILDKITE_AGENT_TAGS = "queue=macos-x64-selfhosted,os=macos,arch=x64,host=mattmacpro";
        BUILDKITE_AGENT_TOKEN_PATH = "/var/run/buildkite-agent/agent-token";
        BUILDKITE_BUILD_PATH = "/var/lib/buildkite-agents/${name}/builds";
        BUILDKITE_HOOKS_PATH = "${pkgs.buildkite-agent}/share/buildkite-agent/hooks";
        BUILDKITE_PLUGINS_PATH = "/var/lib/buildkite-agents/${name}/plugins";
      }
      // mkAgentEnv name;
      UserName = agentUser;
      GroupName = agentUser;
      RunAtLoad = true;
      KeepAlive = true;
      StandardOutPath = "/var/log/buildkite-agent-${name}.log";
      StandardErrorPath = "/var/log/buildkite-agent-${name}.log";
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

  # Buildkite agent service user. The github-runner module used to
  # create this (_github-runner, uid/gid 533); since we hand-roll the
  # launchd daemons we declare it ourselves. uid/gid 533 is preserved
  # so the /var/lib/buildkite-agents state tree (chowned in the
  # extraActivation block) keeps its ownership across the cutover.
  # nix-darwin requires `knownUsers`/`knownGroups` for any
  # non-system-managed account so it knows to create + later clean up
  # the dscl entries.
  users.users.${agentUser} = {
    uid = agentUid;
    gid = agentUid;
    description = "Buildkite agent service user";
    home = "/var/lib/buildkite-agents";
    shell = "/usr/bin/false";
  };
  users.groups.${agentUser} = {
    gid = agentUid;
    description = "Buildkite agent service group";
  };
  users.knownUsers = [ agentUser ];
  users.knownGroups = [ agentUser ];

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
  # This diverges from the MBP (`nix.enable = false`) because the MBP
  # runs Determinate Nix which has its own daemon-management model
  # where letting nix-darwin take over would conflict. mattmacpro uses
  # the standard nixos.org daemon, so nix-darwin manages the plist.
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

    # SSH (Remote Login). Tailscale SSH (enabled via `tailscale up
    # --ssh` in the bootstrap script + the `tailscale set --ssh=true`
    # reconciliation below) handles every interactive access path;
    # the LAN-side native sshd is unneeded attack surface for a
    # runner host that nobody should be SSHing to over the LAN.
    # Earlier configs enabled `com.openssh.sshd` here as a
    # Tailscale-fallback; that's been retired now that Tailscale has
    # been stable for months.
    #
    # `launchctl bootout` is the macOS equivalent of `systemctl
    # disable && systemctl stop`. `disable` flips the persistent
    # enabled bit; `bootout` unloads from the running launchd
    # session. Both 2>/dev/null so re-running a host that's already
    # in the desired state (or one that never had sshd loaded —
    # fresh macOS install) doesn't error.
    /bin/launchctl disable system/com.openssh.sshd 2>/dev/null || true
    /bin/launchctl bootout system/com.openssh.sshd 2>/dev/null || true

    # Tailscale SSH on this host MUST stay set, because the previous
    # block disabled native sshd — without `--ssh` tailscaled won't
    # intercept incoming SSH connections and the host becomes
    # unreachable.
    #
    # nix-darwin doesn't model brew-installed daemons' runtime state
    # (services.tailscale is NixOS-only), and the bootstrap script
    # only runs once. If anyone ever invokes `sudo tailscale up`
    # without `--ssh` (e.g. an unrelated config tweak, or
    # `--reset` for a hostname change) the flag silently drops.
    # Reconcile every activation with `tailscale set` — unlike
    # `tailscale up`, `set` updates one knob without touching any
    # other state, so re-running is idempotent. The `|| true`
    # covers the brief first-boot window where tailscaled isn't
    # yet up.
    /usr/local/bin/tailscale set --ssh=true 2>/dev/null || true
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
  # pf egress filter — UID-scoped lockdown for the agent pool
  # ============================================================
  #
  # Mirror of mattserver's nftables agent-egress rule, adapted for
  # macOS's pf. Same threat model: a malicious workflow lands code-exec
  # as the buildkite-agent user; we cut that UID off from LAN
  # reachability so it can't talk to the router admin UI, the NAS,
  # other workstations, etc.
  #
  # Why pf and not a per-launchd-daemon equivalent: launchd has no
  # PrivateNetwork/IPAddressDeny analog; the actual filter has to
  # live at the kernel netfilter layer. macOS's pf can match on
  # `user <name>` which getpwnam-resolves the username at ruleset
  # load time — robust against UID reshuffles by nix-darwin. (Both
  # agents share the one `_buildkite-agent` user here, so a single
  # `user` match covers the pool — unlike the Linux side where the
  # buildkite-agents module forces per-name users + a group match.)
  #
  # Why a full ruleset (not just an anchor): Apple's default
  # /etc/pf.conf only references com.apple/* anchors. To get OUR
  # rules to actually evaluate, we'd either have to edit /etc/pf.conf
  # (fragile — Apple owns that file) or load our own top-level
  # ruleset. We do the latter. Apple's default anchors are
  # pass-through'd at the top so anything that legitimately needs
  # them (Internet Sharing, AirDrop, etc. — none of which run on
  # this headless box) keeps working.
  #
  # Allowed egress for the agent UID:
  #   - loopback (sccache server lives on lo0)
  #   - DNS (53/udp+tcp) — github.com, crates.io, registries, buildkite.com
  #   - HTTP/HTTPS (80, 443) to public destinations
  #   - git+ssh (22) for `cargo install --git` etc.
  #   - utun* (Tailscale data plane) — tailnet ACLs gate peer access
  #
  # Dropped egress for the agent UID:
  #   - RFC1918 ranges (10/8, 172.16/12, 192.168/16)
  #   - link-local (169.254/16)
  #   - CGNAT (100.64/10) on non-utun interfaces — the utun pass
  #     above covers legitimate tailnet egress; this catches anyone
  #     trying to route around tailscaled
  #
  # The `log` action on the default-deny line writes to pflog0; tail
  # it with `sudo tcpdump -ni pflog0` to see unexpected blocks
  # during the first week of operation.

  environment.etc."pf.agent.conf".text = ''
    # Apple default anchors — pass-through so Internet Sharing /
    # AirDrop / Apple-internal pf use continues to work if anything
    # ever needs them on this box.
    scrub-anchor "com.apple/*"
    nat-anchor "com.apple/*"
    rdr-anchor "com.apple/*"
    anchor "com.apple/*"
    load anchor "com.apple" from "/etc/pf.anchors/com.apple"

    # ---- LAN destination tables ------------------------------------
    table <lan_ranges> const { \
      10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, \
      169.254.0.0/16, 100.64.0.0/10 \
    }

    # ---- Agent egress lockdown -------------------------------------
    # `quick` short-circuits — first match wins, no further
    # evaluation. Without `quick`, pf evaluates the whole ruleset
    # and the LAST matching rule wins (BSD pf semantics), which
    # would let the default-pass rule at the bottom override our
    # blocks.

    # Allow loopback unconditionally (sccache, localhost services).
    pass quick on lo0

    # Tailscale data-plane interface. Apple's tailscaled (formula
    # variant on this host) creates utun* interfaces; the exact
    # suffix is dynamic across reboots. `utun` matches any of them.
    pass out quick on utun proto { tcp udp icmp } user ${agentUser}

    # Drop LAN-range destinations for the agent UID. Listed BEFORE
    # the DNS + public-internet allows so the agent can't fall through
    # into a LAN host that happens to be listening on 53 or 443.
    block drop out log quick proto { tcp udp icmp } \
      from any to <lan_ranges> user ${agentUser}

    # Allow DNS only to non-LAN resolvers. Placed AFTER the LAN-drop
    # (and scoped `to !<lan_ranges>`) so a malicious job can't reach a
    # LAN-side resolver / service on port 53 — the `quick` modifier
    # would otherwise short-circuit before the LAN-drop is evaluated.
    pass out quick proto { tcp udp } from any to !<lan_ranges> port 53 \
      user ${agentUser}

    # Allow public HTTP/HTTPS + git+ssh.
    pass out quick proto tcp from any to any port { 80, 443, 22 } \
      user ${agentUser}

    # Default deny — agent UID can't reach anything else. Logged
    # to pflog0 for forensics.
    block drop out log quick from any to any user ${agentUser}

    # Everything else (mattw's shell, system daemons, anyone not
    # the agent UID) passes unaffected.
    pass out all
  '';

  # Loader daemon — runs at boot, calls `pfctl -ef` to enable pf
  # and load our ruleset. `-E` increments pf's enable refcount;
  # `-f` loads the ruleset (replacing whatever's active, which
  # post-bootstrap is either nothing or Apple's empty default).
  #
  # Type=oneshot semantics on launchd: `RunAtLoad=true, KeepAlive=false`
  # means launchd starts the process once at boot, waits for exit,
  # then leaves pf in the loaded-and-enabled state without keeping
  # the daemon process around. pf kernel state survives the daemon
  # exit — the rules stay loaded.
  #
  # Ordering: `${agentUser}` is created by the users.users block above
  # (applied during nix-darwin activation, before launchd boots our
  # daemons), so getpwnam("${agentUser}") succeeds at pfctl-load time.
  # Worst case if it races (it shouldn't): pfctl -f fails, the daemon
  # exits non-zero, the next reboot retries.
  launchd.daemons.pf-agent-egress = {
    serviceConfig = {
      Label = "com.sealedsecurity.pf-agent-egress";
      ProgramArguments = [
        "/sbin/pfctl"
        "-ef"
        "/etc/pf.agent.conf"
      ];
      RunAtLoad = true;
      KeepAlive = false;
      StandardOutPath = "/var/log/pf-agent-egress.log";
      StandardErrorPath = "/var/log/pf-agent-egress.log";
    };
  };

  # ============================================================
  # Buildkite agents (self-hosted PR-time CI)
  # ============================================================
  #
  # SEA-830: the box runs self-hosted Buildkite agents (was self-hosted
  # GitHub Actions runners pre-SEA-587 migration). test-macos.yml routes
  # to the `macos-x64-selfhosted` queue these agents register.
  #
  # nix-darwin has no services.buildkite-agents module, so the agent
  # launchd daemons are hand-rolled (mkAgentDaemon in the let block).
  # macOS PR jobs run NATIVELY on the agent (no docker plugin), so the
  # full toolchain + the shared sccache server stay, exactly as the old
  # runner had them.
  #
  # Token: the Buildkite *Agent* token (org Agents page), distinct from
  # the BUILDKITE_API_TOKEN the `bk` CLI uses. Provisioned by the
  # bootstrap script as a host-bound encrypted blob; decrypted into
  # tmpfs at boot by the decrypt-agent-token daemon below.

  # Pre-create the agent state dirs with the right ownership. nix-darwin
  # has no tmpfiles abstraction, and custom-named activation script
  # slots are silently ignored (the activation runner only invokes the
  # slot names baked into modules/system/activation-scripts.nix), so we
  # hook the existing `extraActivation` slot. The state tree is kept at
  # /var/lib/buildkite-agents (unchanged from the runner era) so the warm
  # cargo/sccache caches survive the cutover.
  system.activationScripts.extraActivation.text = ''
    echo >&2 "setting up Buildkite agent dirs..."
    /bin/mkdir -p /var/lib/buildkite-agents/shared/.sccache
    /bin/mkdir -p /var/lib/buildkite-agents/sealed-macos/.cargo
    /bin/mkdir -p /var/lib/buildkite-agents/sealed-macos-2/.cargo
    /usr/sbin/chown -R ${agentUser}:${agentUser} /var/lib/buildkite-agents
    /bin/chmod 0755 /var/lib/buildkite-agents/shared

    # Spotlight + Time Machine exclusions on the agent state tree.
    # Two hygiene concerns specific to macOS:
    #
    # 1. Spotlight indexing surfaces matched content via `mdfind` /
    #    Finder search / Quick Look. Build artifacts, intermediate
    #    object files, and (worse) any secret that ever lands in
    #    /var/lib/buildkite-agents/<name>/builds/ during a CI job would
    #    show up in those interfaces — for any user on the box, not
    #    just the agent user. Disabling Spotlight on the tree closes
    #    that exfil path.
    #
    # 2. Time Machine snapshots are content-addressed and persisted;
    #    a secret that briefly existed in a CI workspace can stick
    #    around in TM history for months. Excluding the agent state
    #    tree means TM never sees those bytes in the first place.
    #
    # Both commands fail silently when their dependencies aren't
    # available (mdutil bails if Spotlight isn't running; tmutil
    # bails if TM isn't configured) — fine for a headless box that
    # may not have either active. `|| true` suppresses the exit
    # status either way so a failure here doesn't fail activation.
    echo >&2 "applying agent-state Spotlight + Time Machine exclusions..."
    /usr/bin/mdutil -i off /var/lib/buildkite-agents 2>/dev/null || true
    if [ -d /var/lib/buildkite-agents ]; then
      /usr/bin/tmutil addexclusion /var/lib/buildkite-agents 2>/dev/null || true
    fi
  '';

  # Decrypt the host-bound encrypted agent token into tmpfs at boot.
  # macOS has no systemd-creds; we use the same OCLP-era pattern as the
  # other secrets on this box — an encrypted blob at
  # /etc/buildkite-agent/agent-token.age decrypted by a one-shot launchd
  # daemon. Here we keep it simple: the bootstrap script writes the
  # plaintext token to /etc/buildkite-agent/agent-token (mode 600
  # root:wheel) and this daemon copies it into a tmpfs-like /var/run
  # path readable by the agent group. (macOS /var/run is not a tmpfs but
  # is cleared on boot; the token is re-staged each boot from the
  # root-only source.) See the buildkite-agents handoff doc for the
  # provisioning step.
  launchd.daemons.decrypt-agent-token = {
    serviceConfig = {
      Label = "com.sealedsecurity.decrypt-agent-token";
      ProgramArguments = [
        "/bin/sh"
        "-c"
        ''
          set -e
          install -d -m 0750 -o root -g ${agentUser} /var/run/buildkite-agent
          install -m 0640 -o root -g ${agentUser} \
            /etc/buildkite-agent/agent-token \
            /var/run/buildkite-agent/agent-token
        ''
      ];
      RunAtLoad = true;
      KeepAlive = false;
      StandardOutPath = "/var/log/decrypt-agent-token.log";
      StandardErrorPath = "/var/log/decrypt-agent-token.log";
    };
  };

  # Two agent instances. SEA-672 dropped from 3 → 2: the Mac Pro 2013
  # has 12 logical cores and 64 GB RAM; 3 concurrent cold-cache cargo
  # builds put 12 parallel rustc on 12 cores 1:1, stretching individual
  # compiles ~3× and blowing past a 40min cap. The macOS path filter
  # only fires when sandbox / process-spawn / TLS code changes (a
  # cheaper-to-skip layer than Linux), so N=2 is plenty; warm-cache
  # steady state hits ~1m per macOS job.
  #
  # launchd has no dependency-ordering primitive like systemd's
  # After=/Requires=. The agent daemon retries the token-file read on
  # its own (KeepAlive restarts it until decrypt-agent-token has staged
  # the file), so a boot-time race just costs a restart or two.
  launchd.daemons.buildkite-agent-sealed-macos = mkAgentDaemon "sealed-macos";
  launchd.daemons.buildkite-agent-sealed-macos-2 = mkAgentDaemon "sealed-macos-2";

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
        SCCACHE_DIR = "/var/lib/buildkite-agents/shared/.sccache";
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
      # Run as the buildkite-agent service user so the cache dir
      # ownership matches what the agent-side reads + writes.
      UserName = agentUser;
      GroupName = agentUser;
      StandardOutPath = "/var/log/sccache-server.log";
      StandardErrorPath = "/var/log/sccache-server.log";
    };
  };

  # ============================================================
  # Agent cache cleanup — weekly launchd job
  # ============================================================
  #
  # mattserver has a systemd timer for this; nix-darwin uses launchd.
  # Same find-and-rm logic as nixos/mattserver/system.nix.

  launchd.daemons.agent-cache-cleanup = {
    serviceConfig = {
      Label = "com.sealedsecurity.agent-cache-cleanup";
      ProgramArguments = [
        "/bin/sh"
        "-c"
        ''
          find /var/lib/buildkite-agents -maxdepth 6 \
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
      StandardOutPath = "/var/log/agent-cache-cleanup.log";
      StandardErrorPath = "/var/log/agent-cache-cleanup.log";
    };
  };

  # Used for backwards compat; don't change.
  system.stateVersion = 6;
}

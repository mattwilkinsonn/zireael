{
  config,
  pkgs,
  lib,
  ...
}:

# Shared macOS Buildkite CI agent config for the sealedsecurity org's
# self-hosted Apple-Silicon runners (mattmini, dedicatedmacio-mini,
# awsmac). The three boxes are byte-identical except a handful of
# host knobs — hostname, P-core count, the macOS admin account — so the
# whole ~950-line agent surface (launchd daemon, sccache wiring, pf
# egress, pmset, Glances, secret staging, homebrew, cache cleanup) lives
# here once. A fix like the Determinate Nix switch or the BSD-xargs
# cleanup then lands in one place instead of three. SEA-840.
#
# Per-host knobs (options.sealed.macosBuildkiteAgent):
#   - hostName: drives networking.* + the agent --name + `host=` tag.
#   - cargoBuildJobs: P-core count (M2 Pro mattmini = 6; M4 / M2 EC2 = 4).
#   - adminUser: the macOS admin account home-manager applies to. The
#     owned/rental minis create `mattw`; the EC2 AMI ships `ec2-user`,
#     so awsmac points at that instead of creating an account.
#
# Nix is Determinate-managed on every mac runner (`nix.enable = false`):
# the upstream nixos.org installer's launchd nix-daemon crash-loops on
# the EC2 macOS AMI (dyld library-validation refuses /nix/store dylibs
# in a hardened launchd process when /nix isn't a firmlink-blessed
# mount). Determinate sets the volume + firmlink + daemon up as one
# coherent unit. The trusted-users + binary-cache settings live in
# /etc/nix/nix.custom.conf, written by macos-runner-bootstrap.sh.
#
# Execution model: macOS PR jobs run NATIVELY on the agent (no docker
# plugin — `just nextest-macos-pr` uses the macOS-built sandbox-exec
# path), so the agent keeps the full toolchain (rustup, sccache, the
# brew/Xcode shims). Single agent per box; the agent auto-spawns its own
# sccache server on first `RUSTC_WRAPPER=sccache` call. State lives at
# /var/lib/buildkite-agents/<name>/.
#
# Headless / always-on. Tailscale SSH (LAN-side native sshd unloaded) is
# the only user-facing interface; the tailnet ACLs gate access. pmset is
# locked into "never sleep". The pf agent-egress lockdown cuts the agent
# UID off from RFC1918 (defense-in-depth against a malicious CI job
# pivoting to whatever LAN/VPC the box sits on).

let
  cfg = config.sealed.macosBuildkiteAgent;

  # Git credential helper for checkout-time clone auth — mints a
  # sealedsecurity-ci App installation token over HTTPS so the agent can
  # clone the repo (the Buildkite pipeline's git@github.com: SSH URL is
  # rewritten to HTTPS via the gitconfig below). Shared with mattserver
  # (shared/buildkite-git-credential-app.nix); this host stages the App
  # key to /var/run/buildkite-agent/ci-app-key.pem via the
  # decrypt-agent-secrets launchd daemon. App ID 4045728 is a public
  # identifier, not a secret.
  ciGitCredentialHelper = import ../../shared/buildkite-git-credential-app.nix {
    inherit pkgs;
    appId = "4045728";
    keyPath = "/var/run/buildkite-agent/ci-app-key.pem";
  };

  # Git config for the AGENT processes only — pointed at via
  # GIT_CONFIG_GLOBAL in each agent daemon's EnvironmentVariables (the
  # agent user has no ~/.gitconfig), NOT written to /etc/gitconfig. A
  # system-wide insteadOf rewrite is additive and can't be cancelled at
  # a lower config level, so /etc/gitconfig would silently redirect
  # the admin user's own git@github.com: SSH clones to HTTPS+App-token too.
  agentGitConfig = pkgs.writeText "buildkite-agent-gitconfig" ''
    [url "https://github.com/"]
        insteadOf = git@github.com:
    [credential "https://github.com"]
        helper = ${ciGitCredentialHelper}/bin/buildkite-git-credential-app
  '';

  # Service user for the Buildkite agents. nix-darwin's github-runner
  # module used to create `_github-runner` (uid/gid 533); we now hand-
  # roll the agent launchd daemons (nix-darwin has no buildkite-agents
  # module), so we declare the user ourselves. Renamed to drop the
  # github reference; uid/gid 533 is preserved so the existing
  # /var/lib state-dir ownership carries over without a chown sweep.
  agentUser = "_buildkite-agent";
  agentUid = 533;

  # Single agent instance. The M2 Pro mini has 16 GB unified memory;
  # one cold-cache cargo build on the seal workspace spikes several GB
  # across codegen workers, so a single instance is the safe ceiling.
  # CARGO_BUILD_JOBS is tuned below for the mini's core count.
  #
  # Unlike mattserver (Linux), macOS PR jobs run NATIVELY on the
  # agent — test-macos.yml has no docker plugin (`just
  # nextest-macos-pr` runs directly, using the macOS-built
  # sandbox-exec path). So the agent keeps the full toolchain
  # (rustup, sccache, the brew/Xcode shims). With a single instance
  # the agent auto-spawns its own sccache server on first
  # `RUSTC_WRAPPER=sccache` call — no separate shared-server daemon
  # needed (the SEA-672 shared-server consolidation only mattered for
  # concurrent agents racing one cache).
  #
  # State lives at /var/lib/buildkite-agents/<name>/.

  # Homebrew wrappers for the agent PATH. The hand-rolled launchd
  # daemon (below) sets PATH explicitly from the package list; macOS
  # GUI/CI tools like `brew` live under /opt/homebrew/bin (the Apple
  # Silicon Homebrew prefix) which isn't on that curated PATH, so we
  # shim them in as Nix-managed bin scripts. SEA-640 / SEA-840.
  #
  # Wrapping (vs. directly prepending /opt/homebrew/bin to the runner
  # PATH) keeps the shim list explicit: only `brew` and `brew`-
  # installed binaries we actually depend on get a wrapper. If a
  # workflow ever needs a different Homebrew-managed tool, add it to
  # this list rather than blanket-mounting /opt/homebrew/bin.
  brewShims = pkgs.symlinkJoin {
    name = "brew-shims";
    paths = [
      (pkgs.writeShellScriptBin "brew" ''
        exec /opt/homebrew/bin/brew "$@"
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
    # Command runner — the macOS test pipeline drives the build via
    # `just` recipes (build-wasm-agent-staged, etc.). Pre-SEA-830 the
    # hosted flow brew-installed it per job; the self-hosted agent
    # bakes it into PATH here.
    just
    # NOTE: deliberately no dbus / mold / clang / gcc here — macOS's
    # clang from the Xcode Command Line Tools is what cargo uses on
    # this platform; Linux-only deps would just be dead weight.
  ];

  # Per-instance environment (cargo/rustup/sccache/bun caches). The
  # launchd daemon merges this into EnvironmentVariables. Same shape
  # the github-runner module's `extraEnvironment` carried.
  mkAgentEnv = name: {
    # Per-agent cargo home. A pool-wide /shared registry was the
    # SEA-672 failure shape on the multi-runner boxes: concurrent
    # cargo invocations race during crate-source extraction (cargo
    # creates `registry/src/<crate>-<ver>/` before unpacking, sccache
    # tries to hash the source mid-unpack, gets ENOENT). The
    # index-lockfile serializes registry *index* updates, NOT source
    # unpacks, so a shared registry is only safe when at most one
    # cargo runs at a time. The mini runs a single agent so the race
    # can't happen — but per-agent CARGO_HOME is kept anyway as the
    # correct default (cheap, ~1-2 GB, and right if a 2nd instance is
    # ever added). Confirmed on GHA too: a shared cargo dir between
    # agents broke; per-agent is the rule.
    CARGO_HOME = "/var/lib/buildkite-agents/${name}/.cargo";

    # Isolated per agent — rustup has no concurrent-write protection
    # and corrupts toolchain state if two agents install at once.
    RUSTUP_HOME = "/var/lib/buildkite-agents/${name}/.rustup";

    # CARGO_BUILD_JOBS = the host's performance-core count, so heavy
    # parallel codegen stays on the P-cores. Set per-host via
    # cfg.cargoBuildJobs (M2 Pro mattmini = 6; base M4 mini + mac2-m2
    # EC2 = 4). VERIFY ON BOX: `sysctl -n hw.perflevel0.physicalcpu`.
    CARGO_BUILD_JOBS = toString cfg.cargoBuildJobs;

    # sccache wiring. Agent-level RUSTC_WRAPPER means cargo uses
    # sccache automatically without per-workflow opt-in. With a single
    # agent the first `RUSTC_WRAPPER=sccache` call auto-spawns a
    # per-agent sccache server — no separate shared-server launchd
    # daemon (the SEA-672 consolidation existed only to stop multiple
    # agents racing one shared cache; one agent has no such race). The
    # server persists across that agent's jobs, keeping the local
    # compile cache warm.
    RUSTC_WRAPPER = "sccache";
    SCCACHE_DIR = "/var/lib/buildkite-agents/${name}/.sccache";
    SCCACHE_CACHE_SIZE = "30G";
    # SEA-680: never idle-exit. Re-reading the LRU index off disk on
    # every 10-min idle re-spawn costs ~1-2s; keep the server up.
    SCCACHE_IDLE_TIMEOUT = "0";

    # Per-agent bun + uv install caches. Bun backs docs/site +
    # linear-auto-done bundles; uv is staged for a future Python tool.
    # Both `mkdir -p` their dir on first use, so no activation entry.
    BUN_INSTALL_CACHE_DIR = "/var/lib/buildkite-agents/${name}/.bun-install";
    UV_CACHE_DIR = "/var/lib/buildkite-agents/${name}/.uv-cache";
  };

  # Hand-rolled launchd daemon for one Buildkite agent instance.
  # nix-darwin has no services.buildkite-agents module, so we run the
  # agent under launchd via a small wrapper script. The wrapper exists
  # for three reasons launchd-direct can't handle:
  #
  #   1. Token. buildkite-agent reads the token from the
  #      BUILDKITE_AGENT_TOKEN *value* (there is no *_TOKEN_PATH var).
  #      Putting the value in the plist would leak it (the plist is
  #      world-readable in the Nix store), so the wrapper reads it at
  #      launch from the tmpfs file the decrypt-agent-secrets daemon
  #      stages and exports it in-process.
  #   2. Tags. The tag string contains `=` (queue=macos-arm64-selfhosted),
  #      which launchd's EnvironmentVariables parser mangles — it splits
  #      on the first `=` and treats the rest as the value. Passing tags
  #      as a `--tags` ARG sidesteps launchd env parsing entirely.
  #   3. Logs. launchd opens StandardOutPath as the daemon user
  #      (_buildkite-agent); /var/log is root-owned and not user-
  #      writable, so launchd died with EX_CONFIG (78) before exec. The
  #      wrapper logs to the agent-owned state dir instead.
  #
  # KeepAlive restarts on crash; the agent's own graceful-stop handles
  # in-flight jobs on shutdown. Cargo/sccache cache env (mkAgentEnv)
  # still flows through the plist — those values contain no `=`.
  mkAgentDaemon = name: {
    serviceConfig = {
      Label = "com.sealedsecurity.buildkite-agent-${name}";
      ProgramArguments = [
        "${pkgs.writeShellScript "buildkite-agent-${name}-start" ''
          set -euo pipefail
          # Token by value (off the plist + off argv). The
          # decrypt-agent-secrets daemon stages this file 0640, group
          # _buildkite-agent, which this daemon's user can read.
          BUILDKITE_AGENT_TOKEN="$(cat /var/run/buildkite-agent/agent-token)"
          export BUILDKITE_AGENT_TOKEN
          exec ${pkgs.buildkite-agent}/bin/buildkite-agent start \
            --name ${cfg.hostName}-${name} \
            --tags queue=macos-arm64-selfhosted,os=macos,arch=arm64,host=${cfg.hostName} \
            --build-path /var/lib/buildkite-agents/${name}/builds \
            --hooks-path ${pkgs.buildkite-agent}/share/buildkite-agent/hooks \
            --plugins-path /var/lib/buildkite-agents/${name}/plugins
        ''}"
      ];
      EnvironmentVariables = {
        PATH = "${lib.makeBinPath agentPackages}:/usr/bin:/bin:/usr/sbin:/sbin";
        HOME = "/var/lib/buildkite-agents/${name}";
        # Agent-scoped git config (insteadOf rewrite + credential
        # helper) — GIT_CONFIG_GLOBAL so it applies only to the agent
        # processes, not the admin user's own git. See agentGitConfig.
        GIT_CONFIG_GLOBAL = "${agentGitConfig}";
      }
      // mkAgentEnv name;
      UserName = agentUser;
      GroupName = agentUser;
      RunAtLoad = true;
      KeepAlive = true;
      # Log into the agent-owned state dir, NOT /var/log — launchd opens
      # these paths as _buildkite-agent, which can't write root-owned
      # /var/log (that was the EX_CONFIG 78 spawn failure). The state dir
      # is chowned to the agent in the extraActivation block.
      StandardOutPath = "/var/lib/buildkite-agents/${name}/agent.log";
      StandardErrorPath = "/var/lib/buildkite-agents/${name}/agent.log";
    };
  };
in

{
  options.sealed.macosBuildkiteAgent = {
    enable = lib.mkEnableOption "self-hosted macOS Buildkite CI agent (sealedsecurity org)";

    hostName = lib.mkOption {
      type = lib.types.str;
      example = "mattmini";
      description = ''
        Host identity. Drives networking.{hostName,computerName,
        localHostName}, the Buildkite agent `--name <hostName>-<agent>`,
        and the `host=<hostName>` agent tag (so the fleet is
        self-identifying in the Buildkite agents list).
      '';
    };

    cargoBuildJobs = lib.mkOption {
      type = lib.types.ints.positive;
      default = 4;
      description = ''
        CARGO_BUILD_JOBS for the agent — set to the host's
        performance-core count so heavy codegen stays on the P-cores.
        Default 4 (base M4 mini / mac2-m2 EC2); the M2 Pro mattmini
        overrides to 6. Verify on the box with
        `sysctl -n hw.perflevel0.physicalcpu`.
      '';
    };

    adminUser = lib.mkOption {
      type = lib.types.str;
      default = "mattw";
      description = ''
        The macOS admin account home-manager applies to. Default
        `mattw` (created during install on the owned/rental minis). The
        EC2 AMI ships an `ec2-user` admin already, so awsmac sets this
        to `ec2-user` rather than creating a second account.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # ============================================================
    # Hostname + user
    # ============================================================

    networking.hostName = cfg.hostName;
    networking.computerName = cfg.hostName;
    networking.localHostName = cfg.hostName;

    # User registration. nix-darwin needs `users.users.<name>` declared
    # so home-manager.users.<name> resolves. On the owned/rental minis
    # this is `mattw` (created during macOS install); on the EC2 AMI it's
    # the AMI-provided `ec2-user` (admin, passwordless sudo, SSH keypair
    # already wired — one less account-creation step). Either way this
    # block just points nix-darwin at the existing account.
    users.users.${cfg.adminUser} = {
      name = cfg.adminUser;
      home = "/Users/${cfg.adminUser}";
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

    system.primaryUser = cfg.adminUser;

    # ============================================================
    # Nix
    # ============================================================

    # Determinate Nix manages the daemon — same model as the MBP
    # (`nix.enable = false`). The EC2 macOS AMI is the reason: the
    # upstream nixos.org installer's launchd daemon crash-loops on the
    # AMI because dyld's library-validation refuses to load /nix/store
    # dylibs into a hardened launchd-spawned process when /nix isn't a
    # firmlink-blessed mount (the AMI ships /nix as a plain APFS volume
    # with no /etc/synthetic.conf, so a manual `nix-daemon` works but the
    # launchd one fails with "file system sandbox blocked open()" /
    # OS_REASON_DYLD). Determinate's installer sets the volume + firmlink
    # + daemon up as one coherent, reboot-surviving unit that doesn't trip
    # that validation. nix-darwin must NOT manage the daemon plist on top
    # of Determinate's, so disable it here.
    nix.enable = false;

    # No nix.settings.* here: with `nix.enable = false`, nix-darwin does
    # not write /etc/nix/nix.conf, so those settings would be silently
    # ignored. Determinate owns nix.conf; the trusted-users + substituters
    # + trusted-public-keys go into /etc/nix/nix.custom.conf, written by
    # the bootstrap script's Nix step (same pattern as the MBP's
    # mac-setup.sh). They mirror flake.nix's nixConfig so the garnix +
    # nixos-raspberrypi caches are honored.

    # Same direnv check-phase skip the MBP needs on aarch64-darwin's
    # 25.11 — the fish test runner SIGKILLs inside the build sandbox.
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
        # NOTE: no Swift toolchain managers (xcodes / swiftly) here. seal's
        # macOS CI needs only the Xcode Command Line Tools (clang/ld via
        # the xcodeShims above), not full Xcode or a pinned Swift version.
        # Adding a Swift-version manager would be dead weight for this CI
        # workload. SEA-840.
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
        #   /opt/homebrew/sbin/tailscaled — root-owned system daemon
        #   /opt/homebrew/bin/tailscale   — CLI that talks to it via
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
    # Strict always-on policy. The mini is a headless CI host with
    # no other role — every sleep mode is disabled so SSH stays
    # reachable and CI jobs land instantly. Wake-on-magic-packet and
    # autorestart-on-power-loss as safety nets in case the box ever
    # does power-cycle (closet circuit breaker trip, etc.).
    #
    # pmset is per-power-source on laptops; the mini is AC-only so
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
      /opt/homebrew/bin/tailscale set --ssh=true 2>/dev/null || true
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
    # --hostname=<hostName>` (bootstrap step 4). After that the
    # daemon keeps state in /Library/Tailscale and stays joined
    # across reboots.

    # ============================================================
    # Glances — system metrics dashboard, exposed via Tailscale Serve
    # ============================================================
    #
    # Glances runs in web-server mode on localhost; Tailscale Serve
    # fronts it with TLS on the tailnet hostname. Reachable from any
    # tailnet device at:
    #     https://<hostName>.tail08a5c5.ts.net:9443/
    #
    # No HTTP basic auth on Glances itself — the tailnet boundary
    # provides authentication (only tailnet members can resolve the
    # hostname / reach the IP). Tailscale ACLs already gate which
    # devices can talk to the box; we trust that as the auth layer.
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
          "/opt/homebrew/bin/glances"
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
              if /opt/homebrew/bin/tailscale status --json 2>/dev/null \
                 | grep -q '"BackendState":"Running"'; then
                break
              fi
              sleep 5
            done

            # `tailscale serve` is persistent across reboots once set;
            # re-running it is idempotent. Output is captured to the
            # log file below for debugging.
            /opt/homebrew/bin/tailscale serve --bg --https=9443 \
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

      # Everything else (the admin user's shell, system daemons, anyone not
      # the agent UID) passes unaffected.
      pass out all
    '';

    # Loader daemon — runs at boot, calls `pfctl -ef` to enable pf
    # and load our ruleset. `-e` enables pf (stateless, no refcount);
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
    # SEA-840: the mini runs a self-hosted Buildkite agent. test-macos.yml
    # routes to the `macos-arm64-selfhosted` queue this agent registers
    # (repointed from hosted `macos-medium` once the box is green).
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
    # tmpfs at boot by the decrypt-agent-secrets daemon below.

    # Pre-create the agent state dirs with the right ownership. nix-darwin
    # has no tmpfiles abstraction, and custom-named activation script
    # slots are silently ignored (the activation runner only invokes the
    # slot names baked into modules/system/activation-scripts.nix), so we
    # hook the existing `extraActivation` slot. The state tree is kept at
    # /var/lib/buildkite-agents (unchanged from the runner era) so the warm
    # cargo/sccache caches survive the cutover.
    system.activationScripts.extraActivation.text = ''
      echo >&2 "setting up Buildkite agent dirs..."
      /bin/mkdir -p /var/lib/buildkite-agents/sealed-macos/.cargo
      /bin/mkdir -p /var/lib/buildkite-agents/sealed-macos/.sccache
      # chown by numeric uid:gid, not name. This activation slot runs
      # BEFORE nix-darwin's user-creation step, so `_buildkite-agent`
      # isn't resolvable yet — `chown _buildkite-agent:_buildkite-agent`
      # fails "illegal group name". The numeric 533:533 (pinned in the
      # users.users/groups block above) needs no name lookup.
      /usr/sbin/chown -R ${toString agentUid}:${toString agentUid} /var/lib/buildkite-agents

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
      # Both commands no-op when their dependencies aren't available
      # (mdutil reports "unknown indexing state" when the volume isn't
      # Spotlight-indexed — as is the case for this path; tmutil bails if
      # TM isn't configured) — fine for a headless box that may not have
      # either active. Redirect BOTH stdout and stderr: mdutil writes its
      # "unknown indexing state" / "invalid operation" notice to STDOUT,
      # not stderr, so a bare `2>/dev/null` leaves it cluttering every
      # nix-switch. `|| true` suppresses the exit status so a failure here
      # never fails activation.
      echo >&2 "applying agent-state Spotlight + Time Machine exclusions..."
      /usr/bin/mdutil -i off /var/lib/buildkite-agents >/dev/null 2>&1 || true
      if [ -d /var/lib/buildkite-agents ]; then
        /usr/bin/tmutil addexclusion /var/lib/buildkite-agents >/dev/null 2>&1 || true
      fi
    '';

    # Stage both agent secrets at boot: the Buildkite agent token (read by
    # the agent at start) and the sealedsecurity-ci App key (read at
    # checkout by the git credential helper). macOS has no systemd-creds;
    # the bootstrap writes plaintext to /etc/buildkite-agent/{agent-token,
    # ci-app-key.pem} (mode 600 root:wheel) and this one daemon re-stages
    # group-readable copies under /var/run on every boot (macOS /var/run
    # isn't a tmpfs but is cleared on boot). One daemon for both — mirrors
    # mattserver, where a second unit re-owning the shared dir clobbered
    # the first's group (the regression that broke token reads). This is
    # the existing macpro plaintext-in-/etc tradeoff applied to both.
    launchd.daemons.decrypt-agent-secrets = {
      serviceConfig = {
        Label = "com.sealedsecurity.decrypt-agent-secrets";
        ProgramArguments = [
          "/bin/sh"
          "-c"
          ''
            set -e
            install -d -m 0750 -o root -g ${agentUser} /var/run/buildkite-agent
            install -m 0640 -o root -g ${agentUser} \
              /etc/buildkite-agent/agent-token \
              /var/run/buildkite-agent/agent-token
            install -m 0640 -o root -g ${agentUser} \
              /etc/buildkite-agent/ci-app-key.pem \
              /var/run/buildkite-agent/ci-app-key.pem
          ''
        ];
        RunAtLoad = true;
        KeepAlive = false;
        StandardOutPath = "/var/log/decrypt-agent-secrets.log";
        StandardErrorPath = "/var/log/decrypt-agent-secrets.log";
      };
    };

    # (git config for the agents lives in agentGitConfig, pointed at via
    # GIT_CONFIG_GLOBAL in each agent daemon's EnvironmentVariables above
    # — NOT /etc/gitconfig, so it doesn't rewrite the admin user's own git. See the
    # agentGitConfig comment in the let block.)

    # Single agent instance. 16 GB unified memory is the constraint —
    # one cold-cache cargo build on the workspace can spike several GB
    # across parallel codegen workers, so one agent keeps the box inside
    # 16 GB without swap. The macOS path filter only fires when sandbox /
    # process-spawn / TLS code changes (a cheaper-to-skip layer than
    # Linux), so a single agent is plenty; warm-cache steady state hits
    # ~1m per macOS job. Add a second instance only if the box proves it
    # has the RAM headroom under real concurrent load.
    #
    # launchd has no dependency-ordering primitive like systemd's
    # After=/Requires=. The agent daemon retries the token-file read on
    # its own (KeepAlive restarts it until decrypt-agent-secrets has staged
    # the file), so a boot-time race just costs a restart or two.
    launchd.daemons.buildkite-agent-sealed-macos = mkAgentDaemon "sealed-macos";

    # ============================================================
    # sccache
    # ============================================================
    #
    # No separate sccache-server launchd daemon. With a single agent the
    # agent's own `RUSTC_WRAPPER=sccache` call auto-spawns a per-agent
    # server (SCCACHE_DIR + SCCACHE_IDLE_TIMEOUT=0 set in mkAgentEnv),
    # which persists across that agent's jobs. The SEA-672 shared-server
    # consolidation existed only to give multiple concurrent agents one
    # cache-state owner + restart supervision; a single agent has neither
    # the race nor the need. If a 2nd instance is ever added, run a
    # shared sccache server as its own launchd daemon (one cache-state
    # owner for the concurrent agents) instead of per-agent auto-spawn.

    # ============================================================
    # Agent cache cleanup — weekly launchd job
    # ============================================================
    #
    # mattserver has a systemd timer for this; nix-darwin uses launchd.
    # Same find-and-rm logic as nixos/mattserver/system.nix, but uses
    # `find -exec rm` rather than `find -print0 | xargs -0 -r`: macOS's
    # BSD xargs has no GNU `-r` / `--no-run-if-empty`, and this daemon
    # runs under /bin/sh with the system /bin/xargs (the Nix findutils is
    # on the agent's curated PATH, not this daemon's), so `xargs -r` would
    # fail every scheduled run. `-exec rm -rf {} +` is POSIX, batches the
    # paths like xargs, and runs nothing when find matches nothing.

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
              -exec rm -rf {} +
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
  };
}

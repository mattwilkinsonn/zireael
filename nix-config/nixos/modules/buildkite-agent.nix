{
  config,
  lib,
  pkgs,
  ...
}:

# Shared self-hosted Buildkite CI agent config (sealedsecurity org).
#
# Factored out of nixos/mattserver/system.nix (SEA-830) so multiple
# Linux runner hosts can consume one host-agnostic agent setup
# (SEA-839: the converted trashcan Mac Pro joins mattserver on the
# linux-x64-selfhosted queue). Everything here is host-independent;
# the only per-host knobs are the agent-instance names, the queue
# routing key, and the arch tag (options below). The host tag is
# taken from `networking.hostName` automatically.
#
# Execution model: each PR job runs INSIDE the seal-ci container,
# launched by the Buildkite docker plugin. The agent host only
# provides the agent binary + a docker-compatible CLI; the Rust
# toolchain, sccache, dbus, protobuf, etc. all live in the seal-ci
# image, not on the agent.
#
# Secret staging (the systemd-creds agent-token + ci-app-key blobs)
# is host-bound and stays per-host: each host encrypts its own
# /etc/buildkite-agent/*.cred against its machine-ID. See the
# decrypt-agent-token.service below and the per-host INSTALL.md.

let
  cfg = config.sealed.buildkiteAgent;

  # Shared supplementary group every agent joins so they can all read
  # the one decrypted agent-token file. The nixpkgs
  # services.buildkite-agents module creates a per-agent system user
  # (buildkite-agent-sealed, buildkite-agent-sealed-2, …) with its own
  # primary group, so there's no shared group to chgrp the token to.
  # This group provides one: decrypt-agent-token.service writes the
  # token mode 0640 owned by this group, and each agent (a member)
  # reads it via tokenPath. SEA-830.
  agentTokenGroup = "buildkite-token";

  # Shared group owning the Buildkite cache dir (/cache/bkcache). The
  # Buildkite cache plugin (used by test-linux/lints/live-tests for the
  # cargo target/ + bun-install caches) symlinks each job's cache paths
  # under /cache/bkcache and the docker plugin mounts that root into the
  # container. Under propagate-uid-gid the container (and the cache
  # plugin) run as the agent uid, which must be able to create + write
  # entries under /cache/bkcache. The agents have distinct primary uids,
  # so a single owner can't satisfy both — a shared group + setgid dir
  # (2775, declared via tmpfiles below) lets both write and makes new
  # entries inherit the group. SEA-830.
  agentCacheGroup = "buildkite-cache";

  # Git credential helper for checkout-time clone auth — mints a
  # sealedsecurity-ci App installation token over HTTPS. The host
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
  # (virtualisation.docker below). The host runs real dockerd for CI
  # rather than podman's docker-compat socket: the bwrap-in-container
  # sandbox tests need the same container-exec behavior the hosted
  # Buildkite agents (docker) and future GCP VMs (docker) provide, and
  # podman's rootless-userns nesting diverges from docker for the
  # in-container `bwrap --proc` mount. Standardizing on dockerd keeps
  # ONE portable pipeline across hosted + self-hosted + cloud. The agent
  # user joins the `docker` group to reach /run/docker.sock. (Shared
  # nixos/common.nix still enables podman for the rpis' oci-containers;
  # this module overrides its docker-compat socket off — see below — so
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

  # Per-instance agent config. The buildkite-agents module derives
  # dataDir = /var/lib/buildkite-agent-<name> from the attrset key, so
  # each agent gets its own plugins dir under it (set via extraConfig
  # below) — keyed by `name`. Concurrency is "one job per agent" (the
  # agent claims a job, runs the container, releases); job-internal
  # parallelism (CARGO_BUILD_JOBS) lives in the seal-ci container.
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
    # tags are informational + available for future pinning. `host`
    # comes from networking.hostName so a multi-host fleet stays
    # self-identifying without per-host duplication.
    tags = {
      inherit (cfg) queue arch;
      os = "linux";
      host = config.networking.hostName;
    };

    runtimePackages = agentRuntimePackages;

    # The module's generated buildkite-agent.cfg sets build-path +
    # hooks-path but NOT plugins-path, so any step using a plugin
    # (every PR pipeline uses secret-env + docker) fails at checkout
    # with "Can't checkout plugin without a `plugins-path`". Point it
    # at a per-instance dir under the agent's dataDir
    # (/var/lib/buildkite-agent-<name>, createHome'd + owned by the
    # agent user) so instances don't race a shared plugin checkout.
    # The agent mkdir's it on first use.
    extraConfig = ''plugins-path="/var/lib/buildkite-agent-${name}/plugins"'';

    # Join the shared token-read group (so every agent can read the one
    # decrypted agent-token), the cache group (so all agents can write
    # the shared /cache/bkcache dir under propagate-uid-gid), and the
    # docker group (so the docker plugin can reach /run/docker.sock).
    extraGroups = [
      agentTokenGroup
      agentCacheGroup
      "docker"
    ];
  };

  # Per-agent systemd unit overrides: order behind the secrets decrypt
  # and layer the service environment. Generated from cfg.agentNames so
  # the overrides stay in sync with services.buildkite-agents below —
  # the module names units `buildkite-agent-<name>`. Declared as
  # individual single-key paths (via systemd.services below) so the
  # module system MERGES with the module-generated units rather than
  # replacing them.
  #
  # Per-agent environment, all three host-specific to a persistent
  # self-hosted runner (hosted/ephemeral agents need none of them):
  #   - GIT_CONFIG_GLOBAL: the agent users have no ~/.gitconfig, so
  #     this layers the insteadOf rewrite + App-token credential helper
  #     onto just the agent processes, leaving /etc/gitconfig and
  #     mattw's own git untouched.
  #   - DOCKER_HOST: pin the docker CLI at the real dockerd socket.
  #     shared/linux.nix's shell init exports DOCKER_HOST at the
  #     rootless podman socket for interactive user tooling, and that
  #     value leaks into the agent's service environment — so the
  #     docker plugin's `docker` invocations were silently hitting
  #     podman (5.7.0) instead of dockerd, which reintroduced podman's
  #     netavark networking (a dead 169.254.1.1 first nameserver ->
  #     ~5s DNS timeout per daemon lookup -> 60-70x slower e2e tests).
  #     An explicit service-env value overrides the leak. SEA-830.
  #   - BUILDKITE_GIT_CLEAN_FLAGS: Buildkite's default checkout runs
  #     `git clean -ffxdq` (the `-x` removes gitignored files), which
  #     nukes target/ + the in-checkout cargo registry every job — so a
  #     persistent agent would rebuild cold every time despite the dir
  #     surviving on disk. Exclude them (`-e <pattern>`) so they persist
  #     in-place, per-agent (each agent has its own checkout dir), the
  #     way the GHA self-hosted runners relied on (SEA-640). target-clippy
  #     is the clippy step's separate target dir (CARGO_TARGET_DIR in the
  #     pipeline) — kept apart so clippy's rmeta-only build doesn't poison
  #     the test step's rlib cache. The agent-cache-cleanup timer prunes
  #     stale dirs. SEA-834.
  agentUnitOverrides = lib.listToAttrs (
    map (name: {
      name = "buildkite-agent-${name}";
      value = {
        after = [ "decrypt-agent-token.service" ];
        requires = [ "decrypt-agent-token.service" ];
        environment.GIT_CONFIG_GLOBAL = "${agentGitConfig}";
        environment.DOCKER_HOST = "unix:///run/docker.sock";
        environment.BUILDKITE_GIT_CLEAN_FLAGS = "-ffxdq -e target -e target-clippy -e .cargo-home";
      };
    }) cfg.agentNames
  );

  # Space-joined list of each agent's dataDir for the cache-cleanup
  # find. The module puts each agent's checkouts under
  # /var/lib/buildkite-agent-<name>/builds/.
  agentBuildDirs = lib.concatMapStringsSep " " (
    name: "/var/lib/buildkite-agent-${name}"
  ) cfg.agentNames;
in
{
  options.sealed.buildkiteAgent = {
    enable = lib.mkEnableOption "self-hosted Buildkite CI agents (sealedsecurity org)";

    agentNames = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      example = [
        "sealed"
        "sealed-2"
      ];
      description = ''
        Agent instance names. Each becomes a services.buildkite-agents
        entry (and a buildkite-agent-<name> systemd unit) with its own
        dataDir under /var/lib/buildkite-agent-<name>. Concurrency is
        one job per agent, so the count is effectively the host's
        parallel-job capacity — size it to the core count.
      '';
    };

    queue = lib.mkOption {
      type = lib.types.str;
      default = "linux-x64-selfhosted";
      description = ''
        The Buildkite `queue` tag the agents register. The
        .buildkite/pipelines/*.yml steps route to this key.
      '';
    };

    arch = lib.mkOption {
      type = lib.types.str;
      default = "x64";
      description = "The informational `arch` agent tag.";
    };
  };

  config = lib.mkIf cfg.enable {
    # SEA-830: self-hosted Buildkite agents. PR-time Linux pipelines
    # (lints, test-linux, live-tests, deploy-docs) route to the queue
    # these agents register (cfg.queue). Compile-result caching is
    # sccache → disk L0 + Cloudflare R2 L1, configured inside the
    # container (SEA-834); warm cargo-registry / target dirs persist
    # in-checkout via BUILDKITE_GIT_CLEAN_FLAGS (SEA-834).
    services.buildkite-agents = lib.listToAttrs (
      map (name: {
        inherit name;
        value = mkAgent name;
      }) cfg.agentNames
    );

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
    # /run/docker.sock at podman for the rpis' oci-containers). On a
    # runner host dockerd must own /run/docker.sock instead, so force
    # podman's docker-compat socket off here. Podman stays installed (the
    # shared config still enables the engine) but no longer claims the
    # socket.
    virtualisation.docker.enable = true;
    virtualisation.podman.dockerCompat = lib.mkForce false;
    virtualisation.podman.dockerSocket.enable = lib.mkForce false;

    # Shared token-read group every agent joins. The buildkite-agents
    # module creates per-agent users each with its own primary group;
    # this shared group is what the one decrypted agent-token file is
    # chgrp'd to so every agent can read it regardless of instance count.
    users.groups.${agentTokenGroup} = { };

    # Shared cache group + the setgid cache dirs. All agents are members
    # (extraGroups in mkAgent); the dirs are group-owned mode 2775 so any
    # agent can create entries and the setgid bit makes those entries
    # inherit the group (so another agent can then read/evict them on a
    # later job).
    users.groups.${agentCacheGroup} = { };
    systemd.tmpfiles.rules = [
      "d /cache 0755 root root -"
      "d /cache/bkcache 2775 root ${agentCacheGroup} -"
      # sccache multi-level disk L0. Per-agent subdirs are created by the
      # CI job (mkdir under BUILDKITE_AGENT_NAME); this just provisions
      # the shared parent group-writable + setgid so any agent's
      # container (running as the in-image uid after ci-entrypoint
      # chowns it) can create + own its subdir. Same pattern as
      # /cache/bkcache. SEA-834.
      "d /cache/sccache 2775 root ${agentCacheGroup} -"
    ];

    # Token provisioning. The agents register with the Buildkite *Agent*
    # token (org Agents page) — distinct from the BUILDKITE_API_TOKEN the
    # `bk` CLI uses. Encrypt it once on the host (host-bound via
    # systemd-creds machine-ID encryption); see the per-host INSTALL.md
    # + nixos/scripts/*-encrypt-agent-token.sh. That writes
    # /etc/buildkite-agent/agent-token.cred. The decrypt-agent-token
    # oneshot below decrypts it to /run/buildkite-agent/agent-token
    # (tmpfs, mode 640, readable by the agent group) before any agent
    # unit starts. Plaintext never touches persistent storage after the
    # bootstrap step.
    #
    # The per-agent unit overrides (agentUnitOverrides) and the
    # agent-cache-cleanup oneshot are merged into this attrset so the
    # whole CI surface lives in one systemd.services declaration.
    systemd.services = {
      decrypt-agent-token = {
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

      # Weekly prune of Rust target/ directories left in the agents'
      # build checkouts. The module puts each agent's checkouts under
      # /var/lib/buildkite-agent-<name>/builds/; a single workspace
      # target/ can reach 10-20 GB. 30-day window keeps warm-ish trees
      # for infrequent-push branches while bounding disk. (target/
      # inside the *container* is separate; this is just host-side
      # checkout state.)
      #
      # Covers `target`, `target-clippy` (the clippy step's separate
      # CARGO_TARGET_DIR), and `.cargo-home` — all kept across jobs by
      # the BUILDKITE_GIT_CLEAN_FLAGS exclude above (SEA-834), so they
      # need the same staleness prune.
      agent-cache-cleanup = {
        description = "Prune stale Rust build caches from agent build dirs";
        serviceConfig = {
          Type = "oneshot";
          ExecStart = pkgs.writeShellScript "agent-cache-cleanup" ''
            find ${agentBuildDirs} \
              -maxdepth 6 \
              \( -name "target" -o -name "target-clippy" -o -name ".cargo-home" \) \
              -type d \
              -mtime +30 \
              -print0 2>/dev/null | xargs -0 -r rm -rf
          '';
        };
      };
    }
    // agentUnitOverrides;

    systemd.timers.agent-cache-cleanup = {
      description = "Weekly cleanup of stale agent Rust build artifacts";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "weekly";
        Persistent = true;
      };
    };
  };
}

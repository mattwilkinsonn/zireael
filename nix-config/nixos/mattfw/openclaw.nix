{ pkgs, ... }:

# OpenClaw — native systemd user service via nix-openclaw home-manager
# module. Replaces the previous oci-containers/podman setup (2026-05).
# The user-side config lives in nixos/mattfw/home.nix
# (`programs.openclaw = { ... }`); this file owns the host-side
# plumbing: secret materialization, workspace → GitHub sync, and the
# Tailscale Serve mapping to the gateway port.
#
# Why native instead of a container:
#   - the agent shells out to CLI tools (gh, jj, git, claude, af, ...)
#     that mattw already has provisioned via shared/dev.nix. Running
#     in a container meant duplicating every tool inside the image.
#   - nix-openclaw's `programs.openclaw.runtimePackages` exposes
#     packages on the gateway's PATH (NOT the user's login PATH),
#     so the inheritance now flows the right direction.
#
# Secrets: per-secret files under /run/openclaw-secrets/, owned by
# mattw (uid 1000 / gid 100 = `users`), mode 0600. The home-manager
# module references them via `*.tokenFile = "/run/openclaw-secrets/..."`.
# This is the upstream-blessed shape — tokens never land in the Nix
# store and never get exported into the gateway's environment as
# plaintext env vars.
{
  # OpenClaw secret refresh — pulls gateway token + provider keys
  # from 1Password Server vault using mattw's service account token
  # at ~/.config/op/service-account-token (root reads mattw's 0600
  # file directly; mattfw is single-user so no systemd-creds dance
  # needed). Also writes Akiflow CLI credentials into mattw's $HOME
  # so the `af` CLI works on this headless box without browser-token
  # extraction.
  #
  # The user service `openclaw-gateway.service` (provided by
  # nix-openclaw) reads these via `*.tokenFile` paths in
  # `programs.openclaw.config`. wantedBy/before wiring ensures the
  # files exist before the gateway starts. The user service runs as
  # mattw — same uid that owns the secret files — so the read is
  # straightforward.
  systemd.services.openclaw-env-refresh = {
    description = "Refresh OpenClaw secrets + Akiflow creds from 1P Server vault";
    wantedBy = [ "multi-user.target" ];
    before = [ "user@1000.service" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];

    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      RuntimeDirectory = "openclaw-secrets";
      RuntimeDirectoryMode = "0750";
      RuntimeDirectoryPreserve = "yes";
      # `op` needs a writable HOME for its config dir; without one it errors
      # with "Unable to determine location of config directory" and silently
      # returns empty values from `op read`. Point HOME at a per-invocation
      # state dir so the 1P CLI is happy. None of the paths below resolve
      # against HOME — they're all absolute.
      StateDirectory = "openclaw-env-refresh";
      Environment = [ "HOME=%S/openclaw-env-refresh" ];
      ExecStart = pkgs.writeShellScript "openclaw-env-refresh" ''
        set -euo pipefail

        IFS= read -r OP_SERVICE_ACCOUNT_TOKEN \
          < /home/mattw/.config/op/service-account-token
        export OP_SERVICE_ACCOUNT_TOKEN
        OP="${pkgs._1password-cli}/bin/op"

        # Materialize one file per secret under /run/openclaw-secrets.
        # nix-openclaw consumes these via `*.tokenFile` paths; the
        # gateway reads them at startup, not from env. Empty/missing
        # values fail fast so the gateway never starts with half-set
        # secrets.
        write_secret() {
          local name=$1
          local op_ref=$2
          local val
          val=$("$OP" read "$op_ref")
          [ -n "$val" ] || { echo "ERROR: $name empty from 1P"; exit 1; }
          umask 077
          printf '%s' "$val" > "/run/openclaw-secrets/$name.tmp"
          chown 1000:100 "/run/openclaw-secrets/$name.tmp"
          chmod 0600 "/run/openclaw-secrets/$name.tmp"
          mv "/run/openclaw-secrets/$name.tmp" "/run/openclaw-secrets/$name"
        }

        write_secret gateway-token   'op://Server/OpenClaw Gateway Token/credential'
        write_secret brave-search    'op://Server/OpenClaw Brave Search API Key/credential'
        write_secret discord-token   'op://Server/OpenClaw Discord Bot Token/credential'
        write_secret openrouter      'op://Server/OpenClaw OpenRouter API Key/credential'
        write_secret gh-workspace    'op://Server/OpenClaw Workspace GH Token/token'

        # Ensure the secrets dir itself is mattw-readable. RuntimeDirectory
        # creates it 0750 root:root; we want mattw to read its contents.
        chown 1000:100 /run/openclaw-secrets
        chmod 0750 /run/openclaw-secrets

        # Akiflow CLI credentials — separate target, owned by mattw (uid 1000,
        # gid 100 — `users` group), so the `af` CLI works on this box without
        # browser token extraction. Numeric uid/gid match the secrets above
        # and avoid name-lookup failures if this service ever races with
        # user provisioning on a fresh boot.
        AKICRED=/home/mattw/.config/af/credentials.json
        TMPAKI="$AKICRED.refresh.$$"
        install -d -m 700 -o 1000 -g 100 /home/mattw/.config/af
        "$OP" read 'op://Server/Akiflow CLI Credentials/credentials.json' \
          > "$TMPAKI"
        [ -s "$TMPAKI" ] || { echo "ERROR: Akiflow credentials empty from 1P"; rm -f "$TMPAKI"; exit 1; }
        chown 1000:100 "$TMPAKI"
        chmod 600 "$TMPAKI"
        mv "$TMPAKI" "$AKICRED"

        echo "OpenClaw secrets + Akiflow creds refreshed at $(date -Iseconds)"
      '';
    };
  };

  # Periodic workspace → GitHub push. OpenClaw inits the workspace as
  # a local git repo but ships no remote/push functionality (see
  # docs.openclaw.ai/concepts/agent-workspace). 10-minute cadence;
  # noisy snapshot history is the cost.
  #
  # Native path: workspace lives at /home/mattw/.openclaw/workspace
  # (the nix-openclaw default; configurable via programs.openclaw
  # .workspaceDir if we ever move it). Token comes from the
  # /run/openclaw-secrets/gh-workspace file the refresh service wrote.
  systemd.services.openclaw-workspace-sync = {
    description = "Snapshot OpenClaw agent workspace to GitHub";
    after = [
      "openclaw-env-refresh.service"
      "user@1000.service"
    ];
    wants = [ "openclaw-env-refresh.service" ];

    serviceConfig = {
      Type = "oneshot";
      User = "mattw";
      Group = "users";
      WorkingDirectory = "/home/mattw/.openclaw/workspace";
      ExecStart = pkgs.writeShellScript "openclaw-workspace-sync" ''
        set -euo pipefail
        cd /home/mattw/.openclaw/workspace

        # Disable the user-level gh credential helper for every git
        # invocation in this script. shared/home.nix sets the helper
        # to `!gh auth git-credential`, but gh isn't on this systemd
        # unit's PATH — so without this override every push logs
        # "gh: command not found" before falling through to the
        # URL-embedded PAT. `-c credential.helper=` (empty value)
        # disables the helper for this invocation only.
        GIT="${pkgs.git}/bin/git -c credential.helper="

        # If OpenClaw hasn't initialized the workspace as a git repo yet
        # (early first-run state), bail quietly — the timer will retry.
        if [ ! -d .git ]; then
          echo "workspace not yet git-initialized; skipping sync"
          exit 0
        fi

        # Read the workspace PAT from /run/openclaw-secrets (written by
        # openclaw-env-refresh.service). Read-only inside the unit because
        # we never modify it from here.
        IFS= read -r GH_TOKEN < /run/openclaw-secrets/gh-workspace
        [ -n "$GH_TOKEN" ] || { echo "ERROR: workspace token empty"; exit 1; }

        # First-run + every-run: ensure origin URL embeds the latest PAT.
        # `set-url` is idempotent; `add` is used the first time.
        REMOTE_URL="https://x-access-token:$GH_TOKEN@github.com/mattwilkinsonn/openclaw-workspace.git"
        if $GIT remote get-url origin >/dev/null 2>&1; then
          $GIT remote set-url origin "$REMOTE_URL"
        else
          $GIT remote add origin "$REMOTE_URL"
        fi

        if [ -n "$($GIT status --porcelain)" ]; then
          $GIT add -A
          $GIT \
            -c user.email=mattw@mattfw.local \
            -c user.name='OpenClaw Sync' \
            commit -m "workspace snapshot $(date -Iseconds)"
        fi

        # Push HEAD to main. Exit 0 even on push failure so the timer
        # doesn't go into a failed state — the commit history shows
        # what landed; systemctl status shows the last attempt.
        $GIT push origin HEAD:main 2>&1 || {
          echo "push failed, will retry on next timer firing"
          exit 0
        }
      '';
    };
  };

  systemd.timers.openclaw-workspace-sync = {
    description = "Periodic OpenClaw workspace → GitHub sync";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "5min";
      OnUnitActiveSec = "10min";
      Persistent = true;
    };
  };

  # Tailscale Serve mappings. Two ports terminate on this host:
  #   - :443  → http://localhost:18789   OpenClaw gateway (Control UI)
  #   - :9443 → https://localhost:9090   Cockpit (same convention as the Pis)
  # Both are one-shot `tailscale serve --bg` invocations; the daemon
  # persists the mapping across reboots, so re-running is idempotent.
  # Cockpit's Origins allowlist in nixos/common.nix already templates
  # `${hostName}.tail08a5c5.ts.net:9443` so the proxied login passes.
  # The gateway's controlUi.allowedOrigins is set declaratively in
  # nixos/mattfw/home.nix.
  systemd.services.tailscale-serve-openclaw = {
    description = "Tailscale Serve mappings (OpenClaw gateway + Cockpit)";
    wantedBy = [ "multi-user.target" ];
    after = [
      "tailscaled.service"
      "network-online.target"
    ];
    wants = [
      "tailscaled.service"
      "network-online.target"
    ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = pkgs.writeShellScript "tailscale-serve-openclaw" ''
        ${pkgs.tailscale}/bin/tailscale serve --bg --https=443 http://localhost:18789
        ${pkgs.tailscale}/bin/tailscale serve --bg --https=9443 https+insecure://localhost:9090
      '';
    };
  };

  # Firewall ports.
  #   18789 = OpenClaw gateway + Control UI (LAN debug; Tailscale Serve
  #           handles tailnet access via :443).
  #   9443  = Tailscale Serve listener for Cockpit. Not strictly required
  #           on this firewall — tailscale0 is in trustedInterfaces — but
  #           kept here to mirror the rpi5 convention and to make LAN
  #           access work if Tailscale ever stops serving for some reason.
  networking.firewall.allowedTCPPorts = [
    18789
    9443
  ];
}

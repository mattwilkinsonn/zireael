{
  lib,
  pkgs,
  openclawPackage,
  ...
}:

# mattfw user config — Framework Desktop dev box. shared/load-secrets.nix is
# imported at the flake level (not here) so `op inject` runs on every
# interactive shell — same model as Mac and mattpc-wsl, different from the
# Pis which intentionally skip it.
#
# `openclawPackage` arrives via home-manager.extraSpecialArgs (set in
# flake.nix). It's the pre-built gateway derivation from
# nix-openclaw's own pinned nixpkgs — see the input comment for why
# we pass it explicitly instead of overlaying.

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattfw\" --show-trace";

  # OpenClaw — declarative gateway via nix-openclaw home-manager module.
  # The module is imported in flake.nix; this block configures it.
  # Companion host-side plumbing (secret materialization, workspace
  # sync, Tailscale Serve) lives in nixos/mattfw/openclaw.nix.
  #
  # State paths (defaults from the module):
  #   ~/.openclaw/                  state root (config, sessions, logs)
  #   ~/.openclaw/openclaw.json     gateway config (rendered by Nix from
  #                                  programs.openclaw.config)
  #   ~/.openclaw/workspace/        agent workspace (git-synced upstream
  #                                  to mattwilkinsonn/openclaw-workspace
  #                                  every 10 minutes by the timer in
  #                                  nixos/mattfw/openclaw.nix)
  #   /tmp/openclaw/openclaw-gateway.log   gateway stdout/stderr
  #
  # Secrets are referenced by file path (`*.tokenFile`) so they never
  # land in the Nix store. The files are populated at boot by
  # `openclaw-env-refresh.service` from 1Password.
  #
  # OPENCLAW_NIX_MODE=1 is set by the module's systemd unit — it tells
  # the gateway to treat its config as read-only and not auto-mutate
  # the JSON file. That's what makes the declarative path work.
  programs.openclaw = {
    enable = true;

    # Pre-built gateway from nix-openclaw's pinned nixpkgs (passed in
    # via extraSpecialArgs in flake.nix). Avoids the cross-nixpkgs
    # pnpmDeps hash mismatch — see the flake input comment.
    package = openclawPackage;

    # CLIs the gateway can shell out to. These land on the gateway's
    # PATH ONLY — they're NOT exposed in mattw's login shell (the
    # user-side toolchain is configured separately in shared/dev.nix).
    # The gateway inherits everything an interactive shell would,
    # plus declarative pins for the few tools we want guaranteed.
    #
    # Set `programs.openclaw.exposePluginPackages = true` if you ever
    # want plugin CLIs on the login PATH too — keeping them isolated
    # avoids name collisions with mattw's own toolchain.
    runtimePackages = with pkgs; [
      # Core scripting / search — the agent's bread and butter.
      git
      jq
      ripgrep
      curl
      # GitHub workflows + issue/PR ops.
      gh
      # Headless browser for Playwright-driven plugins. Path is wired
      # below via PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH so Playwright
      # skips its own download into ~/.cache/ms-playwright.
      chromium
    ];

    # Extra env on the gateway wrapper. Two flavors:
    #
    # Plain env vars (PLAYWRIGHT_*): passed straight through.
    #
    # File-backed secrets (OPENCLAW_GATEWAY_TOKEN, DISCORD_BOT_TOKEN,
    # BRAVE_SEARCH_API_KEY, OPENROUTER_API_KEY): the wrapper detects
    # when a value is a path to an existing file and substitutes in
    # the file contents at gateway startup, so secrets never appear
    # on a systemd command line / unit's Environment= block. The
    # files come from /run/openclaw-secrets/, populated by
    # openclaw-env-refresh.service from 1Password.
    #
    # Use `*_FILE`-suffixed names if you want the path itself passed
    # through (e.g. for a tool that takes a token-file argument).
    environment = {
      PLAYWRIGHT_BROWSERS_PATH = "0";
      PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH = "${pkgs.chromium}/bin/chromium";
      PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";

      OPENCLAW_GATEWAY_TOKEN = "/run/openclaw-secrets/gateway-token";
      DISCORD_BOT_TOKEN = "/run/openclaw-secrets/discord-token";
      BRAVE_SEARCH_API_KEY = "/run/openclaw-secrets/brave-search";
      OPENROUTER_API_KEY = "/run/openclaw-secrets/openrouter";
    };

    # Schema-typed config rendered to ~/.openclaw/openclaw.json.
    # Anything not listed here keeps the gateway's built-in defaults.
    # Ported from the rpi5-era openclaw.json (Control UI wizard state)
    # to declarative form. Runtime-only fields (wizard.*, meta.*,
    # auth.profiles.*) are intentionally omitted — the gateway will
    # rematerialize them on first run.
    config = {
      gateway = {
        mode = "local";
        # Loopback bind: gateway only accepts connections from
        # 127.0.0.1/::1. Tailscale Serve fronts the tailnet hop at
        # https://mattfw.tail08a5c5.ts.net/, terminating on the host
        # and proxying to localhost:18789. Pairs with trustedProxies
        # below so the gateway honors X-Forwarded-* headers from the
        # local tailscaled.
        bind = "loopback";
        trustedProxies = [
          "127.0.0.1"
          "::1"
        ];

        # `gateway.auth.token` accepts an inline string OR a secret
        # reference. We omit it here entirely — the gateway falls
        # back to OPENCLAW_GATEWAY_TOKEN from the environment, which
        # the wrapper dereferences from /run/openclaw-secrets above.
        # Same shape the upstream README documents as the recommended
        # path ("set gateway.auth.token OR OPENCLAW_GATEWAY_TOKEN").

        # Control UI's origin allowlist. The Control UI rejects
        # cross-origin loads from anywhere not on this list. Tailscale
        # Serve maps :443 on the tailnet hostname → :18789 locally,
        # so the browser sees a request from the tailnet origin and
        # the gateway needs to know to trust it.
        controlUi = {
          allowedOrigins = [
            "https://mattfw.tail08a5c5.ts.net"
            # Loopback variants for SSH-tunneled debug
            # (`ssh -L 18789:localhost:18789 mattfw` → http://localhost:18789).
            "http://localhost:18789"
            "http://127.0.0.1:18789"
          ];
        };
      };

      # Agent runtime defaults — model routing + workspace. The
      # workspace path is auto-pinned by the home-manager module
      # (programs.openclaw.workspace.pinAgentDefaults defaults true)
      # to ~mattw/.openclaw/workspace, so we don't set it here.
      agents.defaults = {
        # Heartbeat off: this gateway doesn't ping its agents on a
        # timer. Set to e.g. "5m" if we ever want the agent loop
        # to wake periodically.
        heartbeat.every = "0m";

        # Model routing through OpenRouter. The first entry is tried,
        # falls back to the next on rate-limit / outage / etc.
        # `OPENROUTER_API_KEY` env var is set above; the openrouter
        # provider is built into the gateway (no plugin install).
        model = {
          primary = "openrouter/deepseek/deepseek-v4-pro";
          fallbacks = [
            "openrouter/qwen/qwen3.6-plus"
          ];
        };
      };

      # Default channel behavior. `allowlist` means the gateway only
      # responds in groups explicitly listed in
      # channels.<channel>.allowFrom — DMs from allowFrom users still
      # work either way. Per-channel `groups.<id>.requireMention`
      # overrides apply on top.
      channels.defaults = {
        groupPolicy = "allowlist";
      };

      # Discord — the only channel this gateway exposes. The
      # `channels` submodule has a free-form schema (per upstream
      # spec), so the discord block is passed through as-is.
      # The token is materialized from the DISCORD_BOT_TOKEN env var,
      # which the wrapper hydrates from /run/openclaw-secrets/discord-token.
      channels.discord = {
        # TODO: set this to your Discord user ID once known. Until
        # set, the bot accepts no DMs/mentions. Same shape as the
        # Telegram example in the upstream README — list of numeric
        # user IDs and/or guild IDs that may issue commands.
        # allowFrom = [ 0000000000000000 ];
        groups = {
          "*" = {
            requireMention = true;
          };
        };
      };

      # Built-in plugins. `openrouter` + `brave` are first-class to
      # the gateway (not the bundled npm plugins from the catalog).
      # Both are enabled by default if their API keys are set, but
      # listing them explicitly + pinning brave's webSearch config
      # makes the canonical config self-documenting and avoids any
      # "did the wizard run yet?" ambiguity.
      plugins.entries = {
        openrouter.enabled = true;
        brave = {
          enabled = true;
          config.webSearch = {
            # Literal $VAR reference; the gateway interpolates from
            # its env at startup. The wrapper hydrates
            # BRAVE_SEARCH_API_KEY from /run/openclaw-secrets above.
            apiKey = "$BRAVE_SEARCH_API_KEY";
            mode = "web";
          };
        };
      };

      # Web tooling — the search tool dispatches to the brave plugin
      # above.
      tools.web = {
        search = {
          provider = "brave";
          maxResults = 5;
          timeoutSeconds = 30;
        };
      };

      # Group-chat reply visibility — `message_tool` shows the agent's
      # reply plus the tool's structured output to everyone in the
      # group (default is more conservative). Carried forward from
      # the rpi5-era config.
      messages.groupChat.visibleReplies = "message_tool";
    };

    # Bundled plugins. `summarize` is the URL/PDF/YouTube summarizer
    # that fits the dev-assistant use case for this host. Add more
    # from the catalog in nix-openclaw/nix/modules/home-manager/
    # openclaw/plugin-catalog.nix as needed.
    bundledPlugins = {
      summarize.enable = true;
    };

    # systemd user service for the gateway. Default unit name is
    # `openclaw-gateway.service` — managed via:
    #   systemctl --user status openclaw-gateway
    #   journalctl --user -u openclaw-gateway -f
    #   systemctl --user restart openclaw-gateway
    systemd.enable = true;
  };

  # 1Password service-account tokens from 600-perm files. Loaded BEFORE
  # shared/home.nix's load-secrets auto-invoke (which uses `op inject`
  # against these tokens). No biometric, no consent dialog, no IPC to the
  # desktop app — token-based auth.
  #
  # Two tokens for two accounts:
  #
  #   ~/.config/op/service-account-token       → personal account (framework-svc).
  #     Scope: op://Dev/... + op://Server/... — also used by root-side
  #     systemd fetchers (openclaw-env-refresh) reading op://Server/...
  #   ~/.config/op/team-service-account-token  → sealedsecurity team (matt-dev-svc).
  #     Scope: op://Employee Dev/... (interactive shell only).
  #
  # Stored once per host via framework-bootstrap.sh (or replace manually:
  #   install -m 600 -D /dev/stdin ~/.config/op/service-account-token      <<< 'ops_...'
  #   install -m 600 -D /dev/stdin ~/.config/op/team-service-account-token <<< 'ops_...'
  # ). Token rotation: rewrite the relevant file. Same pattern as Mac's
  # Keychain-based load in darwin/home.nix and mattpc-wsl/home.nix.
  #
  # Exported via `envExtra` (writes to ~/.zshenv) instead of
  # `initContent` (writes to ~/.zshrc) so non-interactive shells —
  # systemd user services, cron, scripted invocations — see the
  # tokens too. .zshenv runs unconditionally on shell startup,
  # regardless of interactive/login state.
  #
  # Uses shell builtins (`read`, `[`, `export`) instead of `cat`
  # because systemd user services start with a near-empty PATH where
  # /run/current-system/sw/bin isn't reachable yet — `$(cat ...)`
  # silently returns empty in that environment and the token ends up
  # unset. Builtins are path-independent.
  programs.zsh.envExtra = ''
    if [ -r "$HOME/.config/op/service-account-token" ]; then
      IFS= read -r OP_SERVICE_ACCOUNT_TOKEN < "$HOME/.config/op/service-account-token"
      export OP_SERVICE_ACCOUNT_TOKEN
    fi
    if [ -r "$HOME/.config/op/team-service-account-token" ]; then
      IFS= read -r OP_TEAM_SERVICE_ACCOUNT_TOKEN < "$HOME/.config/op/team-service-account-token"
      export OP_TEAM_SERVICE_ACCOUNT_TOKEN
    fi
  '';
}

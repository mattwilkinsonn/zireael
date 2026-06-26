{
  lib,
  pkgs,
  inputs,
  ...
}:

# shared/dev.nix — Cross-platform "dev tier" tooling.
#
# Imported only by the boxes that are actually used for development
# (mattpc-wsl, mattfw, Mac). Headless / server hosts (rpi4, rpi5,
# mattserver) get the universal `shared/home.nix` set only — keeps
# Pi nix-switch time bounded and avoids cache-miss pulls like the
# pulumi-bin assembly that prompted this split.
#
# Linux-specific build deps (pkg-config, openssl.dev, dbus, mold,
# clang, gnumake + PKG_CONFIG_PATH) live in `shared/linux-build-deps.nix`
# — Mac uses brew + Xcode CLI Tools for the equivalent.
let
  # Agent CLIs all come from the numtide/llm-agents.nix flake. Resolve the
  # per-system package set once here
  # so adding a new agent CLI is a single line in `home.packages` below —
  # `agents.gemini-cli`, `agents.opencode`, etc. — instead of threading a
  # new `fooPackage` arg through flake.nix's per-host extraSpecialArgs and
  # this module's function signature. Browse available names with:
  #   nix eval github:numtide/llm-agents.nix#packages.${pkgs.system} \
  #     --apply builtins.attrNames
  agents = inputs.llm-agents.packages.${pkgs.system};
in
{
  home.packages =
    with pkgs;
    [
      # Language toolchains + package managers
      uv # Python package manager
      bun # JS runtime & package manager
      fnm # fast node manager
      # nodejs: a stable, shell-agnostic Node on PATH at
      # /etc/profiles/per-user/$USER/bin (and /run/current-system/sw/bin),
      # reachable by non-interactive SSH sessions that never source the
      # zsh rc where `fnm env` injects its per-session node. Remote-SSH
      # IDEs / ADEs (VSCode, Cursor, Orca) probe for `node` during their
      # server bootstrap before any interactive shell init runs, so the
      # fnm-managed node alone isn't enough for them. fnm still wins inside
      # interactive terminals (its `fnm env` prepends the multishell dir),
      # so this is purely the fallback the IDE servers resolve against.
      nodejs_24
      # python3: node-gyp (pulled in by any npm dep with a native addon —
      # better-sqlite3, etc.) shells out to `python3` + `make` + a C/C++
      # compiler at install time. `make`/`cc` already come from
      # linux-build-deps.nix's gnumake + clang, but the uv-managed Python
      # is only on PATH via the interactive zsh rc, so non-interactive
      # Remote-SSH/ADE installs hit "Could not find any Python". This puts
      # a plain python3 at /etc/profiles/per-user/$USER/bin for them. The
      # uv-managed python/python3 shims still win in interactive shells
      # (initContent prepends ~/.local/share/uv/python/bin), so this is
      # only the node-gyp fallback — it doesn't change `python3` for
      # day-to-day interactive use.
      python3
      rustup # toolchain manager (sets up rustc + cargo on demand)

      # Rust ecosystem helpers
      # `lib.hiPrio` so the standalone rust-analyzer wins over rustup's bundled
      # rust-analyzer shim (rustup also ships `bin/rust-analyzer` and they collide
      # in the same profile otherwise).
      (lib.hiPrio rust-analyzer)
      cargo-edit # cargo add/rm/upgrade
      cargo-nextest # faster test runner
      cargo-binstall # install prebuilt crate binaries when nixpkgs lacks them
      cargo-update # cargo install-update
      sccache # compiler cache (referenced by RUSTC_WRAPPER below)
      # zig — toolchain + drop-in cross C/C++ compiler (`zig cc`). Used
      # to cross-compile Rust crates' C/asm build scripts (ring, etc.)
      # for other targets without that platform's SDK — e.g. the
      # macOS-target clippy run on Linux (SEA-840). Kept cross-platform
      # so macOS dev boxes can cross-compile too if needed.
      zig
      starship-jj # jj VCS status for the starship prompt (via nixpkgs-unstable overlay)

      # Other dev tools
      biome # JS/TS linter & formatter
      protobuf # protocol buffers
      awscli2 # AWS CLI v2
      pulumi-bin # Pulumi IaC
      wasmtime # WASM runtime
      actionlint # GitHub Actions linter
      markdownlint-cli2 # markdown linter (CI gate for ~/notes/*.md and tracker docs)
      # Graphite CLI (`gt`) — stacked-PR queue layer on top of github.
      # Evaluated as a replacement for GitHub's merge queue (SEA-557:
      # GH's queue doesn't truly batch; Graphite does). Lives in
      # dev.nix not home.nix because Pis don't ship PRs. Unfree (no
      # license specified upstream); `nixpkgs.config.allowUnfree = true`
      # already set in nixos/common.nix. One-time auth per host:
      # `gt auth --token <token>` — token from 1Password (Dev vault),
      # not yet automated via op-cli because gt auth is a one-time
      # action and the token is durable.
      graphite-cli
      # Nix / shell / toml linters — invoked by the hk pre-push hook
      # (~/hk.pkl). Cheap to keep on dev boxes for one-off CLI use too.
      deadnix # unused let bindings / function args in Nix
      statix # anti-pattern lint for Nix (with-in-let, etc.)
      shellcheck # shell script linter (invoked by hk pre-push)
      shfmt # shell script formatter (read-only check mode in hk)
      taplo # TOML formatter (read-only check mode in hk)
      pkl # Apple Pkl CLI — hk's config language; required for `hk validate` etc.
      llvm # LLVM toolchain
      git-filter-repo # rewrite git history
      nixd # Nix language server
      # devenv — the reproducible dev-shell tool (nix underneath) that
      # seal's devenv.nix (SEA-881) and sealed's (SEA-884) build on.
      # The `use devenv` directive in those repos' .envrc shells out to
      # this binary; direnv (already on PATH) drives the entry. The
      # shell itself pins its own toolchains (proto for rust/bun/node +
      # the moon/zig/protoc tools, devenv for system libs), so this is
      # only the launcher — version-staleness here doesn't affect what
      # the repo shells provision.
      devenv

      # AI / LLM tooling. Agent CLIs are referenced via the `agents` binding
      # in the `let` above — add a new one by dropping a single
      # `agents.<name>` line here.
      agents.codex # OpenAI Codex CLI (numtide/llm-agents.nix)
      agents.coderabbit-cli # CodeRabbit CLI (numtide/llm-agents.nix)
      agents.gemini-cli # Google Gemini CLI (numtide/llm-agents.nix)
      agents.omp # Oh My Pi CLI (numtide/llm-agents.nix)
      # Emdash's ADE provider registry detects the `pi` binary (upstream
      # @earendil-works/pi); we run the oh-my-pi fork (`omp`). This shim
      # puts `pi` on PATH so Emdash launches omp. Basic launch + worktree
      # flow work; pi-specific Emdash hooks (Resume) may not, since omp has
      # diverged far from upstream pi — the clean fix is a first-class
      # `omp` provider upstream in Emdash (generalaction/emdash).
      (writeShellScriptBin "pi" ''exec ${agents.omp}/bin/omp "$@"'')

      # jj-ws — workspace helper for the multi-agent wave (creates/forgets
      # jj workspaces under <repo>.ws/). Pairs with the `wave` zellij layout.
      (writeShellScriptBin "jj-ws" (builtins.readFile ../dotfiles/scripts/jj-ws))

      # Misc dev-machine utilities
      rclone # Drive/Dropbox/etc remote sync (used by Berkeley Mono font activation)
      zstd # compression — occasional CLI use
      nmap # network scanner — occasional CLI use
      poppler-utils # PDF CLI tooling (pdftotext, pdfinfo, ...)

      # TeX (cross-platform). ~1GB; bump to scheme-full if you need every package.
      texlive.combined.scheme-medium
    ]
    ++ lib.optionals pkgs.stdenv.isLinux [
      # Filtered D-Bus session-bus proxy — seal's sandbox dispatcher
      # (SEA-780) launches it per spawn so keyring-using CLIs
      # (coderabbit, gh) get scoped Secret Service access inside the
      # bwrap namespace. PATH resolution until seal's bundled-deps
      # release embeds its own copy. Linux-only: bwrap (and thus the
      # dbus proxy) is part of seal's Linux sandbox path; xdg-dbus-proxy
      # has no Darwin build (meta.platforms is Linux-only in nixpkgs).
      xdg-dbus-proxy
    ];

  # Dev-tier shell additions. PATH entries for the toolchains we install
  # above, plus the RUSTC_WRAPPER export.
  programs.zsh.initContent = ''
    export PATH="$HOME/.local/share/uv/python/bin:$PATH"
    export PATH="$HOME/.opencode/bin:$PATH"
    export PATH="$HOME/.bun/bin:$PATH"
    # cargo-binstall installs here; rustup also adds it via ~/.cargo/env below
    export PATH="$HOME/.cargo/bin:$PATH"

    # Cargo/Rust
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    export RUSTC_WRAPPER=sccache

    # Buildkite CLI (`bk`): the org slug isn't a secret and bk reads
    # it from ~/.config/bk.yaml — but `bk configure` writes the slug
    # and the API token together in one keychain-backed operation that
    # aborts on headless hosts with no Secret Service, so bk.yaml never
    # gets created and the org has to come from the env instead. (See
    # SEA-829.) The seal bk bundle forwards this var into the sandbox.
    export BUILDKITE_ORGANIZATION_SLUG=sealedsecurity

    # fnm — initialise per-shell, NOT via _evalcache: `fnm env` mints a fresh
    # multishell symlink and exports its path on each call, so caching pins
    # every shell to the first one and `fnm use` / --use-on-cd then leak
    # across panes.
    command -v fnm >/dev/null && eval "$(fnm env --use-on-cd --shell zsh)"
  '';

  # rustup default toolchain: set stable as default on first install.
  # Subsequent runs are no-ops because `rustup default` (no args) returns 0
  # only when a default is already configured. Cross-platform.
  home.activation.rustupDefault = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    if ! ${pkgs.rustup}/bin/rustup default >/dev/null 2>&1; then
      echo "No rustup default toolchain set; installing stable..."
      run ${pkgs.rustup}/bin/rustup default stable
    fi
  '';

  # uv-managed Python: install latest stable and create python/python3 shims
  # under ~/.local/share/uv/python/bin (already on PATH via initContent above).
  # `--default` makes the unversioned `python`/`python3` commands point at it.
  # `--preview-features python-install-default` opts in to the still-experimental
  # default-install machinery silently rather than printing a warning each run.
  # Idempotent: uv no-ops when the latest version is already installed.
  # On macOS, uv invokes `install_name_tool` (Apple cctools) to patch dylib
  # install names so native extensions build cleanly — `darwin/home.nix`'s
  # augmentActivationPath puts /usr/bin + CLI Tools on PATH for that.
  home.activation.uvPythonDefault = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    run ${pkgs.uv}/bin/uv python install --preview-features python-install-default --default --force
  '';

  # Claude Code: install via the official bash installer if the binary isn't
  # already on disk. Cross-platform (Mac + Linux). Claude Code self-updates on
  # launch, so we only need the installer for the first-time install — checking
  # the install paths instead of re-running every rebuild.
  home.activation.installClaudeCode = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    if [ -x "$HOME/.local/bin/claude" ] || [ -x "$HOME/.claude/local/claude" ]; then
      echo "Claude Code already installed; skipping (it self-updates on launch)"
    else
      echo "Installing Claude Code..."
      export PATH="${pkgs.curl}/bin:$PATH"
      run ${pkgs.curl}/bin/curl -fsSL https://claude.ai/install.sh | ${pkgs.bash}/bin/bash
    fi
  '';

  # rtk init -g: registers the RTK Pre/Post tool-use hooks in
  # ~/.claude/settings.json. Cross-platform — rtk lives at different paths
  # depending on host (cargo-installed on Pi, brew on Mac). Skip if the
  # hooks already look registered (avoids re-writing settings.json on every
  # rebuild). Match `rtk` as a substring — the hook entry writes something
  # like `"command": "rtk-hook"`, which doesn't contain the literal `"rtk"`.
  home.activation.initRtk = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/home/linuxbrew/.linuxbrew/bin:$PATH"
    if command -v rtk >/dev/null 2>&1; then
      if ! grep -q rtk "$HOME/.claude/settings.json" 2>/dev/null; then
        echo "Initializing RTK for Claude Code..."
        run rtk init -g
      fi
    fi
  '';

  # Ensure fnm has the latest Node LTS installed and aliased as the
  # `default` version. Idempotent: `install --lts` is fast no-op when
  # already present, `default` just rewrites the alias. After this
  # runs, any shell or activation can do `fnm exec --using=default --
  # <cmd>` (or `fnm use default` interactively) without hardcoding a
  # Node version.
  home.activation.fnmDefaultLts = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    ${pkgs.fnm}/bin/fnm install --lts
    ${pkgs.fnm}/bin/fnm default lts-latest
  '';

  # Obsidian Headless (open beta) — npm package providing the `ob`
  # command for vault sync / publish from headless contexts (SSH,
  # cron, CI). Cross-platform; paired with the in-app `obsidian` CLI
  # which requires a running desktop app and isn't declarable
  # (Settings → General → "Command line interface" enable, per host).
  # First-time per host: `ob login` to populate stored credentials.
  #
  # Installed via Node's `npm`, not `bun install -g`: obsidian-headless
  # depends on better-sqlite3 which is a native node-gyp binding, and
  # bun's npm-compat layer doesn't yet support those (tracked at
  # oven-sh/bun#4290). Real npm uses the prebuild-install hook which
  # fetches platform-matched binaries (darwin-arm64, darwin-x64,
  # linux-x64, linux-arm64) — no toolchain required.
  #
  # Node comes from fnm (see `fnmDefaultLts` above). `fnm exec
  # --using=default` runs npm with the default Node active — no
  # `fnm use` (which mutates shell state) since exec is the right tool
  # for "run this command under this Node version, then exit."
  # `--prefix=$HOME/.local` lands `ob` at ~/.local/bin (on PATH from
  # shared/home.nix).
  home.activation.installObsidianHeadless =
    lib.hm.dag.entryAfter [ "writeBoundary" "fnmDefaultLts" ]
      ''
        echo "Installing/updating obsidian-headless..."
        mkdir -p "$HOME/.local"
        ${pkgs.fnm}/bin/fnm exec --using=default -- npm install --prefix="$HOME/.local" -g obsidian-headless
      '';

  # Greptile CLI — npm package providing the `greptile` command.
  # Cross-platform dev-tier install, using the same fnm-managed default
  # Node as obsidian-headless. `--prefix=$HOME/.local` puts the binary
  # in ~/.local/bin, already on PATH from shared/home.nix.
  home.activation.installGreptile = lib.hm.dag.entryAfter [ "writeBoundary" "fnmDefaultLts" ] ''
    echo "Installing/updating greptile..."
    mkdir -p "$HOME/.local"
    ${pkgs.fnm}/bin/fnm exec --using=default -- npm install --prefix="$HOME/.local" -g greptile
  '';

  # Obsidian Sync — continuous background sync of ~/notes to the
  # "Notes" remote vault, driven by 1Password creds.
  #
  # Linux-only (mattfw, mattpc-wsl). Mac is the primary editor and
  # the desktop Obsidian.app handles sync directly there; no benefit
  # to a parallel headless client on the same machine. If a use case
  # appears (Mac being closed overnight, sync needed in headless
  # session, etc.), wrap these in `lib.optionalAttrs pkgs.stdenv.isLinux`
  # → equivalent `launchd.agents.*` block.
  #
  # Two systemd user services:
  #
  #   obsidian-bootstrap (oneshot, idempotent): on first run per host,
  #   logs in via `ob login` and registers ~/notes against the remote
  #   "Notes" vault via `ob sync-setup`. Uses 1Password CLI to pull
  #   credentials (username/password/TOTP) — service-account token
  #   from the host's mkBefore zshrc block is reused via the user
  #   `op` config. Subsequent runs detect existing auth + vault
  #   binding and no-op.
  #
  #   obsidian-sync (long-running): runs `ob sync --continuous` once
  #   bootstrap is happy. Restart on failure with backoff. Stops at
  #   user logout.
  #
  # The 1Password item "Obsidian" lives in the Dev vault with fields:
  #   username (string)        → Obsidian email
  #   password (concealed)     → Obsidian password
  #   one-time password (OTP)  → 2FA TOTP seed
  #
  # No `--password` for sync-setup because the Notes vault is using
  # standard sync, not E2EE. If it ever moves to E2EE, add a field
  # and a corresponding --password flag here.
  systemd.user.services.obsidian-bootstrap = lib.mkIf pkgs.stdenv.isLinux {
    Unit = {
      Description = "Bootstrap Obsidian headless: login + vault setup";
      # Wait for network so the API call works on cold-boot.
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      Type = "oneshot";
      RemainAfterExit = true;
      # Inherit the user's full PATH (including ~/.local/bin and
      # ~/.bun/bin where `ob` lives, plus the OP_SERVICE_ACCOUNT_TOKEN
      # exported by the host's mkBefore zshrc block). The wrapper
      # script sources zshrc explicitly because user systemd services
      # otherwise inherit only a minimal env.
      ExecStart = "%h/.local/state/obsidian-sync/bootstrap.sh";
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.obsidian-sync = lib.mkIf pkgs.stdenv.isLinux {
    Unit = {
      Description = "Obsidian Sync — continuous";
      After = [ "obsidian-bootstrap.service" ];
      Requires = [ "obsidian-bootstrap.service" ];
    };
    Service = {
      ExecStart = "%h/.local/state/obsidian-sync/sync.sh";
      Restart = "on-failure";
      RestartSec = 30;
      # Stop the sync cleanly on logout / shutdown so half-written
      # vault state isn't left behind. SIGINT triggers ob sync's
      # graceful-stop path.
      KillSignal = "SIGINT";
      TimeoutStopSec = 30;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # The two wrapper scripts. Kept as files (not inline shell strings
  # in the unit) so they can source ~/.zshenv to pick up the
  # OP_SERVICE_ACCOUNT_TOKEN export and the PATH additions for `ob`.
  # Systemd user services bypass interactive shell init by default.
  # Only materialized on Linux (where the services that consume them
  # run) — keeps Mac home-manager activation noise-free.
  home.file.".local/state/obsidian-sync/bootstrap.sh" = lib.mkIf pkgs.stdenv.isLinux {
    executable = true;
    text = ''
      #!${pkgs.bash}/bin/bash
      set -euo pipefail

      # Source the same env our shells get: PATH (incl. ~/.local/bin
      # and ~/.bun/bin for `ob`, ~/.nix-profile/bin for op),
      # OP_SERVICE_ACCOUNT_TOKEN from keychain/file.
      # shellcheck disable=SC1090
      [ -f "$HOME/.zshenv" ] && source "$HOME/.zshenv"

      # Make `ob` (npm script with `#!/usr/bin/env node`), `op`, and
      # node itself reachable. fnm normally injects its
      # per-session multishell dir into PATH at shell start, but
      # those dirs are torn down on logout — the systemd user
      # instance survives logouts, so we point at the stable
      # alias path `default` instead.
      FNM_NODE_BIN="$HOME/.local/share/fnm/aliases/default/bin"
      export PATH="$HOME/.local/bin:$HOME/.bun/bin:$FNM_NODE_BIN:$HOME/.nix-profile/bin:/run/current-system/sw/bin:$PATH"

      log() { echo "[obsidian-bootstrap] $*"; }

      if ! command -v ob >/dev/null; then
        log "ERROR: ob not on PATH — is obsidian-headless installed?"
        exit 1
      fi
      if ! command -v op >/dev/null; then
        log "ERROR: op (1Password CLI) not on PATH"
        exit 1
      fi
      if [ -z "''${OP_SERVICE_ACCOUNT_TOKEN:-}" ]; then
        log "ERROR: OP_SERVICE_ACCOUNT_TOKEN unset — check the host's mkBefore zshrc"
        exit 1
      fi

      # ob login (no args) prints "Logged in as ..." when authed or
      # exits non-zero otherwise. We use stdout match instead of exit
      # code because exit semantics vary by version.
      if ob login 2>/dev/null | grep -q "Logged in as"; then
        log "Already logged in; skipping login."
      else
        log "Logging in via 1Password creds..."
        OB_EMAIL=$(op item get 'Obsidian' --vault Dev --fields username --reveal)
        OB_PASS=$(op item get 'Obsidian' --vault Dev --fields password --reveal)
        OB_OTP=$(op item get 'Obsidian' --vault Dev --otp)
        ob login --email "$OB_EMAIL" --password "$OB_PASS" --mfa "$OB_OTP"
        log "Login complete."
      fi

      # sync-list-local prints either "Configured vaults:" followed
      # by entries, or "No vaults configured." We match the resolved
      # path against the local listing.
      RESOLVED_NOTES_PATH="$HOME/notes"
      if ob sync-list-local 2>/dev/null | grep -q "Path: $RESOLVED_NOTES_PATH"; then
        log "Notes vault already set up at $RESOLVED_NOTES_PATH; skipping sync-setup."
      else
        log "Setting up sync for $RESOLVED_NOTES_PATH against remote vault 'Notes'..."
        ob sync-setup --vault Notes --path "$RESOLVED_NOTES_PATH" \
          --device-name "$(hostname)"
        log "Sync setup complete."
      fi

      log "Bootstrap done."
    '';
  };

  home.file.".local/state/obsidian-sync/sync.sh" = lib.mkIf pkgs.stdenv.isLinux {
    executable = true;
    text = ''
      #!${pkgs.bash}/bin/bash
      set -euo pipefail
      # shellcheck disable=SC1090
      [ -f "$HOME/.zshenv" ] && source "$HOME/.zshenv"
      # Same PATH dance as obsidian-bootstrap.sh — see comments
      # there for why the fnm `default` alias path is needed.
      FNM_NODE_BIN="$HOME/.local/share/fnm/aliases/default/bin"
      export PATH="$HOME/.local/bin:$HOME/.bun/bin:$FNM_NODE_BIN:$HOME/.nix-profile/bin:/run/current-system/sw/bin:$PATH"

      if ! command -v ob >/dev/null; then
        echo "[obsidian-sync] ERROR: ob not on PATH" >&2
        exit 1
      fi

      exec ob sync --continuous --path "$HOME/notes"
    '';
  };

  # ~/.bunfig.toml: global supply-chain timing defense for `bun install`,
  # `bun add`, `bun update`. Refuses to install a package version
  # published less than 5 days ago. Buys time for the registry to detect
  # and yank a maliciously-published version before it lands in any
  # repo we touch. Cross-platform: bun reads $HOME/.bunfig.toml on both
  # Mac and Linux (verified against bun.com/docs/runtime/bunfig).
  # Per-repo bunfig.toml files at project roots override / supplement
  # this default (shallow-merged with local winning).
  home.file.".bunfig.toml".text = ''
    [install]
    minimumReleaseAge = 7200
  '';

  # hk (https://hk.jdx.dev): not in nixpkgs. cargo-binstall is cross-platform
  # (prebuilt aarch64-darwin + x86_64-linux binaries).
  home.activation.installHk = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    echo "Installing/updating hk..."
    run ${pkgs.cargo-binstall}/bin/cargo-binstall --no-confirm hk
  '';

  # Graphite CLI auth — `gt auth --token <token>` writes
  # ~/.config/graphite/user_config and that's all the CLI checks
  # on subsequent invocations. (gt 1.5+ migrated from the legacy
  # ~/.graphite_user_config path to the XDG-spec location;
  # checking the new path is the correct existence probe.) The
  # activation is idempotent: skip if the config file exists,
  # otherwise read the token from 1Password (Local Dev vault —
  # Graphite operates on the sealedsecurity/seal repo, so
  # Team-scoped) and run `gt auth`.  See SEA-557 for the
  # migration design.
  #
  # The team SA token is read from disk
  # (`~/.config/op/team-service-account-token`) rather than from
  # OP_TEAM_SERVICE_ACCOUNT_TOKEN in env. home-manager
  # activations run with a stripped environment — the user-shell
  # env vars exported by `envExtra` / `initContent` aren't
  # inherited. Same pattern the obsidian-bootstrap wrapper script
  # uses to read OP_SERVICE_ACCOUNT_TOKEN from disk on Linux
  # hosts. Mac stores the token in Keychain instead, so the
  # `team-token` lookup below is a no-op there and gt auth needs
  # to be run manually once per Mac host (or wire it through a
  # darwin-specific `security find-generic-password` branch — out
  # of scope for now since Linux is the primary dev target).
  home.activation.graphiteAuth = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    if [ -f "$HOME/.config/graphite/user_config" ]; then
      echo "Graphite CLI already authed (~/.config/graphite/user_config present); skipping."
    elif [ ! -r "$HOME/.config/op/team-service-account-token" ]; then
      echo "Team SA token file missing at ~/.config/op/team-service-account-token; skipping graphite auth."
      echo "  (Mac hosts store it in Keychain — gt auth runs manually there for now.)"
    else
      echo "Authenticating Graphite CLI from 1Password..."
      # Read team SA token from disk. Builtins (read, [, export)
      # instead of $(cat …) because activation runs without a
      # full PATH — same reasoning as nixos/mattpc-wsl/home.nix
      # at the envExtra block.
      IFS= read -r team_token < "$HOME/.config/op/team-service-account-token"
      # Swap to the team SA for the op call, per
      # shared/load-secrets.nix's pattern (op CLI has no native
      # multi-account-svc-token mode; we swap in/out per read).
      token=$(OP_SERVICE_ACCOUNT_TOKEN="$team_token" \
              ${pkgs._1password-cli}/bin/op read "op://Local Dev/Graphite API Token/credential" 2>/dev/null || true)
      if [ -z "$token" ]; then
        echo "WARN: failed to read Graphite token (op://Local Dev/Graphite API Token/credential)."
        echo "      Either the item is missing, or the team SA can't access it."
        echo "      Run 'gt auth --token <token>' manually once the token is in 1Password."
      else
        # gt auth shells out to `git` (and `node` runs the CLI
        # itself via the wrapper). Activation context has a
        # stripped PATH — without these explicit nix-store
        # entries, gt fails with `spawn git ENOENT` and the
        # whole home-manager-mattw.service unit exits 1. Same
        # workaround installAkiflowCli uses below for gnutar/xz.
        # `|| true` so a transient gt failure (e.g. graphite-cli
        # upgrade temporarily moved a code path) doesn't brick
        # the whole nix-switch — the auth still gets logged and
        # we can retry, but the user's environment doesn't end
        # up half-applied.
        export PATH="${pkgs.git}/bin:${pkgs.graphite-cli}/bin:$PATH"
        gt auth --token "$token" || \
          echo "WARN: gt auth failed; ~/.config/graphite/user_config not written. Retry via 'gt auth --token <…>'."
      fi
    fi
  '';

  # gh-pr-review (https://github.com/agynio/gh-pr-review) — gh CLI
  # extension for inline PR review threads. Not in nixpkgs; install
  # via `gh extension install` and `gh extension upgrade` on
  # subsequent rebuilds. Lets agents pull review comments + thread IDs
  # in one shot instead of stitching together multiple `gh api`
  # GraphQL calls (which always trip the seal permission prompt
  # because raw GQL can hide mutations behind a generic verb).
  #
  # Activation PATH must include gh itself, plus git (gh extension
  # install clones the repo to fetch the precompiled binary metadata).
  # The home-manager activation env is stripped — same pattern the
  # graphiteAuth block above uses.
  home.activation.installGhPrReview = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    EXT_DIR="$HOME/.local/share/gh/extensions/gh-pr-review"
    export PATH="${pkgs.gh}/bin:${pkgs.git}/bin:$PATH"
    if [ -d "$EXT_DIR" ]; then
      echo "Updating gh-pr-review extension..."
      run ${pkgs.gh}/bin/gh extension upgrade gh-pr-review || \
        echo "WARN: gh extension upgrade failed (gh not authed?); skipping."
    else
      echo "Installing gh-pr-review extension..."
      run ${pkgs.gh}/bin/gh extension install https://github.com/agynio/gh-pr-review || \
        echo "WARN: gh extension install failed (gh not authed? run 'gh auth login' once)."
    fi
  '';

  # jj-hooks (jj-hp): not yet in nixpkgs. Drives the pre-push hook
  # gate against the hk config at ~/hk.pkl (see flake.nix's `jj push`
  # alias + ~/hk.pkl for the hook definitions). Drop this activation
  # once jj-hooks lands in nixpkgs and add it to home.packages above.
  home.activation.installJjHooks = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    echo "Installing/updating jj-hooks..."
    run ${pkgs.cargo-binstall}/bin/cargo-binstall --no-confirm jj-hooks
  '';

  # jj-gt: not yet in nixpkgs. Cargo-installed alongside jj-hp.
  home.activation.installJjGt = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    # echo "Installing/updating jj-gt..."
    run ${pkgs.cargo-binstall}/bin/cargo-binstall --no-confirm jj-gt
  '';

  # @schpet/linear-cli: Linear CLI from the same author as the Mac brew tap.
  # Installed via bun so we skip the fnm + node bootstrap. The package's
  # postinstall downloads a .tar.xz and shells out to `tar`, so tar + xz must
  # be on PATH for the activation context. The resulting `linear` binary lands
  # in $HOME/.bun/bin (on PATH from shared/dev.nix initContent).
  home.activation.installLinearCli = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    echo "Installing/updating @schpet/linear-cli..."
    export PATH="${pkgs.gnutar}/bin:${pkgs.xz}/bin:$PATH"
    ${pkgs.bun}/bin/bun install -g @schpet/linear-cli
  '';

  # akiflow-cli — our fork at
  # https://github.com/mattwilkinsonn/akiflow-cli (diverged 2026-05 from
  # code-yeongyu/akiflow-cli). Major additions: local sync cache, rich
  # `ls`/`cal` filters, cleaned `--json` output, `af doctor` + `af
  # refresh`. Build-from-source only (no npm publish, no brew formula);
  # clone + bun install + bun run build, then drop the standalone `af`
  # binary in ~/.local/bin (already on PATH from shared/dev.nix
  # initContent).
  #
  # First run on Mac: produce credentials via `af auth` (browser token
  # extraction), then `op document create` into the Server vault. The
  # sealed mattfw module owns the headless refresh path from 1Password.
  home.activation.installAkiflowCli = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    AF_REPO="https://github.com/mattwilkinsonn/akiflow-cli.git"
    AF_SRC="$HOME/.local/src/akiflow-cli"
    AF_BIN="$HOME/.local/bin/af"
    mkdir -p "$HOME/.local/src" "$HOME/.local/bin"

    # If the checkout exists but its origin URL doesn't match the
    # expected fork URL, blow it away and re-clone. A shallow clone
    # whose remote gets swapped to a repo with diverged history can
    # leave `git fetch` in a state where HEAD never advances — cheaper
    # to start fresh than to repair it. Covers any prior `code-yeongyu`
    # checkout, any partially-migrated state, and any manual remote
    # rewrites.
    if [ -d "$AF_SRC/.git" ]; then
      CURRENT_URL=$(${pkgs.git}/bin/git -C "$AF_SRC" remote get-url origin 2>/dev/null || echo "")
      if [ "$CURRENT_URL" != "$AF_REPO" ]; then
        echo "akiflow-cli: remote was '$CURRENT_URL', expected '$AF_REPO' — re-cloning"
        rm -rf "$AF_SRC"
      fi
    fi

    if [ ! -d "$AF_SRC/.git" ]; then
      echo "Cloning akiflow-cli..."
      # Full clone (not --depth=1): repo is small and a non-shallow
      # history makes future fetch+reset cycles trivially correct.
      ${pkgs.git}/bin/git clone "$AF_REPO" "$AF_SRC"
    else
      echo "Updating akiflow-cli..."
      ${pkgs.git}/bin/git -C "$AF_SRC" fetch origin main
      ${pkgs.git}/bin/git -C "$AF_SRC" reset --hard origin/main
    fi

    # Compare upstream HEAD to last-built marker; skip the build if
    # nothing changed. ~30s build on first run, near-instant otherwise.
    HEAD_SHA=$(${pkgs.git}/bin/git -C "$AF_SRC" rev-parse HEAD)
    if [ -x "$AF_BIN" ] && [ -f "$AF_SRC/.last-built-sha" ] \
       && [ "$(cat "$AF_SRC/.last-built-sha")" = "$HEAD_SHA" ]; then
      echo "akiflow-cli up to date ($HEAD_SHA)"
    else
      echo "Building af binary..."
      (cd "$AF_SRC" && ${pkgs.bun}/bin/bun install && ${pkgs.bun}/bin/bun run build)
      install -m 0755 "$AF_SRC/af" "$AF_BIN"
      echo "$HEAD_SHA" > "$AF_SRC/.last-built-sha"
    fi
  '';

  # Berkeley Mono fonts: paid, can't ship via nixpkgs. Synced down from
  # Google Drive via rclone on each activation. First-time setup per machine:
  #   rclone config         # set up a "gdrive" remote (interactive OAuth)
  # Skips silently if the gdrive remote isn't configured.
  # Source path on Drive: "System Configurations/Fonts/Berkeley Mono/OTF"
  # (TTF variant exists too — adjust the source if you'd rather sync TTF.)
  #
  # Platform branch on install dir:
  #   - macOS: ~/Library/Fonts (CoreText scans it automatically; the system
  #     fontd registers new files within seconds, no cache rebuild needed).
  #   - Linux: ~/.local/share/fonts (fontconfig convention); fc-cache makes
  #     them visible to fontconfig-aware apps (GTK, Qt, Ghostty, Foot, ...).
  home.activation.syncBerkeleyMono = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    if ${pkgs.rclone}/bin/rclone listremotes 2>/dev/null | grep -q "^gdrive:"; then
      echo "Syncing Berkeley Mono fonts from Google Drive..."
      FONT_DIR="${
        if pkgs.stdenv.isDarwin then
          "$HOME/Library/Fonts/BerkeleyMono"
        else
          "$HOME/.local/share/fonts/BerkeleyMono"
      }"
      mkdir -p "$FONT_DIR"
      ${pkgs.rclone}/bin/rclone copy --update \
        "gdrive:System Configurations/Fonts/Berkeley Mono/OTF" \
        "$FONT_DIR" 2>/dev/null || true
      ${
        if pkgs.stdenv.isLinux then
          ''
            if command -v fc-cache >/dev/null 2>&1; then
              fc-cache -f $HOME/.local/share/fonts 2>/dev/null || true
            fi
          ''
        else
          ""
      }
    else
      echo "rclone 'gdrive' remote not configured — run 'rclone config' to enable Berkeley Mono auto-sync."
    fi
  '';
}

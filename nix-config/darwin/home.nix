{
  lib,
  ...
}:

{
  imports = [
    ../shared/home.nix
    ../shared/dev.nix
    ../shared/load-secrets.nix
    ../shared/privatefiles-symlinks.nix
    ../shared/agent-config.nix
  ];

  home.username = lib.mkForce "mattwilkinson";
  home.homeDirectory = lib.mkForce "/Users/mattwilkinson";

  programs.zsh = {
    shellAliases = {
      nix-switch = "sudo HOME=\"$HOME\" /nix/var/nix/profiles/system/sw/bin/darwin-rebuild switch --flake \"$HOME/repos/zireael/nix-config#Matts-MacBook-Pro\" --show-trace";
    };

    profileExtra = lib.mkMerge [
      (lib.mkBefore ''
        # op (1Password CLI) lives in /opt/homebrew/bin, which nix-darwin's base
        # PATH (set-environment) omits and `brew shellenv` only adds in .zshrc —
        # never sourced by the login *non-interactive* `zsh -lc` that Emdash
        # spawns for agents. Prepend it here so `op` is on PATH for the
        # load-secrets block (shared/load-secrets.nix, also profileExtra) in
        # every login shell. Interactive shells still run full `brew shellenv`
        # in .zshrc; the duplicate /opt/homebrew/bin entry is harmless.
        export PATH="/opt/homebrew/bin:$PATH"

        # 1Password service account tokens from Keychain. In .zprofile (login
        # shells) rather than .zshrc so agents get them too — `security` is
        # always on PATH (/usr/bin). Read BEFORE load-secrets' `op inject` so it
        # uses the token directly: no signin, biometric, consent, or TCC prompt.
        #   OP_SERVICE_ACCOUNT_TOKEN      → personal account (macbook-svc).
        #   OP_TEAM_SERVICE_ACCOUNT_TOKEN → sealedsecurity team (matt-dev-svc).
        # Stored once per Mac via mac-setup.sh (security add-generic-password);
        # rotate by re-running with the new token. 2>/dev/null + true => harmless
        # if an entry is missing (load-secrets then warns per-account).
        export OP_SERVICE_ACCOUNT_TOKEN=$(security find-generic-password -a "$USER" -s "OP_SERVICE_ACCOUNT_TOKEN" -w 2>/dev/null || true)
        export OP_TEAM_SERVICE_ACCOUNT_TOKEN=$(security find-generic-password -a "$USER" -s "OP_TEAM_SERVICE_ACCOUNT_TOKEN" -w 2>/dev/null || true)
      '')
      (lib.mkAfter ''
        # Swiftly (Swift toolchain manager)
        . "$HOME/.swiftly/env.sh"
      '')
    ];

    initContent = lib.mkBefore ''
      # macOS PATH via Homebrew — cached (see _evalcache in .zshenv, which runs
      # before .zshrc so the helper is defined here). Absolute brew path because
      # brew isn't on PATH until shellenv runs; `brew shellenv` is otherwise a
      # per-shell subprocess.
      _evalcache brew /opt/homebrew/bin/brew /opt/homebrew/bin/brew shellenv

      # 1Password SSH agent socket. macOS defaults SSH_AUTH_SOCK to
      # Apple's launchd-managed agent (`/var/run/com.apple.launchd.*/Listeners`)
      # which is empty by default. OpenSSH itself ignores this and reads
      # the `IdentityAgent` directive from ~/.ssh/config to find the 1P
      # socket — so `ssh` works fine — but Go-based SSH clients (podman,
      # docker, gh, sometimes git) don't read ~/.ssh/config and only
      # honor SSH_AUTH_SOCK. Without this override they hit Apple's
      # empty agent and fail with "attempted methods [none]". Pointing
      # SSH_AUTH_SOCK at the 1P agent makes every ssh-using tool see the
      # same keys without needing tool-specific config.
      export SSH_AUTH_SOCK="$HOME/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
    '';
  };
  # `load-secrets` (op-backed, cross-platform) lives in shared/load-secrets.nix
  # as `profileExtra` too, ordered right after the token export above (mkBefore).
  # It runs `op inject` against OP_SERVICE_ACCOUNT_TOKEN — token-based auth, no
  # desktop integration, no prompts.

  # Make Apple cctools + standard system paths available as a *fallback*
  # for home-manager activations on macOS. Nix activations otherwise inherit
  # a minimal PATH (just the home-manager bootstrap utils), which means tools
  # like `install_name_tool` (cctools — used by uv to patch dylib install
  # names) aren't reachable. Activations run sequentially in the same shell,
  # so this export carries to every entryAfter "writeBoundary" activation
  # below.
  #
  # APPENDED, not prepended: macOS /usr/bin ships BSD `readlink`, `find`,
  # `sed` etc. that don't support GNU flags home-manager scripts use
  # (`readlink -e`, ...). Putting /usr/bin *after* $PATH means nix-store
  # coreutils (GNU) win for everything they provide; /usr/bin only resolves
  # tools genuinely missing from the nix env (install_name_tool, codesign,
  # xcrun, etc.). Per-activation PATH augmentation for nix-store-specific
  # tools (cargo, curl, bun, fnm, ...) stays. Linux hosts don't need this.
  home.activation.augmentActivationPath = lib.hm.dag.entryBefore [ "writeBoundary" ] ''
    export PATH="$PATH:/usr/bin:/Library/Developer/CommandLineTools/usr/bin"
  '';

  # Symlink .app bundles from brew *formulae* (not casks) into
  # ~/Applications for Spotlight discovery. Spotlight indexes
  # /Applications and ~/Applications by default but ignores
  # /opt/homebrew/, where formula-installed GUI apps live. Casks
  # already drop their .app into /Applications so they don't need
  # this; formulae like qalculate-qt do. Idempotent — `ln -sfn`
  # replaces any existing link with the current target on each run.
  #
  # `find -L` follows symlinks: each `/opt/homebrew/opt/<formula>` is
  # a symlink into the versioned `/opt/homebrew/Cellar/<formula>/<ver>/`
  # directory where the actual .app bundle lives. Without -L, find
  # stops at the symlink and never descends.
  #
  # `lsregister -f <app>` after each symlink is what actually makes
  # Spotlight find the app — symlinks in ~/Applications get indexed
  # by mdworker, but the LaunchServices DB has to be told "this is an
  # app you can launch" or Spotlight skips it from launch results.
  #
  # Symlink names can be remapped via the case statement to give nicer
  # Spotlight matches than the upstream formula name (e.g.
  # qalculate-qt → Qalculate). The .app's CFBundleName isn't touched,
  # so the icon/display label inside the app stays whatever the
  # upstream sets; only what you type in Spotlight changes.
  home.activation.linkBrewFormulaApps = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    APPDIR="$HOME/Applications/Homebrew Apps"
    mkdir -p "$APPDIR"

    # Clean up symlinks from prior layouts (top-level ~/Applications/,
    # pre-rename qalculate-qt name) so we don't leave stale duplicates
    # after migrating to the Homebrew Apps subfolder.
    rm -f "$HOME/Applications/qalculate-qt.app" \
          "$HOME/Applications/Qalculate.app"

    LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

    # Rename CFBundleName/CFBundleDisplayName/CFBundleExecutable for apps
    # where the upstream name (qalculate-qt) is awkward in the Dock/menu
    # bar. Patches the actual bundle's Info.plist via PlistBuddy and
    # renames the executable inside the bundle — brew upgrade reverts
    # both, but each nix-switch re-applies idempotently. PlistBuddy
    # accepts both binary and XML plists.
    #
    # Why rename the executable too: Qt apps (and some macOS lookups)
    # take the running process name from the executable filename inside
    # the bundle, NOT CFBundleName. Setting just the plist names makes
    # the menu bar correct but leaves the Dock label showing the
    # executable basename. Renaming Contents/MacOS/<old> → <new> and
    # updating CFBundleExecutable pulls every surface (Dock tooltip,
    # right-click menu, NSRunningApplication.localizedName) into line
    # with the bundle name.
    set_bundle_name() {
      local app="$1"
      local newname="$2"
      local plist="$app/Contents/Info.plist"
      [ -f "$plist" ] || return 0

      # Both keys: Set the existing entry; if not present, Add it. Some
      # bundles ship only one of the two (e.g. qalculate-qt has only
      # CFBundleDisplayName), so the Set || Add chain handles either
      # shape. PlistBuddy's `Set` errors out on missing keys, hence the
      # explicit fallback.
      /usr/libexec/PlistBuddy -c "Set :CFBundleName $newname" "$plist" 2>/dev/null \
        || /usr/libexec/PlistBuddy -c "Add :CFBundleName string $newname" "$plist" 2>/dev/null \
        || true
      /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName $newname" "$plist" 2>/dev/null \
        || /usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string $newname" "$plist" 2>/dev/null \
        || true

      # Rename the executable inside the bundle if it doesn't already
      # match. Idempotent: if current_exec already equals newname (we've
      # run this before), the mv is skipped.
      local current_exec
      current_exec=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$plist" 2>/dev/null || true)
      if [ -n "$current_exec" ] && [ "$current_exec" != "$newname" ] \
         && [ -f "$app/Contents/MacOS/$current_exec" ]; then
        mv "$app/Contents/MacOS/$current_exec" "$app/Contents/MacOS/$newname"
        /usr/libexec/PlistBuddy -c "Set :CFBundleExecutable $newname" "$plist" 2>/dev/null || true
      fi
    }

    find -L /opt/homebrew/opt -maxdepth 3 -name '*.app' -type d 2>/dev/null | while read -r app; do
      name=$(basename "$app")
      case "$name" in
        qalculate-qt.app)
          name="Qalculate.app"
          set_bundle_name "$app" "Qalculate"
          ;;
      esac
      ln -sfn "$app" "$APPDIR/$name"
      "$LSREGISTER" -f "$APPDIR/$name" 2>/dev/null || true
    done
  '';

  # ─── Files folded in from the old dotfiles repo (Mac-only) ───────────
  #
  # Migrated 2026-05 from the colocated git+jj at $HOME. yabai +
  # skhd are macOS window-manager / hotkey daemons; the 1Password
  # agent.toml gates which vault's SSH keys the agent offers.

  # yabai window manager — `yabairc` is the entry point sourced by
  # the launchd-managed yabai daemon. The other scripts under
  # ~/.config/yabai/ are referenced by yabairc via $HOME/.config/yabai/…
  # paths, so they need the same destination layout home.file gives us.
  home.file.".config/yabai/yabairc" = {
    source = ../dotfiles/yabai/yabairc;
    executable = true;
  };
  home.file.".config/yabai/aw-layout.sh" = {
    source = ../dotfiles/yabai/aw-layout.sh;
    executable = true;
  };
  home.file.".config/yabai/columns.sh" = {
    source = ../dotfiles/yabai/columns.sh;
    executable = true;
  };
  home.file.".config/yabai/cycle-display.sh" = {
    source = ../dotfiles/yabai/cycle-display.sh;
    executable = true;
  };
  home.file.".config/yabai/display-event.sh" = {
    source = ../dotfiles/yabai/display-event.sh;
    executable = true;
  };
  home.file.".config/yabai/display-setup.sh" = {
    source = ../dotfiles/yabai/display-setup.sh;
    executable = true;
  };
  home.file.".config/yabai/g9-layout.sh" = {
    source = ../dotfiles/yabai/g9-layout.sh;
    executable = true;
  };
  home.file.".config/yabai/move-to-display.sh" = {
    source = ../dotfiles/yabai/move-to-display.sh;
    executable = true;
  };
  home.file.".config/yabai/reset-splits.sh" = {
    source = ../dotfiles/yabai/reset-splits.sh;
    executable = true;
  };
  home.file.".config/yabai/rules.sh" = {
    source = ../dotfiles/yabai/rules.sh;
    executable = true;
  };

  # skhd hotkey daemon — the skhdrc dispatches cmd+alt-* shortcuts to
  # yabai commands. Not executable (parsed by skhd, not run directly).
  home.file.".config/skhd/skhdrc".source = ../dotfiles/skhd/skhdrc;

  # 1Password SSH agent config — controls which vaults' keys the
  # agent offers to ssh clients. Personal vault only on this box.
  home.file.".config/1Password/ssh/agent.toml".source = ../dotfiles/onepassword/ssh/agent.toml;

  # Zed editor config — macOS GUI app installed via Homebrew cask in
  # system.nix. Keep runtime state under ~/.config/zed/prompts unmanaged.
  xdg.configFile."zed/settings.json".source = ../dotfiles/zed/settings.json;
}

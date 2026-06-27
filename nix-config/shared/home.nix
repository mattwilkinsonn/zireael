{ pkgs, lib, ... }:

{
  home.stateVersion = "25.11";

  programs.home-manager.enable = true;

  # Force a dark terminal background for OMP's theme detection. Under Zellij
  # on macOS OMP can't read the real background (OSC 11 passthrough is broken
  # there) and falls back to the macOS *system* appearance — often Light — so
  # it picks a light theme on a dark terminal. COLORFGBG is OMP's Tier-2
  # source, checked before that fallback (bg=0 <8 => dark). Set via zsh
  # envExtra (~/.zshenv, sourced unconditionally by every zsh) rather than
  # home.sessionVariables, whose hm-session-vars.sh is login-gated and guarded
  # by __HM_SESS_VARS_SOURCED — fresh shells / Zellij panes inherit a stale
  # guard and never pick the var up.
  programs.zsh.envExtra = ''
    export COLORFGBG="15;0"
  '';

  # Packages — universal set, present on every host (including headless
  # Pis + mattserver). Dev-tier tooling (toolchains, IaC, language
  # linters, heavy build deps) lives in shared/dev.nix and is imported
  # only by the boxes that actually need it (Mac, mattfw, mattpc-wsl).
  home.packages = with pkgs; [
    # Modern CLI tools
    eza # ls replacement
    bat # cat with syntax highlighting
    delta # better git diffs
    ripgrep # fast grep
    fd # fast find
    bottom # btm — TUI system monitor
    zellij # tmux-alike multiplexer with friendlier UX (tabs/splits in any terminal)
    nushell # structured data shell
    helix # modal terminal editor (`hx`)
    aria2 # multi-connection downloader
    mprocs # run multiple processes in one terminal
    scc # code counter
    watch # procps `watch` command (for Linux; macOS has it via brew)

    # Lightweight dev / ops tools — small enough to live everywhere.
    just # task runner
    gh # GitHub CLI — useful from SSH on Pis for private clones / issue lookups
    nixfmt # official Nix formatter (RFC 166) — small, lets `vi` edit work everywhere
    nil # Nix language server — same rationale
    imagemagick # image conversion / cropping (`magick`); used by snlfilm/scripts/img-to-avif.sh fallback path + ad-hoc cast-photo recrops

    # git hook managers for jj-hooks testing.
    pre-commit
    prek
    lefthook

    # Container tooling — TUI for Docker/Podman socket inspection.
    # Pairs with Podman Desktop (GUI) installed per-host (cask on Mac,
    # winget on Windows). Useful from any terminal including SSH.
    lazydocker

    # jj VCS — universal because every machine touches the same repos.
    jujutsu # jj VCS
    jjui # jj VCS TUI
    just-lsp # language server for justfiles
  ];

  # Zsh
  programs.zsh = {
    enable = true;

    history = {
      size = 10000;
      save = 10000;
      path = "$HOME/.zsh_history";
      ignoreDups = true;
      share = true;
    };

    autocd = true;

    profileExtra = builtins.readFile ../dotfiles/zsh/profile.zsh;

    sessionVariables = { };

    initContent = lib.mkMerge [
      # _evalcache is defined here (interactive init), not in envExtra/.zshenv, so
      # a bash process that sources ~/.zshenv (e.g. the Linux Obsidian systemd
      # wrappers under `set -euo pipefail`) never parses its zsh-only syntax. The
      # low mkOrder puts it before the mkBefore brew blocks (darwin/home.nix,
      # shared/linux.nix) that call it.
      (lib.mkOrder 250 ''
        # _evalcache <key> <bin> <command...>: run a tool's shell-init once, cache
        # the output, and source the cache on later shells — regenerating only when
        # the tool's resolved path or mtime changes. Turns each `eval "$(tool init)"`
        # from a per-shell subprocess into a plain file source. No-ops if <bin> is
        # absent (preserves the old `command -v … &&` guards).
        zmodload -F zsh/stat b:zstat 2>/dev/null
        _evalcache() {
          # No `emulate -L zsh`: its -L localizes option changes, so a setopt run by
          # a sourced init (e.g. starship's `setopt promptsubst`) is reverted when
          # this function returns — which breaks the prompt (shows raw $(starship …)).
          local key=$1 bin=$2; shift 2
          local binpath; binpath=$(command -v "$bin" 2>/dev/null) || return 0
          local cache="''${XDG_CACHE_HOME:-$HOME/.cache}/zsh/evalcache/''${key}.zsh"
          local -a st; zstat -A st +mtime -- "$binpath" 2>/dev/null
          local stamp="#''${binpath:A}@''${st[1]:-0}"
          local head=""
          [[ -r $cache ]] && IFS= read -r head < "$cache"
          if [[ $head != $stamp ]]; then
            mkdir -p -- "''${cache:h}"
            local tmp="''${cache}.$$.tmp"
            if { print -r -- "$stamp"; "$@" } >| "$tmp" 2>/dev/null; then
              command mv -f -- "$tmp" "$cache"
            else
              command rm -f -- "$tmp"; eval "$("$@" 2>/dev/null)"; return
            fi
          fi
          source "$cache"
        }
      '')
      (builtins.readFile ../dotfiles/zsh/zshrc)
    ];

    plugins = [
      {
        name = "ohmyzsh-git";
        file = "plugins/git/git.plugin.zsh";
        src = pkgs.fetchFromGitHub {
          owner = "ohmyzsh";
          repo = "ohmyzsh";
          rev = "7c10d9839f05b8b73e26aa4e0f04cc886fbba6a6";
          hash = "sha256-Z9Cy3IYACSrhDYBEyXAC+6oYVD4CfiNOXOcekU8bZsw=";
        };
      }
      {
        name = "fast-syntax-highlighting";
        src = pkgs.fetchFromGitHub {
          owner = "zdharma-continuum";
          repo = "fast-syntax-highlighting";
          rev = "v1.55";
          hash = "sha256-DWVFBoICroKaKgByLmDEo4O+xo6eA8YO792g8t8R7kA=";
        };
      }
      {
        name = "zsh-autopair";
        src = pkgs.fetchFromGitHub {
          owner = "hlissner";
          repo = "zsh-autopair";
          rev = "449a7c3d095bc8f3d78571b2c8c4a8eca7d78e22";
          hash = "sha256-3zvOgIi+q7+sTXrT+r/4v98qjeiEL4Wh64rxBYnwJvQ=";
        };
      }
      {
        name = "you-should-use";
        src = pkgs.fetchFromGitHub {
          owner = "MichaelAquilina";
          repo = "zsh-you-should-use";
          rev = "1.9.0";
          hash = "sha256-+3iAmWXSsc4OhFZqAMTwOL7AAHBp5ZtGGtvqCnEOYc0=";
        };
      }
      {
        name = "fzf-tab";
        src = pkgs.fetchFromGitHub {
          owner = "Aloxaf";
          repo = "fzf-tab";
          rev = "v1.1.2";
          hash = "sha256-Qv8zAiMtrr67CbLRrFjGaPzFZcOiMVEFLg1Z+N6VMhg=";
        };
      }
    ];

    completionInit = builtins.readFile ../dotfiles/zsh/completion.zsh;
  };

  # Zsh autosuggestions (built-in home-manager module)
  programs.zsh.autosuggestion.enable = true;
  programs.zsh.historySubstringSearch.enable = true;

  # Starship prompt
  programs.starship = {
    enable = true;
    enableZshIntegration = false; # cached in dotfiles/zsh/zshrc via _evalcache
    settings = builtins.fromTOML (builtins.readFile ../dotfiles/starship/starship.toml);
  };

  # fzf
  programs.fzf = {
    enable = true;
    enableZshIntegration = false; # cached in dotfiles/zsh/zshrc via _evalcache
  };

  # zoxide
  programs.zoxide = {
    enable = true;
    enableZshIntegration = false; # cached in dotfiles/zsh/zshrc via _evalcache
  };

  # Git
  programs.git = {
    enable = true;
    signing.format = "openpgp";
    settings = {
      user.name = "Matt Wilkinson";
      user.email = "mattwilki17@gmail.com";
      init.defaultBranch = "main";
      credential."https://github.com".helper = "!gh auth git-credential";
      credential."https://gist.github.com".helper = "!gh auth git-credential";
    };
  };

  # Delta (git diff viewer)
  programs.delta = {
    enable = true;
    enableGitIntegration = true;
    options = {
      navigate = true;
      side-by-side = true;
      line-numbers = true;
    };
  };

  # bat
  programs.bat = {
    enable = true;
  };

  # direnv
  programs.direnv = {
    enable = true;
    enableZshIntegration = false; # cached in dotfiles/zsh/zshrc via _evalcache
    nix-direnv.enable = true;
    # Suppress the noisy `direnv: export +VAR +VAR ~VAR …` diff line on
    # every directory entry. With a devenv/nix-direnv shell the export
    # set is huge (compiler wrappers, NIX_* vars, PROTO_*), so the diff
    # is pure noise — the `direnv: loading …` lines already confirm the
    # load. Keeps stderr readable without hiding the load status.
    config.global.hide_env_diff = true;
  };

  # eza
  programs.eza = {
    enable = true;
    enableZshIntegration = true;
    icons = "auto";
    git = true;
  };

  # Ghostty — multiple `font-family` lines build a fallback chain.
  home.file.".config/ghostty/config".source = ../dotfiles/ghostty/config;

  # Foot (Wayland-native terminal). Comma-separated family list builds a
  # fallback chain. Only loaded on Linux but harmless to declare cross-platform
  # via home.file.
  home.file.".config/foot/foot.ini".source = ../dotfiles/foot/foot.ini;
  home.file.".config/ghostty/themes/NightOwlDark".source = ../dotfiles/ghostty/themes/NightOwlDark;
  home.file.".config/ghostty/themes/NightOwlLight".source = ../dotfiles/ghostty/themes/NightOwlLight;

  # Zellij multiplexer. `keybinds clear-defaults=true` means the file is
  # the full keymap — swap-layout binds are intentionally omitted so a
  # stray Alt [ / Alt ] / tmux-space can't reshuffle the workspace.
  home.file.".config/zellij/config.kdl".source = ../dotfiles/zellij/config.kdl;
  # Layouts for the multi-agent workflow + helpers (wave / push / sysmonitor).
  # Whole-dir source so new layouts are picked up automatically. Launch with
  # `zellij --layout <name>`, or in a session via Ctrl-t w|p|m (see config.kdl)
  # or the `zwave`/`zpush`/`zmon` aliases. See skill://multi-agent-wave.
  home.file.".config/zellij/layouts".source = ../dotfiles/zellij/layouts;
  # Drop a stale REAL ~/.config/zellij/layouts dir (left by a prior generation
  # that managed per-file layouts) so Home Manager can replace it with the
  # whole-dir symlink above instead of aborting checkLinkTargets ("in the way").
  home.activation.cleanStaleZellijLayouts = lib.hm.dag.entryBefore [ "checkLinkTargets" ] ''
    d="$HOME/.config/zellij/layouts"
    if [ -d "$d" ] && [ ! -L "$d" ]; then
      run rm -rf "$d"
    fi
  '';
  # Zellij layout-switch aliases — run inside a session to open the layout as a
  # new tab (fresh launch is `zellij --layout <name>`; keys are Ctrl-t w|p|m).
  programs.zsh.shellAliases = {
    zwave = "zellij action new-tab --layout wave";
    zpush = "zellij action new-tab --layout push";
    zmon = "zellij action new-tab --layout sysmonitor";
  };

  # ─── Files folded in from the old dotfiles repo ──────────────────────
  #
  # Migrated 2026-05 from the colocated git+jj at $HOME. Source content
  # lives at ../dotfiles/<category>/<name>; home-manager symlinks it into
  # place at activation. Editing the source file in-place propagates on
  # the next `nix-switch`.
  #
  # Why source-from-files instead of inline `text = ''…''`:
  #   - Keeps the .nix readable (status-line.sh is 100 lines).
  #   - Preserves shellcheck / language-mode tooling on the source.
  #   - One-line nix change to swap a file vs a multi-line text block.

  programs.bash = {
    enable = true;
    # `initExtra` lands AFTER the Debian default shape home-manager
    # carries; `profileExtra` runs at login. PATH lines mirror the
    # zsh setup so bash-via-SSH gets the same view.
    initExtra = builtins.readFile ../dotfiles/bash/bashrc;
    profileExtra = builtins.readFile ../dotfiles/bash/profile;
  };

  # Markdownlint config — picked up by markdownlint-cli2 from any cwd
  # that walks up to $HOME.
  home.file.".markdownlint.jsonc".source = ../dotfiles/markdownlint/markdownlint.jsonc;

  # jj (Jujutsu) user config. Includes templater + per-repo scope
  # overrides (sealedsecurity email) + the `jj fix` rustfmt config and
  # the `jj push` alias to jj-hp.
  xdg.configFile."jj/config.toml".source = ../dotfiles/jj/config.toml;

  # jjui (jj TUI) user config — keybindings for jj-hp integration.
  xdg.configFile."jjui/config.toml".source = ../dotfiles/jjui/config.toml;

  # Claude Code config — `settings.json` + the status-line script it
  # references. `.claude/CLAUDE.md` lives in privatefiles (symlinked
  # at ~/.claude/CLAUDE.md); only the config + scripts are managed
  # via nix.
  home.file.".claude/settings.json".source = ../dotfiles/claude/settings.json;
  home.file.".claude/scripts/status-line.sh" = {
    source = ../dotfiles/claude/scripts/status-line.sh;
    executable = true;
  };
}

{ pkgs, ... }:

{
  home.stateVersion = "25.11";

  programs.home-manager.enable = true;

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

    profileExtra = ''
      export CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1
    '';

    sessionVariables = { };

    initContent = ''
      export PATH="$HOME/.local/bin:$PATH"

      # Directory navigation
      setopt AUTO_PUSHD
      setopt PUSHD_IGNORE_DUPS

      # jj (Jujutsu) CLI completion
      source <(COMPLETE=zsh jj)

      # jj-hp (Jujutsu Hooks) CLI completion — only when installed
      # (cargo-installed binary; not present on hosts without dev.nix).
      command -v jj-hp >/dev/null 2>&1 && eval "$(jj-hp completions zsh)"

      # jj-gt CLI completion — only when installed (cargo-installed
      # binary; not present on hosts without dev.nix).
      command -v jj-gt >/dev/null 2>&1 && eval "$(jj-gt completions zsh)"

      # gt (Graphite CLI) completion. gt emits yargs-style
      # completions via `gt completion zsh`. Only wire when
      # installed (graphite-cli is in dev.nix → not on hosts
      # without dev tooling). The completion script is dynamic —
      # the script registers a function that re-invokes `gt
      # --get-yargs-completions` on TAB to enumerate live values.
      command -v gt >/dev/null 2>&1 && eval "$(gt completion zsh 2>/dev/null)"
    '';

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

    completionInit = ''
      autoload -U compinit && compinit
    '';
  };

  # Zsh autosuggestions (built-in home-manager module)
  programs.zsh.autosuggestion.enable = true;
  programs.zsh.historySubstringSearch.enable = true;

  # Starship prompt
  programs.starship = {
    enable = true;
    enableZshIntegration = true;
    # jj (Jujutsu) VCS status via starship-jj (installed by the cargo-install
    # step in scripts/{linux,mac}-setup.sh — not in nixpkgs).
    # See https://gitlab.com/lanastara_foss/starship-jj
    settings = {
      # Built-in git modules replaced below so they don't double-print with
      # custom.jj in jj-colocated repos.
      git_branch.disabled = true;
      git_commit.disabled = true;
      git_state.disabled = true;
      git_metrics.disabled = true;
      git_status.disabled = true;

      # starship runs `when` through the module's `shell`, so we use
      # `bash -c` for both modules and invoke starship-jj / git from the
      # command body. (Using starship-jj as the shell doesn't work because
      # `when` then gets parsed as an unrecognized starship-jj subcommand.)

      # Shows in jj repos.
      custom.jj = {
        shell = [
          "bash"
          "--noprofile"
          "--norc"
          "-c"
        ];
        command = "starship-jj --ignore-working-copy starship prompt";
        format = "$output";
        ignore_timeout = true;
        use_stdin = false;
        when = "jj root --ignore-working-copy";
      };

      # Fallback for pure-git repos (no jj). Minimal: branch + dirty mark.
      custom.git = {
        shell = [
          "bash"
          "--noprofile"
          "--norc"
          "-c"
        ];
        command = ''branch=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); dirty=""; git diff --quiet 2>/dev/null || dirty="*"; echo "$branch$dirty"'';
        when = "! jj root --ignore-working-copy 2>/dev/null && git rev-parse --git-dir";
        format = "on [$output](bold purple) ";
      };

      # Position ${custom} immediately after $directory (where the old
      # git modules used to sit). Pre-directory modules are listed
      # explicitly, $all picks up everything after (package, languages,
      # cloud, etc.), then $line_break + $character on the next line.
      format = "$username$hostname$localip$shlvl$singularity$kubernetes$directory\${custom}$all$line_break$character";
    };
  };

  # fzf
  programs.fzf = {
    enable = true;
    enableZshIntegration = true;
  };

  # zoxide
  programs.zoxide = {
    enable = true;
    enableZshIntegration = true;
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
    enableZshIntegration = true;
    nix-direnv.enable = true;
  };

  # eza
  programs.eza = {
    enable = true;
    enableZshIntegration = true;
    icons = "auto";
    git = true;
  };

  # Ghostty — multiple `font-family` lines build a fallback chain.
  home.file.".config/ghostty/config".text = ''
    theme = NightOwlDark
    font-family = "Berkeley Mono"
    font-family = "IosevkaTerm Nerd Font"
    font-family = "Cascadia Code"
    font-size = 14
  '';

  # Foot (Wayland-native terminal). Comma-separated family list builds a
  # fallback chain. Only loaded on Linux but harmless to declare cross-platform
  # via home.file.
  home.file.".config/foot/foot.ini".text = ''
    font=Berkeley Mono:size=11, IosevkaTerm Nerd Font, Cascadia Code, monospace
  '';
  home.file.".config/ghostty/themes/NightOwlDark".text = ''
    palette = 0=#011627
    palette = 1=#ef5350
    palette = 2=#22da6e
    palette = 3=#addb67
    palette = 4=#82aaff
    palette = 5=#c792ea
    palette = 6=#21c7a8
    palette = 7=#ffffff
    palette = 8=#575656
    palette = 9=#ef5350
    palette = 10=#22da6e
    palette = 11=#ffeb95
    palette = 12=#82aaff
    palette = 13=#c792ea
    palette = 14=#7fdbca
    palette = 15=#ffffff
    background = 011627
    foreground = d6deeb
    cursor-color = 7e57c2
    selection-background = 5f7e97
    selection-foreground = dfe5ee
  '';
  home.file.".config/ghostty/themes/NightOwlLight".text = ''
    palette = 0=#403f53
    palette = 1=#de3d3b
    palette = 2=#08916a
    palette = 3=#e0af02
    palette = 4=#288ed7
    palette = 5=#d6438a
    palette = 6=#2aa298
    palette = 7=#f0f0f0
    palette = 8=#989fb1
    palette = 9=#de3d3b
    palette = 10=#08916a
    palette = 11=#daaa01
    palette = 12=#288ed7
    palette = 13=#d6438a
    palette = 14=#2aa298
    palette = 15=#f0f0f0
    background = fbfbfb
    foreground = 403f53
    cursor-color = 403f53
    selection-background = e0e0e0
    selection-foreground = 403f53
  '';

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
    initExtra = ''
      export PATH="$HOME/.local/bin:$PATH"
      export PATH="$HOME/.npm-global/bin:$PATH"
      # Linuxbrew on Linux dev hosts. Harmless no-op on Mac (brew lives
      # at /opt/homebrew there and is loaded by zsh's macOS-specific
      # initContent) and on servers without brew installed.
      [ -x /home/linuxbrew/.linuxbrew/bin/brew ] && eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)"
      [ -r "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    '';
    profileExtra = ''
      # zsh sets these in initContent; bash gets the same via .profile.
      [ -d "$HOME/.local/bin" ] && PATH="$HOME/.local/bin:$PATH"
      [ -d "$HOME/bin" ] && PATH="$HOME/bin:$PATH"
      [ -r "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    '';
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

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

    initContent = builtins.readFile ../dotfiles/zsh/zshrc;

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
    enableZshIntegration = true;
    settings = builtins.fromTOML (builtins.readFile ../dotfiles/starship/starship.toml);
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
  home.file.".config/ghostty/config".source = ../dotfiles/ghostty/config;

  # Foot (Wayland-native terminal). Comma-separated family list builds a
  # fallback chain. Only loaded on Linux but harmless to declare cross-platform
  # via home.file.
  home.file.".config/foot/foot.ini".source = ../dotfiles/foot/foot.ini;
  home.file.".config/ghostty/themes/NightOwlDark".source = ../dotfiles/ghostty/themes/NightOwlDark;
  home.file.".config/ghostty/themes/NightOwlLight".source = ../dotfiles/ghostty/themes/NightOwlLight;

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

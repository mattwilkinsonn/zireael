{ pkgs, ... }:

{
  # Determinate Nix manages the daemon; disable nix-darwin's management
  nix.enable = false;

  # direnv check phase fails on aarch64-darwin in 25.11. The fish test runner
  # is SIGKILL'd inside the sandbox (`make: *** [GNUmakefile:150: test-fish]
  # Killed: 9`), distinct from the earlier zsh sigsuspend/autoconf hang fixed
  # by nixpkgs#513971 (merged to master 2026-04-27, not backported to
  # release-25.11 as of 2026-05-03). Skipping checks until either the
  # backport lands or the fish-test failure is diagnosed upstream.
  nixpkgs.overlays = [
    (_: prev: {
      direnv = prev.direnv.overrideAttrs (_: {
        doCheck = false;
      });
    })
  ];

  # System-level packages
  environment.systemPackages = with pkgs; [
    nixfmt

    # ── system profiling / diagnostics ─────────────────────────────
    # Cross-platform subset of nixos/common.nix's profiling toolkit.
    # macOS has `system_profiler`, `top`, and `fs_usage` built in;
    # these fill the same gaps Linux gets from sysstat/iotop/etc.

    # Faster htop alternative. Interactive process viewer; replaces
    # Activity Monitor for terminal sessions.
    htop
    # Disk SMART (works on macOS for both internal + external
    # SATA/NVMe via IOKit shim).
    smartmontools
    # Interactive disk-usage browser. Same role as `du | sort`
    # but instant.
    ncdu
    # Packet capture.
    tcpdump
    # Combined ping + traceroute. mtr -r for loss% per hop.
    mtr
    # DNS lookup utilities (`dig`).
    dnsutils
    # "What's holding /that/ file/socket open."
    lsof
    # psmisc (pstree + killall + fuser) used to be in this list but
    # nixpkgs dropped darwin support upstream (psmisc-23.7 lists
    # Linux-only in meta.platforms). killall is in macOS core; pstree
    # / fuser come via brew if needed (`brew install pstree fuser`).
    # Pipe-viewer progress bars.
    pv

    # ── network throughput ─────────────────────────────────────────
    # Bandwidth benchmark client+server (`iperf3 -s` / `-c`).
    iperf3
    # Modern per-process bandwidth viewer (Rust). Mac analog of
    # `nethogs`. Uses pcap so needs root.
    bandwhich

    # ── modernised coreutils + nicer terminal UX ───────────────────
    # See nixos/common.nix for the system-wide-not-home-manager
    # rationale. Same set, minus Linux-only entries.
    ripgrep
    fd
    bat
    eza
    btop
    jq
    yq-go
  ];

  # Fonts (system-wide — nix-darwin links these into /Library/Fonts so every
  # macOS app sees them). JetBrains Mono Nerd Font is the fallback for the
  # Nerd glyphs Departure Mono doesn't ship (prompt icons, etc.).
  fonts.packages = with pkgs; [
    departure-mono
    nerd-fonts.jetbrains-mono
    nerd-fonts.iosevka-term
    cascadia-code
  ];

  # macOS doesn't honor fontconfig the way Linux does — most Mac apps use
  # CoreText. So this fallback chain is informational; per-app font config
  # (Ghostty, Konsole, etc.) needs to spell it out explicitly.

  # Homebrew (managed by nix-darwin)
  homebrew = {
    enable = true;
    onActivation = {
      autoUpdate = true;
      upgrade = true;
      # Temporary: was `cleanup = "zap"`. Homebrew/brew@f38cd4b
      # ("Convert bundle subcommands") moved `--cleanup` into a
      # separate `cleanup` subcommand and made the `install`
      # subcommand reject `--cleanup` / `--zap` outright. nix-darwin's
      # current `brewBundleCmd` emits `brew bundle ... --cleanup`
      # (for `"uninstall"`) or `--cleanup --zap` (for `"zap"`)
      # against the install subcommand, which means BOTH modes fail
      # on current brew. Only `"none"` works.
      #
      # Trade-off while we wait for nix-darwin#1774:
      #   - "none" skips the cleanup pass entirely.
      #   - Brews/casks removed from this Brewfile won't get
      #     auto-uninstalled. Run `brew bundle cleanup --force --zap
      #     --file=<brewfile>` manually if drift matters.
      #
      # Switch back to "zap" once nix-darwin#1774 lands on master and
      # a flake.lock bump pulls the fix.
      cleanup = "none";
    };
    brews = [
      # terminal-notifier — desktop banners for the `notify` extension.
      # Homebrew builds it native arm64 (xcodebuild -arch); the nixpkgs
      # build ships the upstream x86 .app, which fails to post under
      # Rosetta on Apple Silicon, so it lives here, not in systemPackages.
      "terminal-notifier"
      "rtk"
      "xcodes"
      "vjeantet/tap/alerter"
      "asheshgoplani/tap/agent-deck"
      "swiftly"
      "coreutils"
      "duti"
      "mas" # Mac App Store CLI — drives `homebrew.masApps` install/upgrade.
      "qalculate-qt"
      "podman"
      # zstd: stays declared even though dev.nix also installs pkgs.zstd.
      # podman links against libzstd at runtime, so `brew bundle cleanup`
      # can't remove it anyway (refuses to uninstall installed dependents).
      # Declaring it here makes the situation explicit instead of leaving
      # it as a silent transitive that re-appears on every `brew autoremove`.
      # Mac PATH order puts /opt/homebrew/bin before ~/.nix-profile/bin, so
      # `which zstd` resolves to the brew binary; the nixpkgs version still
      # ends up on Linux dev boxes via shared/dev.nix.
      "zstd"
      "libkrun/krun/krunkit"
      "asmvik/formulae/yabai"
      "asmvik/formulae/skhd"
    ];
    casks = [
      "1password"
      "1password-cli"
      "claude"
      "emdash" # ADE: parallel agent worktrees + in-app PR/Linear/diff/CI; launches omp via the pi-provider shim (see dev.nix)
      "docker-desktop"
      "google-chrome"
      "spotify"
      "visual-studio-code"
      "discord"
      "transmission"
      "steam"
      "zoom"
      "microsoft-teams"
      "whatsapp"
      "adobe-creative-cloud"
      "ghostty"
      "slack"
      "mitmproxy"
      "obsidian"
      "gcloud-cli"
      "google-drive"
      "termius"
      "tailscale-app" # GUI app + bundled CLI. The CLI ships inside the bundle;
      # enable it via the tray menu → "Install CLI" (drops a symlink to
      # /usr/local/bin/tailscale). Not declarable from Nix.
      "sunsama"
      "linear"
      "rustdesk"
      "meld"
      "dockdoor"
      "betterdisplay"
      "backdrop"
      "keka"
      "logi-options+"
      "orbstack"
      "raspberry-pi-imager"
      "arc"
      "akiflow"
      "codex-app"
      "typora"
      "raycast"
      "notion"
      "stablyai/orca/orca"
      "transmit"
    ];
    # Mac App Store apps. IDs from `mas search <name>` / the App Store
    # share-link `/id<NUMBER>`. Requires a one-time manual App Store
    # sign-in (mas can't authenticate on modern macOS) and the app must
    # already be associated with the Apple ID. Versions aren't pinnable
    # and removal here doesn't uninstall — prefer a cask when one exists.
    masApps = {
      "WiFi Explorer: Scanner" = 494803304;
    };
  };

  # macOS system defaults
  system.defaults = {
    dock = {
      autohide = false;
      show-recents = true;
    };
    finder = {
      AppleShowAllExtensions = true;
      AppleShowAllFiles = true;
      FXPreferredViewStyle = "Nlsv";
    };
    NSGlobalDomain = {
      AppleShowAllExtensions = true;
      InitialKeyRepeat = 15;
      KeyRepeat = 2;
      "com.apple.trackpad.forceClick" = false;
      "com.apple.swipescrolldirection" = false; # Disable natural scrolling
    };
    trackpad = {
      TrackpadThreeFingerTapGesture = 2; # Tap with three fingers = look up
      Clicking = false; # Tap to click off
    };
    screensaver = {
      askForPassword = true;
      askForPasswordDelay = 3600; # 1 hour grace period
    };
    WindowManager = {
      EnableStandardClickToShowDesktop = false;
    };
  };

  # Display sleep (pmset supports per-power-source; nix-darwin's power.sleep does not)
  # Default app associations (via duti; handles extensionless files like Dockerfile/Makefile via UTIs)
  system.activationScripts.postActivation.text = ''
    pmset -b displaysleep 60   # battery: 1 hour
    pmset -c displaysleep 180  # power adapter: 3 hours

    DUTI=/opt/homebrew/bin/duti
    if [ -x "$DUTI" ]; then
      CODE=com.microsoft.VSCode
      TYPORA=abnerworks.Typora

      # VSCode for source code + plain text UTIs (covers Dockerfile, Makefile, Justfile, etc.)
      "$DUTI" -s "$CODE" public.source-code all
      "$DUTI" -s "$CODE" public.shell-script all
      "$DUTI" -s "$CODE" public.plain-text all
      "$DUTI" -s "$CODE" public.script all
      "$DUTI" -s "$CODE" com.apple.log all
      "$DUTI" -s "$CODE" public.xml all
      "$DUTI" -s "$CODE" public.json all
      "$DUTI" -s "$CODE" public.yaml all
      "$DUTI" -s "$CODE" public.comma-separated-values-text all
      "$DUTI" -s "$CODE" public.tab-separated-values-text all

      for ext in \
        js jsx ts tsx mjs cjs \
        py rb php pl \
        rs go \
        c h cpp hpp cc cxx \
        java kt scala clj cljs \
        swift m mm \
        lua \
        hs elm \
        ex exs \
        ml fs \
        sh bash zsh fish \
        css scss sass less \
        vue svelte astro \
        json yaml yml toml \
        xml ini conf cfg env properties \
        csv tsv sql \
        nix \
        log txt rst adoc; do
        # Error -50 = dynamic UTI (macOS has no app declaring this extension); benign
        "$DUTI" -s "$CODE" ".$ext" all 2>/dev/null || true
      done

      # Typora for markdown (overrides the plain-text default above)
      "$DUTI" -s "$TYPORA" .md all
      "$DUTI" -s "$TYPORA" .markdown all
      "$DUTI" -s "$TYPORA" net.daringfireball.markdown all
    fi
  '';

  # Allow Touch ID for sudo
  security.pam.services.sudo_local.touchIdAuth = true;

  system.primaryUser = "mattwilkinson";

  # Used for backwards compat; don't change
  system.stateVersion = 6;
}

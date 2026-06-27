{ pkgs, ... }:

{
  # Fonts — installed into the Linux side for any Linux GUI app.
  fonts.fontconfig = {
    enable = true;
    # System-wide monospace fallback chain. Apps requesting "monospace" — or
    # falling back when a primary font lacks glyphs — walk this list in order.
    # Berkeley Mono is paid (manual or rclone install); the rest are nix-managed.
    defaultFonts.monospace = [
      "Berkeley Mono"
      "IosevkaTerm Nerd Font"
      "Cascadia Code"
      "monospace"
    ];
  };

  home = {
    username = "mattw";
    homeDirectory = "/home/mattw";

    packages = with pkgs; [
      departure-mono
      nerd-fonts.jetbrains-mono
      nerd-fonts.iosevka-term
      cascadia-code
      meld # visual diff/merge tool
      mitmproxy # CLI proxy (Mac side gets the GUI cask via brew)
      ghostty
      # terminal — Linux-only in nixpkgs (no aarch64-darwin
      # support); Mac uses the official .dmg or `brew install
      # --cask ghostty`. Pi inherits this since it's aarch64-
      # linux which is supported, but it's headless so unused.
      # _1password-cli intentionally NOT here — added per-host: rpis get
      # op via environment.systemPackages in nixos/common.nix; Mac side
      # gets it bundled with the 1Password desktop app cask;
      # mattpc-wsl gets it from common.nix (NixOS module).
      chromium
      # headless-capable browser used by Puppeteer for:
      #   - sealed/sealedsecurity.com og-image generation (`just site`).
      #   - sealed/docs PDF generation (`just docs`).
      #   - sealed/branding SVG → PNG rasterizer (`just brand-png`).
      # Puppeteer's bundled-Chrome postinstall doesn't work on NixOS
      # because (a) it tries to `tar`/`unzip` outside of any FHS
      # sandbox, and (b) even when it succeeds the resulting binary
      # is dynamically linked against glibc paths that don't exist
      # on NixOS. We point Puppeteer at this nixpkgs chromium via
      # PUPPETEER_EXECUTABLE_PATH below and skip the download with
      # PUPPETEER_SKIP_DOWNLOAD=1. Mac dev hosts use Puppeteer's
      # bundled Chrome (or the brew cask) — this entry is Linux-only.
    ];
  };

  # `nix-switch` is per-host:
  #   - rpi5/rpi4: lib.mkForce-overridden in nixos/rpi{5,4}/home.nix to
  #     `sudo nixos-rebuild switch --flake … #rpi{5,4}`.
  #   - mattfw / mattserver / mattpc-wsl: lib.mkForce per-host in their
  #     nixos/<host>/home.nix.
  # No default alias here so per-host overrides aren't shadowed.

  # Puppeteer (used by sealed/) — point it at the nixpkgs chromium
  # above and skip its own Chrome download. Without this, every
  # `bun install` in sealed/ runs a postinstall that fetches Chrome
  # and either fails (no `tar`/`unzip` outside an FHS env) or
  # produces a non-launchable binary (glibc path mismatch). Set via
  # envExtra (appended to .zshenv) rather than home.sessionVariables
  # so long-lived parent shells pick up changes on the next tab
  # without a full session restart — see linux-build-deps.nix for
  # the full rationale.
  programs.zsh.envExtra = ''
    export PUPPETEER_SKIP_DOWNLOAD=1
    export PUPPETEER_EXECUTABLE_PATH=${pkgs.chromium}/bin/chromium
  '';

  programs.zsh.initContent = ''
    [[ $- != *i* ]] && return

    # linuxbrew: not declarative on NixOS (nix-homebrew is Darwin-only),
    # but installed via `ensure_linuxbrew` in each Linux dev host's
    # bootstrap script. Used for tools where nixpkgs lags upstream by
    # months, or for Mac↔Linux dev workflow parity (e.g. `brew style`
    # on the zireael tap formulae, hk's pkl tooling). Servers (rpis,
    # mattserver) don't install brew — this hook just no-ops there.
    [ -f /home/linuxbrew/.linuxbrew/bin/brew ] && eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)"

    # DOCKER_HOST auto-detect: prefer the rootless user socket, fall
    # back to the root podman socket (rpis — containers declared as
    # root-owned oci-containers in nixos/common.nix). Picked up by
    # lazydocker, docker-cli, podman-cli, and VSCode devcontainers
    # without per-tool config. Mac uses OrbStack's local Docker
    # socket and doesn't need this.
    if [ -S "/run/user/$UID/podman/podman.sock" ]; then
      export DOCKER_HOST="unix:///run/user/$UID/podman/podman.sock"
    elif [ -S /run/podman/podman.sock ]; then
      export DOCKER_HOST="unix:///run/podman/podman.sock"
    fi
  '';
  # `load-secrets` (op-backed, cross-platform) lives in
  # shared/home.nix programs.zsh.initContent — runs after this
  # mkBefore block, by which point op is on PATH via the per-host
  # install (NixOS env packages on Linux hosts, brew on Mac).

  # Rootless container storage. NixOS's /etc/containers/storage.conf (from
  # virtualisation.podman) hardcodes the rootful runroot
  # "/run/containers/storage"; podman remaps that for rootless, but the
  # skopeo bundled in nix2container ("devenv container copy") honors it
  # literally and dies with `mkdir /run/containers: permission denied`. Pin
  # both paths under $HOME here so every containers/storage tool (skopeo,
  # podman, buildah) writes where the user can — and so it still works when
  # $XDG_RUNTIME_DIR is unset (cron, `sudo -u`). containers/storage expands $HOME.
  xdg.configFile."containers/storage.conf".text = ''
    [storage]
    driver = "overlay"
    graphroot = "$HOME/.local/share/containers/storage"
    runroot = "$HOME/.local/share/containers/runroot"
  '';
}

{
  description = "Matt's system configuration";

  nixConfig = {
    extra-substituters = [
      "https://nixos-raspberrypi.cachix.org"
      # nix-openclaw CI publishes prebuilt gateway derivations here.
      # Without it, `nix-switch mattfw` rebuilds the upstream OpenClaw
      # gateway (pnpm + TypeScript) from scratch on every input bump —
      # ~30 min on Strix Halo. Garnix is the project-maintained cache;
      # the public key below pins the signature we'll accept, so a
      # cache compromise can't ship us unsigned artifacts. Drop both
      # lines if we ever want to rebuild from source instead.
      "https://cache.garnix.io"
    ];
    extra-trusted-public-keys = [
      "nixos-raspberrypi.cachix.org-1:4iMO9LXa8BqhU+Rpg6LQKiGa2lsNh/j2oiYLNOQ5sPI="
      "cache.garnix.io:CTFPyKSLcx5RMJKfLo5EEPUObbA78b0YQ2DTCJXqr9g="
    ];
  };

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    nixpkgs-darwin.url = "github:nixos/nixpkgs/nixpkgs-25.11-darwin";
    # For packages not yet backported to 25.11 (klassy, howdy). Also the
    # base channel for dev hosts via shared/unstable-wholesale.nix — Mac
    # + Linux dev boxes pull every package through this.
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    # For fast-moving CLI tools (jj, etc.) where even nixos-unstable lags
    # the upstream release by days/weeks. Master skips Hydra's full build
    # set, so cache hits are worse — keep this input limited to packages
    # that actually need bleeding-edge versions (see shared/overlays.nix).
    nixpkgs-master.url = "github:nixos/nixpkgs/master";
    # home-manager + nix-darwin split into stable + unstable inputs.
    # nix-darwin enforces a strict branch check against the resolved
    # pkgs — running nix-darwin-25.11 against unstable 26.05 errors out.
    # home-manager master also drops support for stable nixpkgs (refers
    # to lib paths only in 26.05+), so we need both branches:
    #
    # - Dev hosts (Mac, mattfw, mattpc-wsl) use the unstable inputs to
    #   match shared/unstable-wholesale.nix.
    # - Server hosts (rpi4, rpi5, mattserver) use the 25.11 inputs to
    #   match stable nixpkgs.
    #
    # Collapse back to a single home-manager input when stable releases
    # catch up (likely after 26.05 lands and unstable becomes 26.11).
    home-manager = {
      url = "github:nix-community/home-manager/release-25.11";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager-unstable = {
      url = "github:nix-community/home-manager/master";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/master";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    # Stable-branch nix-darwin for mattmacpro. The Mac Pro 2013 is
    # nixpkgs's last x86_64-darwin generation (deprecated after
    # 26.05); pairing stable nixpkgs with the matching stable
    # nix-darwin avoids the strict-branch check that pairs nix-darwin
    # master with nixpkgs-unstable on the MBP. Also dodges the
    # infinite-recursion issue in home-manager-master's eager
    # `pkgs.formats.toml` evaluation when `_module.args.pkgs` is
    # `lib.mkForce`-replaced via shared/unstable-wholesale.nix —
    # mattmacpro doesn't use unstable-wholesale (it's a server host).
    nix-darwin-stable = {
      url = "github:nix-darwin/nix-darwin/nix-darwin-25.11";
      inputs.nixpkgs.follows = "nixpkgs-darwin";
    };
    nixos-raspberrypi = {
      url = "github:nvmd/nixos-raspberrypi/main";
    };
    nixos-wsl = {
      url = "github:nix-community/NixOS-WSL/main";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    plasma-manager = {
      url = "github:nix-community/plasma-manager";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
    };
    # Declarative Flatpak — `services.flatpak.packages` mirrors how
    # nix-darwin manages Homebrew on the Mac side. Used in
    # nixos/rpi5/desktop.nix for fast-moving GUI apps where nixpkgs
    # version lag matters but reproducibility doesn't.
    nix-flatpak.url = "github:gmodena/nix-flatpak/v0.7.0";

    # nix-openclaw — declarative OpenClaw gateway. Used on mattfw only.
    # Intentionally does NOT `follow` our nixpkgs: the upstream gateway
    # derivation pins a `pnpmDeps` fixed-output hash that was computed
    # against the exact nixpkgs revision their flake.lock points at.
    # Overriding the nixpkgs follow swaps pnpm versions, which changes
    # the lockfile resolution and breaks the hash check. Letting them
    # use their own pinned nixpkgs also matches what Garnix's binary
    # cache was built against — so substitution from cache.garnix.io
    # actually hits instead of forcing a from-source build.
    nix-openclaw.url = "github:openclaw/nix-openclaw";
    codex-cli.url = "github:sadjow/codex-cli-nix";
    llm-agents.url = "github:numtide/llm-agents.nix";
  };

  outputs =
    inputs@{
      nixpkgs,
      home-manager,
      home-manager-unstable,
      nix-darwin,
      nixos-raspberrypi,
      nixos-wsl,
      nix-flatpak,
      ...
    }:
    let
      hmModule = {
        home-manager.useGlobalPkgs = true;
        home-manager.useUserPackages = true;
        home-manager.backupFileExtension = "hm-backup";
        # Make `inputs` available inside home-manager modules — needed so
        # platform-specific home modules can import plasma-manager + reference
        # unstable nixpkgs.
        home-manager.extraSpecialArgs = { inherit inputs; };
      };
    in
    {
      # macOS (Apple Silicon) — nix-darwin + home-manager
      darwinConfigurations."Matts-MacBook-Pro" = nix-darwin.lib.darwinSystem {
        system = "aarch64-darwin";
        specialArgs = {
          inherit inputs;
          system = "aarch64-darwin";
        };
        modules = [
          ./shared/unstable-wholesale.nix
          ./darwin/system.nix
          home-manager-unstable.darwinModules.home-manager
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
            home-manager.backupFileExtension = "hm-backup";
            home-manager.extraSpecialArgs = {
              codexPackage = inputs.codex-cli.packages.aarch64-darwin.codex;
              coderabbitPackage = inputs.llm-agents.packages.aarch64-darwin.coderabbit-cli;
            };
            home-manager.users.mattwilkinson = import ./darwin/home.nix;
          }
        ];
      };

      # mattmacpro — Mac Pro 2013 trashcan (Intel Xeon E5 + 64 GB DDR3),
      # OCLP'd up to macOS Sonoma. Headless self-hosted GitHub Actions
      # runner pool for the sealedsecurity org (macOS x64 primary).
      #
      # Server-class host — uses STABLE nixpkgs + stable nix-darwin +
      # stable home-manager, NOT the unstable-wholesale path the MBP
      # uses. The MBP is an interactive dev box where staying close to
      # upstream master matters; mattmacpro is a CI runner where
      # cache-aligned + bug-free is what matters. Same split as
      # mattserver vs. mattfw on the Linux side.
      #
      # Pkgs is constructed in the `let` block below and passed
      # explicitly via both nix-darwin's specialArgs and home-manager's
      # extraSpecialArgs. This is structurally different from how the
      # other hosts do it (they rely on useGlobalPkgs + a generated
      # pkgs), but it's the only way to short-circuit a stable
      # home-manager evaluation cycle on x86_64-darwin:
      #
      #   - System eval needs config.assertions
      #   - home-manager-nixos/common.nix flattens assertions from
      #     every user's HM config
      #   - To get a user's config, every HM module's let-block
      #     evaluates (k9s.nix's yamlFormat, aerospace.nix's
      #     tomlFormat, etc. all force `pkgs`)
      #   - useGlobalPkgs sets _module.args.pkgs which requires config
      #     evaluation → infinite recursion
      #
      # Passing pkgs as a specialArg means the function signature
      # `{ pkgs, ... }:` resolves at call time without going through
      # _module.args.pkgs. Same pattern Apple Silicon nix-darwin uses
      # internally — it just isn't the default with useGlobalPkgs.
      #
      # Targeted overlays (jj etc.) still come via shared/overlays.nix,
      # applied as part of the pkgs construction below.
      darwinConfigurations."mattmacpro" =
        let
          system = "x86_64-darwin";
          # Inline overlay: jj from unstable (same as shared/overlays.nix
          # gives mattserver). Can't import shared/overlays.nix here
          # because it's structured as a NixOS-style module
          # (`{ nixpkgs.overlays = [...]; }`), not a bare overlay
          # function. If this list grows, factor out into a shared
          # overlays-as-list module that both this entry and
          # shared/overlays.nix can consume.
          jjFromUnstable = _final: _prev: {
            inherit (inputs.nixpkgs-unstable.legacyPackages.${system}) jujutsu jjui;
          };
          pkgs = import inputs.nixpkgs-darwin {
            inherit system;
            config = {
              allowUnfree = true;
            };
            overlays = [ jjFromUnstable ];
          };
        in
        inputs.nix-darwin-stable.lib.darwinSystem {
          inherit system;
          specialArgs = {
            inherit inputs system pkgs;
          };
          modules = [
            { nixpkgs.pkgs = pkgs; }
            ./darwin/mattmacpro/system.nix
            home-manager.darwinModules.home-manager
            {
              home-manager.useGlobalPkgs = true;
              home-manager.useUserPackages = true;
              home-manager.extraSpecialArgs = {
                inherit inputs pkgs;
              };
              home-manager.users.mattw = import ./darwin/mattmacpro/home.nix;
            }
          ];
        };

      # Raspberry Pi 5 (headless): Tailscale exit node + Kimai for
      # hours.sealedsecurity.com. Technitium DNS lives on rpi4.
      # Add ./nixos/rpi5/desktop.nix to modules to re-enable KDE for debug.
      # nix-flatpak module included so re-enabling desktop.nix's services.flatpak
      # block resolves; harmless when desktop.nix isn't imported (nothing
      # references services.flatpak).
      nixosConfigurations."rpi5" = nixos-raspberrypi.lib.nixosSystem {
        specialArgs = { inherit inputs; };
        modules = [
          (
            { ... }:
            {
              imports = with nixos-raspberrypi.nixosModules; [
                raspberry-pi-5.base
                raspberry-pi-5.display-rp1
                raspberry-pi-5.bluetooth
              ];
            }
          )
          nix-flatpak.nixosModules.nix-flatpak
          ./shared/overlays.nix
          ./nixos/common.nix
          ./nixos/rpi5/system.nix
          ./nixos/rpi5/kimai.nix
          # ./nixos/rpi5/desktop.nix  # uncomment to re-enable KDE Plasma + GUI flatpaks for debug
          home-manager.nixosModules.home-manager
          hmModule
          {
            home-manager.users.mattw = {
              imports = [
                ./shared/home.nix
                ./shared/linux.nix
                ./nixos/home.nix
                ./nixos/rpi5/home.nix
              ];
            };
          }
        ];
      };

      # Framework Desktop (Ryzen AI Max+ 395, 128GB LPDDR5X). Primary use:
      # VSCode Remote-SSH dev box + local LLM inference target. KDE Plasma
      # is installed (./nixos/mattfw/desktop.nix) but the display manager
      # is held off the boot path — `systemctl start sddm` brings it up
      # on demand for local debug. See nixos/mattfw/INSTALL.md for the
      # fresh-install steps (NixOS 25.11 ISO via Ventoy → BIOS UMA tweak →
      # LUKS+btrfs partition → nixos-install → framework-bootstrap.sh).
      #
      # Uses nixos-unstable's lib.nixosSystem (not stable nixpkgs') so the
      # NixOS module set matches the wholesale-unstable pkgs. Stable
      # modules reference packages that have been renamed in 26.05
      # (xow_dongle-firmware → xone-dongle-firmware, etc.) and break
      # when paired with unstable pkgs.
      nixosConfigurations."mattfw" = inputs.nixpkgs-unstable.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = {
          inherit inputs;
          system = "x86_64-linux";
        };
        modules = [
          nix-flatpak.nixosModules.nix-flatpak
          ./shared/unstable-wholesale.nix
          ./nixos/common.nix
          ./nixos/mattfw/system.nix
          ./nixos/mattfw/desktop.nix
          ./nixos/mattfw/openclaw.nix
          home-manager-unstable.nixosModules.home-manager
          hmModule
          {
            # Augment hmModule's extraSpecialArgs with the pre-built
            # OpenClaw gateway package. nix-openclaw publishes it under
            # `packages.<system>.openclaw`, built against the upstream-
            # pinned nixpkgs (which is what cache.garnix.io's prebuild
            # uses too). We pass that derivation directly to the
            # home-manager `programs.openclaw.package` option instead
            # of trying to overlay nix-openclaw onto our own pkgs —
            # different pnpm version between the two nixpkgs revisions
            # produces a different pnpmDeps fixed-output hash and
            # breaks the build (see flake input comment for details).
            home-manager.extraSpecialArgs = {
              openclawPackage = inputs.nix-openclaw.packages.x86_64-linux.openclaw;
              codexPackage = inputs.codex-cli.packages.x86_64-linux.codex;
              coderabbitPackage = inputs.llm-agents.packages.x86_64-linux.coderabbit-cli;
            };
            home-manager.users.mattw = {
              imports = [
                ./shared/home.nix
                ./shared/linux.nix
                ./shared/dev.nix
                ./shared/linux-build-deps.nix
                ./shared/load-secrets.nix
                ./shared/privatefiles-symlinks.nix
                inputs.nix-openclaw.homeManagerModules.openclaw
                ./nixos/mattfw/home.nix
              ];
            };
          }
        ];
      };

      # mattserver — Old gaming PC (AMD Ryzen 3600 + RX 5700 XT, 32 GB DDR4).
      # Roles: ZFS backup receive target, self-hosted GitHub Actions runners
      # (personal + sealedsecurity org), KDE gaming station. Boots straight
      # to SDDM by default — flip `bootToDesktop` in desktop.nix to revert
      # to headless boot.
      nixosConfigurations."mattserver" = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = { inherit inputs; };
        modules = [
          nix-flatpak.nixosModules.nix-flatpak
          ./shared/overlays.nix
          ./nixos/common.nix
          ./nixos/mattserver/system.nix
          ./nixos/mattserver/desktop.nix
          home-manager.nixosModules.home-manager
          hmModule
          {
            home-manager.users.mattw = {
              imports = [
                ./shared/home.nix
                ./shared/linux.nix
                # No ./shared/load-secrets.nix — mattserver runs no
                # OP service-account tokens (see nixos/mattserver/INSTALL.md
                # "Security posture"). Secret loading via `op inject` would
                # have nothing to authenticate with.
                ./nixos/mattserver/home.nix
              ];
            };
          }
        ];
      };

      # Raspberry Pi 4 (headless DNS server: Technitium + Cockpit). Built
      # from this config via:
      #   nix build .#nixosConfigurations.rpi4.config.system.build.sdImage
      # Then dd the resulting image to an SD card and boot.
      nixosConfigurations."rpi4" = nixos-raspberrypi.lib.nixosSystem {
        specialArgs = { inherit inputs; };
        modules = [
          (
            { ... }:
            {
              imports = with nixos-raspberrypi.nixosModules; [
                raspberry-pi-4.base
                raspberry-pi-4.bluetooth
              ];
            }
          )
          ./shared/overlays.nix
          ./nixos/common.nix
          ./nixos/rpi4/system.nix
          ./nixos/rpi4/dns.nix
          home-manager.nixosModules.home-manager
          hmModule
          {
            home-manager.users.mattw = {
              imports = [
                ./shared/home.nix
                ./shared/linux.nix
                ./nixos/home.nix
                ./nixos/rpi4/home.nix
              ];
            };
          }
        ];
      };

      # mattpc-wsl — NixOS-WSL2 distro on the gaming PC (i9-14900KS + RTX
      # 4080 + 64 GB DDR5). Windows 11 is the bare-metal OS; this is the
      # Linux dev environment running under WSL2. Earlier hardware was
      # dual-booted with Bazzite — see git log for the bazzite/ tree.
      #
      # Tailscale runs on the Windows host; mirrored networking in
      # windows/.wslconfig makes the host's tailnet reachable from inside
      # WSL with no extra setup, so no tailscale daemon inside the distro.
      #
      # SSH lives on port 2222 inside WSL so it doesn't fight Windows's
      # OpenSSH on port 22 (which is what `ssh mattw@mattpc` hits for
      # `btm`-over-SSH system monitoring on the Windows side). With
      # mirrored networking, Windows-side `ssh -p 2222 mattw@localhost`
      # and tailnet `ssh -p 2222 mattw@mattpc` both reach the WSL distro.
      nixosConfigurations."mattpc-wsl" = inputs.nixpkgs-unstable.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = {
          inherit inputs;
          system = "x86_64-linux";
        };
        modules = [
          nixos-wsl.nixosModules.default
          ./shared/unstable-wholesale.nix
          ./nixos/common.nix
          ./nixos/mattpc-wsl/system.nix
          home-manager-unstable.nixosModules.home-manager
          hmModule
          {
            home-manager.extraSpecialArgs = {
              inherit inputs;
              codexPackage = inputs.codex-cli.packages.x86_64-linux.codex;
              coderabbitPackage = inputs.llm-agents.packages.x86_64-linux.coderabbit-cli;
            };
            home-manager.users.mattw = {
              imports = [
                ./shared/home.nix
                ./shared/linux.nix
                ./shared/dev.nix
                ./shared/linux-build-deps.nix
                ./shared/load-secrets.nix
                ./shared/privatefiles-symlinks.nix
                ./nixos/mattpc-wsl/home.nix
              ];
            };
          }
        ];
      };
    };
}

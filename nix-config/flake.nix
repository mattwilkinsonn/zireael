{
  description = "Matt's personal system configuration (MacBook Pro + WSL)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    # Base channel for the dev hosts via shared/unstable-wholesale.nix —
    # the Mac + WSL dev boxes pull every package through this.
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    # home-manager master drops support for stable nixpkgs (refers to lib
    # paths only in 26.05+); the dev hosts track unstable, so we use the
    # master branch following nixpkgs-unstable.
    home-manager-unstable = {
      url = "github:nix-community/home-manager/master";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/master";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    nixos-wsl = {
      url = "github:nix-community/NixOS-WSL/main";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    codex-cli.url = "github:sadjow/codex-cli-nix";
    llm-agents.url = "github:numtide/llm-agents.nix";
  };

  outputs =
    inputs@{
      home-manager-unstable,
      nix-darwin,
      nixos-wsl,
      ...
    }:
    let
      hmModule = {
        home-manager.useGlobalPkgs = true;
        home-manager.useUserPackages = true;
        home-manager.backupFileExtension = "hm-backup";
        # Make `inputs` available inside home-manager modules.
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
              inherit inputs;
            };
            home-manager.users.mattwilkinson = import ./darwin/home.nix;
          }
        ];
      };

      # mattpc-wsl — NixOS-WSL2 distro on the gaming PC (i9-14900KS + RTX
      # 4080 + 64 GB DDR5). Windows 11 is the bare-metal OS; this is the
      # Linux dev environment running under WSL2.
      #
      # Tailscale runs on the Windows host; mirrored networking in
      # windows/.wslconfig makes the host's tailnet reachable from inside
      # WSL with no extra setup, so no tailscale daemon inside the distro.
      #
      # SSH lives on port 2222 inside WSL so it doesn't fight Windows's
      # OpenSSH on port 22.
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

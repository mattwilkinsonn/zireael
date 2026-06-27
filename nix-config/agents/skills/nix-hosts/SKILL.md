---
name: nix-hosts
description: "Matt's nix-config flake — host configs, shared modules, secret loading, and the nix-switch converge loop."
---

# Nix Hosts

Matt's machines are declared in one flake under the `nix-config/` directory
inside the `~/repos/zireael` monorepo (`~/repos/zireael/nix-config/flake.nix`);
`nix-config` is not a separate repository checkout. It tracks `nixos-25.11`
plus a `nixos-unstable` channel; the dev hosts pull every package through
unstable via `shared/unstable-wholesale.nix`, and home-manager follows its
`master` branch.

## Hosts

Two hosts exist in `flake.nix` — do not assume others.

| Host | Output | Platform | Stack |
| --- | --- | --- | --- |
| `Matts-MacBook-Pro` | `darwinConfigurations` | `aarch64-darwin` | nix-darwin + home-manager |
| `mattpc-wsl` | `nixosConfigurations` | `x86_64-linux` | NixOS-WSL2 + home-manager |

- `Matts-MacBook-Pro` is the Apple Silicon MacBook Pro. User `mattwilkinson`.
- `mattpc-wsl` is the NixOS-WSL2 distro on the gaming PC; Windows 11 is the
  bare-metal OS and this is the Linux dev environment under WSL2. User `mattw`.
  SSH runs on port 2222 inside WSL to avoid Windows OpenSSH on 22; Tailscale
  runs on the Windows host (mirrored networking), so no daemon inside the distro.

## Shared modules (`shared/`)

Composed per host via home-manager `imports`:

- `home.nix` — cross-platform universal package set + zsh. Present on every host.
- `dev.nix` — heavy dev tier (toolchains, IaC, agent CLIs, linters). Imported only
  by boxes actually used for development.
- `load-secrets.nix` — 1Password-CLI secret loading; defines a re-runnable
  `load-secrets` shell function. On `op` failure it skips the export entirely
  (never sets an empty value) so each tool falls back to its own credential
  discovery instead of sending blank credentials.
- `privatefiles-symlinks.nix` — declarative `$HOME` symlinks via
  `mkOutOfStoreSymlink`, pointing at the live `~/repos/privatefiles/...` paths so
  edits propagate without rebuilding the flake. Covers agent instruction files
  and workspace meta.
- `unstable-wholesale.nix` — routes the dev hosts' package set through the
  unstable channel.
- `linux.nix`, `linux-build-deps.nix` — Linux-only runtime + build deps
  (pkg-config, openssl, dbus, mold, clang). The Mac uses Homebrew + Xcode CLI
  Tools for the equivalent.

## Per-host overlays

- `darwin/system.nix` — nix-darwin system config + Homebrew (`brews`, `casks`,
  `masApps`).
- `darwin/home.nix` — Mac home-manager; imports the shared modules and defines
  the `nix-switch` alias.
- `nixos/common.nix` — shared NixOS config.
- `nixos/mattpc-wsl/{system.nix,home.nix}` — WSL host overlay.

## Converge: `nix-switch`

`nix-switch` is the single re-converge command on every host (a zsh alias):

- Mac → `darwin-rebuild switch --flake ~/repos/zireael/nix-config#Matts-MacBook-Pro`.
- WSL → `nixos-rebuild switch` against the `mattpc-wsl` flake output.

Run it after editing any module. Because the privatefiles symlinks are
out-of-store, edits to the linked content (not the flake) take effect with no
rebuild.

## Package management priority

1. `shared/home.nix` `home.packages` — first choice, cross-platform Nix.
2. `shared/dev.nix` — dev-tier Nix tooling for dev hosts only.
3. Mac GUI apps → Homebrew **casks** in `darwin/system.nix` (Nix has no
   first-class macOS app bundles); CLI tools that nixpkgs lacks or lags → brews.
4. Linux build/runtime deps → `shared/linux*.nix`.

Prefer Nix; reach for Homebrew only for GUI casks and the few CLI gaps nixpkgs
can't cover on macOS.

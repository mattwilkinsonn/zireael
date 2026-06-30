# Nix Host Platform

## Overview

The `nix-config/` flake declares every machine Matt converges. Its
`description` names the scope as `"Matt's personal system configuration
(MacBook Pro + WSL)"` (`flake.nix:2`).

Two nixpkgs channels feed the flake:

- A **stable** channel pinned to `nixos-25.11`:
  `nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";`
  (`flake.nix:20`).
- An **unstable** channel:
  `nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";`
  (`flake.nix:23`).

home-manager follows its `master` branch — not a release tag —
`url = "github:nix-community/home-manager/master";` (`flake.nix:28`)
— and that input's nixpkgs is pinned to unstable via
`inputs.nixpkgs.follows = "nixpkgs-unstable";` (`flake.nix:29`). The
header comment records why: "home-manager master drops support for
stable nixpkgs (refers to lib paths only in 26.05+); the dev hosts
track unstable, so we use the master branch following nixpkgs-unstable"
(`flake.nix:24-26`).

Both declared hosts are **dev hosts**, and both pull their entire
package set through the unstable channel by importing
`shared/unstable-wholesale.nix` in their module list (`flake.nix:71`
for the Mac, `flake.nix:104` for WSL). That module replaces the
resolved package set wholesale:
`_module.args.pkgs = lib.mkForce (` (`unstable-wholesale.nix:41`)
followed by `import inputs.nixpkgs-unstable {` (`unstable-wholesale.nix:42`).
Its header explains the intent — "replace the resolved package set with
one evaluated directly from nixpkgs-unstable. Used by dev hosts ... so
every CLI tool lands at its latest packaged version without per-package
overlay entries" (`unstable-wholesale.nix:7-11`).

### Flake inputs

| Input | URL | nixpkgs follows | Source |
| --- | --- | --- | --- |
| `nixpkgs` | `github:nixos/nixpkgs/nixos-25.11` | — (stable root) | `flake.nix:20` |
| `nixpkgs-unstable` | `github:nixos/nixpkgs/nixos-unstable` | — (unstable root) | `flake.nix:23` |
| `home-manager-unstable` | `github:nix-community/home-manager/master` | `nixpkgs-unstable` | `flake.nix:27-29` |
| `nix-darwin` | `github:nix-darwin/nix-darwin/master` | `nixpkgs-unstable` | `flake.nix:31-33` |
| `nixos-wsl` | `github:nix-community/NixOS-WSL/main` | `nixpkgs` (stable) | `flake.nix:35-37` |
| `llm-agents` | `github:numtide/llm-agents.nix` | — | `flake.nix:39` |
| `hk` | `github:jdx/hk/v1.48.0` | `nixpkgs-unstable` | `flake.nix:40-42` |

The agent CLIs (`omp`, `codex`, `claude-code`, ...) resolve from
`llm-agents`, and the flake declares the numtide binary cache so
`nix-switch` substitutes them instead of compiling from source:
`extra-substituters = [ "https://cache.numtide.com" ];`
(`flake.nix:13`).

## Hosts

The flake declares **exactly two** host outputs. No other host exists in
`flake.nix`.

| Host | Output | Platform | User | Stack |
| --- | --- | --- | --- | --- |
| `Matts-MacBook-Pro` | `darwinConfigurations` | `aarch64-darwin` | `mattwilkinson` | nix-darwin + home-manager |
| `mattpc-wsl` | `nixosConfigurations` | `x86_64-linux` | `mattw` | NixOS-WSL2 + home-manager |

Grounding for each cell:

- **`Matts-MacBook-Pro`** — declared at
  `darwinConfigurations."Matts-MacBook-Pro" = nix-darwin.lib.darwinSystem {`
  (`flake.nix:64`); platform `system = "aarch64-darwin";`
  (`flake.nix:65`); user
  `home-manager.users.mattwilkinson = import ./darwin/home.nix;`
  (`flake.nix:81`); the home-manager stack is wired by
  `home-manager-unstable.darwinModules.home-manager` (`flake.nix:73`).
  The flake comment labels it "macOS (Apple Silicon) — nix-darwin +
  home-manager" (`flake.nix:63`).
- **`mattpc-wsl`** — declared at
  `nixosConfigurations."mattpc-wsl" = inputs.nixpkgs-unstable.lib.nixosSystem {`
  (`flake.nix:96`); platform `system = "x86_64-linux";`
  (`flake.nix:97`); the NixOS-WSL2 stack is wired by
  `nixos-wsl.nixosModules.default` (`flake.nix:103`) plus
  `home-manager-unstable.nixosModules.home-manager` (`flake.nix:107`);
  user `home-manager.users.mattw = {` (`flake.nix:113`). The flake
  comment labels it "mattpc-wsl — NixOS-WSL2 distro on the gaming PC
  ... Windows 11 is the bare-metal OS; this is the Linux dev
  environment running under WSL2" (`flake.nix:86-88`).

### mattpc-wsl WSL specifics

`nixos/mattpc-wsl/system.nix` records the WSL2 deviations from a normal
NixOS host: "No bootloader", "No filesystem declarations", SSH on
2222, and no tailscale daemon (`nixos/mattpc-wsl/system.nix:7-14`).
Concretely:

- `wsl = { enable = true; defaultUser = "mattw";`
  (`nixos/mattpc-wsl/system.nix:19-21`).
- `networking.hostName = "mattpc-wsl";`
  (`nixos/mattpc-wsl/system.nix:29`).
- SSH is forced off Windows's port 22:
  `services.openssh.ports = lib.mkForce [ 2222 ];`
  (`nixos/mattpc-wsl/system.nix:35`),
  because "Windows-side OpenSSH owns 22 ... and `mirrored` networking
  shares ports between Windows and WSL, so 22 would collide"
  (`nixos/mattpc-wsl/system.nix:31-34`).
- The firewall is disabled in favour of the Windows firewall:
  `networking.firewall.enable = false;`
  (`nixos/mattpc-wsl/system.nix:42`).

## Module composition

Each host's module list mixes per-host overlays with home-manager and
the shared modules. home-manager is wired once via the `hmModule`
binding —
`home-manager.useGlobalPkgs = true;` (`flake.nix:55`),
`home-manager.useUserPackages = true;` (`flake.nix:56`),
`home-manager.backupFileExtension = "hm-backup";` (`flake.nix:57`) —
used directly by `mattpc-wsl` (`flake.nix:108`); the Mac inlines the
same three settings (`flake.nix:75-77`).

Per-host shared-module imports:

| Module | Mac | mattpc-wsl |
| --- | --- | --- |
| `shared/unstable-wholesale.nix` | yes (`flake.nix:71`) | yes (`flake.nix:104`) |
| `shared/home.nix` | yes (`darwin/home.nix:14`) | yes (`flake.nix:115`) |
| `shared/dev.nix` | yes (`darwin/home.nix:15`) | yes (`flake.nix:117`) |
| `shared/load-secrets.nix` | yes (`darwin/home.nix:16`) | yes (`flake.nix:119`) |
| `shared/privatefiles-symlinks.nix` | yes (`darwin/home.nix:17`) | yes (`flake.nix:120`) |
| `shared/agent-config.nix` | yes (`darwin/home.nix:18`) | yes (`flake.nix:121`) |
| `shared/linux.nix` | no | yes (`flake.nix:116`) |
| `shared/linux-build-deps.nix` | no | yes (`flake.nix:118`) |

The Mac's shared imports come through `darwin/home.nix`, whose
`imports` block lists `../shared/home.nix`, `../shared/dev.nix`,
`../shared/load-secrets.nix`, `../shared/privatefiles-symlinks.nix`,
and `../shared/agent-config.nix` (`darwin/home.nix:13-19`). The WSL host
imports its shared set inline in the flake (`flake.nix:114-123`),
adding the two Linux-only modules the Mac omits.

The user identity is set per platform: `home.username = lib.mkForce
"mattwilkinson";` (`darwin/home.nix:21`) on the Mac, and
`username = "mattw";` with `homeDirectory = "/home/mattw";`
(`shared/linux.nix:19-20`) on Linux.

### Shared modules

| Module | Role | Evidence |
| --- | --- | --- |
| `shared/home.nix` | Universal cross-platform package set + zsh; present on every host | `shared/home.nix:35-39` |
| `shared/dev.nix` | Heavy "dev tier" — toolchains, agent CLIs, linters; dev hosts only | `dev.nix:9-15` |
| `shared/load-secrets.nix` | 1Password-CLI secret loading; defines re-runnable `load-secrets` | `load-secrets.nix:3-8`, `load-secrets.nix:137` |
| `shared/privatefiles-symlinks.nix` | Out-of-store `$HOME` symlinks into `~/repos/privatefiles` | `privatefiles-symlinks.nix:30-31` |
| `shared/agent-config.nix` | Out-of-store symlinks of agent config into `$HOME` | `agent-config.nix:26-27` |
| `shared/unstable-wholesale.nix` | Routes the package set through nixpkgs-unstable | `unstable-wholesale.nix:41-42` |
| `shared/linux.nix` | Linux-only fonts, GUI deps, and the Linux user identity | `linux.nix:18-22` |
| `shared/linux-build-deps.nix` | Linux-only C/C++/Rust/Tauri build environment | `linux-build-deps.nix:3-11` |

`shared/dev.nix` is scoped to development boxes: "Imported only by the
boxes that are actually used for development ... Headless / server
hosts ... get the universal `shared/home.nix` set only"
(`dev.nix:11-14`). `shared/linux-build-deps.nix` is likewise
Linux-only: "Imported only by Linux dev hosts ... Mac handles the
equivalent via brew + Xcode CLI Tools" (`linux-build-deps.nix:7-9`).

Both symlink modules use `mkOutOfStoreSymlink` so edits to the linked
content take effect without rebuilding the flake:
`linkOut = path: config.lib.file.mkOutOfStoreSymlink "${privatefiles}/${path}";`
(`privatefiles-symlinks.nix:31`) and
`linkAgent = path: config.lib.file.mkOutOfStoreSymlink "${agents}/${path}";`
(`agent-config.nix:27`). The privatefiles module header states the
payoff: "`nix-switch` becomes the single re-converge button for dev
hosts" (`privatefiles-symlinks.nix:14`).

### Requirement: Universal vs dev-tier package separation

The universal set SHALL live in `shared/home.nix` and be present on
every host; dev-tier tooling SHALL live in `shared/dev.nix` and be
imported only by development boxes.

#### Scenario: A tool is needed everywhere

- **Given** a package useful on every host (including headless ones).
- **When** it is declared.
- **Then** it is added to the universal set guarded by the comment
  "Packages — universal set, present on every host" and the list
  `home.packages = with pkgs; [` (`shared/home.nix:35-39`).

#### Scenario: A heavy dev toolchain is needed

- **Given** a toolchain, agent CLI, or linter only dev boxes use.
- **When** it is declared.
- **Then** it goes in `shared/dev.nix`'s
  `home.packages =` block (`dev.nix:32`), which the module documents as
  the "dev tier" imported only by dev boxes (`dev.nix:9-15`).

## Secret loading

### Requirement: 1Password-CLI secret loading

Secrets SHALL be loaded into the shell environment through the
1Password CLI by a re-runnable `load-secrets` shell function defined in
`shared/load-secrets.nix`. The module is "Cross-platform 1Password CLI
secret loading. Imported only on hosts that should auto-load API keys
into shell env" (`load-secrets.nix:3-4`), and the function is declared
at `load-secrets() {` (`load-secrets.nix:137`). It is defined "as a
function (not a one-shot block) so it can be re-run in the current
shell after unlocking / signing in" (`load-secrets.nix:28-30`).

The function SHALL run only when the `op` binary is present —
`if command -v op >/dev/null; then` (`load-secrets.nix:40`) — and each
`op` call SHALL be wrapped in a timeout so a locked or slow 1Password
cannot hang non-interactive login shells:
`typeset -ga _ot=(${pkgs.coreutils}/bin/timeout 8)`
(`load-secrets.nix:46`).

#### Scenario: op succeeds

- **Given** `op` is on PATH and a service-account token is set.
- **When** a login shell starts and finds a required key unset —
  `if [ -z "''${LINEAR_API_KEY:-}" ] || [ -z "''${OPENROUTER_API_KEY:-}" ]; then`
  (`load-secrets.nix:198`).
- **Then** it auto-invokes `load-secrets` (`load-secrets.nix:199`),
  which injects the rendered secrets into the environment.

#### Scenario: op fails or is missing

- **Given** `op` fails, times out, or is unavailable.
- **When** `load-secrets` runs.
- **Then** it SHALL skip the export entirely rather than set an empty
  value, so downstream tools fall back to their own credential
  discovery. The module calls this "the load-bearing bit: a bare
  `export FOO=$(op read … 2>/dev/null)` *sets* `FOO=""` when `op`
  fails ... Skipping the export entirely lets each tool fall back to
  its own credential discovery and surface a clear error"
  (`load-secrets.nix:32-39`).

### Per-host token provisioning

The `op` service-account tokens are provided per host, before
`load-secrets` runs:

- **Mac** reads them from the macOS Keychain in `.zprofile`:
  `export OP_SERVICE_ACCOUNT_TOKEN=$(security find-generic-password -a "$USER" -s "OP_SERVICE_ACCOUNT_TOKEN" -w 2>/dev/null || true)`
  (`darwin/home.nix:49`), plus the team token (`darwin/home.nix:50`).
  `op` itself ships with the 1Password app cask (see overlays).
- **mattpc-wsl** reads them from `0600` files via `envExtra`
  (`~/.zshenv`) so non-interactive shells see them too:
  `IFS= read -r OP_SERVICE_ACCOUNT_TOKEN < "$HOME/.config/op/service-account-token"`
  (`nixos/mattpc-wsl/home.nix:42-43`), plus the team token
  (`nixos/mattpc-wsl/home.nix:46-47`). `op` comes from
  `_1password-cli` in `nixos/common.nix:37`.

## Converge

### Requirement: Per-host converge command

Each host SHALL expose a single `nix-switch` zsh alias that rebuilds
and switches to that host's flake output. The alias is defined per
host (no shared default — "No default alias here so per-host overrides
aren't shadowed", `shared/linux.nix:54-59`).

#### Scenario: Converge the Mac

- **Given** the user runs `nix-switch` on `Matts-MacBook-Pro`.
- **When** the alias expands.
- **Then** it runs `darwin-rebuild switch` against the Mac output:
  `nix-switch = "sudo HOME=\"$HOME\" /nix/var/nix/profiles/system/sw/bin/darwin-rebuild switch --flake \"$HOME/repos/zireael/nix-config#Matts-MacBook-Pro\" --show-trace";`
  (`darwin/home.nix:26`).

#### Scenario: Converge mattpc-wsl

- **Given** the user runs `nix-switch` on `mattpc-wsl`.
- **When** the alias expands.
- **Then** it runs `nixos-rebuild switch` against the WSL output:
  `programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattpc-wsl\" --show-trace";`
  (`nixos/mattpc-wsl/home.nix:8`).

Both aliases use `sudo`, point at `$HOME/repos/zireael/nix-config#<host>`,
and pass `--show-trace`. Because the symlink modules are out-of-store,
edits to the linked content take effect with no rebuild
(`privatefiles-symlinks.nix:14`).

## Package management

### Requirement: Package-declaration priority

A package SHALL be declared in the first applicable layer below; later
layers are reserved for what earlier layers cannot express.

1. **`shared/home.nix` `home.packages`** — first choice, cross-platform
   Nix on every host (`shared/home.nix:39`).
2. **`shared/dev.nix` `home.packages`** — dev-tier Nix tooling, dev
   hosts only (`dev.nix:32`; scope at `dev.nix:9-15`).
3. **Homebrew on the Mac** — GUI apps go in `casks`
   (`darwin/system.nix:141`; e.g. `"1password"`, `"claude"`,
   `"ghostty"`, `darwin/system.nix:142-157`); CLI tools nixpkgs
   lacks or lags go in `brews` (`darwin/system.nix:112`). The
   Homebrew block is owned by nix-darwin:
   `homebrew = { enable = true;` (`darwin/system.nix:88-89`).
4. **`shared/linux*.nix`** — Linux runtime and build deps:
   `shared/linux.nix`'s `packages = with pkgs; [` (`linux.nix:22`) and
   `shared/linux-build-deps.nix`'s `home.packages = with pkgs; [`
   (`linux-build-deps.nix:11`).

#### Scenario: A macOS GUI app is needed

- **Given** a GUI application with no first-class Nix macOS bundle.
- **When** it is declared for the Mac.
- **Then** it is added to `casks` in `darwin/system.nix:141`, not to a
  `home.packages` list.

#### Scenario: A Rust/C build dependency is needed on Linux

- **Given** a `.dev` output or linker the Linux build path needs.
- **When** it is declared.
- **Then** it goes in `shared/linux-build-deps.nix`
  (`linux-build-deps.nix:11`), described as the "Linux-only C/C++ build
  environment" the Mac instead covers "via brew + Xcode CLI Tools"
  (`linux-build-deps.nix:3-9`).

## Per-host overlays

- **`darwin/system.nix`** — nix-darwin system config. Determinate Nix
  owns the daemon, so nix-darwin's management is off:
  `nix.enable = false;` (`darwin/system.nix:5`). It defines the Homebrew
  block (`darwin/system.nix:88`) and sets
  `system.primaryUser = "mattwilkinson";` (`darwin/system.nix:283`).
- **`darwin/home.nix`** — Mac home-manager entrypoint: imports the
  shared modules (`darwin/home.nix:13-19`) and defines the Mac
  `nix-switch` alias (`darwin/home.nix:26`).
- **`nixos/common.nix`** — shared NixOS config. Enables flakes and
  trusts `mattw`: `trusted-users = [ "root" "mattw" ];`
  (`common.nix:9-12`) and `accept-flake-config = true;`
  (`common.nix:20`) — together these let `nix-switch` honour the
  flake's `extra-substituters` and pull the agent CLIs from the binary
  cache instead of rebuilding. It installs `_1password-cli`
  (`common.nix:37`) and sets `system.stateVersion = "25.11";`
  (`common.nix:221`).
- **`nixos/mattpc-wsl/system.nix`** — the WSL host overlay (WSL enable,
  hostname, SSH port, firewall — see WSL specifics above).
- **`nixos/mattpc-wsl/home.nix`** — WSL home-manager overlay: the
  `nix-switch` alias (`nixos/mattpc-wsl/home.nix:8`) and token
  export (`nixos/mattpc-wsl/home.nix:42-47`).

## Notes: source vs. skill

Per the evidence rule, the source wins where it disagrees with
`skill://nix-hosts` or with stale in-tree comments:

- **Host count.** `skill://nix-hosts` and several module comments
  reference a broader fleet (`mattfw`, `mattserver`, `mattmini`,
  `rpi4`, `rpi5`) — e.g. `unstable-wholesale.nix:13-16`,
  `dev.nix:11-14`, `privatefiles-symlinks.nix:23-26`. Those hosts are
  **not** declared in this `flake.nix`, which defines exactly the two
  outputs above (`flake.nix:64`, `flake.nix:96`). This spec describes
  only those two.
- **Binary cache identity.** `nixos/common.nix:13-19` comments that the
  flake "advertises the nixos-raspberrypi cachix in nixConfig". The
  actual `nixConfig` advertises the numtide cache:
  `extra-substituters = [ "https://cache.numtide.com" ];`
  (`flake.nix:13`). The comment is stale; the source is authoritative.

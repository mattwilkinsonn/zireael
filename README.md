# zireael

Personal monorepo: CLI tools I maintain + the Nix configuration that
provisions every machine I run. The CLI tools are listed first because
they're what most external visitors are here for; `nix-config/` is the
larger but more personal half.

## CLI tools

| Tool | Path | Language | Description |
| --- | --- | --- | --- |
| `jj-hooks` / `jj-hp` | [`tools/jj-hooks`](./tools/jj-hooks) | Rust | Run pre-commit / lefthook / hk hooks against [jj](https://github.com/jj-vcs/jj) bookmark pushes. |
| `jj-gt` | [`tools/jj-gt`](./tools/jj-gt) | Rust | Bridge jj bookmark stacks and [Graphite](https://graphite.dev) PR stacks. |
| `akiflow-cli` (`af`) | [`tools/akiflow-cli`](./tools/akiflow-cli) | TypeScript / Bun | [Akiflow](https://akiflow.com) task-management CLI (fork of [`code-yeongyu/akiflow-cli`](https://github.com/code-yeongyu/akiflow-cli)). |
| Homebrew formulae | [`Formula`](./Formula) | Ruby | `brew install mattwilkinsonn/zireael/<tool>`. |

### Install

#### Homebrew (any tool)

```bash
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-hooks   # or jj-gt, or akiflow-cli
```

#### Cargo (Rust tools)

##### Recommended - `cargo binstall`

Install [`cargo binstall` first](https://github.com/cargo-bins/cargo-binstall)

```bash
cargo binstall jj-hooks   # ships `jj-hooks` + `jj-hp`
cargo binstall jj-gt
```

`binstall` looks for binaries on GitHub Releases first, and falls back to
regular `install` if it can't find one for your architecture. 99% of the
time it will just install the prebuilt binary, saving you the compile
time.

##### Standard `cargo install`

If you don't want to use `binstall,` you can always compile yourself with `cargo install`.

```bash
cargo install jj-hooks   # ships `jj-hooks` + `jj-hp`
cargo install jj-gt
```

#### Manual download

Each tagged release attaches `tar.gz` binaries for `darwin-arm64`,
`linux-x64`, and `linux-arm64` to its GitHub Release. Grab them at
<https://github.com/mattwilkinsonn/zireael/releases>.

## Nix configuration

[`nix-config/`](./nix-config) is a multi-host
[Nix flake](https://nix.dev/concepts/flakes.html) covering my personal
machines. The sealedsecurity fleet (CI runners + the inference box)
lives in the company `sealed` repo under `infra/nix/` and consumes
this flake's `shared/` modules as an input, so `home.nix`, overlays,
and `nixos/common.nix` stay single-sourced here.

| Host | Platform | Role | Flake target |
| --- | --- | --- | --- |
| Matts-MacBook-Pro | aarch64 darwin | Primary dev box | `darwinConfigurations.Matts-MacBook-Pro` |
| mattpc-wsl | x86_64 NixOS (WSL2) | Windows-host dev environment | `nixosConfigurations.mattpc-wsl` |

Manages the OS configuration ([`nixos/`](./nix-config/nixos),
[`darwin/`](./nix-config/darwin)) and the user-level home-manager
configuration ([`shared/home.nix`](./nix-config/shared/home.nix) +
per-host overrides). Loose dotfiles
([`nix-config/dotfiles/`](./nix-config/dotfiles)) — bashrc, jj config,
yabai / skhd scripts, Claude Code settings, etc. — are materialized
by home-manager as symlinks into `$HOME` at activation.

Windows-side bootstrap ([`nix-config/windows/`](./nix-config/windows))
covers the WSL host's pre-NixOS provisioning (winget DSC, PowerShell
profile, WSL config).

Private content (Tailscale ACL, agent instruction files like CLAUDE.md
/ SEAL.md, sealedsecurity workspace meta files) lives in a separate
private repo at `~/repos/privatefiles/` and is symlinked into place;
it is intentionally not under nix-config so this repo can stay public.

### Migrating a host

For hosts already running my older "dotfiles repo at $HOME" layout,
[`nix-config/shared/scripts/migrate-from-dotfiles.sh`](./nix-config/shared/scripts/migrate-from-dotfiles.sh)
is the one-shot migration: clones zireael (+ privatefiles for dev
boxes), authors the symlinks, rebuilds against the new flake path,
archives the old `~/.git` / `~/.jj` for safe deletion later.

### Bootstrapping a new host

Per-platform bootstrap scripts under
[`nix-config/{darwin,nixos}/scripts/`](./nix-config) handle fresh
installs: Determinate Nix installer, gh auth + zireael clone, first
`darwin-rebuild` / `nixos-rebuild switch`. See the per-host
`INSTALL.md` files for the click-through walkthroughs (Framework
laptop, Mac mini, mattserver, mattlinuxpro, rpi4/5, mattpc-wsl,
Matts-MacBook-Pro).

## Development

```bash
just install-deps   # one-time: installs rust toolchain, hk, jj, gh, gt, bun, ...
just ci             # auto-detect which tools' paths the working-copy diff touches
                    # and run only those tools' CI suites. Mirrors the per-tool
                    # GitHub workflow exactly — same recipes, same checks.
just ci-all         # unconditional: every tool's full suite.
```

CI shape:

+ Per-tool workflows path-filtered (`jj-hooks.yml`, `jj-gt.yml`,
  `akiflow-cli.yml`, `tap.yml`, `nix-config.yml`).
+ `ci-base-rust.yml` + `ci-base-nix.yml` are reusable workflow templates.
+ Per-host `nix eval` jobs gated on per-host path filters in
  [`.github/path-filters/nix-config-hosts.yml`](./.github/path-filters/nix-config-hosts.yml)
  — an edit to one host's files only triggers that host's eval, not the
  whole 7-host matrix. Nightly runs the full matrix unconditionally to
  catch upstream flake-input drift.

Per-tool recipes are delegated to each tool's own `Justfile`. See
[`tools/<name>/README.md`](./tools/) for tool-specific details.

## Repository history

This repo replaces these previous standalone repos:

+ [`mattwilkinsonn/jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks) → `tools/jj-hooks`
+ [`mattwilkinsonn/jj-gt`](https://github.com/mattwilkinsonn/jj-gt) → `tools/jj-gt`
+ [`mattwilkinsonn/akiflow-cli`](https://github.com/mattwilkinsonn/akiflow-cli) (fork) → `tools/akiflow-cli`
+ [`mattwilkinsonn/homebrew-tap`](https://github.com/mattwilkinsonn/homebrew-tap) → `Formula`
+ [`mattwilkinsonn/dotfiles`](https://github.com/mattwilkinsonn/dotfiles)
  (private; archived after every host migrates) → `nix-config`

The CLI-tool repos are archived on GitHub and link back here. The
dotfiles repo retires when the last host's migration completes.

## License

Dual-licensed under MIT OR Apache-2.0. See [`LICENSE.md`](./LICENSE.md) for
details. `tools/akiflow-cli/` carries an extra MIT-only exception inherited
from its upstream.

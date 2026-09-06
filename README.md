# zireael

Personal monorepo: the public CLI tools I maintain.

## CLI tools

The Rust CLI tools have moved to their own standalone repos; this monorepo is
being repurposed. Each tool now lives at, and is installed from, its own repo:

| Tool | Repo | Language | Description |
| --- | --- | --- | --- |
| `jj-hooks` / `jj-hp` | [`mattwilkinsonn/jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks) | Rust | Run pre-commit / lefthook / hk hooks against [jj](https://github.com/jj-vcs/jj) bookmark pushes. |
| `jj-gt` | [`mattwilkinsonn/jj-gt`](https://github.com/mattwilkinsonn/jj-gt) | Rust | Bridge jj bookmark stacks and [Graphite](https://graphite.dev) PR stacks. |

### Install

#### Homebrew

```bash
brew tap mattwilkinsonn/tap
brew install mattwilkinsonn/tap/jj-hooks   # ships jj-hooks + jj-hp
brew install mattwilkinsonn/tap/jj-gt
```

> **Migrating off an old tap.** If you previously tapped
> `mattwilkinsonn/zireael` (or a per-tool tap), uninstall the tools and untap
> it before tapping `mattwilkinsonn/tap` — Homebrew refuses to install a
> same-named formula from a second tap while the old one is still installed:
>
> ```bash
> brew uninstall jj-hooks jj-gt      # only the ones you actually installed
> brew untap mattwilkinsonn/zireael  # only the taps you actually added —
> brew untap mattwilkinsonn/jj-hooks # `brew tap` lists yours
> brew untap mattwilkinsonn/jj-gt
> brew tap mattwilkinsonn/tap
> brew install mattwilkinsonn/tap/jj-hooks mattwilkinsonn/tap/jj-gt
> ```

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

Each tool's tagged releases attach `tar.gz` binaries for `darwin-arm64`,
`linux-x64`, and `linux-arm64`. Grab them from the standalone repo's Releases
page: [jj-hooks](https://github.com/mattwilkinsonn/jj-hooks/releases) /
[jj-gt](https://github.com/mattwilkinsonn/jj-gt/releases).

## Development

```bash
direnv allow        # one-time: enters the devenv shell (rust, bun, node, moon, hk, jj, linters)
moon ci             # run every affected task (the same gate CI + the pre-push hook run)
moon run <p>:ci     # one project's checks (p = jj-hooks | jj-gt | tap | root)
```

CI shape: one workflow ([`.github/workflows/ci.yml`](./.github/workflows/ci.yml)) runs `moon ci` in the devenv shell on every PR.

+ [`nightly.yml`](./.github/workflows/nightly.yml) runs the full matrix daily; [`post-merge.yml`](./.github/workflows/post-merge.yml) re-runs the affected gate on `main`.
+ Toolchains are pinned in [`.prototools`](./.prototools) (bun/node/moon) + [`rust-toolchain.toml`](./rust-toolchain.toml) and provided by [`devenv.nix`](./devenv.nix).

## Docs

[`docs/`](./docs) holds two kinds of document, mirrored by domain:

+ **`designs/<domain>/`** — point-in-time **design records** (the *why*; frozen once decided).
+ **`specs/<domain>/`** — the **living source of truth** for how a component *currently* behaves.

Domains: `platform/` (CI), `tools/` (jj-gt, jj-hooks), `agents/` (multi-agent coordination). A behavior change updates the matching `specs/` doc **in the same PR**; see [`docs/README.md`](./docs/README.md).

## Repository history

These tools were briefly consolidated into this monorepo, then moved back to
their own standalone repos:

+ [`mattwilkinsonn/jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks) (`jj-hooks` / `jj-hp`)
+ [`mattwilkinsonn/jj-gt`](https://github.com/mattwilkinsonn/jj-gt)

The full monorepo-era history of each tool is preserved here; ongoing work
happens in the standalone repos.

## License

Dual-licensed under MIT OR Apache-2.0. See [`LICENSE.md`](./LICENSE.md) for
details.

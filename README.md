# zireael

Monorepo for personal CLI tools.

| Tool | Path | Language | Description |
| --- | --- | --- | --- |
| `jj-hooks` / `jj-hp` | [`tools/jj-hooks`](./tools/jj-hooks) | Rust | Run pre-commit / lefthook / hk hooks against [jj](https://github.com/jj-vcs/jj) bookmark pushes. |
| `jj-gt` | [`tools/jj-gt`](./tools/jj-gt) | Rust | Bridge jj bookmark stacks and [Graphite](https://graphite.dev) PR stacks. |
| `akiflow-cli` (`af`) | [`tools/akiflow-cli`](./tools/akiflow-cli) | TypeScript / Bun | [Akiflow](https://akiflow.com) task-management CLI (fork of [`code-yeongyu/akiflow-cli`](https://github.com/code-yeongyu/akiflow-cli)). |
| Homebrew formulae | [`Formula`](./Formula) | Ruby | `brew install mattwilkinsonn/zireael/<tool>`. |

## Install

### Homebrew (any tool)

```bash
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-hooks   # or jj-gt, or akiflow-cli
```

### Cargo (Rust tools)

#### Recommended - `cargo binstall`

Install [`cargo binstall` first](https://github.com/cargo-bins/cargo-binstall)

```bash
cargo binstall jj-hooks   # ships `jj-hooks` + `jj-hp`
cargo binstall jj-gt
```

`binstall` looks for binaries on GitHub Releases first, and falls back to
regular `install` if it can't find one for your architecture. 99% of the
time it will just install the prebuilt binary, saving you the compile
time.

#### Standard `cargo install`

If you don't want to use `binstall,` you can always compile yourself with `cargo install`.

```bash
cargo install jj-hooks   # ships `jj-hooks` + `jj-hp`
cargo install jj-gt
```

### Manual download

Each tagged release attaches `tar.gz` binaries for `darwin-arm64`,
`linux-x64`, and `linux-arm64` to its GitHub Release. Grab them at
<https://github.com/mattwilkinsonn/zireael/releases>.

## Development

```bash
just install-deps   # one-time: installs rust toolchain, hk, jj, gh, gt, bun, ...
just ci             # auto-detect which tools' paths the working-copy diff touches
                    # and run only those tools' CI suites. Mirrors the per-tool
                    # GitHub workflow exactly — same recipes, same checks.
just ci-all         # unconditional: every tool's full suite.
```

Per-tool recipes are delegated to each tool's own `Justfile`. See
[`tools/<name>/README.md`](./tools/) for tool-specific details.

## Repository structure

This repo replaces the previous standalone repos:

- [`mattwilkinsonn/jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks) → `tools/jj-hooks`
- [`mattwilkinsonn/jj-gt`](https://github.com/mattwilkinsonn/jj-gt) → `tools/jj-gt`
- [`mattwilkinsonn/akiflow-cli`](https://github.com/mattwilkinsonn/akiflow-cli) (fork) → `tools/akiflow-cli`
- [`mattwilkinsonn/homebrew-tap`](https://github.com/mattwilkinsonn/homebrew-tap) → `Formula`

All four are archived on GitHub and link back here.

## License

Dual-licensed under MIT OR Apache-2.0. See [`LICENSE.md`](./LICENSE.md) for
details. `tools/akiflow-cli/` carries an extra MIT-only exception inherited
from its upstream.

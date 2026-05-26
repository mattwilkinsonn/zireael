# zireael

Monorepo for personal CLI tools.

| Tool | Path | Language | Description |
|---|---|---|---|
| `jj-hooks` / `jj-hp` | [`tools/jj-hooks`](./tools/jj-hooks) | Rust | Run pre-commit / lefthook / hk hooks against [jj](https://github.com/jj-vcs/jj) bookmark pushes. |
| `jj-gt` | [`tools/jj-gt`](./tools/jj-gt) | Rust | Bridge jj bookmark stacks and [Graphite](https://graphite.dev) PR stacks. |
| `akiflow-cli` (`af`) | [`tools/akiflow-cli`](./tools/akiflow-cli) | TypeScript / Bun | [Akiflow](https://akiflow.com) task-management CLI (fork of [`code-yeongyu/akiflow-cli`](https://github.com/code-yeongyu/akiflow-cli)). |
| Homebrew formulae | [`tap/Formula`](./tap/Formula) | Ruby | `brew install mattwilkinsonn/zireael/<tool>`. |

## Install

### Homebrew (any tool)

```bash
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-hooks   # or jj-gt, or akiflow-cli
```

### Cargo (Rust tools)

```bash
cargo install jj-hooks   # ships `jj-hooks` + `jj-hp`
cargo install jj-gt
```

Or via `cargo binstall jj-hooks` / `cargo binstall jj-gt` for prebuilt
binaries (no compile).

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

- ~~`mattwilkinsonn/jj-hooks`~~ → `tools/jj-hooks`
- ~~`mattwilkinsonn/jj-gt`~~ → `tools/jj-gt`
- ~~`mattwilkinsonn/akiflow-cli` (fork)~~ → `tools/akiflow-cli`
- ~~`mattwilkinsonn/homebrew-tap`~~ → `tap/Formula`

All four are archived on GitHub and link back here.

## License

Dual-licensed under MIT OR Apache-2.0. See [`LICENSE.md`](./LICENSE.md) for
details. `tools/akiflow-cli/` carries an extra MIT-only exception inherited
from its upstream.

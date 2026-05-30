# Homebrew tap

Formulae for [zireael](../README.md) tools, installable via:

```bash
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-hooks   # or jj-gt, or akiflow-cli
```

## Available formulae

| Formula | Binaries | Notes |
| --- | --- | --- |
| `jj-hooks` | `jj-hooks`, `jj-hp` | Runs pre-commit / lefthook / hk hooks against jj bookmark pushes. |
| `jj-gt` | `jj-gt` | Bridges jj bookmark stacks and Graphite (gt) PR stacks. |
| `akiflow-cli` | `af` | Akiflow task-management CLI (fork of `code-yeongyu/akiflow-cli`). |

## How releases work

Each formula tracks the GitHub Release tarballs attached to a `v*` tag on
this monorepo. The `release.yml` workflow at the repo root:

1. Builds release binaries for `darwin-arm64`, `linux-x64`, `linux-arm64`.
2. Computes SHA256 for every tarball.
3. Rewrites the `version` line and per-platform `sha256` strings in each
   `Formula/*.rb` in-place.
4. Commits the bumped formulae back to `main` and creates the GitHub Release.

**Don't hand-edit the `version` or `sha256` lines** — the workflow owns them.

## Adding a new tool

When a new `tools/<name>/` lands and needs a Homebrew formula:

1. Add a stub `Formula/<name>.rb` here with `version "0.0.0"` and four
   zero-filled `sha256` values.
2. Add the tool's binary build to `.github/workflows/release.yml`'s matrix.
3. Add a per-formula bump step to the same workflow.
4. Tag the next release — the workflow will fill in the real values.

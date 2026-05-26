# Homebrew tap

Formulae in this directory ship via the `mattwilkinsonn/zireael` tap:

```bash
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-hooks   # or jj-gt, or akiflow-cli
```

Each formula tracks the GitHub Release tarballs attached to a `v*` tag on
this repo. The `release.yml` workflow rewrites the per-platform `sha256`
and `version` fields automatically — don't hand-edit them.

Formulae land in subsequent phases of the monorepo migration.

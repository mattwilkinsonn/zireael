# Homebrew formulae (retired)

These formulae are **retired** and frozen at the last monorepo release
(0.3.11). `jj-hooks` and `jj-gt` now ship from the consolidated
[`mattwilkinsonn/tap`](https://github.com/mattwilkinsonn/homebrew-tap):

```bash
brew tap mattwilkinsonn/tap
brew install mattwilkinsonn/tap/jj-hooks mattwilkinsonn/tap/jj-gt
```

If you still have the old tap installed, uninstall and untap it first —
Homebrew refuses to install a same-named formula from a second tap while the
old one is still installed:

```bash
brew uninstall jj-hooks jj-gt      # only the ones you actually installed
brew untap mattwilkinsonn/zireael
brew tap mattwilkinsonn/tap
brew install mattwilkinsonn/tap/jj-hooks mattwilkinsonn/tap/jj-gt
```

This repo is going private, after which these formulae stop resolving and
their release assets stop downloading. They carry no Homebrew `disable!`
stamp. A stamp is only read after `brew update` refreshes the tap clone, and
Homebrew refreshes it on the same commands that fetch the tarball
(`brew install`, `brew upgrade`, `brew outdated`), so for almost everyone the
stamp would fire on the same invocation as the download failure and buy
nothing. Both tools moved to their own standalone repos
([`jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks),
[`jj-gt`](https://github.com/mattwilkinsonn/jj-gt)), whose READMEs document
the same install path.

## Available formulae

| Formula | Binaries | Notes |
| --- | --- | --- |
| `jj-hooks` | `jj-hooks`, `jj-hp` | Runs pre-commit / lefthook / hk hooks against jj bookmark pushes. |
| `jj-gt` | `jj-gt` | Bridges jj bookmark stacks and Graphite (gt) PR stacks. |

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
```

This repo is going private, so these files stop being fetchable and carry no
`disable!` stamp: a stamp only reaches machines that run `brew update` while
the tap is still readable, which is a window this repo will not have. Anyone
left on the old tap sees a download failure instead, and recovers with the
commands above. Both tools moved to their own standalone repos
([`jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks),
[`jj-gt`](https://github.com/mattwilkinsonn/jj-gt)), whose READMEs document
the same install path.

## Available formulae

| Formula | Binaries | Notes |
| --- | --- | --- |
| `jj-hooks` | `jj-hooks`, `jj-hp` | Runs pre-commit / lefthook / hk hooks against jj bookmark pushes. |
| `jj-gt` | `jj-gt` | Bridges jj bookmark stacks and Graphite (gt) PR stacks. |

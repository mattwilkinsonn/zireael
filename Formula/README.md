# Homebrew formulae (retired)

These formulae are **retired**. `jj-hooks` and `jj-gt` now ship from the
consolidated [`mattwilkinsonn/tap`](https://github.com/mattwilkinsonn/homebrew-tap):

```bash
brew tap mattwilkinsonn/tap
brew install mattwilkinsonn/tap/jj-hooks   # or jj-gt
```

Both `Formula/*.rb` here carry a Homebrew `disable!` stamp, so a `brew`
command against this tap errors out and points at the tap above. If you
previously tapped `mattwilkinsonn/zireael`, uninstall the tools and untap it
before tapping the new one — Homebrew refuses to install a same-named formula
from a second tap while the old one is still installed:

```bash
brew uninstall jj-hooks jj-gt      # only the ones you actually installed
brew untap mattwilkinsonn/zireael
brew tap mattwilkinsonn/tap
brew install mattwilkinsonn/tap/jj-hooks mattwilkinsonn/tap/jj-gt
```

## Available formulae

| Formula | Binaries | Notes |
| --- | --- | --- |
| `jj-hooks` | `jj-hooks`, `jj-hp` | Runs pre-commit / lefthook / hk hooks against jj bookmark pushes. |
| `jj-gt` | `jj-gt` | Bridges jj bookmark stacks and Graphite (gt) PR stacks. |

## Why these are retired

Both tools moved to their own standalone repos
([`jj-hooks`](https://github.com/mattwilkinsonn/jj-hooks),
[`jj-gt`](https://github.com/mattwilkinsonn/jj-gt)) and now release through the
consolidated `mattwilkinsonn/tap`. The formulae here are frozen at their last
monorepo release (0.3.11) and `disable!`-stamped; they receive no further
updates. Install from `mattwilkinsonn/tap` (top of this file).

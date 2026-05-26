# jj-gt

Bridge [jj](https://jj-vcs.github.io) bookmark stacks and
[Graphite](https://graphite.dev/) (`gt`) PR stacks in one command.

`jj` doesn't know about gt's stack model. gt tracks stack relationships via
per-branch `refs/branch-metadata/<branch>` git refs that record each branch's
parent. jj doesn't create or maintain these refs, so:

- `gt submit` doesn't know stack parents — every PR targets `main`.
- `gt log` shows a flat list, not a stack.
- The Graphite web app doesn't render the stack widget on PRs.

`jj-gt` automates the gt-track step by walking jj's revset graph to derive
`(branch, parent_branch)` pairs, then drives `gt submit --stack` end-to-end.
One command, full stack.

It also fills two other gaps:

- `jj-gt fetch` is a Graphite-aware replacement for `jj git fetch` —
  fetches trunk, backfills tracking metadata, runs `gt sync` for branch
  cleanup, restacks orphaned children with `jj rebase`, and prunes
  `gtmq_*` queue-test artifacts.
- `jj-gt status` prints stack-wide PR + queue state for the current
  stack in one query.

## What jj-gt is and isn't

**IT IS.** A glue layer that does jj↔gt impedance matching for three
workflows:

- **Wraps `gt submit`** (`jj-gt submit`) — sets up metadata refs, runs
  hooks against the right diff range, invokes `gt submit --stack`, and
  restores `@`. gt still does the actual push; we drive it.
- **Composes `gt sync` + `jj rebase` + local cleanup** (`jj-gt fetch`).
- **Queries `gh` for stack-wide PR state** (`jj-gt status`).

**IT ISN'T.**

- Not a stack editor. Use jj directly (`jj split`, `jj rebase -s`,
  `jj squash`, etc.).
- Not a queue manager. Use the Graphite web app or `gt`/`gh` directly.
- Not a `jj` extension. No `.jj`-internal knowledge beyond what
  `jj log`/`jj bookmark list`/`jj git export` give us.
- Not a `gt` replacement. `gt log`, `gt modify`, `gt merge`,
  `gt unbranch` all stay as-is.
- Not a hook runner. Hook execution comes from the `jj_hooks` library
  (the lib half of [jj-hooks](https://github.com/mattwilkinsonn/jj-hooks)).

## Installation

### Via cargo binstall (recommended)

```bash
cargo binstall jj-gt
```

This pulls a prebuilt binary from the GitHub Releases page — no compile
step.

### Via Homebrew tap

```bash
brew install mattwilkinsonn/tap/jj-gt
```

### From source

```bash
jj git clone https://github.com/mattwilkinsonn/jj-gt
cargo install --path .
```

### Prereqs

- [`jj`](https://jj-vcs.github.io) on PATH.
- [`gt`](https://graphite.dev/docs/graphite-cli) on PATH
  (`npm i -g @withgraphite/graphite-cli`).
- [`gh`](https://cli.github.com) on PATH, authenticated against the
  remote that hosts your PRs.
- The repo must already be tracked by Graphite — run `gt init` once.

## Usage

```text
USAGE:
    jj-gt <COMMAND>

COMMANDS:
    submit      Track + submit selected bookmarks as a stack
                (drives `gt submit --stack` end-to-end).
    track       Sync refs/branch-metadata/* without submitting
                (manual escape hatch — same logic as submit minus the
                gt-submit invocation).
    fetch       Graphite-aware replacement for `jj git fetch`: fetches
                trunk, backfills metadata refs, runs `gt sync` for
                branch cleanup, restacks orphaned children with `jj
                rebase`, prunes `gtmq_*` queue-test artifacts, and
                falls back to merge-marker scan for PRs gt sync misses.
    status      Print stack-wide PR + queue state in stack order.
    log         Print the derived stack as jj-gt sees it (debug).
    init        Print suggested aliases + setup reminders.
    completions Emit a shell completion script.
```

### Examples

```bash
# Submit the whole current stack
jj-gt submit --all

# Submit two specific bookmarks as a stack
jj-gt submit -b bottom--athena -b top--athena

# Submit as draft PRs, set merge-when-ready
jj-gt submit --all --draft --merge-when-ready

# Preview what would happen
jj-gt submit --all --dry-run

# Graphite-aware fetch
jj-gt fetch

# Show the stack with PR + queue state
jj-gt status
```

### Tab completion

```bash
# zsh
eval "$(jj-gt completions zsh)"
# bash
eval "$(jj-gt completions bash)"
# fish
jj-gt completions fish | source
```

Dynamic completers TAB-expand bookmark and remote names by shelling
back into `jj-gt` with `COMPLETE=<shell>` set — no jj working-copy
snapshot per keypress (uses `--ignore-working-copy`).

## How parent derivation works

For each selected bookmark `B`, jj-gt asks jj which other bookmark sits
immediately upstream on the current graph. One revset per bookmark:

```text
jj log -r 'heads(::<B> & bookmarks() ~ <B> ~ ::<trunk>)' \
       -T 'bookmarks.map(|b| b.name()).join("\n") ++ "\n"' \
       --no-graph
```

Reads as: "find the head commit(s) of the set of bookmarks that are
ancestors of `<B>`, excluding `<B>` itself, and also excluding everything
that's already an ancestor of trunk."

The output is the parent bookmark name(s) — usually one, zero if the
bookmark sits directly on trunk.

## License

Apache-2.0. See [LICENSE](./LICENSE).

## Development

`cargo nextest run` runs the full test suite (unit + integration). The
suite is partitioned into three tiers:

1. **Unit tests** — pure functions, no subprocess calls. Always run.
2. **`tests/gt_live.rs`** — drives `gt track` against a tempdir
   colocated jj+git repo. Needs `gt` + `jj` on PATH; no network.
   Skipped silently if either binary is missing.
3. **`tests/gh_live.rs` + `tests/gt_submit_live.rs`** — hit real
   GitHub. Off by default; opt in via env vars (see below).

### One-time setup for the live GitHub tests

```bash
just setup-live-fixture
```

This creates `<your-gh-user>/jj-gt-live-tests` on github, pushes a
trivial main, and opens one persistent fixture PR for the gh test to
query against. Idempotent — re-running on an existing setup is a no-op.

Once the fixture exists:

```bash
just test-live-gh       # gh pr list smoke
just test-live-submit   # full jj-gt submit end-to-end; creates + closes 2 PRs per run
```

Both recipes set the required env vars (`JJ_GT_LIVE_GH=1`,
`JJ_GT_LIVE_SUBMIT=1`, `JJ_GT_LIVE_REPO`, `JJ_GT_LIVE_REPO_URL`)
automatically; override on the command line if you want to point
them at a different fixture repo.

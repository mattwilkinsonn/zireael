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

**The submit promise.** `jj-gt submit`'s goal is *"make Graphite's
state match what you currently have locally in jj, every time."* That
means a few opinionated defaults you might want to opt out of:

- `gt submit --always` is on by default — gt re-pushes every PR's
  base ref even when it thinks nothing changed. The opt-out is
  `--no-always`. Without this, gt's skip-unchanged heuristic can
  leave a PR's base on a stale `graphite-base/N` marker from a
  previous interrupted submit; the next `jj-gt submit` then no-ops
  and the PR shows as "conflicting" on GitHub even though the
  local stack is correct.
- `gt submit --publish` is on by default. Opt-out via `--no-publish`
  for keeping PRs in draft (or use `--draft` to create as draft).
- Pre-push hooks run per-bookmark in parallel by default. Opt out
  with `--hooks-sequential` (one bookmark at a time) or
  `--hooks-tip-only` (only run hooks for the tip).

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
brew tap mattwilkinsonn/zireael https://github.com/mattwilkinsonn/zireael
brew install mattwilkinsonn/zireael/jj-gt
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
                (drives `gt submit --stack` end-to-end). The full
                ancestor chain from trunk up to `-b <tip>` is
                submitted automatically.
    track       Sync refs/branch-metadata/* without submitting
                (manual escape hatch — same logic as submit minus the
                gt-submit invocation).
    fetch       Graphite-aware replacement for `jj git fetch`: fetches
                trunk, backfills metadata refs, runs `gt sync` for
                branch cleanup, restacks orphaned children with `jj
                rebase`, prunes `gtmq_*` queue-test artifacts, and
                falls back to merge-marker scan for PRs gt sync misses.
    reconcile   Reconcile gt's tracking metadata + (optionally) remote
                refs with jj's current view of the bookmark graph.
                Standalone version of the pre-submit reconcile step.
    status      Print stack-wide PR + queue state in stack order.
    log         Print the derived stack as jj-gt sees it (debug).
    init        Print suggested aliases + setup reminders.
    completions Emit a shell completion script.
```

### How `jj-gt submit` picks the stack

Submit is always stack-shaped. There's no `--stack` flag because
there's no other mode — the full ancestor chain from trunk up to
each `-b <tip>` is included automatically.

```text
       trunk    bottom    mid    head
main ───●─────────●────────●──────●

jj-gt submit -b head
  → walks back to find bottom + mid on the ancestor chain
  → tracks bookmarks with gt in bottom→top order (gt rejects
    `gt track <child> --parent <parent>` if the parent isn't
    tracked yet)
  → pushes the whole stack via `gt submit --stack`
```

To submit a subset, name each bookmark with its own `-b` flag —
`jj-gt submit -b mid -b head` submits just those two, omitting
`bottom`. Multiple `-b` flags for unrelated tips submit each as
its own independent stack — `jj-gt submit -b feature-a-tip -b
feature-b-tip` fans out to two `gt submit --stack` invocations,
one per stack. Per-bookmark pre-push hooks run in parallel by
default; use `--hooks-sequential` for serial execution with live
runner output, or `--hooks-tip-only` to skip the per-bookmark
gate and run hooks once against the full `trunk..tip` range.

### Examples

```bash
# Submit the whole stack ending at `head` (the common case)
jj-gt submit -b head

# Submit two specific bookmarks as a stack (skips anything above
# `top--athena` and anything between `bottom--athena` and trunk
# that isn't in the selection)
jj-gt submit -b bottom--athena -b top--athena

# Submit two unrelated stacks in one invocation — fans out to one
# `gt submit --stack` per tip
jj-gt submit -b feature-a-tip -b feature-b-tip

# Submit every bookmark on the @-ancestor chain
jj-gt submit --all

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

## jjui integration

`jj-gt init` walks you through a one-time setup that installs six
[jjui](https://github.com/idursun/jjui) actions + keybindings so
the common `jj-gt` flows are reachable from inside the TUI:

| Action | Default key | What it does |
| --- | --- | --- |
| `jj-gt-submit-selected` | `x s` | Submit the bookmark at the focused commit (`jj-gt submit -r context.commit_id()`) |
| `jj-gt-fetch` | `x f` | Run the Graphite-aware fetch + cleanup pipeline (`jj-gt fetch`) |
| `jj-gt-track-selected` | `x t` | Sync `refs/branch-metadata/*` for the focused bookmark (`jj-gt track -r context.commit_id()`) |
| `jj-gt-submit` | `x S` | Submit every bookmark on the @-ancestor stack (`jj-gt submit`) |
| `jj-gt-track` | `x T` | Sync metadata refs for every bookmark on the stack (`jj-gt track`) |
| `jj-gt-reconcile` | `x r` | Re-track adjacent diverged bookmarks + push rebased SHAs (`jj-gt reconcile`) |

Order matters: jjui's `x`-prefix overlay surfaces candidates in the
order they appear in the config, so the most-frequent actions
(submit-selected, fetch, track-selected) land at the top of the
menu and the recovery/whole-stack flows further down. Lowercase =
focused-bookmark-only. Uppercase = whole stack. Same lowercase/
uppercase split jj-hp uses for `x p`/`x P`.

The selected variants use `-r context.commit_id()` rather than
`-b <name>` because jjui's lua context exposes the focused commit's ID
but not its bookmark name(s). jj-gt's own resolver finds the
bookmark(s) at that commit; if the commit has zero bookmarks, jj-gt
errors clearly; if multiple, the multi-stack fan-out submits each
independently.

If you'd rather hand-edit `~/.config/jjui/config.toml` instead of
running `jj-gt init`, append:

```toml
[[actions]]
name = "jj-gt-submit-selected"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "submit", "-r", context.commit_id())
  revisions.refresh()
"""

[[actions]]
name = "jj-gt-fetch"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "fetch")
  revisions.refresh()
"""

[[actions]]
name = "jj-gt-track-selected"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "track", "-r", context.commit_id())
  revisions.refresh()
"""

[[actions]]
name = "jj-gt-submit"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "submit")
  revisions.refresh()
"""

[[actions]]
name = "jj-gt-track"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "track")
  revisions.refresh()
"""

[[actions]]
name = "jj-gt-reconcile"
lua = """
  jj_async("util", "exec", "--", "jj-gt", "reconcile")
  revisions.refresh()
"""

[[bindings]]
action = "jj-gt-submit-selected"
seq = ["x", "s"]
scope = "revisions"
desc = "jj-gt submit selected bookmark(s)"

[[bindings]]
action = "jj-gt-fetch"
seq = ["x", "f"]
scope = "revisions"
desc = "jj-gt fetch"

[[bindings]]
action = "jj-gt-track-selected"
seq = ["x", "t"]
scope = "revisions"
desc = "jj-gt track selected bookmark(s)"

[[bindings]]
action = "jj-gt-submit"
seq = ["x", "S"]
scope = "revisions"
desc = "jj-gt submit (whole stack)"

[[bindings]]
action = "jj-gt-track"
seq = ["x", "T"]
scope = "revisions"
desc = "jj-gt track (whole stack)"

[[bindings]]
action = "jj-gt-reconcile"
seq = ["x", "r"]
scope = "revisions"
desc = "jj-gt reconcile"
```

The `revisions.refresh()` after each `jj_async` repaints jjui's
revisions pane so the post-submit bookmark moves are visible
immediately.

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

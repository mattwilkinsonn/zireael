---
name: vcs-jj
description: "Jujutsu (jj) reference: mental model, everyday workflows, revsets, bookmarks, workspaces, colocated git, and recovery."
---

# Jujutsu (`jj`) Reference

Use this any time you run a VCS operation in a jj repo (any directory containing `.jj/`). In jj repos, use jj commands exclusively — never reach for git.

**You may commit; you never push.** Matt pushes himself — never run `jj git push` in any form. Use a Conventional Commits subject (`feat(scope):`, `fix(scope):`, `refactor(scope):`, …) plus at most two sentences of body.

## Mental Model

- The working copy IS a commit (`@`). File edits auto-amend into `@` — there is no staging area and no `git add`; all changes are tracked automatically.
- Branches are anonymous by default. A line of work is just a commit and its ancestors; you attach a human-readable name with a *bookmark* only when you need one (e.g. for a PR).
- Change IDs (letters, e.g. `nmwwolux`) are stable across rewrites; commit IDs (hex) change whenever a commit is rewritten. Prefer change IDs for anything you reference across edits.
- Commits are mutable until pushed. Conflicts are stored *in* commits, so you can resolve them later instead of at merge time.
- The operation log records every repo mutation. `jj undo` reverts the last operation; `jj op restore <id>` rewinds to any prior state.

## Critical Rules

- **Never** run git commands in a jj repo — use the jj equivalent.
- **Always** pass `-m` to `commit` / `describe`. Without it an editor opens and hangs a non-interactive agent. For the same reason, **never** use `-i` (interactive TUI) flags.
- Write the *complete* message in the `-m` call (subject, plus a body if the change warrants one). Don't leave a placeholder subject to "finish later" — the deferred step gets dropped and nobody notices until the commit is inspected.
- After `jj commit -m "msg"` the content is in `@-` (the parent) and the new `@` is empty. Target `@-` for bookmarks.
- **Advance the commit boundary before starting the next step.** When stacking commits, run `jj new -m "<next message>"` the moment you finalize a commit, *before* editing any file for the next step. Edits before `jj new` auto-amend into the previous commit; edits after land in the next. If you reach for `jj split` / `jj squash --into` to untangle mixed work, you skipped a `jj new` — split *before* editing, not after.

## Everyday Workflows

Start a new line of work:

```bash
jj new main -m "feat: description"     # new change off main
# ... edit files (auto-tracked) ...
jj commit -m "feat: description"       # finalize @; a new empty @ is created
```

Amend / reword the current change:

```bash
jj describe -m "better message"        # change @'s message only
jj squash                              # fold @ into its parent
```

Edit an existing commit in place:

```bash
jj edit <change-id>                    # make that commit the working copy
# ... fix ...
jj new                                 # step off it when done
```

Rebase your work onto an updated base:

```bash
jj git fetch
jj bookmark set main -r main@origin    # advance local main
jj rebase -d main@origin               # rebase YOUR commits onto it
```

> Never `jj rebase -s main` against a tracked bookmark — `main` is immutable. Target the commit above it, or use `main@origin` as the destination.

Split a mixed change, or auto-route hunks to their correct ancestors:

```bash
jj split 'glob:tests/**'               # split out tests by path: no diff TUI, but it still prompts for each side's description
jj absorb                              # distribute @'s hunks into the ancestor commits that touched those lines
```

Operate on specific files:

```bash
jj commit -m "msg" file1 file2         # commit only these paths
jj commit -m "msg" 'glob:src/**/*.rs'  # ...or by glob
jj restore --from @- file.txt          # restore one file from the parent
```

## git -> jj Translation

| Task | git | jj |
| --- | --- | --- |
| Status / diff / log | `git status` / `diff` / `log` | `jj st` / `jj diff` / `jj log` |
| Show a commit | `git show <ref>` | `jj show <rev>` |
| Amend | `git commit --amend` | `jj squash` or `jj describe -m "msg"` |
| Switch | `git checkout <ref>` | `jj new <rev>` or `jj edit <rev>` |
| New branch | `git checkout -b <name>` | `jj new main -m "desc"` (+ bookmark) |
| List branches | `git branch` | `jj bookmark list` |
| Stash | `git stash` | `jj new` (old work stays in the parent) |
| Cherry-pick | `git cherry-pick` | `jj duplicate -d @ <rev>` (plain `jj duplicate` keeps the original parents) |
| Revert | `git revert` | `jj revert -r <rev>` |
| Blame | `git blame` | `jj file annotate` |
| Worktree | `git worktree add` | `jj workspace add` |
| Undo | (complex) | `jj undo` |

## Bookmarks

Bookmarks are named pointers (jj's equivalent of a git branch). They do **not** auto-advance the way the working copy does.

```bash
jj bookmark create <name> -r @         # create, tracking @'s change ID
jj bookmark list                       # list (after commit, the bookmark points at @-)
jj bookmark set <name> -r <change-id>  # move a bookmark (surgical, per-ref)
jj bookmark delete <name>              # remove after a PR merges
```

- `bookmark create` tracks the **change ID**, so it stays correct after `jj commit` moves the content to `@-` — no re-set needed.
- Moving a named bookmark like `main` is always explicit: `jj bookmark set main -r main@origin` (fast-forward) or `-r @-`.

## Revsets

| Expression | Meaning |
| --- | --- |
| `@` / `@-` / `@--` | Working copy / its parent / its grandparent |
| `trunk()` | Main/master branch tip |
| `mine()` | Changes you authored |
| `bookmarks()` / `remote_bookmarks()` | All local / remote bookmark tips |
| `::x` | All ancestors of `x` |
| `x::y` | DAG range: ancestors of `y` that descend from `x` |
| `x..y` | Set difference: in `y` but not `x` |
| `heads(x)` / `roots(x)` | Commits in `x` with no children / no parents in `x` |
| `description(pat)` | Commits whose description matches a pattern |
| `file(path)` | Commits that modified the given path |
| `present(x)` | `x` if it exists, else the empty set |
| `empty()` / `conflicts()` | Commits with no diff / with unresolved conflicts |
| `x \| y` / `x & y` / `~x` | Union / intersection / complement |

## Workspaces

`jj workspace` gives you multiple working directories backed by one `.jj/` store — useful for running several lines of work (or several agents) at once without trampling each other's files. An automated dev environment may provision these for you.

```bash
jj workspace list                      # show all workspaces
jj workspace add ../feature-ws -r main # new isolated working directory
jj workspace forget <name>             # detach a workspace you're done with
jj workspace update-stale              # re-sync a workspace another op left stale
```

- `edit`, `new`, and `workspace add/forget` are workspace-local — they move only the current workspace's `@`. Bookmarks are repo-wide refs, so `bookmark set/create/delete` is visible in every workspace; it's still surgical (one ref, no graph rewrite, no other `@` moved).
- Graph rewrites (`rebase`, `squash`, `split`, `abandon`, `describe`) are visible to every workspace sharing the store — safe on your own commits, disruptive if another workspace's `@` sits on the commit you rewrite. Check `jj op log` before rewriting something you didn't author.
- **When you resume work on an existing workspace, run `jj workspace update-stale` first.** If another workspace operated in between, yours is stale; editing files on a stale `@` can let jj reset to a fresh op state and discard on-disk edits without ever snapshotting them. After editing, confirm the change landed in the *commit* with `jj diff -r <bookmark> --stat` — passing tests only prove the file is on disk, not that it's committed.

## Colocated git + jj

- A **colocated** repo has both `.jj/` and `.git/`; git-aware tools read `.git/` naturally. Drive all VCS through jj — git is just the transport that talks to the remote.
- **Secondary workspaces lack a `.git/`.** Wire one up so git-aware tools (e.g. `gh`) work: write a `.envrc` that sets `GIT_DIR` to the primary repo's `.git`, then `direnv allow` (or `source .envrc`). Treat the `.envrc` write and the approval as one atomic step.

```bash
echo 'export GIT_DIR=<primary-repo>/.git' > .envrc
direnv allow
```

## Common Pitfalls

1. **Bookmarks don't auto-advance** after commit. `bookmark create` tracks the change ID (stays correct); moving `main` needs an explicit `bookmark set -r @-` / `-r main@origin`.
2. **`@` after `jj commit` is empty** — the content is in `@-`.
3. **`jj new` is not `git commit`.** `jj new` creates a new empty change; `jj commit` finalizes `@`.
4. **`::` is a DAG range, `..` is set difference** — not interchangeable.
5. **Empty commits are normal** — they just mean "ready to work here."
6. **`Commit is immutable` error** — you targeted a tracked bookmark (e.g. `main`) directly in a rebase. Target the commit above it, or use `main@origin` as the destination.
7. **Divergent change IDs (`xxxx/0`, `xxxx/1`)** — two workspaces rewrote the same commit concurrently. Don't abandon blindly; inspect each side and promote the one you want:

   ```text
   jj log -r 'all() & description(glob:"*<phrase>*")'   # list divergent siblings
   jj diff -r <id>/0 --stat                              # inspect each side's contents
   jj bookmark set --allow-backwards <bookmark> -r <chosen-id>   # promote the right one
   jj abandon <other-id>                                 # drop the other
   ```

## Troubleshooting & Recovery

- **Undo the last operation:** `jj undo`.
- **Inspect history:** `jj op log` — the `<user>@<host> <workspace>@` column shows which workspace ran each op (handy for "where did my change go?").
- **Recover an abandoned/hidden commit:** abandoned commits are hidden, not deleted, and stay addressable by change/commit ID. `jj edit <change-id>` un-hides it onto your `@`; then `jj bookmark set <name> -r <id>` to re-point. Find it with `jj log -r 'all() & description(glob:"*<phrase>*")'`.
- **Move a bookmark back:** `jj bookmark set --allow-backwards <name> -r <commit>` — surgical and per-ref (`set` refuses a backward/sideways move without `--allow-backwards`). Prefer this smallest-primitive fix over a global rewind.
- **Drop your own uncommitted work:** `jj abandon @` — this already leaves you on a fresh empty working copy, so no follow-up `jj new` is needed.
- **`jj op restore <op-id>`** rewinds the whole repo to a prior operation — every ref, every bookmark, and every workspace's `@`. It's the right tool for a single-workspace mistake, but where multiple workspaces share one `.jj/` it yanks every other workspace's state too, so reach for a per-ref fix first.
- **Never `jj abandon` a commit you haven't inspected** — an undescribed `(no description set)` commit next to yours is often another workspace's live WIP. Check `jj show <id> --stat` and `jj op log --limit 5` first.

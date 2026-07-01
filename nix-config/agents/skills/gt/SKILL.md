---
name: gt
description: "Git + Graphite (gt) reference for agents: per-agent clone, stacked branches/PRs, create/modify/submit/sync/restack/get, branch naming, stacking on a pushed base, and the review-fix loop."
---

# Git + Graphite (`gt`) Reference

Use this for every VCS operation in your per-agent git clone. Agents work in a plain `git` clone and drive branches, stacks, and PRs through **Graphite (`gt`)** — this is the agent VCS workflow. `jj` is Matt's own tool for reviewing the wave; you don't run it in your clone.

**You may commit, create branches, and push/submit your own feature branches** (over the seal-bot token), then run the review loop to merge-ready (`skill://autonomous-review`). Commit as Matt with a `Co-Authored-By: seal <noreply@sealedsecurity.com>` trailer and a Conventional Commits subject (`feat(scope):`, `fix(scope):`, `refactor(scope):`, …) plus at most two sentences of body (`rule://commit-conventions`). Hard limits, enforced by the push-guard: never push or force-push `main`, never merge (the human gate), never push/PR/issue outside `mattwilkinsonn/*` + `sealedsecurity/*`.

## Mental Model

- **Your clone is yours.** Each agent gets its own `~/agents/workspaces/<codename>/<repo>` with its own `.git/`. No shared store — so no cross-agent rebase, no divergent change IDs, no stale-working-copy surprises. You never coordinate on VCS state, only on files, at PR/merge time.
- **Plain git underneath; `gt` layers a stack on top.** `trunk` is `main`. A **stack** is a chain of branches, each building on its parent, each becoming one PR. `gt` records every branch's parent as metadata so it can rebase the whole chain for you.
- **One branch = one PR.** A branch usually holds a single commit; it may carry extra commits (e.g. review fixes). `downstack` = the branches below yours (ancestors); `upstack` = above (descendants).
- **The core verbs:** `create` (new stacked branch + commit), `modify` (amend or add a commit, then restack descendants), `restack` (rebase the stack onto its parents), `sync` (pull trunk + restack + clean up merged branches), `submit` (push + open/update PRs).

## Critical Rules

- **Always non-interactive.** Pass `--no-interactive` and `-m "msg"` to `create` / `modify` / `submit`; a bare invocation opens an editor or TUI and hangs a headless agent. For the same reason, never use the patch (`-p`) or `--interactive-rebase` flags.
- **Never `--ai`.** It regenerates the PR title/description non-deterministically on every submit, clobbering your prose and dropping issue links. You author the PR title + description yourself (`rule://commit-conventions`).
- **Review fixes are a NEW commit, never an amend.** Use `gt modify --commit` (`-c`) for a fix on a pushed branch. Amending or squashing a pushed commit rewrites history and fails to re-trigger the review bots (`skill://autonomous-review`).
- **Never push/force-push `main`; never merge.** Merge is the human gate. Feature branches on `mattwilkinsonn/*` + `sealedsecurity/*` only.
- **Commit as Matt** (per-repo email) with the `Co-Authored-By: seal <noreply@sealedsecurity.com>` trailer.

## Clone Setup

Your clone is provisioned at `~/agents/workspaces/<codename>/<repo>`. To create one:

```sh
git clone <origin-url> ~/agents/workspaces/<codename>/<repo>
cd ~/agents/workspaces/<codename>/<repo>
git checkout main && git pull
```

`gt` auto-initializes on first use in the repo, detecting `main` as trunk (or run `gt init` explicitly). Push auth flows through the `gh` credential helper (the seal-bot account), so `gt submit` pushes over the seal-bot token with no extra setup.

Origins (HTTPS): `zireael` → `github.com/mattwilkinsonn/zireael`; `oh-my-pi` → `github.com/mattwilkinsonn/oh-my-pi`; `sealed` / `compass` → `github.com/sealedsecurity/<repo>`.

## Everyday Workflow

```sh
gt sync                                              # pull main, restack, prompt-delete merged branches
# ... edit files ...
gt create <codename>-<issue>-<slug> \
  --update --message "type(scope): summary" --no-ai --no-interactive
gt submit --no-interactive                           # push + open the PR
```

- **`gt sync`** pulls the latest `main`, rebases your open branches onto it, and prompts to delete merged/closed branches (`--no-restack` skips the rebase, `-f` skips prompts). Run it at the start of a change.
- **`gt create [name]`** creates a new branch stacked on the current branch and commits staged changes. `--update` (`-u`) stages tracked changes first; `--all` (`-a`) also stages untracked files. `-m` is repeatable — pass the subject, body, and `Co-Authored-By: seal <…>` trailer as separate `-m` values (each becomes a paragraph).
- **`gt submit`** force-pushes (with lease) your branch and opens/updates its PR. In `--no-interactive` mode it skips the metadata prompt and creates the PR in **draft**; author the title/description yourself and mark it ready:

```sh
gh pr edit <n> --title "type(scope): summary" --body-file <file>
gh pr ready <n>
```

Add `--no-stack` to submit only the current branch (skip the prompt to include branches above it). **`gt` does not hoist the co-author trailer or issue links into the PR body** — and Graphite's merge-queue squash builds the `main` commit from the PR title + description — so put the `Co-Authored-By: seal <noreply@sealedsecurity.com>` trailer and any issue refs (`Refs #N` / `Closes #N`) at the **bottom of the PR description** yourself, or they're lost on merge (`rule://commit-conventions`). Keep the description accurate as review commits land.

## The Review-Fix Loop

Never amend a pushed commit. Land each review fix as a new commit and re-submit:

```sh
# ... edit files to address a finding ...
gt modify --commit --update --message "fix(scope): address review" --no-interactive
gt submit --no-interactive --update-only             # push the new commit; re-triggers the bots
```

`gt modify --commit` adds a commit to the current branch and restacks any descendants; `gt submit --update-only` (`-u`) pushes only branches that already have open PRs. Full loop — wait for the bots, auto-fix bot-only findings, surface judgment calls, iterate to merge-ready: `skill://autonomous-review`.

## Branch Naming

`<codename>-<issue>-<short-desc>` — in multi-agent work lead with the **codename** as the lane tag (the *one* place a persona name belongs — keep it out of commit messages, PR titles/descriptions, and code), then the issue ref, then a short kebab description: `hudson-sea-930-woodpecker-fleet`. No issue → `<codename>-<short-desc>` (e.g. `cook-compass-scaffold`). Solo work drops the codename: `sea-931-mattmini-adminuser`. No `user/` prefix.

## Stacking

### Your own stack

To stack a second change on your own in-progress branch, check it out and create on top:

```sh
gt checkout <base-branch>                            # or gt up / gt down to move within the stack
gt create <codename>-<issue>-<slug2> -u -m "…" --no-ai --no-interactive
gt submit --stack --no-interactive                   # submits the whole stack (alias: gt ss)
```

`gt restack` rebases the stack if a parent moved; a `gt modify` on a lower branch auto-restacks everything above it.

### On another agent's pushed base (cross-clone)

Separate clones don't share branches, so the base must be **on origin** first (its owner has submitted it). Then pull it into your clone and build on it:

```sh
gt get <base-branch>                                 # fetch their branch (+ downstack) from remote, tracked
gt create <codename>-<issue>-<slug> -u -m "…" --no-ai --no-interactive
gt submit --no-stack --no-interactive               # open a PR for your branch only; don't touch theirs
```

- An **in-progress base keeps moving** — when its owner pushes fixes or Matt rebases it, re-pull and restack: `gt get <base-branch>` then `gt restack`. Once the base merges, `gt sync` moves your branch onto `main`.
- **Coordinate with the base's owner** (via Matt) before submitting a stack on their unmerged PR, so you don't resubmit or race their branch.
- **Disjoint files?** If your work touches entirely different files from the base, work off `main` and let `gt sync` rebase once at the end — stack only when you genuinely need the base's changes to build or test.

## Command Reference

| Task | Command |
| --- | --- |
| Sync trunk + restack + clean up merged | `gt sync` |
| New stacked branch + commit | `gt create <name> -u -m "…" --no-ai --no-interactive` |
| Amend the current branch's commit | `gt modify -u --no-interactive` |
| Add a new commit (review fix) | `gt modify -c -u -m "…" --no-interactive` |
| Rebase the stack onto its parents | `gt restack` |
| Push + open/update PR(s) | `gt submit --no-interactive` |
| Submit the whole stack | `gt submit --stack --no-interactive` (`gt ss`) |
| Push only branches with open PRs | `gt submit --update-only --no-interactive` (`-u`) |
| Switch / navigate branches | `gt checkout <branch>` (`gt co`), `gt up`, `gt down` |
| Show the stack | `gt log`, `gt log short` (`gt ls`) |
| Fetch a teammate's pushed branch | `gt get <branch>` |
| Fix a branch's parent metadata | `gt track --parent <parent>` |
| Undo the last `gt` mutation | `gt undo` |

## Troubleshooting

- **Corrupted stack metadata** (a branch shows the wrong parent): `gt track --parent <parent>` re-points it, then `gt restack` rebuilds the chain.
- **Undo a `gt` mutation:** `gt undo` reverts the most recent Graphite operation.
- **A pushed branch won't submit** (trunk out of sync): `gt sync` first, then `gt submit`.
- **Conflicts during restack/sync:** `gt` drops you into a git rebase — fix the files, `git add`, `git rebase --continue`, then re-run the `gt` command.
- **Matt reviews with jj.** He points `jj` at the clones read-only; you never run `jj` in your clone — all your VCS is `git` + `gt`.

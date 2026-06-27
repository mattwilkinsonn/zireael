---
name: multi-agent-wave
description: "Run several agents in parallel: headless task subagents vs persistent peer sessions, conflict-first slot assignment, shared tracker"
---

# Multi-Agent Wave

A playbook for running more than one agent at once. Two models, picked by what you are parallelizing. All agents share one model — codenames are scaffolding for the human's eye, never a memory or specialization model.

## Two parallelism models

### 1. Headless `task` subagents (decompose one task)

Spawn ephemeral subagents from inside a single session with the `task` tool.

- No terminal panes; they run in-process and return a result to the spawning agent.
- Nest at most 3 deep. Keep batches wide, not deep.
- Use when one task splits into independent, disjoint-file pieces (edit unrelated files, map an unknown subsystem, write tests for separate modules) that you reassemble in one place.
- The spawning agent owns verification — subagents skip lint/build/test gates; you run them once over the union of changed files when the batch returns.
- Subagents have no shared chat history; front-load every path, fact, and acceptance criterion into the assignment. They can still reach peers live via `irc`.

### 2. Persistent peer sessions (the visible wave)

One top-level `omp` agent per workspace, each its own long-lived session.

- **Emdash** — single-repo cockpit: one task per slot, all on the same repo.
- **Ghostty + Zellij panes** — cross-repo: one pane per slot, each pointed at its own repo/worktree.
- Peers coordinate over the `irc` tool (direct or broadcast) plus a shared markdown tracker. The tracker is the wave's state; `irc` is for live questions and unblocking.
- Use when the work is several genuinely separate jobs that each want their own context, run for a long time, or span repos — not slices of one task.
- **Workspace layout.** Agent workspaces are grouped **by agent, not repo**: `~/agents/workspaces/<slot>-<codename>/<repo>/` — one dir per agent, a repo-named jj workspace inside for each repo it touches (e.g. `~/agents/workspaces/4-shackleton/{seal,walrus,sealed}`, `~/agents/workspaces/11-nansen/{zireael,sealed}`). Canonical repo clones stay under `~/repos/`; never sit agent workspaces beside them; the `jj-ws` helper script is superseded — don't use it for wave workspaces (it makes `<repo>.ws/` siblings). Pre-create **from the repo** — `jj workspace add` only creates the leaf dir, so make the per-agent parent first: `mkdir -p ~/agents/workspaces/<slot>-<codename> && jj workspace add --name <slot>-<codename> ~/agents/workspaces/<slot>-<codename>/<repo> -r 'trunk()'`. `--name` keeps the name unique per store; `-r 'trunk()'` bases on the repo's own main/master tip (don't hard-code `main` — cross-repo waves hit `master`-default repos). For a slot **stacked** on another's unmerged work, base on the dependency's tip instead, per *Slot assignment → Stacking*: `-r <base-bookmark>@origin` if that base is already pushed, or its local bookmark/change-id if it's only local in a shared store. Retire in two steps: `jj workspace forget <slot>-<codename>` unregisters the workspace but leaves its checkout on disk, so follow with `rm -rf ~/agents/workspaces/<slot>-<codename>/<repo>` (or the whole `<slot>-<codename>/` dir once every repo in it is forgotten). Keeps the top level clean and every workspace for an agent in one place.

### Choosing

Slices of one task that reassemble in one place → `task` subagents. Separate long-running jobs, especially across repos or needing independent review checkpoints → peer sessions. When unsure, prefer `task`; promote to peer sessions only when a slice needs its own durable context.

## Slot assignment: conflict-avoidance first

Assign by what a slot can safely touch, then by priority — **not** by specialization (all agents are the same model and can work any area).

1. **Disjoint files first.** A candidate is only eligible for a slot if its file set does not overlap any in-flight slot's file set. Overlap → stall the candidate or have the human re-order, explicitly.
2. **Then priority.** Among conflict-free candidates, take the highest priority. Standing rule: flake / CI-health fixes jump the queue — a broken pipeline taxes the whole wave.
3. **Stacking.** If B genuinely depends on A's unmerged work, branch B from A's tip rather than forcing them disjoint; note the dependency in the tracker.

## Tracker: single source of truth

One markdown file, owned and edited by the human supervisor. Read it first; it is authoritative when live state has drifted. Update it immediately after every assignment, status change, or finish. Sections:

- **Agent/state table** — slot, codename, current task, status.
- **Conflict map** — file ranges and which slot owns them; a row clears when that work merges.
- **Candidate queue** — waiting items ordered by priority (flake/CI first), with any stacking/blocked-by notes.

Markdownlint-clean (blank lines around headings, lists, tables; compact `| a | b |` rows).

## Carried-over discipline

These hold for every agent in a wave, both models:

- **Asking-first checkpoints.** Before removing/replacing code, changing a public API, or choosing between plausible approaches, present 2-3 options with a recommendation and wait. Halt immediately on pushback.
- **BDD-then-TDD gating.** Outer behavior test first, then unit tests for the new logic, run them and confirm they fail, then implement to green. Bugfixes get a regression test that fails before and passes after.
- **Commit/submit policy.** A slot MAY commit (Conventional Commits subject, terse body) **and create/move its own bookmarks** — only *pushing/submitting* is the human's. Creating a bookmark is local, not a push: when a slot's change is ready, it makes the bookmark itself and hands the human the submit command — `jj-gt submit -b <bookmark> --ai` (the `--ai` drafts the PR title + description on first push). **Bookmark naming:** `<codename>-<issue>-<short-desc>` — codename **first** as the lane tag, then issue ref, then short kebab desc (e.g. `hudson-sea-930-woodpecker-fleet-conversion`); no issue → `<codename>-<short-desc>` (e.g. `cook-compass-scaffold`). No `user/` prefix; the codename is required.
- **Commit promptly.** Wave agents share one `.jj/`, so another slot's or the supervisor's op can rebase your `@` between steps. **Snapshot with `jj describe -m "msg"` after each edit pass — before you run lint or tests** (the `-m` keeps it non-interactive — a bare `jj describe` opens `$EDITOR` and hangs a headless slot) — so committed work rebases cleanly; uncommitted work is what `update-stale` discards. **Reserve `jj new` (stepping off to a fresh change) until that step's checks pass** — running it before verification lands your fixups in the empty child, not the change being verified. If a stale `@` ever diverts your edits, recover them per `skill://vcs-jj` (Troubleshooting & Recovery).

## Codenames / personas

Optional, human-facing only. Pick a codename theme with distinct first letters for eye-scanning if it helps you track slots. It is labeling for the supervisor, not a behavioral or memory difference between agents. Keep it out of commit messages, PR titles/descriptions, and code — with **one** deliberate exception: the `<codename>-` **prefix** on the branch/bookmark name, which is the lane tag the supervisor reads to tell whose work a PR is (see Commit/submit policy above).

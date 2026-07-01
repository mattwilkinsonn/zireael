---
name: multi-agent-wave
description: "Run several agents in parallel: headless task subagents vs persistent peer sessions, conflict-first assignment, shared tracker"
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

- **Emdash** — single-repo cockpit: one task per agent, all on the same repo.
- **Ghostty + Zellij panes** — cross-repo: one pane per agent, each pointed at its own clone.
- Peers coordinate over the `irc` tool (direct or broadcast) plus a shared markdown tracker. The tracker is the wave's state; `irc` is for live questions and unblocking.
- Use when the work is several genuinely separate jobs that each want their own context, run for a long time, or span repos — not slices of one task.
- **Workspace layout.** Each agent works in its **own git clone**, grouped by agent: `~/agents/workspaces/<codename>/<repo>/` — one dir per agent, a plain clone (its own `.git/`) per repo it touches (e.g. `~/agents/workspaces/shackleton/{seal,walrus,sealed}`, `~/agents/workspaces/nansen/{zireael,sealed}`). Canonical clones stay under `~/repos/`; don't sit agent workspaces beside them. Create one with `git clone <origin-url> ~/agents/workspaces/<codename>/<repo>`, then `cd` in and `git checkout main && git pull`; drive branches, stacks, and PRs with Graphite (`gt`) — full workflow in `skill://gt`. An isolated clone means no sibling or supervisor op can touch your VCS state, so you coordinate only on overlapping files, at PR/merge time. Retire an agent by removing its dir: `rm -rf ~/agents/workspaces/<codename>/<repo>` (or the whole `<codename>/` dir once every repo in it is done).
- **Agent prompts are the spin-up artifact.** Each agent launches from its prompt at `~/notes/wave/prompts/<codename>-prompt.md`. **Spinning up a new agent or re-tasking an existing one = rewrite that prompt in place** (task, repo/clone, conflict-scoped file zone, constraints, branch name, acceptance — all of it). Never write a separate handoff/assignment doc beside it; a second file drifts from and competes with the prompt. The supervisor's `prompts/mercator-prompt.md` is the same pattern.

### Choosing

Slices of one task that reassemble in one place → `task` subagents. Separate long-running jobs, especially across repos or needing independent review checkpoints → peer sessions. When unsure, prefer `task`; promote to peer sessions only when a slice needs its own durable context.

## Assigning work: conflict-avoidance first

Assign by what an agent can safely touch, then by priority — **not** by specialization (all agents are the same model and can work any area).

1. **Disjoint files first.** A candidate is only eligible for an agent if its file set does not overlap any in-flight agent's file set. Overlap → stall the candidate or have the human re-order, explicitly.
2. **Then priority.** Among conflict-free candidates, take the highest priority. Standing rule: flake / CI-health fixes jump the queue — a broken pipeline taxes the whole wave.
3. **Stacking.** If B genuinely depends on A's unmerged work, branch B from A's tip rather than forcing them disjoint; note the dependency in the tracker.

## Tracker: single source of truth

One markdown file, owned and edited by the human supervisor. Read it first; it is authoritative when live state has drifted. Update it immediately after every assignment, status change, or finish. Sections:

- **Agent/state table** — codename, current task, status.
- **Conflict map** — file ranges and which agent owns them; a row clears when that work merges.
- **Candidate queue** — waiting items ordered by priority (flake/CI first), with any stacking/blocked-by notes.

Markdownlint-clean (blank lines around headings, lists, tables; compact `| a | b |` rows).

## Carried-over discipline

These hold for every agent in a wave, both models:

- **Asking-first checkpoints.** Before removing/replacing code, changing a public API, or choosing between plausible approaches, present 2-3 options with a recommendation and wait. Halt immediately on pushback.
- **BDD-then-TDD gating.** Outer behavior test first, then unit tests for the new logic, run them and confirm they fail, then implement to green. Bugfixes get a regression test that fails before and passes after.
- **Commit + submit policy.** An agent commits (Conventional Commits subject), creates its own feature branch, **and submits it** over the seal-bot token — `gt submit --no-interactive` (no `--ai`; author the PR title + description — `rule://commit-conventions`) — then runs the review loop to merge-ready (`skill://autonomous-review`). Author/committer = Matt (per-repo email) + `Co-Authored-By: seal <noreply@sealedsecurity.com>` (`rule://commit-conventions`). Hard limits (push-guard-enforced): never push or force-push `main`, never merge (the human gate), never push/PR/issue outside `mattwilkinsonn/*` + `sealedsecurity/*`. **Branch naming:** `<codename>-<issue>-<short-desc>` — codename **first** as the lane tag, then issue ref, then short kebab desc (e.g. `hudson-sea-930-woodpecker-fleet-conversion`); no issue → `<codename>-<short-desc>` (e.g. `cook-compass-scaffold`). No `user/` prefix; the codename is required. Full git + `gt` workflow: `skill://gt`.
- **Your clone is isolated.** Each agent works in its own git clone (own `.git/`), so no sibling's or the supervisor's op can rebase your work mid-edit — you own your VCS state end to end. Commit as you go with `gt` and drive your own branch; coordinate only on overlapping *files*, at PR/merge time (`skill://gt`).

## Codenames / personas

Optional, human-facing only. Pick a codename theme with distinct first letters for eye-scanning if it helps you track agents. It is labeling for the supervisor, not a behavioral or memory difference between agents. Keep it out of commit messages, PR titles/descriptions, and code — with **one** deliberate exception: the `<codename>-` **prefix** on the branch name, which is the lane tag the supervisor reads to tell whose work a PR is (see Commit/submit policy above).

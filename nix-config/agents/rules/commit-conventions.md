---
description: "Commit message conventions: Conventional Commits subject, terse body, scope vocabulary from history; agent may commit but never pushes"
---

# Commit Conventions

The diff tells the story — the message just names the change. Keep it concise.

## Subject line

Use a Conventional Commits prefix; never ship a subject without one:

- `feat(scope):` — new behavior.
- `fix(scope):` — bug fix.
- `refactor(scope):` — restructure with no behavior change (a pure cleanup is always `refactor`).
- `docs(scope):` — docs only.
- `perf(scope):` — performance only.
- `chore(scope):` — tooling, deps, housekeeping.

The scope is the affected area (`daemon`, `cli`, `parser`, `auth`, …). Match the scope vocabulary the repo's existing `git log --oneline` / `jj log` history already uses rather than inventing new ones.

## Body

Cap the body at ~2 sentences. This applies to big mechanical renames too: a one-line subject plus a ≤3-line "what got renamed, what's explicitly kept" summary is plenty. Never enumerate every category (crate renames, env vars, schema, comment sweep, "what's kept" lists) — the diff already shows all of it. A 100-line commit message is writing for an imaginary reviewer who won't read the diff instead of the real one who will.

Use inline `code` for identifiers and paths — it renders well on hosted diffs and carries into the PR description.

## Push policy

The agent MAY create commits + create/move bookmarks. The agent NEVER pushes or submits — Matt does, in every form: `git push`, `git push --force`, `jj git push`, `jj-gt submit`, and any variant or subcommand. When a change is ready, create the bookmark and hand Matt the submit command — `jj-gt submit -b <bookmark> --ai` (the `--ai` drafts the PR title + description on first push) — then stop.

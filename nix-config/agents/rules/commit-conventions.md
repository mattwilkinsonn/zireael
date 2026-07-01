---
description: "Commit message conventions + identity: Conventional Commits subject, terse body; commit as Matt with a seal co-author trailer; push your own feature branches via the seal-bot token — never main, never merge, allowlisted owners only"
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

The scope is the affected area (`daemon`, `cli`, `parser`, `auth`, …). Match the scope vocabulary the repo's existing `git log --oneline` history already uses rather than inventing new ones.

## Body

Cap the body at ~2 sentences. This applies to big mechanical renames too: a one-line subject plus a ≤3-line "what got renamed, what's explicitly kept" summary is plenty. Never enumerate every category (crate renames, env vars, schema, comment sweep, "what's kept" lists) — the diff already shows all of it. A 100-line commit message is writing for an imaginary reviewer who won't read the diff instead of the real one who will.

Use inline `code` for identifiers and paths — it renders well on hosted diffs and carries into the PR description.

## Attribution

Commits are authored **and committed as Matt** — this keeps his contribution graph (co-author trailers don't earn squares; only author/committer do). Per-repo email: `matt@sealedsecurity.com` for `sealedsecurity/*`, `mattwilki17@gmail.com` for personal repos (`mattwilkinsonn/*`). Add an agent-attribution trailer: `Co-Authored-By: seal <noreply@sealedsecurity.com>`.

## Push policy

The agent commits, creates branches, **and pushes/submits its own feature branches** over the seal-bot token, then runs the review loop to merge-ready (`skill://autonomous-review`). Submit with `gt submit` (**no `--ai`** — you author the PR title + description; see below).

These stay hard limits, enforced by the push-guard:

- **Never push or force-push `main`; never merge** — merge is the human gate.
- **Owner allowlist:** push, open PRs, and file issues only on `mattwilkinsonn/*` and `sealedsecurity/*` — never an upstream/OSS repo (e.g. `can1357/*`).

## PR title + description

You write the PR title and description yourself — **never `--ai`**. Graphite's `--ai` regenerates the body non-deterministically every submit, clobbering your prose and dropping issue links; without it, `gt submit` leaves the description under your control.

- Write it like a good commit body — what changed and why — and **update it as review-loop commits land** so it stays accurate. Set/update via `gh pr edit <n> --body …` or the GitHub MCP `update_pull_request`.
- **Put the co-author trailer and issue links in the PR description yourself** — `gt` doesn't hoist them. At the **bottom of every PR description** add the `Co-Authored-By: seal <noreply@sealedsecurity.com>` trailer and any issue links via the magic words (`Refs #N`, `Closes #N`). Graphite's merge-queue squash builds the `main` commit from the **PR title + description**, so a trailer or ref that lives only in your commits is dropped on merge — co-authorship isn't recorded and issues don't auto-close. Keep the trailer in your commits too (per Attribution), but the description is what lands.

## Branch naming

`<name>-<issue>-<short-desc>` — codename/lane-tag first in multi-agent work (the **one** place a persona name belongs — never in the subject/body, PR content, or code), then the issue ref, then a short kebab description (e.g. `hudson-sea-865-aws-provider`). No issue → `<name>-<short-desc>` (e.g. `cook-compass-scaffold`). No `user/` prefix; solo work drops the codename.

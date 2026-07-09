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

**Corollary — commit author never signals a human takeover.** Because every agent commit is authored *as Matt*, the author/committer field carries **no** human-vs-agent signal: `Matt Wilkinson` on a PR head is the norm, not evidence Matt took the PR over by hand. To attribute a commit to an agent, read the **branch codename** (canonically the `<codename>-` prefix; some issue-first branches carry it as a `--<codename>` suffix, so check both ends) and/or the reflog in that agent's clone — **never** the author field. Misreading `author=Matt` as a human takeover has made supervisors wrongly stand agents down off their own live lanes.

Issue assignee is **ownership, not edit-actor**. Agents act as Matt, so a wave issue an agent is working is **assigned to Matt** (`matt@sealedsecurity.com`). The `seal` bot user is only the *edit actor* — status changes, comments, and Linear writes land under `seal` (not Matt) so the audit trail is separable — but it is **never an assignee**. Never set `seal` (or any agent codename) as the assignee of an issue; agent↔issue mapping lives in the wave tracker, not the Linear assignee field. File new issues assigned to Matt (or unassigned if genuinely unowned).

## Push policy

The agent commits, creates branches, **and pushes/submits its own feature branches** over the seal-bot token, then runs the review loop to merge-ready (`skill://autonomous-review`). Submit with `gt submit` (**no `--ai`** — you author the PR title + description; see below). **Open PRs only with `gt submit`, never `gh pr create`** — `gh pr create` authors under the `gh` CLI account (`seal-agent`), not the seal-bot identity, so the PR lands under the wrong user and Graphite can't track/re-submit it; `gh` is edit-only on an already-open PR (`gh pr edit`/`ready`).

These stay hard limits, enforced by the push-guard:

- **Never push or force-push `main`; never merge** — merge is the human gate.
- **Owner allowlist:** push, open PRs, and file issues only on `mattwilkinsonn/*` and `sealedsecurity/*` — never an upstream/OSS repo (e.g. `can1357/*`).
- **`gh pr create` is hard-blocked** (allowlisted owner or not) — the push-guard redirects it to `gt submit`. Opening a PR is the one GitHub op that goes through `gt`, not the MCP or `gh`.
- **GitHub API calls go MCP-first, `gh` last-resort.** `gh` burns the shared GraphQL bucket the whole wave depends on. Read PR state via `mcp__litellm_tern_get_review_state`; set PR metadata/ready via `mcp__litellm_github_update_pull_request`; other ops via `mcp__litellm_github_*`. Use `gh` only when the MCP tool is genuinely unavailable — and say so.

## PR title + description

You write the PR title and description yourself — **never `--ai`**. Graphite's `--ai` regenerates the body non-deterministically every submit, clobbering your prose and dropping issue links; without it, `gt submit` leaves the description under your control.

- Write it like a good commit body — what changed and why — and **update it as review-loop commits land** so it stays accurate. Set/update via the GitHub MCP `mcp__litellm_github_update_pull_request` (routed through the LiteLLM gateway; run `search_tool_bm25` to activate it if it isn't already live), or `gh pr edit <n> --body …` as fallback.
- **End every PR description with the `Co-Authored-By` trailer as its last line** — `gt` doesn't hoist it. Graphite's merge-queue squash builds the `main` commit from the **PR title + description**, so the description's last line becomes the commit's last line, and GitHub records co-authorship only when `Co-Authored-By: seal <noreply@sealedsecurity.com>` is that final line — nothing (prose, headings, other text) after it. Put issue links (`Refs #N` / `Closes #N`) on the lines just above it. Keep the trailer in your commits too (per Attribution), but the description is what lands via the queue.

## Branch naming

`<name>-<issue>-<short-desc>` — codename/lane-tag first in multi-agent work (the **one** place a persona name belongs — never in the subject/body, PR content, or code), then the issue ref, then a short kebab description (e.g. `hudson-sea-865-aws-provider`). No issue → `<name>-<short-desc>` (e.g. `cook-compass-scaffold`). No `user/` prefix; solo work drops the codename. A minority of issue-first branches instead carry the codename as a trailing `--<codename>` suffix (e.g. `sea-865-aws-provider--hudson`); both forms are valid, so when attributing a branch to an agent (per the Attribution corollary above) check **both ends**.

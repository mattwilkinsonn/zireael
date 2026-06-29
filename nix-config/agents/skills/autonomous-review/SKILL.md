---
name: autonomous-review
description: "Push your branch, wait for the review bots, then run the review loop yourself — auto-fix + auto-resolve bot-only findings, surface judgment calls to the human in the terminal, iterate until the PR is merge-ready. The human only merges."
---

# Autonomous PR review loop

Use after you've implemented and committed a change on a feature branch and own
it end-to-end to merge-ready. This replaces the manual "human pushes → tells you
to check comments → you fix → repeat" churn: you push, wait for the bots, and
run the loop yourself, surfacing only what needs a human decision.

Triage discipline (the four surfaces, enumerate-every-reviewer, the resolve
rules) lives in `skill://github-pr-review` — this skill adds the **loop** and the
**autonomy to act on bot-only findings** without round-tripping the human.

## The loop

1. **Commit + submit.** Commit your change, create/move your bookmark, and submit
   it yourself: `jj-gt submit -b <bookmark> --ai` (the `--ai` drafts the PR title + description on first push). Attribution per `rule://commit-conventions`:
   the commit is authored as Matt with the per-repo email, trailered
   `Co-Authored-By: seal <noreply@sealedsecurity.com>`, and pushed over the
   seal-bot token. Feature branches on allowlisted-owner repos only (see
   Boundaries).
2. **Wait for the bots.** Run `wait-for-reviews <pr>` with `bash` `async: true`
   and a generous `timeout` (~1800s). Then yield — you're woken when it returns
   (bots reviewed the head, or are rate/usage-limited, or the 20-min backstop
   fired). No webhook/cron needed.
3. **Triage** all four surfaces per `skill://github-pr-review`; enumerate every
   reviewer (CodeRabbit / Greptile / cubic / Codex + any human).
4. **Act:**
   - **Auto-fix + auto-resolve** clear, **bot-only** findings you can resolve
     without a judgment call (about half are mechanical — rename, guard, dep
     bump, lint, doc, deprecation). Land each as a **new commit** on the PR tip
     (never amend/squash a pushed PR — that fails to re-trigger the bots), move
     the bookmark over it, and resolve the bot-only thread per the
     `github-pr-review` resolve discipline (bot-authored, no human comments, fix
     live on head, re-reviewed). The fix + resolve speaks — no prose reply.
   - **Surface judgment calls** to the human via the terminal structured-question
     format (`ask`): design forks, API/contract changes, disagreement with a
     finding, anything a human commented on, deferrals, security-sensitive calls.
     One **batched** `ask` per round — never one question at a time.
5. **Push fixes → loop to step 2.** Repeat (usually several rounds) until no
   actionable bot findings remain and CI is green.
6. **Hand off.** PR is merge-ready; the human does the final review + merge — the
   one gate that stays human.

## Autonomy boundary

- **Do autonomously:** mechanical bot findings + resolving the bot-only threads
  they came from (after the fix is live and re-reviewed).
- **Surface, don't decide:** design choices, contract/API changes, disagreements,
  any human-authored thread, deferrals, security calls. Never free-hand a reply
  that speaks for the human; never resolve a human thread without their OK
  (`github-pr-review` Discipline still governs).

## What "reviewed" means (the wait primitive)

`wait-for-reviews` encodes the per-bot signals; for a manual check: each bot tags
the commit it reviewed (match the head SHA); **CodeRabbit edits its summary
comment in place** (key on its content/`updated_at`, not "a new comment");
**CodeRabbit rate-limit** (`rate limited by coderabbit.ai`) and **Codex
usage-limit** (`usage limits for code reviews`) → that bot is skipped, don't
block on it; Codex's 👍 reaction is not a reliable "done" signal. 20-min backstop
then proceed.

## Boundaries

- **Never push to `main`; never merge** — merge is the human gate; the push-guard
  enforces both.
- **Owner allowlist:** push, open PRs, and file issues only on `mattwilkinsonn/*`
  and `sealedsecurity/*` repos — never an upstream/OSS repo (e.g. `can1357/*`).
  Feature branches only, under the seal-bot identity. The push-guard enforces
  this on top of the seal-bot token's repo scope.

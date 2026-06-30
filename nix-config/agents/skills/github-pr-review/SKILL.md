---
name: github-pr-review
description: "Triage PR feedback across all four surfaces (GitHub MCP; gh CLI fallback); surface findings; reply only with Matt's approval; resolve only settled bot-only threads after a live fix and re-review, or a Matt-OK'd deferral."
---

# Triaging PR Feedback

Review feedback on a PR lives on **four independent surfaces**. Checking one or two and inferring the rest are empty is the classic miss — a PR can be fully actionable while showing zero unresolved inline threads. Enumerate all four, every time. The companion principle lives in `rule://enumerate-pr-review-surfaces`.

Use the **GitHub MCP** (`pull_request_read`) as the primary tool — structured output, one call per surface, no shell-quoting or `--jq` plumbing. The `gh` CLI is fallback only (see the last section).

When you own a PR end-to-end — push, fix, drive to merge-ready — run this triage inside the loop in `skill://autonomous-review`; the discipline below is unchanged, the loop just adds the autonomy to act on bot-only findings without round-tripping the human.

## The four surfaces

| # | Surface | What lives there | `pull_request_read` method |
| --- | --- | --- | --- |
| a | Inline review threads | Per-line conversations; carry `is_resolved` / `is_outdated` / `is_collapsed` + thread IDs | `get_review_comments` |
| b | Review bodies | `reviews[].body` — summaries, "Actionable comments: N", outside-diff items, failed-to-post findings | `get_reviews` |
| c | Top-level comments | Plain issue-comments on the PR (bot linkbacks, walkthroughs, human notes) | `get_comments` |
| d | CI checks | Build / test / check-run status | `get_check_runs` (and `get_status` for the combined commit status) |

Inline threads and review bodies are **different shapes** — a reviewer's summary body (e.g. an "Actionable comments posted: N" block, or findings GitHub couldn't post inline because they fall outside the patch range) is *not* an inline thread and carries no thread ID. A thread that has since auto-resolved or gone outdated can still reference an unaddressed finding, so never let a resolved/outdated flag silently drop a body-level finding from view. Always read the review bodies (`get_reviews`) in full, independent of thread state.

## Pulling each surface

```text
pull_request_read  get_review_comments  owner/repo #N   → (a) inline threads + thread metadata + IDs
pull_request_read  get_reviews          owner/repo #N   → (b) every review body + state + author
pull_request_read  get_comments         owner/repo #N   → (c) top-level conversation
pull_request_read  get_check_runs       owner/repo #N   → (d) CI check runs
```

Each inline thread from `get_review_comments` carries a stable thread node ID (e.g. `PRRT_kwDO…`) plus `is_resolved` / `is_outdated`. That ID is the handle for a reply or a resolve — both only under the conditions in Discipline below.

## Enumerate every reviewer

`get_reviews` returns **one entry per review submission**. A PR is usually reviewed by multiple parties — humans plus several automated bots (CodeRabbit, cubic, Greptile, Codex, …) — each a separate review object with its own `state` and its own inline `comments[]`. One bot's "actionable comments" count says nothing about another's findings, which sit in a different review object. Read **every** review body, and for any reviewer with inline findings, read them in full via `get_review_comments`.

Never report "no findings from X" unless you observed X's review with zero inline comments — a grep that matched some *other* reviewer's note is not evidence about X.

Most automated bots auto-resolve their own threads when the flagged code stops applying — but **not all**: Codex never does, and CodeRabbit doesn't while rate-limited, so some threads stay open after re-review and you clear them (see Discipline). Bots also post non-actionable boilerplate (status linkbacks, stack-management notes, walkthrough summaries, merge-queue instructions) as top-level comments. Filter the known boilerplate out of surface (c); what remains — a human comment, a bot finding outside its summary — is real and must be triaged.

## Reading thread state

- `is_resolved` — resolved threads are usually noise when triaging, but read the body before dismissing.
- `is_outdated` — attached to a since-rewritten line; **not** the same as resolved. An outdated-but-unresolved thread is either still relevant or needs resolving — don't auto-drop it.

## Discipline: replies need Matt's go-ahead; resolves only when settled

**Replying — only what Matt has approved.** Draft a reply if one's useful, but don't post it until Matt approves the exact wording. Never free-hand "addressed in fixup" or "we're not doing this" — an unapproved reply speaks for the maintainer on his own PR.

**Resolving — only post-fix, only what's settled.** You may resolve an inline thread when either:

1. it's a **bot-authored** thread **with no human comments in it** (a human reply turns it into Matt's call, even if a bot opened it), **genuinely addressed** by a fix that's **committed and live on the PR head**, and the reviewers have had their **re-review pass** but left it open — some bots don't auto-resolve their own comments (**Codex never does**; **CodeRabbit doesn't while rate-limited**, frequent now), while cubic / Greptile / healthy CodeRabbit close their own and you clear the stragglers; or
2. Matt has **OK'd deferring it** — note the tracking-issue reference in your turn summary; only post it as a thread reply if Matt approved that wording (resolving the thread itself needs no reply). This is the one case you may resolve a **human-opened** thread: Matt's explicit approval to defer is what authorizes it.

Never resolve an **addressed** thread before its fix is live (that hides an unaddressed finding) — the Matt-OK'd deferral in (2) is the one exception, since a deferral has no live fix by definition. Otherwise these stay his call: a **human-authored** thread (outside the (2) deferral), an out-of-scope or disagreed thread, or a deferral Matt hasn't approved.

Note every reply/resolve in the turn summary (thread + the commit or issue behind it). For anything you're not touching, surface it verbatim — path, line, author, body excerpt — marked Addressed (cite the commit), Deferred (cite the issue + Matt's OK), or Disagreed (reasoning).

## Where review-fix commits go

When you do address feedback (autonomously for bot-only findings, or with Matt's go-ahead otherwise — `skill://autonomous-review`), land the fix as a **new commit** and move the PR's bookmark up over it — so the fix rides the **same PR the comments are on**, updating it and its threads in place.

- **Always a new commit. Never amend or squash a pushed PR's commits.** The auto-reviewers (Greptile, CodeRabbit, cubic, Codex) often don't re-trigger on a rewritten (amended/squashed) commit — a fresh commit on top reliably re-triggers them. Extra commits don't pollute history: every repo squash-merges into `main`.
- **Don't spin the fix onto a new bookmark** — that opens a *separate* PR disconnected from the review, and the original threads never see it.
- jj/Graphite: `jj new` on the PR's tip, edit, then `jj bookmark set <pr-bookmark> -r @` to carry the bookmark over the fix.

## Fallback: the `gh` CLI

Reserve the CLI for when the MCP can't express something (rare). Prefer higher-level `gh` commands; never run mutations (`gh api -X POST/PATCH/DELETE`, `gh pr review/comment`) without explicit confirmation.

- Inline threads + review bodies: a review-comments extension (e.g. `gh-pr-review`) or the review-comments REST/GraphQL endpoints. These usually don't support `--jq`; pipe to external `jq`.
- `gh pr view <N> --json comments` (top-level), `gh pr checks <N>` (CI).
- `gh api` only when nothing higher-level works, paired with a note on why.

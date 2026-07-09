---
description: "Triage PR/code-review feedback by enumerating all reviewer surfaces with zero-count evidence before reporting status"
---

# Enumerate PR Review Surfaces Before Reporting

Code-review feedback lives on several independent surfaces. A PR can be fully actionable while one surface looks empty, so before reporting "what does this PR still need?" you MUST check ALL of them, every time:

- **(a) Inline review threads** — per-line conversations attached to the diff. These auto-resolve or go outdated on a rebase, so an empty unresolved-thread list does NOT mean nothing was flagged.
- **(b) Review summary bodies** — the top-level body of each review submission. Findings often sit here, not inline: an "actionable comments" count, outside-diff-range items, and especially any **"comments failed to post"** notice (when the inline-comment API errored, the findings fall back into the body). Scan summary bodies regardless of thread-resolution state.
- **(c) Top-level PR comments** — plain issue-comments on the PR (humans and some bots, e.g. automated reviewers, post here). These are a distinct surface from review threads.
- **(d) CI / check status** — build and check failures live here, not in any review. **These are two independent GitHub API buckets — the commit-STATUS bucket and the check-RUNS bucket — and a failure in one is invisible in the other.** On `sealedsecurity/sealed`, Woodpecker `CI (pr)` posts as a commit **status**, so it appears ONLY in `/commits/<sha>/status` (combined-state), NEVER in `/commits/<sha>/check-runs`. Concluding "green" from `/check-runs` alone is a false-green trap (it hid a red `CI (pr)` on #461/#472/#474 — the bot check-runs were all green while the pipeline was failing). You MUST query the **combined commit status** (`gh api /commits/<sha>/status` or `gh pr checks`) AND check-runs, on the exact head SHA, and report green only when BOTH are green.

A PR is reviewed by multiple parties (humans plus bots), each a separate object with its own findings. Enumerate every reviewer — never read one and infer the rest are empty.

**Never report a surface "clean" without zero-count evidence.** "No findings from X" is a factual claim about X's findings array; it is only true once you have observed a zero count for X — not when a check for some other reviewer came back empty.

**A green check-run is not a triaged finding.** A reviewer bot posts inline P1/P2 findings *and still reports its check-run `success`* — the check means "the bot ran," not "you addressed it." Likewise a summary verdict ("approved", "5/5 safe to merge") is the bot's overall take, not a per-finding resolution: findings sit in the inline threads underneath an approved review. Never infer "no findings" from a green check-run or an approval verdict — you MUST read each reviewer's inline threads (surface a) and summary body (surface b) and observe a zero count. The check-run status (surface d) answers "did CI/the bot run + pass its own gate," a different question from "are there findings to address."

**Seam/owner approval is a different surface from reviewer triage.** A service/area owner approving the *architecture* or *seam* of a change never covers the review bots' line-level findings — those are independent objects you still MUST enumerate. Owner sign-off ≠ CodeRabbit/Greptile/cubic/Codex triaged.

**After a force-push to a new head, findings are re-triaged on the new head — never assumed carried.** A rebase/force-push changes the head SHA; prior-head reviews and cached review-state (e.g. a tern snapshot) still point at the old SHA. Re-fetch review state for the new head and re-run the enumeration; do not report a rebased PR's status from the pre-rebase triage.

**Replies always need Matt's go-ahead; resolves only in pre-authorized cases.** Surface findings — paste each (path, line, author, body excerpt) and mark Addressed (cite the fix), Deferred (cite the issue), or Disagreed (reasoning). Post a reply only once Matt approves the exact wording. **Without further go-ahead** you may resolve only: (a) a **bot-authored** thread **with no human comments in it** genuinely fixed and live on the PR head after reviewers re-reviewed but left it open (Codex never auto-resolves; CodeRabbit doesn't while rate-limited), or (b) a deferral Matt has OK'd (if that resolve needs a posted issue-reference reply, the reply wording needs his approval too — otherwise just note the issue in your summary). Everything else stays his call: **human-authored threads, out-of-scope or disagreed items, and any thread whose fix isn't yet live.** Detail: `skill://github-pr-review`.

Tooling: read `skill://github-pr-review`.

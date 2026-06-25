---
description: "Triage PR/code-review feedback by enumerating all reviewer surfaces with zero-count evidence before reporting status"
---

# Enumerate PR Review Surfaces Before Reporting

Code-review feedback lives on several independent surfaces. A PR can be fully actionable while one surface looks empty, so before reporting "what does this PR still need?" you MUST check ALL of them, every time:

- **(a) Inline review threads** — per-line conversations attached to the diff. These auto-resolve or go outdated on a rebase, so an empty unresolved-thread list does NOT mean nothing was flagged.
- **(b) Review summary bodies** — the top-level body of each review submission. Findings often sit here, not inline: an "actionable comments" count, outside-diff-range items, and especially any **"comments failed to post"** notice (when the inline-comment API errored, the findings fall back into the body). Scan summary bodies regardless of thread-resolution state.
- **(c) Top-level PR comments** — plain issue-comments on the PR (humans and some bots, e.g. automated reviewers, post here). These are a distinct surface from review threads.
- **(d) CI / check status** — build and check failures live here, not in any review.

A PR is reviewed by multiple parties (humans plus bots), each a separate object with its own findings. Enumerate every reviewer — never read one and infer the rest are empty.

**Never report a surface "clean" without zero-count evidence.** "No findings from X" is a factual claim about X's findings array; it is only true once you have observed a zero count for X — not when a check for some other reviewer came back empty.

**Do not auto-reply to or resolve review threads.** Surface findings to Matt — paste each one (path, line, author, body excerpt) and mark it Addressed (cite the fix), Deferred, or Disagreed (with reasoning). He replies and resolves; he is the maintainer and merger, so those decisions are his.

Tooling: read `skill://github-pr-review`.

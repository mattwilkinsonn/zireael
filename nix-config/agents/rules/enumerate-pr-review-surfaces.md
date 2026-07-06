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

**Replies always need Matt's go-ahead; resolves only in pre-authorized cases.** Surface findings — paste each (path, line, author, body excerpt) and mark Addressed (cite the fix), Deferred (cite the issue), or Disagreed (reasoning). Post a reply only once Matt approves the exact wording. **Without further go-ahead** you may resolve only: (a) a **bot-authored** thread **with no human comments in it** genuinely fixed and live on the PR head after reviewers re-reviewed but left it open (Codex never auto-resolves; CodeRabbit doesn't while rate-limited), or (b) a deferral Matt has OK'd (if that resolve needs a posted issue-reference reply, the reply wording needs his approval too — otherwise just note the issue in your summary). Everything else stays his call: **human-authored threads, out-of-scope or disagreed items, and any thread whose fix isn't yet live.** Detail: `skill://github-pr-review`.

**Merge-ready ⟺ every review thread resolved.** No exceptions. Never report a PR merge-ready while any thread is unresolved — enumerate the unresolved threads (path, author, why) and drive the count to **zero** first. `is_outdated` and `is_resolved` are **distinct**: an outdated thread (line moved / file changed / rebased away) is still an **open conversation** until explicitly resolved — outdated is never "handled." A stale-because-superseded thread (its file/finding no longer exists) still gets a one-line reply saying so, then a resolve; never resolve a thread silently.

**A thread is resolved only when it is fixed or properly deferred — never left open, never resolved bare.** Deferral is a strict ordered protocol: **(1) post a comment** on the thread explaining the deferral, **(2) file the follow-up issue**, **(3) resolve the thread** with the issue reference in that closing comment. All three, in order. "Bucket-limited", "outdated", or "will do later" are reasons to *defer* (which means run the protocol), never to leave a thread open or to resolve it without the comment+issue. If the GraphQL bucket blocks the resolve, the deferral is an **owed action you MUST carry** until the bucket recovers — it never falls off; the PR is not merge-ready until it's done.

Tooling: read `skill://github-pr-review`.

---
description: "No retries, ever: never add test/CI/op retries — a retry masks a defect; make the flaky thing deterministic or fail closed and loud instead."
condition:
  - "retries\\s*[:=]\\s*[1-9]"
  - "--retr(y|ies)[= ][1-9]"
  - "\\.retry(Times)?\\s*\\("
  - "nextest[^\\n]*\\bretries\\b\\s*[:=\\s]\\s*[1-9]"
  - "retry:\\s*\\n\\s+\\w"
scope:
  - text
  - thinking
  - tool
interruptMode: always
---

# No retries, ever

**Never add a retry.** No test retries (`nextest retries=N`, `jest.retryTimes`, a
`retry:` block), no CI-step retries, no catch-and-retry loops around a flaky
network / push / IO op. When the instinct is "just retry it," the answer is
always no.

## Why

A retry masks a defect instead of fixing it, and ships the bug:

- A **flaky test** is a real bug — a race, a nondeterministic assert, or a
  wall-clock dependency. Retrying until it passes hides the signal and leaves the
  race in the code.
- An **unreliable op** that "works on retry" has an unhandled failure mode.
  Retry-until-green converts a visible failure into a silent, intermittent one.

## Instead

- **Flaky test → make it deterministic.** Event-gate, don't time-gate: drive the
  system until the observed state, never `sleep`-and-hope. Use controlled/virtual
  time (`tokio::time::pause`, fake timers) for budget/latency asserts so load
  can't perturb them. Or remove a redundant wall-clock assert entirely when the
  semantic asserts already pin the contract.
- **Unreliable op (network / push / IO) → fix the root cause, or fail closed and
  surface it loudly** so a human sees it. A bounded timeout that fails the step
  with a clear diagnosis beats a retry loop that eventually errors anyway.
- **Re-running a pipeline against *fixed* code is fine** — that's a fresh run of a
  corrected tree. Re-running the *same* code hoping it flakes green is the banned
  thing.

## Examples

- Runloop render-IO-stall flake → fixed by **event-gating** (drive quit until the
  observed exit), not a retry.
- A budget/latency assert that flaked under load → fixed by **removing the
  redundant wall-clock assert** (the semantic asserts already catch the
  regression deterministically), not `retries = 2`.
- A CI image push that could stall → **bounded timeout + fail-loud** as tracked
  debt, not a silent retry loop.

## The one non-retry that's allowed

Re-running CI/tests against a **changed tree** (you fixed the defect, now you
verify the fix). That is a fresh run, not a retry of the same code. If you cannot
point to what you changed, it is a retry — don't.

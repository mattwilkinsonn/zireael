---
description: "Red → Green workflow: write BDD + unit tests first, watch them fail, then implement the smallest fix that turns them green."
---

# Red → Green Workflow (BDD-then-TDD, Tests Fail BEFORE the Fix)

For non-trivial features and bugfixes, write the tests **first**, run them and **watch them fail (Red)**, then write the fix and watch them turn **Green**.

## The four steps

1. **BDD: an end-to-end / integration test that exercises the user-facing behavior.** This is the test that proves "the bug is fixed" or "the feature is real". It runs through the same harness production traffic does (real e2e harness, a test that spawns the daemon, a full UI test) — *not* a mock. Its job is to fail before the fix and pass after, with no scaffolding the implementation could "teach to the test."
2. **TDD: unit tests for any new helper / classifier / pure function the fix introduces.** Pin every branch of the new logic. They are cheap to run and lock the policy in place against future drift.
3. **Run the tests. They MUST fail.** This is the Red step. Observe and record the failure output. If a test passes before the fix, it is testing the wrong thing — rewrite it until it fails for the right reason.
4. **Then implement.** With Red verified, write the smallest implementation that turns both layers Green. Run again and confirm they pass.

The Red step is non-negotiable. "I wrote the test and the fix together and they pass" is not Red → Green — that is "tests written" with no proof the test would have caught the bug.

```text
1. Write tests → run → see them fail → record output.
2. Write fix → run → see them pass → record output.
```

Both states must be observable: "before fix: 2 failures, after fix: 0 failures." Without the Red observation, the test claim is unverified.

## Reproduce the user's failure shape verbatim

**The BDD test's setup must reproduce the user's reported failure shape verbatim, not the shape your hypothesis suggests.** This is what stops you from writing a test that confirms your mental model instead of the user's bug.

When the user says "X is broken when I do Y under conditions Z", the test setup is conditions Z exactly — not "Z plus a hint that points the test at the right struct", not "a richer config that happens to include the value your hypothesized fix would write", not "Z' where Z' is what you assume Z resolves to internally." Literally Z, in the same shape the user has on disk.

The trap: encoding the answer into the setup. If you pre-populate the input with the value the fix is supposed to produce, the test goes green while the real broken pipeline is never exercised.

Two cross-checks before claiming a BDD test exercises the bug:

1. **The config, command, and environment in the test setup must be reachable from a copy-paste of what the user reported.** If the repro is "config with sections A and B, run command C", the setup is sections A and B (no section D added "for convenience") and command C (not a synthetic equivalent). Anything added for setup convenience pins your hypothesis, not the bug.
2. **If deleting your fix would still let the test pass for the wrong reason** — because the setup is rich enough that the test never depended on the fix — the test is wrong. Rewrite the setup until it fails for *exactly* the right reason, then check the failure output against the user's actual reported symptom. They should match.

When in doubt, place the user's reported repro (their config, command, failure output) next to the test setup. If the user's input shape is not staring back at you in the test, the test is not testing the bug. This rule is upstream of "watch it fail": a test that fails Red for the wrong reason and passes Green after the wrong fix walks the workflow and proves nothing.

## Why both layers

BDD tests prove the bug was real and the fix is end-to-end correct, but they are slow and do not pin every branch. Unit tests pin every branch but do not prove the integration. Skip BDD and "passes its tests" can mean "passes the fake test written next to the fix"; skip TDD and a future refactor can quietly drop a branch the BDD test happens not to exercise.

## Refactors and genuinely-untestable code

- **Pure refactor, no behavior change:** the BDD layer is just "the existing e2e tests still pass."
- **Genuinely untestable end-to-end** (UI keystroke timing, real terminal escape sequences, third-party APIs with no mock): say so explicitly in the response and lean harder on the unit layer — do *not* silently skip BDD because it is hard.

Test order in review: the BDD test reads first (it is the contract), the unit tests second (the implementation detail), the implementation third. Always look for the cleanest long-term solution, not just the smallest diff.

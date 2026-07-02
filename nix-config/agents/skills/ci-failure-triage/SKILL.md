---
name: ci-failure-triage
description: "Diagnose a red CI check correctly before fixing: read the real log, separate your failures from pre-existing ones, and tell a code bug from an environment/permission gap — then verify the fix in a way that could actually reproduce the failure."
---

# CI failure triage

Use when a PR check is red. The job is to reach the **right** fix, which starts
with the **right diagnosis** — not to change code until the check turns green. The
two expensive mistakes are fixing the wrong layer (a code change for an env/config
problem) and "verifying" a fix in an environment that could never have reproduced
the failure. This skill is the discipline that avoids both; `skill://woodpecker-ci`
is how you pull the log, `skill://autonomous-review` is the loop this runs inside.

## 1. Read the actual failure, never guess

A check name + "failed" is not a diagnosis. Pull the real step log
(`skill://woodpecker-ci`: decode the check's `details_url` → `pipeline ps` →
`log show <step>`), find the concrete error line, and read it literally. The fix
follows from the actual message (`exit code`, `Resource not accessible`,
`permission denied`, a specific assertion), not from what the check *sounds* like.

## 2. Separate your failures from the pre-existing ones

On a shared trunk a PR's checks include failures your diff didn't cause —
other lanes' drift, secret-gated deploy jobs a PR can't run, flaky infra. Before
fixing anything, bucket each red check:

- **Yours** — the failing step touches files/areas in your diff. Fix these.
- **Not yours** — the log points at paths you didn't touch (another subtree's
  lint drift), or a job that needs secrets/permissions a PR fork can't have
  (deploy, image publish, preview). Confirm from the log, then leave them and say
  so — don't chase green on a check your change didn't break.

Grounding: read the log's file paths and error, not the check name. `root-shfmt`
failing on `oss/other-subtree/*.sh` is not your docs PR's problem.

## 3. Classify: code bug vs environment / permission

This is the classification that most often gets fixed at the wrong layer. Ask what
*kind* of failure the log shows:

- **Code / logic** — wrong output, a thrown error in your code, a failing
  assertion. Fixable in the diff.
- **Environment / permission / config** — a token missing a scope
  (`Resource not accessible by personal access token`, `403`), a missing secret
  or env var, a runner without a tool, a network/registry denial. **The fix is
  the environment, not the code.** Swapping API surfaces, retrying, or
  restructuring the call does **not** grant a missing permission — e.g. GitHub
  REST and GraphQL need the *same* resource scopes, so switching between them
  never fixes a token-scope `403`. Name the missing grant and surface it (it's
  usually an out-of-band owner action: add the token scope, provision the secret).

A fail-closed tool that exits non-zero on a permission error is often behaving
**correctly** — the red check is the environment gap surfacing, not a bug to code
around.

## 4. Verify a fix in an environment that could reproduce the failure

A fix is proven only by a check that *could have caught the original failure*.
The trap: validating with credentials, inputs, or a config that differ from CI in
exactly the dimension that failed.

- A **token/permission** failure cannot be reproduced or cleared with a
  broader-scoped local token. Your interactive `gh`/cloud creds usually outrank
  CI's least-privilege token, so a local run passing proves the *code path*, not
  the *scope*. A scope fix is verified by the CI token (or an equivalently-scoped
  one), or confirmed as an out-of-band grant — never by a local run on a
  privileged token.
- A **secret/env** failure is verified with the same var unset/set as CI, not
  with your shell's ambient value.
- A **code** failure is the case a local unit/integration run does verify —
  reproduce it red, then green.

State which dimension you verified against. "It passed locally" is a false green
when the local environment can't hit the failing dimension — say what your run
did and didn't exercise.

## The loop

1. Pull the real log (`skill://woodpecker-ci`).
2. Bucket every red check: yours vs not-yours (§2).
3. For each of yours, classify code vs env/permission (§3).
4. Fix code in the diff; for env/permission, name the grant and surface it.
5. Verify against an environment that reproduces the failing dimension (§4) — or
   say it's gated on an out-of-band change and mark the check expected-red until
   then.
6. Report not-yours checks as such, with the log evidence, rather than chasing
   them.

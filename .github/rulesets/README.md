# GitHub branch rulesets

Each `*.json` file in this directory is a GitHub Ruleset (the
modern replacement for classic branch protection). Apply via:

```bash
gh api repos/mattwilkinsonn/zireael/rulesets \
    -X POST --input .github/rulesets/main-protection.json
```

To update an existing ruleset, you'll need its ID:

```bash
gh api repos/mattwilkinsonn/zireael/rulesets | jq '.[] | {id, name}'
gh api repos/mattwilkinsonn/zireael/rulesets/<id> \
    -X PUT --input .github/rulesets/main-protection.json
```

To delete:

```bash
gh api repos/mattwilkinsonn/zireael/rulesets/<id> -X DELETE
```

## Files

- `main-protection.json` — required-status-checks gate on `main`.
  Enforces:
  - Linear history, no force-push, no delete, conversation
    resolution required.
  - PR required (zero approvals, so single-author is fine).
  - **Squash-merge only** — `rebase` and `merge` (merge commit)
    are disabled. Keeps `main`'s history linear by construction.
  - **Code-owner review required** (paired with zero-approval
    count: see below for the single-author-friendly logic).
  - All per-tool CI checks pass (or skip — `skipped` counts).
  - Repo Admin bypass (`actor_id: 5`).

### Single-author code-owner gating

The ruleset sets `require_code_owner_review: true` AND
`required_approving_review_count: 0`. That combination looks
contradictory but is actually the recommended pattern for
single-author / small repos:

- `required_approving_review_count: 0` means no approvals are
  enforced. A self-author PR can land without anyone approving
  it.
- `require_code_owner_review: true` is a no-op when the approval
  count is zero — GitHub doesn't enforce a "code owner must
  approve" clause without at least one required approval.
- BUT the CODEOWNERS file still drives auto-assignment of
  reviewers when a PR opens (so future collaborators get the
  right reviewer wired up).

When the second human shows up, bump `required_approving_review_count`
to `1` and the code-owner gate activates without further config.

Until then, no review is enforced — the gate is the CI checks
(via `required_status_checks`).

## Coexistence with classic branch protection

Rulesets and classic branch-protection rules can both be active
on the same branch — they're enforced **additively**, so the
strictest setting wins on any given dimension. If you have a
classic rule already set:

```bash
gh api repos/mattwilkinsonn/zireael/branches/main/protection
```

…either delete it (so the ruleset is the single source of truth):

```bash
gh api repos/mattwilkinsonn/zireael/branches/main/protection -X DELETE
```

…or keep it, knowing that any setting present in both has to be
consistent in both files. The ruleset is easier to version-control
(JSON in-repo) so prefer migrating to ruleset-only.

## Bypass actors

The `bypass_actors` array uses GitHub's repo-role IDs:

| role | actor_id |
| --- | --- |
| Read | 1 |
| Triage | 2 |
| Write | 3 |
| Maintain | 4 |
| Admin | 5 |

A `bypass_mode: "always"` entry lets that role push directly to the
branch ignoring the ruleset — useful as an escape hatch when CI is
broken and you need to land a fix. Set to `"pull_request"` to only
allow bypass via PRs (the default for most rules).

## Required-status-checks names

The `required_status_checks[].context` strings must match the
**job display name** GitHub renders in PR check listings. For
reusable-workflow calls (jj-hooks.yml + jj-gt.yml calling
ci-base-rust.yml), the rendered name is the called job's `name:`
field, not the caller's job-id. See `docs/specs/platform/ci.md` for the full
mapping.

## Two-step rollout for new required checks

1. Land the workflow change first. The new job runs on the next
   PR; its display name appears in the PR's checks list.
2. After it's reported a conclusion on at least one PR, add it
   to the ruleset's `required_status_checks` list and re-apply.

Doing it in reverse locks the gate: the required check exists in
the ruleset but no commit on `main` has a corresponding status,
so nothing can merge until the workflow lands separately.

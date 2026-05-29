# CI matrix

zireael's CI lives in seven GitHub Actions workflows + one reusable
workflow. This page is the at-a-glance map: which workflow runs
when, on what runner, gated by which filter.

## Quick reference

| Workflow | File | Jobs | Triggers | Path filter | Runner |
| --- | --- | --- | --- | --- | --- |
| `jj-hooks` | `jj-hooks.yml` | jj-hooks CI (Lints + Test ×2 OSes) | PR | `jj-hooks.yml` | GitHub ubuntu / macos |
| `jj-gt` | `jj-gt.yml` | jj-gt CI (Lints + Test ×2 OSes) | PR | `jj-gt.yml` | GitHub ubuntu / macos |
| `akiflow-cli` | `akiflow-cli.yml` | akiflow-cli CI | PR | `akiflow-cli.yml` | GitHub ubuntu |
| `tap` | `tap.yml` | tap CI | PR | `tap.yml` | GitHub macos (needs `brew style`) |
| `docs` | `docs.yml` | docs CI | PR | `docs.yml` | GitHub ubuntu |
| `post-merge` | `post-merge.yml` | Rust lints + akiflow lints + docs + tap (cheap subset) | push: main | none | GitHub ubuntu / macos |
| `nightly` | `nightly.yml` | (full set, no filter) | cron + dispatch | none | GitHub ubuntu / macos |
| `release` | `release.yml` | matrix builds + publish + tap bump + crates publish | push: tags `v*` | none | GitHub (per-target) |
| _(reusable)_ | `ci-base-rust.yml` | Lints + Test matrix | `workflow_call:` from `jj-hooks.yml` + `jj-gt.yml` + `nightly.yml` | n/a | callee inherits |

## CI model

PR-only validation. Direct pushes to `main` don't fire any of the
PR workflows by design — the assumption is changes land via PRs
once branch-protection's required-status-checks gate is wired up.

The nightly cron catches drift from environmental changes (toolchain
auto-updates, registry cache invalidation, third-party API shape
changes — anything that doesn't show up as a diff in this repo).

Exceptions:

- `release.yml` keeps `push: tags v*` — it publishes binaries.
- `nightly.yml` keeps `schedule:` + `workflow_dispatch:` — the
  daily backstop.

### Graphite merge queue

zireael lives on a personal GitHub account, which doesn't support
Graphite's merge queue. The `withgraphite/graphite-ci-action`
optimize_ci step in each workflow is a no-op without
`GRAPHITE_TOKEN` set; it's wired up so the migration to an org
account (or Graphite extending MQ to personal repos) is a single
secret-add, not a workflow edit.

### Fixup pushes

`concurrency: cancel-in-progress: false` on PR workflows means a
fixup-push to a PR branch doesn't cancel the in-flight run for
the previous SHA — both finish to completion. Cost is ~1 wasted
CI run per fixup; cheaper than chasing cancelled checks against
required-status-checks gates.

## Runner choice

Zireael isn't large enough to justify Blacksmith. All workflows
run on GitHub-hosted runners.

| Surface | Runner |
| --- | --- |
| Linux ARM (release, nightly) | `ubuntu-24.04-arm` |
| Linux x64 (everything else Linux) | `ubuntu-latest` |
| macOS ARM (release, nightly, tap lint) | `macos-latest` |

Per-target release matrix:

| Target | Runner |
| --- | --- |
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |

No `x86_64-apple-darwin` target — none of the binaries shipped from
this repo run on Intel macs. Add `macos-26-intel` to release.yml's
matrix if that changes.

## Path filters

Single source of truth per filter, in `.github/path-filters/<name>.yml`.
Two consumers per filter:

1. The corresponding workflow (`dorny/paths-filter` reads the
   file directly).
2. Local `just ci` (the `_filter-touched <name>` recipe in
   `Justfile` parses the file with awk and compares to
   `jj diff --from main@origin`).

Drift between remote and local CI is structurally impossible —
both consumers read the same file.

| Filter | File | Consumers | Fires |
| --- | --- | --- | --- |
| `jj-hooks` | `jj-hooks.yml` | `jj-hooks.yml`, `just ci-jj-hooks` (via `_filter-touched`) | jj-hooks CI |
| `jj-gt` | `jj-gt.yml` | `jj-gt.yml`, `just ci-jj-gt` | jj-gt CI (filter includes `tools/jj-hooks/**` since jj-gt path-depends on it) |
| `akiflow-cli` | `akiflow-cli.yml` | `akiflow-cli.yml`, `just ci-akiflow-cli` | akiflow-cli CI |
| `tap` | `tap.yml` | `tap.yml`, `just ci-tap` | tap CI |
| `docs` | `docs.yml` | `docs.yml`, `just ci-docs` | docs CI |

### Filter parsing limits

The Justfile's `_filter-touched` recipe parses filter files with
awk and supports only:

- `path/**` (recursive directory match)
- `exact/file/path` (exact match)

`dorny/paths-filter` supports full glob syntax. If a filter file
ever uses a more exotic pattern (`tools/**/Cargo.toml`, character
classes, alternation), the Justfile recipe needs an expanded parser
to match. Today every filter file stays within the simple
`path/**` + exact-file subset.

## Trigger flow per event

### Feature-branch push (no PR open yet)

Nothing fires. CI doesn't validate feature-branch pushes in
isolation. The local `just ci` covers fmt + clippy + tests
before the push leaves the developer's machine; the path-filtered
remote workflows fire when the PR opens.

### `pull_request`

Always runs (per workflow that fires):

- **Optimize CI** (Graphite no-op without token)
- **Detect &lt;Thing&gt; Changes** (paths-filter probe)

Runs if path filter matches:

- **jj-hooks CI** (if `jj-hooks` matches) — calls ci-base-rust.yml:
  - **Lints (jj-hooks)** — fmt-check + clippy
  - **Test (jj-hooks, ubuntu-latest)** — cargo nextest
  - **Test (jj-hooks, macos-latest)** — cargo nextest
- **jj-gt CI** (if `jj-gt` matches) — same shape as jj-hooks CI
- **akiflow-cli CI** (if `akiflow-cli` matches) — bun install +
  biome + tsc + bun test
- **tap CI** (if `tap` matches) — brew style on the formulae
- **docs CI** (if `docs` matches) — markdownlint-cli2

A doc-only PR fires only the `docs` workflow — every other
workflow's `changes` job reports `changed=false` and the `ci` job
skips (`skipped` counts as passing for required-status-checks).

### `schedule` (nightly backstop)

06:00 UTC daily, plus `workflow_dispatch` for manual reruns.
Ignores path filters and runs every tool's full check set:

- **jj-hooks (nightly)** — full Rust matrix
- **jj-gt (nightly)** — full Rust matrix
- **akiflow-cli (nightly)** — bun lint + typecheck + test
- **tap (nightly)** — brew style
- **docs (nightly)** — markdownlint

`workflow_dispatch` runs the same set.

### `push: main`

Post-merge sanity checks via `post-merge.yml` — the cheap subset
of the PR-time gate, no path filter, runs unconditionally on
every push to main. Catches "main is broken now" within ~1 minute
without re-running the expensive bits already covered by the PR.

What runs:

- **Rust lints (post-merge)** — `cargo fmt --check` +
  `cargo clippy --workspace --all-targets` (~1m warm cache).
- **akiflow-cli lints (post-merge)** — `bun install` + `bunx
  biome check` + `bunx tsc --noEmit` (~30s).
- **docs (post-merge)** — `markdownlint-cli2 "**/*.md"` (instant).
- **tap (post-merge)** — `brew style Formula/*.rb` (~5s).

What does NOT run on push: main (relies on the PR check set +
nightly backstop):

- `cargo nextest run` across both Rust crates × two OSes.
- `bun test` for akiflow-cli (~290 tests, 5min p95).

**Dedup against PRs:** GitHub doesn't natively dedup
"this commit was just merged from a green PR" — a squash-merge
creates a new commit on main with a new SHA, so the PR's checks
don't transfer. You'd need custom workflow logic that queries
the API for "find the closed PR for this SHA, check its CI
results" — ~50 LoC, not worth the complexity for zireael's
scale. The pragmatic answer: the cheap subset on push:main
catches the obvious breakages (a fmt regression slipped in via
admin bypass, a markdown lint regression in a doc commit) at
trivial cost.

Other push:main triggers (not CI):

- `release.yml` — `push: tags v*` triggers the release matrix,
  tap-bump, and crates publish.

### `push: tags v*`

Runs the release matrix:

- jj-hooks + jj-gt binaries built per-target (Rust matrix)
- akiflow-cli `af` binary built per-target (bun compile matrix)
- One GitHub Release attached with every tarball + .sha256
- Tap formulae auto-bumped + committed back to main (skipped on prereleases)
- `cargo publish jj-hooks` then `cargo publish jj-gt` (skipped on prereleases)

Prerelease tags (`v0.3.0-rc.1`) skip the tap-bump + cargo-publish
jobs so the version number isn't burned on crates.io or the tap.

## Required status checks

Branch protection enforces the following checks pass (or skip —
`skipped` counts as passing). Wired via the GitHub Ruleset JSON
at `.github/rulesets/main-protection.json`. Apply with
`gh api repos/mattwilkinsonn/zireael/rulesets -X POST --input
.github/rulesets/main-protection.json`.

| Required check | Workflow | Notes |
| --- | --- | --- |
| `Lints (jj-hooks)` | jj-hooks CI / ci-base-rust | Always runs when jj-hooks fires |
| `Test (jj-hooks, ubuntu-latest)` | jj-hooks CI / ci-base-rust | Always |
| `Test (jj-hooks, macos-latest)` | jj-hooks CI / ci-base-rust | Always |
| `Lints (jj-gt)` | jj-gt CI / ci-base-rust | Always when jj-gt fires |
| `Test (jj-gt, ubuntu-latest)` | jj-gt CI / ci-base-rust | Always |
| `Test (jj-gt, macos-latest)` | jj-gt CI / ci-base-rust | Always |
| `akiflow-cli CI` | akiflow-cli.yml | Inline single-job |
| `tap CI` | tap.yml | Inline single-job |
| `docs CI` | docs.yml | Inline single-job |

In addition to status checks, the ruleset enforces:

- **Squash-merge only.** `rebase` and `merge` (merge commit) are
  disabled. PRs land as a single squashed commit on `main`,
  keeping history linear by construction.
- **Code-owner review required** — see `.github/CODEOWNERS` for
  the assignment table. Paired with `required_approving_review_count: 0`
  so a single-author repo isn't locked out today; the file is in
  place for when collaborators arrive and the approval count
  bumps to 1.
- Linear history, no force-push, no branch deletion, conversation
  resolution required.

Doc-only PRs see most status checks reported as `skipped` (path
filter didn't match) and merge fine — the gate passes when every
check reports a conclusion, regardless of whether it actually ran.

GitHub's ruleset keys required-status-checks on the **job display
name** (the value after `name:` in the job, or the rendered name
of a matrix expansion). When you add a new job or rename one,
two-step rollout:

1. Land the workflow change first (job appears in PR checks).
2. After it's reported a conclusion on at least one PR, add it
   to the ruleset's required-status-checks list.

Reverse order locks the queue: the new check is required but no
PR has reported on it yet, so nothing merges until the workflow
lands separately.

## Local `just ci`

`just ci` mirrors the same gating locally:

- Reads `.github/path-filters/<name>.yml` via `_filter-touched`.
- For each filter whose pattern matches the working-copy diff
  (`jj diff --from main@origin --to @`), runs the corresponding
  `just ci-<tool>` recipe.
- `just ci-all` ignores the filter and runs every tool — useful
  for belt-and-braces before pushing.

Per-tool recipes:

| Recipe | Commands |
| --- | --- |
| `ci-jj-hooks` | `cargo fmt -p jj-hooks --check` + clippy + nextest |
| `ci-jj-gt` | same shape; live tests excluded (`-E 'not test(gh_live)...'`) |
| `ci-jj-gt-live` | live integration tests; needs `JJ_GT_LIVE_*` env |
| `ci-akiflow-cli` | bun install + biome + tsc + bun test (5min cap) |
| `ci-tap` | `brew style Formula/*.rb` (degrades to warn on Linux) |
| `ci-docs` | `markdownlint-cli2 "**/*.md"` |

Drift between local and remote is structurally minimised by:

- Same path-filter files consumed by both.
- Same lint / test invocations (the recipes wrap the same `cargo
  fmt` / `bunx biome check` etc. that the remote workflows run).
- Same config files: `clippy.toml`, `biome.json`,
  `.markdownlint-cli2.jsonc`, `hk.pkl`, all in the repo root or
  tool dir.

The remaining gap is environment: a contributor running an older
toolchain, no `markdownlint-cli2` on PATH, etc. The
`install-deps` recipe at repo root tries to backfill that for
mattfw / mattpc-wsl / Darwin via per-tool delegated install
recipes.

## Fail-fast policy and timeouts

Every test step has a `timeout-minutes` cap so a hung test
doesn't burn an hour of runner time. `--no-fail-fast` on
`cargo nextest` so one failure doesn't mask the rest.

Current values:

| File | Surface | Step | Job |
| --- | --- | --- | --- |
| `ci-base-rust.yml` | Lints | (sub-second) | 10m |
| `ci-base-rust.yml` | Test (per package, per OS) | (default) | 20m |
| `akiflow-cli.yml` | bun test | 3m | 5m |
| `tap.yml` | brew style | (instant) | 5m |
| `docs.yml` | markdownlint | (instant) | 3m |
| `release.yml` | per-target build | (build dominates) | 30m |
| `release.yml` | publish jobs | (instant) | 5–10m |
| `nightly.yml` | each tool's full run | (uses ci-base-rust caps) | 20m |

`release.yml`'s `strategy.fail-fast: false` lets the Linux x64 and
Linux ARM builds complete even if macOS fails (or vice versa) so
you see every target's failure mode in one run.

## Maintenance notes

- **Adding a new tool** — see the
  [phase A → phase B → phase C](../zireael-migration-plan.md) flow
  for the canonical layout. The new tool needs:
  1. Path filter at `.github/path-filters/<tool>.yml`
  2. Workflow at `.github/workflows/<tool>.yml` (consume
     `ci-base-rust.yml` if Rust, inline otherwise)
  3. Path entry in `.github/workflows/release.yml`'s matrix +
     the bump-formulae.py script
  4. `hk.pkl` step gated on the new file glob
  5. `Justfile` recipe + entry in `just ci`'s auto-detect chain
  6. Update the required-status-checks list in the ruleset JSON
     (after at least one PR has run the new workflow)
- **Renaming a job** — the required-status-check string changes
  too. Follow the two-step rollout (above) to avoid locking the
  queue.
- **Bumping an action version** — generally safe; verify with
  actionlint before merging. The big breaking points are
  `upload-artifact@v5+` (unique artifact names per run) and
  `download-artifact@v8+` (merge-on-default).

# CI matrix

zireael's gate is **`moon ci`** — one command that runs every affected
`runInCI` moon task, identical locally (the `hk` pre-push hook) and in CI
(`.github/workflows/ci.yml`). Toolchains are pinned by `proto`
(`.prototools`) + `rust-toolchain.toml` and provided by a `devenv`
dev-shell (`devenv.nix`); moon runs *system* tasks against that PATH (no
`.moon/toolchain.yml`). This page maps which workflow runs when, on what
runner.

## Quick reference

| Workflow | File | Runs | Triggers | Runner |
| --- | --- | --- | --- | --- |
| `ci` | `ci.yml` | `moon ci` (affected) | PR | ubuntu |
| `post-merge` | `post-merge.yml` | `moon ci` scoped to the merge diff | push: main | ubuntu |
| `nightly` | `nightly.yml` | `moon run :ci` (full) | cron + dispatch | ubuntu + macos |
| `release` | `release.yml` | matrix builds + publish + tap bump + crates publish | push: tags `v*` | per-target |

## The moon gate

`moon ci` resolves the affected `runInCI` tasks (scoped to the diff via
`MOON_BASE`/`MOON_HEAD`) and runs them with moon's cache. One command
covers every project:

| Project | Source | `ci` task deps |
| --- | --- | --- |
| `jj-hooks` | `tools/jj-hooks` | `fmt` + `clippy` + `test` (cargo) |
| `jj-gt` | `tools/jj-gt` | `fmt` + `clippy` + `test` (cargo; `dependsOn: jj-hooks`) |
| `tap` | `Formula` | `brew-style` (guarded when brew absent) |
| `root` | `.` | `markdownlint` + `actionlint` + `nixfmt` + `deadnix` (devenv.nix) |

Non-gate tasks are `runInCI: false`: `jj-gt:test-live` (needs
`JJ_GT_LIVE_*` creds), `root:release`, `root:install-debug`.

### Toolchain

- `.prototools` pins `bun`, `node`, `moon` (the single version source).
- `rust-toolchain.toml` pins the exact Rust channel; `devenv.nix` builds
  it via fenix.
- `devenv.nix` provides proto, the Rust toolchain, every linter
  (nixfmt/deadnix/statix/nil/shellcheck/shfmt/taplo/actionlint/markdownlint),
  `hk`, `jj`, and the hook-framework backends jj-hooks' tests drive
  (pre-commit/prek/lefthook/pkl). `.envrc` enters it via `use devenv`.

## CI model

PR-validated via `moon ci`. Direct pushes to `main` land via squash-merge
(branch protection); `post-merge.yml` re-runs the affected gate on the new
`main` SHA to confirm the squashed commit is green. The nightly cron
catches drift from environmental changes (toolchain auto-updates, registry
cache invalidation) that don't show as a diff.

### Graphite skip

The `withgraphite/graphite-ci-action` optimize_ci step is a no-op without
`GRAPHITE_TOKEN`; it's wired so migrating to an org account (or Graphite
extending MQ to personal repos) is a single secret-add.

### Fixup pushes

`concurrency: cancel-in-progress: false` on `ci.yml` — a fixup-push
doesn't cancel the in-flight run; both finish. ~1 wasted run per fixup,
cheaper than chasing cancelled required-checks.

## Runner choice

GitHub-hosted runners throughout.

| Surface | Runner |
| --- | --- |
| Linux x64 (`moon ci`, post-merge, nightly full) | `ubuntu-latest` |
| macOS (tap, nightly macOS leg) | `macos-latest` |
| Linux ARM (release) | `ubuntu-24.04-arm` |

Per-target release matrix: `aarch64-apple-darwin` → `macos-latest`;
`x86_64-unknown-linux-gnu` → `ubuntu-latest`; `aarch64-unknown-linux-gnu`
→ `ubuntu-24.04-arm`. No `x86_64-apple-darwin` target — nothing shipped
here runs on Intel macs.

## Affected detection

moon scopes `ci` to the diff via `MOON_BASE`/`MOON_HEAD`:

- **`ci.yml` (PR):** base = PR base SHA, head = PR head SHA.
- **`post-merge.yml` (push: main):** base = `github.event.before`, head =
  the new SHA — the merge range.
- **`nightly.yml`:** no base — `moon run :ci` runs every project's `ci`
  unconditionally (a fresh runner has no moon cache, so everything
  re-runs, which is the point: catch upstream drift).

There are no path filters: `moon ci` on Linux is the whole PR gate.

## Trigger flow per event

### Feature-branch push (no PR)

Nothing fires. The local `hk` pre-push hook runs `moon ci` before the push
leaves the machine.

### `pull_request`

- **Optimize CI** (Graphite no-op without token)
- **moon ci (linux)** — the full affected gate
- **moon CI** — the rollup, the single required status check

### `schedule` (nightly backstop)

06:00 UTC daily + `workflow_dispatch`. Skipped when `main` hasn't advanced
since the last nightly. Runs `moon run :ci` (full, all projects) on Linux and
`tap:ci` on macOS. A `report` job posts the aggregate status to the HEAD
commit.

### `push: main`

`post-merge.yml` runs `moon ci` scoped to the merge range. Non-blocking;
confirms the squashed `main` SHA is green.

### `push: tags v*`

`release.yml`: per-target binary builds (Rust matrix),
one GitHub Release with every tarball + `.sha256`, tap-formula auto-bump
committed back to `main`, then `cargo publish jj-hooks` → `jj-gt`.
Prerelease tags (`v0.3.0-rc.1`) skip the tap-bump + cargo-publish jobs.

## Required status checks

Branch protection (GitHub Ruleset JSON at
`.github/rulesets/main-protection.json`) requires the single **`moon CI`**
rollup context (the `ci` job in `ci.yml`, `if: always()`, treating
`success`/`skipped` as pass). Apply with
`gh api repos/mattwilkinsonn/zireael/rulesets/<id> -X PUT --input
.github/rulesets/main-protection.json`.

The ruleset also enforces: squash-merge only; code-owner review required
(paired with `required_approving_review_count: 0` so a single-author repo
isn't locked out); linear history; no force-push; no branch deletion;
conversation resolution required.

GitHub keys required-status-checks on the **job display name**. When you
rename the rollup job, follow the two-step rollout: land the workflow
change first (the new name appears in PR checks), then after it reports a
conclusion on one PR, update the ruleset — reverse order locks merging
(the required check exists but no commit has reported it).

## Local gate

`hk`'s pre-push hook is a one-line shell over `moon ci` (`hk.pkl`),
installed on devenv shell entry (`hk install`, idempotent). It runs the
same affected gate as CI. Bypass with `HK=0 git push` or
`git push --no-verify`.

Dev bootstrap is `direnv allow` — the devenv shell provides the whole
toolchain; there is no separate install step.

## Fail-fast policy and timeouts

Every job has a `timeout-minutes` cap. `cargo nextest` uses
`--no-fail-fast` so one failure doesn't mask the rest.

| File | Job | Cap |
| --- | --- | --- |
| `ci.yml` | moon ci (linux) | 30m |
| `post-merge.yml` | moon ci | 30m |
| `nightly.yml` | moon run :ci / tap | 45m |
| `release.yml` | per-target build | 30m |

`release.yml`'s `strategy.fail-fast: false` lets each target's failure
surface in one run.

## Maintenance notes

- **Adding a new project** — create its `<dir>/moon.yml` (system tasks +
  a `ci` aggregate), register it in `.moon/workspace.yml`. `moon ci` picks
  it up automatically; no workflow, path-filter, or ruleset edit needed
  (the single `moon CI` rollup already gates it).
- **Adding a gate check** — add a task to the relevant `moon.yml` (with a
  `runInCI` default of true) and wire it into that project's `ci` deps.
  Local + CI pick it up with no workflow change.
- **A new release target** — add it to `release.yml`'s matrix + the
  `bump-formulae.py` script.
- **Bumping bun/node/moon** — edit `.prototools` (proto re-installs on the
  next shell entry / CI run). Bumping Rust — edit `rust-toolchain.toml`
  and the fenix `sha256` in `devenv.nix`.
- **Bumping an action version** — verify with actionlint. Breaking points:
  `upload-artifact@v5+` (unique names per run), `download-artifact@v8+`
  (merge-on-default).

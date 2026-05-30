# Changelog

All notable changes to this repository are tracked here. The monorepo uses
a single version number that applies to all tools published from a given
release tag (jj-hooks, jj-gt, akiflow-cli, and the Homebrew formulae).

## [Unreleased]

### Added — jj-hooks

- `jj-hooks.setup` config: declare a list of commands that run inside the
  ephemeral worktree before the hook runner fires. Use this when hooks
  depend on install-time resources (`node_modules`, `.venv`, etc.) that
  aren't in the committed tree — the worktree starts without them, so a
  setup step like `bun install --frozen-lockfile` is what puts them in
  place. Each step is an array-of-tables entry with an optional `name`
  and a required `run` argv. Issue jj-hooks#9.
- `JJ_HOOKS_WORKSPACE` env var: set by jj-hp for every setup step and
  hook subprocess, pointing at the workspace `jj-hp` was invoked from.
  Setup steps that prefer copying / hardlinking install resources over
  a full reinstall can `cp -al "$JJ_HOOKS_WORKSPACE/node_modules" .`.
- `jj-hp run --all-files`: ignore the revset's diff range and run every
  hook against every tracked file in the worktree. Useful for "lint
  everything once" after a wide refactor without crafting a revset that
  happens to cover every glob-gated step. Per-runner mapping:
  `pre-commit`/`prek`/`lefthook` use `--all-files`; `hk` uses
  `--glob '*'` (its documented `-a/--all` flag doesn't actually override
  stage-hook ref bounds in v1.45.0). `jj-hp push` always uses the diff
  range — the bookmark's ref bounds are its identity.

## [0.3.0] — 2026-05-26

Initial monorepo release. Consolidates four previously-standalone repos:

- `mattwilkinsonn/jj-hooks` (was at v0.2.1)
- `mattwilkinsonn/jj-gt` (was at v0.1.0)
- `mattwilkinsonn/akiflow-cli` (fork; was at v0.1.1)
- `mattwilkinsonn/homebrew-tap`

### Added — jj-hooks (was v0.2.1 → now v0.3.0)

- Retry-after-fixup recovery path: when hooks fail AND produce a fixup
  commit (e.g. hk's intra-bookmark step parallelism racing on
  `.git/index.lock` while a separate auto-fixing step legitimately
  modifies files), jj-hooks re-runs the hook backend against the fixup
  commit. If the re-run is clean, the abort message becomes a single
  "hooks modified files; re-run on fixup commit was clean" line instead
  of the confusing two-line "hook failed + hooks modified files" output.
  Opt-out via `--no-retry-after-fixup`. Issue jj-hooks#11.
- `HookOutcome` grew `retried: bool` and `initial_failure: bool` fields.
  `run_for_update` now takes a `RunOpts` parameter. **Breaking change**
  for any external `jj_hooks::hooks` consumer; the only known one was
  `jj-gt`, which is updated in this same release.

### Added — jj-gt (was v0.1.0 → now v0.3.0)

- Version jump aligns with the monorepo's single-version policy. No
  behavioral changes vs the previous v0.1.0; just absorbs the
  retry-after-fixup recovery transparently through jj-hooks.

### Added — akiflow-cli (was v0.1.1 → now v0.3.0)

- Version jump aligns with the monorepo's single-version policy. No
  behavioral changes vs the previous v0.1.1. The upstream MIT license is
  preserved at `tools/akiflow-cli/LICENSE`; the rest of the monorepo is
  dual-licensed under MIT OR Apache-2.0.

### Changed — repository structure

- Switched from four standalone repos to one monorepo. Old repos are
  archived on GitHub with banners pointing here.
- `brew install` path changed from `mattwilkinsonn/tap/<name>` to
  `mattwilkinsonn/zireael/<name>` (tap is now in-repo at `tap/`).
- Local CI: `just ci` at the monorepo root auto-detects which tools'
  paths the working-copy diff touches and runs only those tools'
  recipes. Same recipes the remote GitHub workflows invoke. Mirrors the
  pattern from `sealedsecurity/seal`.

[Unreleased]: https://github.com/mattwilkinsonn/zireael/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/mattwilkinsonn/zireael/releases/tag/v0.3.0

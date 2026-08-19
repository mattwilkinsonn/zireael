# Design: jj-hp pre-push gate — ephemeral-worktree cache cost + env-blind boundary

Status: **proposed**
Domain: tools

Tracking issue: `mattwilkinsonn/zireael#294`. Crate: `tools/jj-hooks` (binary
`jj-hp`), released, currently v0.3.10. Follow-up to #289/#291 (the v0.3.9
`repo_env` devenv-propagation fix, `docs/designs/tools/jj-hp-devenv-hook-env.md`).

## Problem / Intent

The pre-push gate runs `moon ci` (via hk) inside a fresh detached worktree
under `/tmp/jj-hooks-worktree-*`. Two DISTINCT residual failure modes, both
source-proven this session, neither a moon bug (the moon-fork lane is dropped:
moon 2.3.5 already scopes affected correctly and is not jj-blind for our usage;
the fix lives entirely in `tools/jj-hooks`):

- **Mode A — cold ephemeral-worktree `target/` (the "hang").** NOT an
  affected-scoping bug: bare `moon ci` on a 1-file change resolves
  `Resolved targets: 2` (measured, moon 2.3.5), tight in every configuration.
  NOT a missing compiler cache either: **sccache is already active and
  effective** (measured this session — see Evidence base). The real cost is
  that the fresh `/tmp` worktree's cargo `target/` is EMPTY, so the
  correctly-scoped, sccache-backed build still pays a from-scratch link +
  incremental-cache miss. Measured on the actual gate command
  (`cargo clippy -p jj-hooks --all-targets`, sccache active): **~35s in a cold
  `/tmp` worktree even with a warm sccache**, versus **~2-3s incremental in a
  warm `target/`**. sccache caches individual rustc invocations across clones
  but does NOT cache proc-macro/build-script crate outputs or the final link,
  and cargo's incremental compilation is disabled under `RUSTC_WRAPPER` — so a
  warm `target/` is the only thing that closes the last order of magnitude. The
  fix is **`target/` locality**, complementary to (not a replacement for)
  sccache.
- **Mode B — env-blind gate (the `.envrc`-not-allowed boundary).** #291's
  `apply_repo_env` propagates the devenv env into the gate subprocess
  (`hooks.rs:1028`), but only when the primary workspace's `.envrc` is
  `direnv allow`ed; otherwise `compute` hits `report_blocked` and degrades to
  `EnvPatch::Disabled`, and the gate runs without the repo toolchain. "Blocked"
  is a property of the PRIMARY workspace root (where the env is computed), NOT
  the ephemeral worktree — a normally-used clone is never blocked. It fires on
  a clone whose `.envrc` was never allowed (a freshly-provisioned clone before
  anything has allowed it). OMP's `bash.direnv: "auto"` already auto-allows for
  agent clones it has run a command in, so this is a residual (non-OMP usage,
  or a clone before its first OMP bash command), but a real one.

The two modes are independent — separately designed, separately landable,
each its own red→green test.

## Global Constraints

- **The frozen v0.3.9 non-regression contract holds** (inherited from the
  sibling record, frozen on merge). When `workspace_root` has no `.envrc`,
  direnv is absent, the export fails, the mechanism is opted out — OR (Mode B)
  a `direnv allow` attempt itself fails — the subprocess environment MUST be
  byte-identical to pre-fix: parent env + `JJ_HOOKS_WORKSPACE`. Env-load
  failure is never fatal — warn once (tracing) and fall back.
- **The git repo-location strip stays load-bearing.** `apply_repo_env`
  unconditionally strips the git repo-location env family (derived at runtime
  from `git rev-parse --local-env-vars`) from the child (`repo_env.rs:100-136`),
  because this repo's secondary-workspace `.envrc.local` sets `GIT_DIR` at the
  primary `.git`; propagated into the detached worktree it would corrupt the
  primary index. **A Mode A `CARGO_TARGET_DIR` injection MUST NOT reintroduce
  this hazard**: the injected variable is jj-hp-authored, is never a `GIT_*`
  variable, and is applied AFTER `apply_repo_env` so a repo-env-carried value
  can never win over it.
- **Primary-`target/` contention is accepted (Mode A).** Pointing the gate's
  `CARGO_TARGET_DIR` at the PRIMARY workspace's `target/` is deliberate: it
  reuses the user's own warm dev builds, so even the first gated push on a Rust
  change is near-instant. The cost — cargo's `.cargo-lock` briefly serializes
  the gate against a live `rust-analyzer`/`cargo build` in the primary, and a
  gate build of the pushed commit may overwrite an artifact the working copy
  differs on (a subsequent dev recompile of that one crate) — is accepted as a
  few-second wait, never corruption (cargo fingerprints by content, so sharing
  causes recompiles, never a false green). An opt-out exists for users who want
  isolation. This note covers the single-push/live-editor case; the heavier
  N-divergent-commit jj-gt batch contention is treated separately (the Mode A
  concurrency note and Open Question A residual).
- **`CARGO_TARGET_DIR` is set unconditionally, not Rust-gated.** Simpler than
  detecting whether a push touches Rust, and harmless for non-cargo repos:
  nothing in a JS/nix gate reads `CARGO_TARGET_DIR`. A non-Rust-primary
  consumer repo may regrow first-party Rust over time, so an always-on
  mechanism needs no revisit when it does.
- **sccache is present and stays the cross-clone layer.** The per-user nix
  profile provides sccache (`RUSTC_WRAPPER=sccache`,
  `SCCACHE_DIR=~/.cache/sccache`, backed by a shared redis L2); this repo
  inherits it from the profile. This design adds NO compiler-cache mechanism —
  it adds `target/` locality on top of the existing sccache. The two are
  complementary (sccache = per-rustc-invoke across clones; `target/` = link +
  incremental within a clone).
- **Version floors.** Rust toolchain `1.96.0` (`rust-toolchain.toml`). moon pin
  `2.3.5`, bun `1.3.14`, node `24.18.0` (`.prototools:6-8`). No new runtime
  dependency for either mode (sccache already present).
- **Red→green tests.** Each code task carries a test that fails before the
  change and passes after, hermetic where possible, on the existing harness:
  the tempdir remote+primary fixture (`tests/harness/mod.rs:16-29`), the
  batch-API tests (`tests/parallel_batch.rs:9-20`), the CLI pipeline tests
  (`tests/push.rs`), the real-direnv live fixture with isolated `XDG_*` state
  (`tests/repo_env_real.rs:38-72`, nextest-only per its header at `:14-20`).
- **Commit identity.** Each task lands as its own green PR; commits authored as
  seal with trailer `Co-authored-by: Matt Wilkinson <mattwilki17@gmail.com>`
  (`rule://commit-conventions`). This record ships first as a docs-only PR.
- **Markdownlint-clean** under `.markdownlint-cli2.jsonc` (MD013 off).

## Evidence base (cited this session)

File:line references were read this session in the clone at
`/home/mattw/agents/workspaces/zireael/zireael`.

- **sccache is active and effective (measured).** `sccache 0.16.0` on PATH via
  the per-user nix profile; `RUSTC_WRAPPER=sccache`,
  `SCCACHE_DIR=~/.cache/sccache`; live `sccache --show-stats`: **87.45% overall
  hit rate, 80.74% on Rust** (8050 hits / 1080 misses at time of read),
  multi-level with a redis L2. Inside `direnv exec` the wrapper is present. The
  design record's earlier "no compiler cache" premise was WRONG for local runs
  and is corrected here.

- **The cold-worktree cost is `target/`, not sccache (measured).**
  `cargo clippy -p jj-hooks --all-targets -- -D warnings` in a fresh detached
  worktree (`git worktree add --detach` off `HEAD`, empty `target/`), sccache
  active:
  - cold worktree, sccache coldish: **1m 06s**
  - cold worktree (empty `target/`), sccache warm from the prior run: **38s**
    (33s compile)
  - warm `target/` incremental (the primary repo): **~2-3s**

  So a fully-warm sccache still leaves ~35s of link + incremental-miss cost in
  a fresh `target/`; a warm `target/` is what removes it.

- **Worktree creation — where Mode A injects** (`worktree.rs:40-55`): a fresh
  `TempDir::with_prefix("jj-hooks-worktree-")` per bookmark per push, removed on
  `Drop` (`worktree.rs:83-97`). Nothing CWD-local survives between pushes — the
  cold `target/` is structural, hence the injected `CARGO_TARGET_DIR` must point
  OUTSIDE the worktree (at the primary).

- **The env seam — `run_subprocess`** (`hooks.rs:1017-1029`): builds the
  `Command`, `current_dir(cwd)` (the worktree), `apply_repo_env(&mut cmd,
  workspace_root)`, then `cmd.env("JJ_HOOKS_WORKSPACE", workspace_root)`.
  `workspace_root` here is the PRIMARY repo root (passed unchanged from
  `run_once`, `hooks.rs:933`/`:961`), so `CARGO_TARGET_DIR = workspace_root.join
  ("target")` targets the primary. Mode A slots one `cmd.env` AFTER
  `apply_repo_env`.

- **The twin setup-step seam** (`setup.rs:150-156`): same shape —
  `apply_repo_env(&mut cmd, workspace_root)` then `cmd.env("JJ_HOOKS_WORKSPACE",
  ...)`.

- **The #291 mechanism Mode B extends** (`repo_env.rs`):
  - `EnvPatch::{Disabled, Patch}` (`repo_env.rs:32-41`). Mode B adds NO variant
    — auto-allow resolves the blocked state into a normal `Patch`, so the enum
    is unchanged and there is no semver break on the published crate.
  - `compute` (`repo_env.rs:183-222`) — detection, export, cleared-`DIRENV_*`
    retry; the blocked branch hits `report_blocked` then degrades to `Disabled`
    (`:196-199`, `:206-209`). This is exactly where the auto-allow attempt slots.
  - `report_blocked` (`repo_env.rs:228-236`) — the one visible signal today.
  - `repo_env_enabled` / `enabled_from` (`repo_env.rs:165-180`) — the opt-out
    pattern Mode B's off-switch mirrors. Note the current lenient parse
    (`:179`: any non-`"off"` value enables); the new off-switches parse
    strictly (bool / explicit values) and warn on unrecognized input.
  - `apply_repo_env` (`repo_env.rs:100-136`) — merge + the unconditional
    git-family strip (call at `:106`, `strip_git_local_env` defined at `:151`).

- **Eager once-per-batch population — the three entrypoints**
  (`hooks.rs:159-160` in `run_for_update`, the CLI-push path via `push.rs` →
  `run_for_update` and `lib.rs:462`; plus the two batch fan-outs
  `hooks.rs:437-439` and `:560-562`). All three call
  `repo_env::repo_env(workspace_root, repo_env_enabled(jj))`. Because the
  auto-allow lives INSIDE `compute`'s blocked branch, it fires wherever
  `repo_env` is called — all three entrypoints — with no per-entrypoint logic.

- **Empirical repro (moon 2.3.5)**: bare `moon ci` on a 1-file change →
  `Affected by changes: all` (status-filter label), `Requested targets: 38`
  (pre-filter count), **`Resolved targets: 2`** (`root:markdownlint` +
  `jj-hooks:ci`) — tight in every tested config. Clean-env `direnv export json`
  (jj-hp's retry path) is complete (6 keys, ~11KB incl. `PROTO_HOME` + proto
  shim PATH), so Mode B is the not-allowed boundary, not export incompleteness.

- **Non-Rust-primary consumer repo (the other high-frequency gate consumer):**
  - It is a bun/TypeScript monorepo with ZERO first-party Rust. All Rust lives
    in vendored fork subtrees, and its `moon ci` EXCLUDES those subtrees, so
    ordinary TS work triggers NO Rust compile at all.
  - Each Rust fork's cargo tasks are a separate moon project scoped TIGHTLY:
    subtree-relative `sources` + the two repo-root toolchain-config files
    (`.cargo/config.toml`, `rust-toolchain.toml`). No over-selection — a fork
    rebuilds only when its own subtree or a root toolchain-config file changes.
    So the fork recompile an agent hits is LEGIT subtree work paying the
    cold-`target/` cost, which Mode A fixes; there is no moon-scoping bug to fix.
  - sccache is wired there too (`RUSTC_WRAPPER`, shared redis L2) and its
    `.cargo/config.toml` sets `-Zshare-generics=y`.

- **OMP auto-direnv (Mode B context):** OMP's `bash.direnv: "auto"` auto-allows
  a repo's `.envrc` on first bash in its tree; the host trust DB is broadly
  populated and the primary clones in use are allowed. jj-hp's auto-allow is
  therefore a backstop for the residual (a clone before its first OMP bash
  command, or non-OMP usage), not the primary path.

- **Test harness grounding**: tempdir remote+primary+`PRE_COMMIT_HOME` fixture
  (`tests/harness/mod.rs:16-29`); batch tests (`tests/parallel_batch.rs:9-20`,
  3-bookmark builder `:25-50`); real-direnv isolation
  (`tests/repo_env_real.rs:38-72` — `isolate_direnv_state`, `write_envrc`,
  `direnv_allow`) with the nextest-only requirement (`:14-20`).

## Approach

The two modes get independent, small mechanisms; either can land without the
other.

### Mode A — point the gate's `CARGO_TARGET_DIR` at the primary `target/`

At the subprocess seam, after `apply_repo_env`, set `CARGO_TARGET_DIR` to
`workspace_root.join("target")` (the PRIMARY repo's target dir), unconditionally
unless opted out:

- `run_subprocess` (`hooks.rs:1017-1029`) and `run_steps` (`setup.rs:150-156`)
  each gain one `cmd.env("CARGO_TARGET_DIR", …)` AFTER `apply_repo_env` (so a
  repo-env-carried value can never win) and alongside `JJ_HOOKS_WORKSPACE`.
- The gate's cargo builds reuse the user's own warm artifacts: the second push
  onward — and, because it's the PRIMARY target, even the FIRST gated push on a
  change the user already built locally — is a ~2-3s incremental build instead
  of ~35s.
- sccache continues underneath as the cross-clone rustc-invoke cache; this adds
  the link + incremental layer sccache can't provide.
- Frozen git-strip contract untouched: the injected variable is jj-hp-authored,
  never `GIT_*`, applied after `apply_repo_env`.
- Concurrency: the N parallel gate worktrees of a jj-gt batch, plus a live
  `rust-analyzer`, now share the primary `target/`. Cargo's `.cargo-lock`
  serializes the BUILD phase. For the single-worktree CLI push (the common
  path) this is a serialized warm build that beats a cold one. For the jj-gt
  BATCH path the win is conditional: N worktrees each build a DIFFERENT
  bookmark commit against one `target/`, and cargo fingerprints by content, so
  divergent Rust across the batch re-invalidates shared fingerprints,
  degrading toward serialized near-cold rebuilds rather than one warm build
  (never a false green — fingerprinting forces the recompile, it cannot skip a
  stale one). Build-compatible batches (no divergent Rust) still get the warm
  win. This batch degradation is accepted for the first release (the CLI push
  is the acute pain) and flagged as an Open Question residual for T1 to bound.
  Separately, the `cargo nextest` execution store lives at `<target>/nextest/…`
  outside the build lock; the CLI push is single-worktree (no concurrency), but
  the batch path runs N concurrent nextest against the shared store — T1 MUST
  verify this is safe and, if not, give the test step a per-worktree nextest
  store while build artifacts stay shared.
- Opt-out mirrors the repo-env pattern for users who want gate isolation from
  their primary `target/`: `JJ_HOOKS_NO_GATE_CACHE` env var and jj config
  `jj-hooks.gate-cache = "off"`, read once at the entrypoints, strict parse
  (only `"off"` disables; warn on any other non-empty value), never an ambient
  global.

**Rejected for Mode A:**

- **Dedicated `~/.cache/jj-hooks/<clone>/target`**: isolated from the user's
  editor, but cold once per clone (first Rust push pays ~35s) and never reuses
  the user's dev builds. The primary-`target/` contention it avoids is a
  few-second serialized wait; the reuse it gives up is the whole win. Rejected
  in favor of the primary `target/` (contention accepted, Global Constraints).
- **A new compiler cache (sccache/anything)**: already present and effective
  (87%/80% hit). Nothing to add.
- **Persistent-worktree-reuse** (keep `/tmp/jj-hooks-worktree-*` across pushes):
  a much larger lifecycle/GC change (`WORKTREE_CREATE_LOCK`, staleness, N live
  worktrees per batch). `target/` locality gets the same win without giving up
  the clean-tree-per-target-commit correctness of the ephemeral model.
- **`.moon/cache` locality**: a NON-ISSUE — every gated moon task is
  `cache: false` (`tools/jj-hooks/moon.yml:15,20,25,29`; the consumer repo's
  fork tasks likewise), so moon caches no task output for the gate. Cargo's
  `target/` is the only meaningful cache. Not a task.
- **A `--base`/`--head` wrapper for moon**: solves a non-problem —
  `Resolved targets: 2` is already tight.

### Mode B — auto-`direnv allow` the primary `.envrc`, with an off-switch

In `compute`'s existing blocked branch (`repo_env.rs:196-209`), when a `.envrc`
is present but not allowed and auto-allow is enabled (the default), jj-hp runs
`direnv allow <workspace_root>` itself, then re-runs the export and returns the
resulting `Patch` — resolving the blocked state instead of merely reporting it:

- **No new `EnvPatch` variant, no semver break, no `require` mode.** The blocked
  state is RESOLVED (allowed → normal `Patch`) rather than carried through the
  type system. This drops the entire `EnvPatch::Blocked` / `#[non_exhaustive]` /
  0.4.0-bump question — the published enum is unchanged.
- **Fires at all three eager-populate entrypoints for free**: the logic lives
  inside `compute`, which every `repo_env` call reaches, so the CLI-push path
  and both batch paths are covered with zero per-entrypoint code.
- **Trust boundary**: `direnv allow` marks the `.envrc` trusted so direnv will
  execute it. This is NOT a new trust boundary for the gate — the gate already
  runs the repo's own `moon` tasks (arbitrary code from the same tree). The one
  real side effect: `direnv allow` mutates the GLOBAL direnv trust DB, so direnv
  will thereafter auto-load that `.envrc` in the user's future INTERACTIVE
  shells too. That side effect is why the off-switch exists.
- **Off-switch**: `JJ_HOOKS_NO_DIRENV_ALLOW` env var and jj config
  `jj-hooks.repo-env-autoallow = false` (a bool on a new key, not a new string
  value on the existing `repo-env` key — a separate axis avoids the lenient
  string parse and keeps the two behaviors independent). When off, the blocked
  branch behaves exactly as today (`report_blocked` → `Disabled`,
  byte-identical env).
- **Never fatal**: if `direnv allow` or the re-export fails, warn once and fall
  back to today's `Disabled` (frozen contract). Auto-allow only ever improves
  the blocked case; it can never make a previously-working gate worse.
- **Backstop, not primary**: OMP auto-direnv already covers agent clones it has
  touched; this closes the residual (fresh clone pre-first-bash, non-OMP usage).

**Rejected for Mode B:**

- **Carry a `Blocked` variant + `require` hard-stop + loud failure message**
  (the prior design): once jj-hp can auto-resolve the blocked state, carrying it
  through a semver-breaking enum variant and aborting the push is
  over-engineered — the user's intent (a working gate) is better served by
  fixing the state than by refusing to run. Dropped.
- **Auto-`proto install` / reproducing devenv inside jj-hp**: a second,
  devenv-specific env-construction mechanism beside the generic direnv patch —
  the drift class #291 removed. Rejected; `direnv allow` reuses the one existing
  mechanism.

### Landing order

**Mode A first** (T1): the acute, frequent pain (every Rust push), purely
additive (one env var on the child), touches no contract. Mode B (T2) is
independent. T3 closes docs + release.

## Plan

First release ships T1 + T2 + T3.

### T1 — Mode A: primary-`target/` `CARGO_TARGET_DIR` for gate subprocesses

Wire into `run_subprocess` (`hooks.rs:1017-1029`) and `run_steps`
(`setup.rs:150-156`); add the opt-out read at the entrypoints.

Interfaces (small — a helper module `gate_cache.rs` or inline in `repo_env.rs`
alongside the opt-out pattern):

```rust
/// Read the opt-out once at the entrypoints (mirrors repo_env_enabled,
/// repo_env.rs:165-180; strict parse — only "off" disables, warn otherwise)
/// and cache it process-globally, exactly as `repo_env` caches its patch:
/// 1. JJ_HOOKS_NO_GATE_CACHE env var — any non-empty value disables.
/// 2. jj config `jj-hooks.gate-cache` — "off" disables; unset/"auto" enables.
pub fn gate_cache_enabled(jj: &crate::jj::JjCli) -> bool;

/// Set CARGO_TARGET_DIR on `cmd` to `<workspace_root>/target` — called AFTER
/// apply_repo_env so jj-hp's value wins over any patch-carried/inherited one.
/// Takes no `enabled` param: like `apply_repo_env`, it reads the opt-out from
/// the process-global set at the entrypoints, so it slots into the low-level
/// `run_subprocess`/`run_steps` spawn helpers (which have no `JjCli` in scope).
pub fn apply_gate_cache(cmd: &mut Command, workspace_root: &Path);
```

Consumes: `workspace_root` (already at both spawn sites); `JjCli` only at the
entrypoints for the opt-out read, cached process-globally and read back in
`apply_gate_cache` — mirroring how `apply_repo_env` reads `repo_env`'s cache,
so the low-level spawn helpers need no new parameter.

Produces: gate children run with `CARGO_TARGET_DIR = <primary>/target`; gate
builds reuse the user's warm artifacts.

Test cycle (red→green, existing harness):

- Unit: `gate_cache_enabled` — env opt-out beats config; `"off"` disables;
  unset/`"auto"` enables; an unrecognized value warns and enables.
- Integration (`TestRepo`, `tests/harness/mod.rs:16-29`): a pre-push hook whose
  script writes `$CARGO_TARGET_DIR` to a file; run the pipeline; assert the
  child saw `<primary>/target` (red before T1: unset). Twin through a setup step
  (`run_steps`).
- Precedence: parent env carries a decoy `CARGO_TARGET_DIR`; assert the child
  sees the jj-hp value, and with the opt-out set sees the decoy (byte-identical
  fallback).
- Frozen-contract guard: with gate-cache enabled, the child still has no
  `GIT_DIR`/git-family variable (composes with the existing strip tests) and no
  other new variable appears.
- Parallel: 3-bookmark stack via `run_for_updates_parallel`
  (`tests/parallel_batch.rs:25-50`); all three children report the SAME
  `CARGO_TARGET_DIR`, AND a real concurrent `cargo nextest run` from the N
  worktrees against the shared dir does not corrupt/false-fail (validates the
  nextest-store concurrency claim; if unsafe, T1 gives the test step a
  per-worktree store). Measure the divergent-Rust batch case (bookmarks with
  differing Rust) to bound the fingerprint-thrash degradation noted above.

Acceptance: driver-run — `jj-vine submit` of a 1-file Rust change completes the
gate in ~2-3s (not ~35s+) when the primary `target/` is warm, without
`--no-hooks`; `Resolved targets: 2` unchanged.

### T2 — Mode B: auto-`direnv allow` in `compute`, with off-switch

Extend `compute`'s blocked branch (`repo_env.rs:196-209`) and add the off-switch
read at the entrypoints (threaded like `repo_env_enabled`).

Behavior: blocked `.envrc` + auto-allow enabled (default) → run
`direnv allow <workspace_root>` → re-run the export → return the `Patch`. On
`direnv allow` failure or a still-failing re-export → warn once, fall back to
`Disabled` (byte-identical, as today). Auto-allow disabled → today's behavior
exactly (`report_blocked` → `Disabled`).

Consumes: the existing blocked detection (`repo_env.rs:196-199`,`:206-209`), the
export path, the config-read pattern (`:165-180`).

Produces: a freshly-provisioned clone's blocked `.envrc` is auto-resolved so the
gate gets the full toolchain; opt-out for users who don't want the global
trust-DB mutation.

Test cycle (red→green):

- Unit (fake-direnv shim, `src/repo_env.rs` tests): blocked signature + autoallow
  on → shim records a `direnv allow` invocation and the second export returns a
  `Patch` (red: today it returns `Disabled` with no allow); autoallow off → no
  allow invoked, returns `Disabled` (byte-identical); off-switch parse — env
  beats config, `false` disables, unset enables, unrecognized warns.
- Live fixture (`tests/repo_env_real.rs`, isolated `XDG_*`, nextest-only):
  blocked-before-allow with autoallow on → jj-hp allows it and the export
  yields a `Patch` carrying the repo env; with autoallow off → stays blocked
  (`Disabled`). Assert the isolated trust DB (not the user's global) records the
  allow, so the test is hermetic.
- Failure path: a `direnv allow` that fails (planted failing shim) → warn +
  `Disabled`, gate still runs (non-fatal contract).
- Pipeline (harness): blocked `.envrc`, autoallow on, passing hook → gate runs
  WITH the repo env (assert a toolchain-dependent step succeeds that fails when
  env-blind); autoallow off → unchanged from today.

Acceptance: a freshly-provisioned clone whose `.envrc` was never allowed runs
the gate with the full toolchain on first push (no manual `direnv allow`); the
off-switch restores today's behavior.

### T3 — docs + release

Update `tools/jj-hooks/README.md` (the primary-`target/` gate-cache mechanism +
`JJ_HOOKS_NO_GATE_CACHE`/`jj-hooks.gate-cache`; the auto-`direnv allow` behavior,
its global-trust-DB side effect, and the
`JJ_HOOKS_NO_DIRENV_ALLOW`/`jj-hooks.repo-env-autoallow` off-switch); changelog
entries referencing #294; version bump — the single `[workspace.package]
.version` field (root `Cargo.toml:10`, currently `0.3.10`; the internal
path-dep pin auto-rewrites on publish). No public API change (no `EnvPatch`
variant), so `0.3.11` is the natural bump.

Test cycle: `cargo fmt -p jj-hooks -- --check`, `cargo clippy -p jj-hooks
--all-targets -- -D warnings`, `cargo nextest run -p jj-hooks`, markdownlint on
touched docs — all green (`rule://pre-finish-checks`). Driver-run acceptance:
both T1 and T2 scenarios re-verified on the released binary.

## Open Questions

All prior open questions are resolved by Matt's rulings this session; recorded
here for the reviewer, with residuals flagged:

- **[A — mechanism] RESOLVED: sccache (already present) + primary-`target/`
  reuse, set unconditionally.** Measured: sccache alone leaves ~35s in a cold
  worktree; a warm primary `target/` closes it to ~2-3s. Set `CARGO_TARGET_DIR`
  always (not Rust-gated) — simpler, harmless for non-cargo repos, and a
  non-Rust-primary consumer repo may regrow first-party Rust. Contention with a
  live editor is accepted
  (few-second serialized wait, never corruption), off-switch provided. RESIDUALS
  for T1 to settle in code, not forks: (1) the `cargo nextest` execution store
  under a shared `target/` on the jj-gt BATCH path (N concurrent) — verify safe,
  else per-worktree nextest store; (2) N divergent-commit worktrees sharing one
  `target/` thrash cargo fingerprints under the serialized `.cargo-lock`,
  degrading the batch toward serialized rebuilds — bound it, or scope the
  warm-build guarantee to build-compatible batches, and add a divergent-commit
  case to the T1 parallel measurement.
- **[B — posture] RESOLVED: auto-`direnv allow` + off-switch.** Resolves the
  blocked state rather than carrying it — drops the `EnvPatch::Blocked` variant,
  the `require` mode, and the crates.io semver question entirely. Trust is not a
  new boundary (the gate already runs repo code); the only real cost is the
  global-trust-DB mutation, gated by the off-switch. OMP auto-direnv already
  covers the common agent case, so this is a backstop. Not fatal on failure.
- **[consumer repo — the hypothesis "a PR that didn't need to recompile a
  fork"] CHECKED: not a moon over-selection bug.** The consumer repo's fork
  tasks are tightly scoped (subtree `**/*` + two root toolchain-config files);
  a fork rebuilds only on genuine subtree/toolchain changes. The recompile an
  agent hit was legit fork-subtree work paying the cold-`target/` cost — Mode A
  fixes it. No moon-scoping change needed.

## Tasks

Landing order: T1 (Mode A) first — acute, frequent, contract-free; T2 (Mode B)
independent; T3 closes.

- [ ] T1 — Mode A: set `CARGO_TARGET_DIR = <primary>/target` on gate children
  at `run_subprocess` + `run_steps` after `apply_repo_env`, unconditional with
  opt-out `JJ_HOOKS_NO_GATE_CACHE` / `jj-hooks.gate-cache = "off"` (strict
  parse); unit + harness + precedence + frozen-contract-guard + parallel
  (concurrent-nextest-on-shared-dir) tests; driver acceptance (~2-3s warm push)
- [ ] T2 — Mode B: auto-`direnv allow` in `compute`'s blocked branch (resolves
  to `Patch`; no `EnvPatch` variant), off-switch `JJ_HOOKS_NO_DIRENV_ALLOW` /
  `jj-hooks.repo-env-autoallow = false`, never-fatal fallback to `Disabled`;
  fake-shim, live-fixture (hermetic trust DB), failure-path, and pipeline tests
- [ ] T3 — README (gate-cache + auto-allow + trust-DB side effect + both
  off-switches) + changelog + version bump to `0.3.11` (root `Cargo.toml:10`);
  full fmt/clippy/nextest/markdownlint green; driver re-acceptance on the
  released binary

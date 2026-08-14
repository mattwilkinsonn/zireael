# Design: jj-hp runs hook subprocesses inside the repo devenv

Status: **proposed**
Domain: tools

Tracking issue: `mattwilkinsonn/zireael#289`. Crate: `tools/jj-hooks` (binary
`jj-hp`), released, currently v0.3.8.

## Problem / Intent

`jj-hp` runs pre-push hooks (`moon ci` via hk, and the other runners) inside an
ephemeral detached worktree under `/tmp`, and the hook subprocess inherits only
jj-hp's own process environment — the repo's direnv/devenv environment is never
loaded. Any tool the runner shells out to (moon, biome, proto shims) therefore
resolves against the *system* PATH instead of the devenv pins, producing
false-red gates (system biome 2.4.16 vs devenv 2.5.4) and hard failures (dead
`~/.proto` shims breaking `moon ci` proto detection). Matt ruled the fix: jj-hp
propagates the repo's devenv/direnv environment into the hook subprocess, so
the local gate runs in the same environment CI does (`devenv shell -- moon
ci`). This record designs the mechanism; the class-level choice is settled.

## Global Constraints

- **No regression for non-direnv repos.** jj-hooks is a released public tool.
  When `workspace_root` has no `.envrc`, or `direnv` is not on jj-hp's PATH, or
  the export fails for any reason (unallowed `.envrc`, direnv error), the
  subprocess environment MUST be byte-identical to today's: inherit the parent
  env plus `JJ_HOOKS_WORKSPACE`. Failure to load the env is never fatal — warn
  once (tracing) and fall back.
- **Captured-output purity.** `run_subprocess` folds child stdout+stderr into
  the user-facing failure buffer (`hooks.rs:1014-1018`); `run_steps` does the
  same (`setup.rs:161-165`). The env-acquisition step MUST NOT leak direnv's
  output into those buffers. The guarantee is structural: the export runs as
  a *separate* subprocess whose stdout/stderr jj-hp pipes itself and routes
  only to `tracing` — no channel exists from the export to the capture
  buffer. `DIRENV_LOG_FORMAT=""` is set as a secondary nicety to reduce
  direnv's own chatter; it is NOT the guarantee (empirically it does not
  silence devenv's bootstrap stderr — see F4 below).
- **Export runs against `workspace_root`, never the temp worktree.** The
  `/tmp/jj-hooks-worktree-*` path is not direnv-allowed; the allowed `.envrc`
  lives at `workspace_root`, which `run_subprocess` and `run_steps` already
  receive.
- **Once per (invocation × workspace_root).** A parallel batch spawns N
  subprocesses per bookmark (one per `from_ref`, `hooks.rs:916-942`) across M
  bookmarks; the devenv env is computed exactly once per jj-hp process and
  reused, following the `PklWarmCache` per-run-cache precedent
  (`hooks.rs:336-370`).
- **Version floors.** Rust toolchain `1.96.0` (`rust-toolchain.toml`, cited in
  the sibling record). `direnv export json` has been stable since direnv 2.x —
  no new floor imposed on users; absence of direnv simply means fallback.
  New dep: `serde_json` (already in the workspace catalog, root
  `Cargo.toml:29` — `serde_json = "1.0.145"`).
- **Commit identity.** Each task lands as its own green PR; commits authored as
  seal with trailer `Co-authored-by: Matt Wilkinson <mattwilki17@gmail.com>`
  (`rule://commit-conventions`). This record ships first as a docs-only PR
  (no `docs/specs/` files touched, so no `Spec-impact:` line required).
- **Markdownlint-clean** under `.markdownlint-cli2.jsonc` (MD013 off, MD060
  compact).
- **Red→green tests.** Each code task carries a test that fails before the
  change and passes after, hermetic where possible (fake `direnv` shim on a
  test-controlled PATH). The cross-repo compass biome repro is the driver's
  acceptance test, not a unit test.
- **Git repo-location env is stripped from the patch.** `apply_repo_env`
  removes the git repo-location env family — at minimum `GIT_DIR`,
  `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`
  (the `git rev-parse --local-env-vars` set) — before the patch reaches the
  child. Load-bearing: this repo's documented secondary-workspace flow drops
  a `.envrc.local` setting `GIT_DIR` at the primary repo's `.git`
  (`.envrc:14-17`); propagated into the detached temp worktree, it would make
  every git invocation inside the hook child (hk's diff computation, moon's
  affected detection) resolve HEAD/index against the PRIMARY workspace —
  wrong changed-file sets at best, primary-index mutation at worst. Enforced
  with a T1/T2 test.

## Evidence base (cited this session)

All file:line references were read this session in the clone at
`/home/mattw/agents/workspaces/zireael/zireael`.

- **Bug site 1 — hook subprocess env** (`tools/jj-hooks/src/hooks.rs:1000-1003`):

  ```rust
  let mut cmd = Command::new(&argv[0]);
  cmd.args(&argv[1..])
      .current_dir(cwd)
      .env("JJ_HOOKS_WORKSPACE", workspace_root);
  ```

  `cwd` is the temp worktree (`wt.path()` at the call sites,
  `hooks.rs:910` and `hooks.rs:938`). The child inherits only jj-hp's env plus
  that one variable. Signature (`hooks.rs:994-999`):

  ```rust
  fn run_subprocess(
      argv: &[String],
      cwd: &Path,
      workspace_root: &Path,
      capture: Option<&mut String>,
  ) -> Result<bool>
  ```

- **Bug site 2 — setup-step env** (`tools/jj-hooks/src/setup.rs:150-154`):

  ```rust
  let output = Command::new(&step.run[0])
      .args(&step.run[1..])
      .current_dir(worktree)
      .env("JJ_HOOKS_WORKSPACE", workspace_root)
      .output()?;
  ```

  Same shape, same gap. `run_steps` signature (`setup.rs:133`, doc comment above at `:132`):
  `pub fn run_steps(steps: &[SetupStep], worktree: &Path, workspace_root: &Path) -> Result<String>`.

- **Third spawn site — hk cache warm** (`tools/jj-hooks/src/hooks.rs:303-308`):

  ```rust
  fn run_hk_validate(argv: &[String], cwd: &Path) -> bool {
      tracing::info!("warming hk Pkl cache: {argv:?}");
      match Command::new(&argv[0])
          .args(&argv[1..])
          .current_dir(cwd)
          .output()
  ```

  `hk validate` evaluates `hk.pkl` inside the worktree; it gets the same env
  treatment for consistency (hk itself may be devenv-pinned).

- **N subprocesses per bookmark** (`hooks.rs:916`, `hooks.rs:938`): the
  default path iterates `for from_ref in from_refs` and calls
  `run_subprocess(&argv, wt.path(), workspace_root, captured.as_mut())?` per
  iteration — the per-subprocess vs once-per-run cost fork is real.

- **Captured-buffer contract** (`hooks.rs:1014-1018`):

  ```rust
  buf.push_str(&format!("$ {}\n", argv.join(" ")));
  buf.push_str(&String::from_utf8_lossy(&output.stdout));
  if !output.stderr.is_empty() {
      buf.push_str(&String::from_utf8_lossy(&output.stderr));
  }
  ```

  Anything the mechanism writes to the *child's* stderr lands in the user's
  failure dump — hence the export-in-a-separate-process constraint.

- **Runner resolution walks jj-hp's own PATH**
  (`tools/jj-hooks/src/runner.rs:227-236`):

  ```rust
  fn which(bin: &str) -> Option<PathBuf> {
      let path = std::env::var_os("PATH")?;
      for dir in std::env::split_paths(&path) {
          let candidate = dir.join(bin);
          if candidate.is_file() {
              return Some(candidate);
          }
      }
      None
  }
  ```

  Layer 4 of `resolve_runner_argv(runner, jj, workspace_root,
  primary_git_dir, stage)` (`runner.rs:273-279`; resolution-order doc at
  `runner.rs:243-267`). This resolves the *runner* binary (hk / prek /
  pre-commit / lefthook) against the parent PATH — see fork 5.

- **PklWarmCache precedent — once-per-run serialized warm**
  (`hooks.rs:336-339`):

  ```rust
  #[derive(Default)]
  struct PklWarmCache {
      warmed: std::sync::Mutex<std::collections::HashSet<PathBuf>>,
  }
  ```

  Instantiated once per batch entrypoint (`hooks.rs:433`, `hooks.rs:553`) and
  threaded into `run_once` as `warm: Option<&PklWarmCache>` (`hooks.rs:744`).

- **Repo devenv wiring** (`.envrc:6-12`):

  ```sh
  source_url "https://raw.githubusercontent.com/cachix/devenv/82c0147677e510b247d8b9165c54f73d32dfd899/direnvrc" "sha256-7u4iDd1nZpxL4tCzmPG0dQgC5V+/44Ba+tHkPob1v2k="
  ...
  use devenv
  ```

  Plus per-workspace overrides via `source_env_if_exists .envrc.local`
  (`.envrc:17`) — a devenv-specific exporter would miss these; direnv honors
  them.

- **The gate CI runs, and the parity target** (`.github/workflows/ci.yml:3-4`,
  `:65`): "One workflow runs `moon ci` … inside the devenv shell" and
  `run: devenv shell -- moon ci`. Local pre-push is `hk.pkl:29-31`:

  ```text
  ["ci"] = new Step {
      check = "moon ci"
  }
  ```

  jj-hp's temp-worktree subprocess is the only `moon ci` path that runs
  *without* the devenv env; this design reproduces what `devenv shell --`
  does for CI.

- **Dependency availability**: `tools/jj-hooks/Cargo.toml:42-54` lists deps
  (serde yes, serde_json no); root `Cargo.toml:29` has
  `serde_json = "1.0.145"` in `[workspace.dependencies]`, so the crate adds
  `serde_json = { workspace = true }` only.

- **Error surface** (`tools/jj-hooks/src/error.rs:4-38`): `JjHooksError` — no
  new variant needed; env-load failure is non-fatal by constraint (tracing
  warn + fallback), never an `Err`.

- **Runner selection coupling — `prefer_prek_when_available`**
  (`tools/jj-hooks/src/hooks.rs:830-838`):

  ```rust
  let prek_present = crate::runner::resolve_runner_argv(
      Runner::Prek,
      jj,
      workspace_root,
      primary_git_dir,
      stage,
  )
  .is_ok();
  crate::runner::prefer_prek_when_available(r, prek_present)
  ```

  `resolve_runner_argv` is not only the pre-check: its `Runner::Prek` probe
  feeds the pre-commit→prek auto-switch. Any change to what the resolver can
  see (e.g. a repo-env PATH fallback) changes runner *selection* — see
  Fork 5.

- **Layer 4 returns a bare name; the child resolves it**
  (`tools/jj-hooks/src/runner.rs:332-335`):

  ```rust
  // (4) Plain $PATH.
  if which(runner.bin()).is_some() {
      return Ok(vec![runner.bin().into()]);
  }
  ```

  The spawned `Command` therefore resolves the runner binary via `execvp`
  against the *child's* env — after T1+T2, the merged PATH — regardless of
  which PATH satisfied the pre-check. This is why T1+T2 alone fix every
  observed fleet manifestation and the pre-check ordering is cosmetic.

- **Release plumbing — internal path-dep pin** (root `Cargo.toml:43`):

  ```toml
  jj-hooks = { version = "0.3.7", path = "tools/jj-hooks" }
  ```

  The workspace-dep floor is a separate field from the crate version; T5's
  bump checklist must touch both or the release auto-rewrite drifts.

### Empirical findings (driver-run, this clone, direnv 2.37.1)

Run against the real `.envrc` (`use devenv`) before freeze:

- **F1 — core mechanism confirmed.** `direnv export json` from a bare parent
  env (`env -i`, system-only PATH, cwd = repo root) exits 0 and the JSON
  `.PATH` carries the full devenv toolchain (`/nix/store/...` entries for
  proto, nixfmt, shellcheck, taplo, markdownlint, …) — exactly the PATH
  CI's `devenv shell -- moon ci` uses. The load-bearing correctness
  question is answered yes.
- **F2 — cost.** Warm export ~1.6 s; a cold first export streams the full
  devenv/nix bootstrap on stderr. Once-per-process caching is worth it.
- **F3 — inherited stale `DIRENV_DIFF` poisons the export.** With a
  foreign/corrupt `DIRENV_DIFF` in the parent env, `direnv export json`
  exits 1 (`direnv: error Revert() failed: unmarshal() base64 decoding:
  illegal base64 data at input byte 7`). Retrying with
  `DIRENV_DIR`/`DIRENV_FILE`/`DIRENV_DIFF`/`DIRENV_WATCHES` removed from
  the subprocess env recovers: exit 0, full devenv PATH, correct repo
  `DIRENV_DIR`, fresh `DIRENV_*` keys in the JSON. Folded into Approach
  step 2 as the failure retry.
- **F4 — `DIRENV_LOG_FORMAT=""` is not the capture-purity guarantee.** It
  does not silence devenv's bootstrap stderr; the guarantee is the
  separate-subprocess structure (see Global Constraints).

## Approach

**Export-once, apply-per-spawn.** A new module
`tools/jj-hooks/src/repo_env.rs`:

1. **Detect**: the mechanism activates iff the T4 opt-outs permit it (read
   once at the batch entrypoints, passed in as `enabled`) AND
   `workspace_root/.envrc` exists AND `direnv` resolves on jj-hp's own PATH
   (same `which` walk as `runner.rs:227-236`). Otherwise the patch is
   `Disabled` and every spawn is byte-identical to today.
2. **Export once**: run `direnv export json` with
   `current_dir(workspace_root)` (the allowed `.envrc` location, never the
   temp worktree), stdout piped, stderr piped and routed to `tracing::warn`
   only on failure. The export subprocess inherits the full parent env —
   `DIRENV_DIFF`/`DIRENV_DIR` included — overriding only
   `DIRENV_LOG_FORMAT=""` (load-bearing invariant; see step 4). Parse the
   JSON object (`HashMap<String, Option<String>>` — direnv emits `null` for
   variables to unset) with `serde_json`. Outcome handling:
   - **Exit 0 with EMPTY stdout is a success**: the parent shell already
     has *this* repo's env loaded, so the diff is empty — `Patch(empty)`,
     no warn (the everyday interactive case; direnv.el handles the same
     output as "no changes"). `Disabled` is reserved for non-zero exit,
     malformed *non-empty* output, or detection failure.
   - **On export failure (non-zero exit or parse error), retry once with
     `DIRENV_DIR`, `DIRENV_FILE`, `DIRENV_DIFF`, and `DIRENV_WATCHES`
     removed** from the subprocess env. Empirically (F3): a corrupt/stale
     inherited `DIRENV_DIFF` makes the export exit 1 (`Revert() failed:
     unmarshal() base64 decoding`); with the four vars removed it recovers
     — exit 0, full devenv PATH, `DIRENV_DIR` pointing at the repo. Only if
     the retry also fails does the patch degrade to `Disabled`.
   - **Blocked `.envrc` (fresh clone, `direnv allow` never run)** degrades
     to `Disabled` but emits ONE visible `eprintln` — not just an invisible
     `tracing::warn`: `` jj-hp: .envrc present but not allowed; hooks run without the repo env (run `direnv allow` in <workspace_root>) ``.
     This is the most common first-run state; without a visible signal it
     is indistinguishable from the pre-fix bug. All other failures stay at
     `tracing::warn`.
3. **Compute eagerly, cache once per process**: the env is computed at the
   batch entrypoints (`hooks.rs:433` / `hooks.rs:553` vicinity, before
   worktree creation) — a natural seam for a one-line visible notice on a
   cold `use devenv` eval (`loading repo env (first run may take a
   while)…`), and it keeps the cold export from being paid under the
   `PklWarmCache` lock. The result lands in a module-level
   `OnceLock<Mutex<HashMap<PathBuf, Arc<EnvPatch>>>>` keyed by canonicalized
   `workspace_root` (a first `use devenv` eval is paid exactly once, not
   N×M times — empirically ~1.6 s warm, full nix bootstrap cold); spawn
   sites only read the cache (missing entry ⇒ `Disabled`). Process-global
   rather than threaded, because `run_subprocess` and `run_steps` already
   have `workspace_root` in hand and threading a new parameter would touch
   six-plus signatures for no isolation benefit (tests key on unique
   tempdir workspace roots).
4. **Apply as a merge**: `apply_repo_env(&mut cmd, workspace_root)` sets each
   `Some(v)` via `Command::env` and removes each `None` via
   `Command::env_remove`. No `env_clear()` — direnv's export is a *diff*
   against the invoking environment, so merging is the correct semantics in
   all three launch states: (a) launched from a shell where direnv already
   loaded *this* repo's env — the diff is empty, behavior unchanged; (b)
   launched from a bare harness env — the diff prepends the devenv PATH and
   sets the devenv variables; (c) launched from a shell where direnv has a
   *different* repo's env loaded (an agent working across repos) — direnv
   uses the inherited `DIRENV_DIFF` to reverse the previous repo's load
   before applying this repo's `.envrc`, so the emitted diff both removes
   the foreign entries and prepends the right ones. Case (c) is why the
   invariant holds: **the export subprocess inherits the full parent env
   (`DIRENV_DIFF`/`DIRENV_DIR` included), overriding only
   `DIRENV_LOG_FORMAT`** — never "sanitize" `DIRENV_*` on the happy path;
   the cleared-env pass exists only as the failure retry in step 2.
   `apply_repo_env` strips the git repo-location env family (`GIT_DIR`,
   `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`,
   `GIT_OBJECT_DIRECTORY`) from the patch — see Global Constraints.
   `JJ_HOOKS_WORKSPACE` is set after the patch so it always wins.
5. **Applied at all three worktree spawn sites**: `run_subprocess`
   (`hooks.rs:1000-1003`), `run_steps` (`setup.rs:150-154`), and
   `run_hk_validate` (`hooks.rs:303-308`) call the one shared helper. The
   pure-git plumbing spawns (`changed_files` `hooks.rs:1069`, `delete_git_ref`
   `hooks.rs:1212`, `run_git*` `hooks.rs:1228-1259`, worktree create/remove)
   stay untouched — git needs no devenv.
6. **Runner resolution — deferred.** `resolve_runner_argv` is unchanged in
   this release. After T1+T2 the runner binary is resolved via `execvp`
   against the *child's* merged PATH at spawn time (layer 4 returns a bare
   name, `runner.rs:332-335`), so devenv-pinned runners already win where
   it matters; a repo-env-PATH fallback in the pre-check would also
   silently change runner *selection* via `prefer_prek_when_available`
   (`hooks.rs:830-838`). See Fork 5 and the T3 Open Question.
7. **Escape hatch** (fork 7): automatic by default; jj config
   `jj-hooks.repo-env = "off"` (read via the existing `JjCli` config-get
   pattern, cf. `read_runner_bin_config`, `runner.rs:354-357`) and env var
   `JJ_HOOKS_NO_REPO_ENV=1` both force `Disabled`. Both are read once at
   the batch entrypoints and passed into the eager `repo_env()` call as a
   parameter — never an ambient process-global flag (fork 6).
   Automatic-only was rejected: a released tool needs a same-day off-switch
   if a repo's `.envrc` turns out to be hostile to hook runs (e.g. mutates
   GIT_DIR — this very repo ships a GIT_DIR-setting `.envrc.local`
   pattern, `.envrc:14-17`).

### Why not the alternatives (summary — details below)

`direnv exec` per subprocess pays the direnv evaluation N×M times and pipes
direnv's stderr chatter straight into the captured failure buffer;
`devenv shell --` couples the tool to devenv/nix and misses plain-direnv repos
and `.envrc.local`; `devenv print-dev-env` likewise. Export-once via direnv is
the only option that is generic (any `.envrc`, devenv or not), pays the cost
once, keeps the capture buffer clean, and needs zero new runtime dependencies
(absence of direnv = fallback).

## Alternatives considered

### Fork 1 — how to obtain the env

- **(a) Wrap argv in `direnv exec <workspace_root> <argv…>`** — rejected.
  Correctness is fine (direnv evaluates and execs), but: (i) cost is per
  subprocess — `run_once` spawns one subprocess per `from_ref`
  (`hooks.rs:916-938`) per bookmark, so a 3-ancestor × 4-bookmark batch pays
  12 direnv evaluations instead of 1; (ii) direnv logs to the *child's*
  stderr, which `run_subprocess` folds into the user-facing buffer
  (`hooks.rs:1016-1018`) — suppressible with `DIRENV_LOG_FORMAT=""` but any
  direnv *error* would still land there; (iii) it rewrites every argv, which
  interacts with `splice_runner_prefix` and the `$ argv` header line the
  capture buffer prints (`hooks.rs:1014`), leaking mechanism into UX.
- **(b) `direnv export json` once, parse, apply via `Command::envs`** —
  **chosen.** Full env including PATH (direnv computes exactly what `cd` into
  the repo would load, honoring `use devenv`, `watch_file`, and
  `.envrc.local`); one evaluation per run; the export's stderr lives in a
  separate subprocess jj-hp controls and never touches the capture buffer;
  dependency surface is "direnv if present, else fallback" — no hard
  dependency.
- **(c) `devenv shell -- <argv>`** — rejected. Requires `devenv` *and* nix on
  PATH (a bigger, less common surface than direnv); pays a nix eval per
  subprocess; skips `.envrc.local`; and couples a generic released tool to
  one env manager. Same objections apply to `devenv print-dev-env` /
  `nix print-dev-env` as export sources: devenv-only, misses the direnvrc
  layer and `.envrc.local`, and adds a devenv dependency where direnv already
  abstracts it.

### Fork 2 — where the env is applied

- **Shared helper in a new `repo_env.rs` module — chosen.** Three spawn sites
  need it (`run_subprocess`, `run_steps`, `run_hk_validate`); both files
  already import across module boundaries (setup.rs is called from hooks.rs,
  `hooks.rs:775`). Duplicating the merge logic three times was rejected as
  drift-prone.
- **Threading an explicit cache struct (PklWarmCache-style) — rejected** for
  the cache location: it would add a parameter to `run_for_update`,
  `run_for_updates_parallel`, `run_for_updates_sequential`, `run_once`,
  `run_subprocess`, and `run_steps` (six-plus signatures) purely to avoid a
  module-level static, while tests get equivalent isolation from unique
  tempdir `workspace_root` keys. `PklWarmCache` is per-batch because hk's
  cache is *worktree*-keyed and each batch creates fresh worktrees; the repo
  env is *workspace_root*-keyed and immutable for the life of the process, so
  a process-global cache is the honest scope.

### Fork 3 — detection and fallback

Chosen: `.envrc` exists at `workspace_root` AND `direnv` on jj-hp's PATH;
export outcomes then split three ways (Approach step 2): exit 0 with empty
stdout is a *success* (`Patch(empty)` — the parent shell already has this
repo's env loaded; warning here would be a lie, and it would fire on every
push from a loaded shell); a blocked `.envrc` degrades to `Disabled` with
one visible `eprintln` pointing at `direnv allow` (a fresh clone is the
most common first-run state — an invisible `tracing::warn` would make the
mechanism's absence indistinguishable from the pre-fix bug); every other
failure degrades to `Disabled` after the cleared-`DIRENV_*` retry, with a
single `tracing::warn`. Alternatives rejected: probing for `devenv.nix`
(devenv-specific — plain-direnv repos also deserve the fix); erroring when
`.envrc` exists but direnv is missing (would regress repos where a `.envrc`
exists but contributors don't use direnv — the non-regression constraint
forbids any new failure mode).

### Fork 4 — captured-output pollution

Chosen: the export subprocess pipes both stdout (the JSON) and stderr
(routed into `tracing::warn` on failure only). Because the hook child
itself is spawned directly (no direnv wrapper), nothing new can reach the
capture buffer at `hooks.rs:1014-1018` / `setup.rs:161-165` — a structural
guarantee of the export-once approach, not a filter. `DIRENV_LOG_FORMAT=""`
is set as a secondary nicety; empirically (F4) it does NOT silence devenv's
bootstrap stderr, so it must never be relied on for purity.

### Fork 5 — interaction with `resolve_runner_argv` (deferred)

The runner pre-check resolves via jj-hp's own PATH (`runner.rs:227-236`,
called from `hooks.rs:855-856`). A repo-env-PATH fallback (T3) was designed
and is now **deferred from the first release**, for two reasons:

- **Redundant with `execvp` for every observed failure.** Layer 4 returns
  the bare binary name (`runner.rs:332-335`), which the child resolves
  against its own merged PATH at spawn time — so after T1+T2 a
  devenv-pinned runner already wins where it matters, regardless of which
  PATH satisfied the pre-check. T1+T2 alone fix all observed fleet
  manifestations (biome, proto, nilaway); T3 would only improve the
  pre-check's `RunnerNotFound` message (`error.rs:26-35`) in the unobserved
  devenv-only-runner case.
- **It silently changes runner selection.** `resolve_runner_argv` also
  feeds `prefer_prek_when_available` via the `Runner::Prek` probe
  (`hooks.rs:830-838`): a repo that devenv-pins prek, where the user never
  installed it globally, would flip `prek_present` false→true and switch
  the executing runner from pre-commit to prek — an unwanted behavior
  change on a released tool for a benefit no observed bug needs.

See the T3 Open Question (the headline decision for Matt) and the deferred
T3 section in the Plan.

### Fork 6 — env-load cost, caching, and where it's computed

Chosen: computed **eagerly at the batch entrypoints** (`hooks.rs:433` /
`hooks.rs:553` vicinity, before worktree creation), stored in a
process-global `OnceLock` cache keyed by canonicalized `workspace_root`,
populated under a mutex so parallel workers serialize on the single export
(precedent: `PklWarmCache::warm_once` holds its lock across the warm,
`hooks.rs:354-369`); spawn sites only read the cache. Eager placement gives
a natural seam for a one-line visible notice on a cold `use devenv` eval
(`loading repo env (first run may take a while)…`) and keeps the cold
export from being paid under the `PklWarmCache` lock. The T4 opt-out is a
*parameter* of this eager population call — read once from env var + jj
config at the entrypoints — never an ambient process-global flag, which
would race in-process test harnesses and be fragile against init ordering.
Cold-cache cost (first `use devenv` eval; empirically a full nix bootstrap)
is paid once per process; warm loads are ~1.6 s via `.direnv/`. Rejected:
per-bookmark or per-subprocess export (N×M cost); lazy compute inside the
spawn sites (no UI seam for the cold-eval notice, and the export would be
paid under the warm lock); persisting the parsed env to disk between jj-hp
invocations (staleness tracking duplicates what direnv's own cache already
does).

### Fork 7 — opt-out

Chosen: automatic, plus `jj-hooks.repo-env = "off"` jj config and
`JJ_HOOKS_NO_REPO_ENV=1` env var. Automatic-only rejected (released tool,
needs a same-day off-switch); opt-in rejected (the fleet bug exists precisely
because nobody opts in to plumbing).

## Plan

### T1 — `repo_env` module: detect, export (with retry), parse, cache

New file `tools/jj-hooks/src/repo_env.rs`; register in `lib.rs`; add
`serde_json = { workspace = true }` to `tools/jj-hooks/Cargo.toml`.

Interfaces:

```rust
/// The environment delta direnv would apply on entering `workspace_root`.
pub enum EnvPatch {
    /// No .envrc, no direnv, export failed, or opted out — spawn unchanged.
    Disabled,
    /// Apply: set each Some(v), remove each None key. An EMPTY map is a
    /// success (parent env already loaded — empty diff), not Disabled.
    Patch(std::collections::HashMap<String, Option<String>>),
}

/// Compute-and-cache, called EAGERLY once at each batch entrypoint before
/// worktree creation. `enabled` carries the T4 opt-outs, read once at the
/// entrypoint — no ambient global. First caller runs `direnv export json`
/// (cwd = workspace_root; full parent env inherited with only
/// DIRENV_LOG_FORMAT="" overridden; stdout piped, stderr -> tracing).
/// On failure, retries once with DIRENV_DIR/DIRENV_FILE/DIRENV_DIFF/
/// DIRENV_WATCHES removed; only a failed retry degrades to Disabled.
pub fn repo_env(workspace_root: &Path, enabled: bool) -> std::sync::Arc<EnvPatch>;

/// Merge the cached patch into `cmd`: env() for Some, env_remove() for
/// None, after stripping the git repo-location family (GIT_DIR,
/// GIT_WORK_TREE, GIT_INDEX_FILE, GIT_COMMON_DIR, GIT_OBJECT_DIRECTORY).
/// No-op for Disabled or a missing cache entry. Never touches
/// JJ_HOOKS_WORKSPACE (callers set it after this call).
pub fn apply_repo_env(cmd: &mut std::process::Command, workspace_root: &Path);
```

Detection inside `repo_env`: `enabled` AND
`workspace_root.join(".envrc").is_file()` AND `direnv` found by a PATH walk
(reuse/mirror `which`, `runner.rs:227-236`).

Outcome handling per Approach step 2: empty-stdout success ⇒ `Patch(empty)`;
failure ⇒ one cleared-`DIRENV_*` retry; blocked `.envrc` ⇒ `Disabled` plus
the one visible `eprintln`; other failures ⇒ `Disabled` + `tracing::warn`.

Test cycle (red→green; hermetic fake-direnv shim except the last item):

- Fake `direnv` executable in a tempdir prepended to a test-scoped PATH,
  emitting a canned JSON (`{"BIOME_PIN":"devenv","DROP_ME":null,"PATH":"/devenv/bin:..."}`);
  assert `repo_env` yields `Patch` with the set and unset entries.
- No `.envrc` → `Disabled`. `.envrc` present, no direnv on PATH → `Disabled`.
- Exit 0 + EMPTY stdout → `Patch(empty)`, and no warning fires.
- Blocked `.envrc` (fake direnv emitting the blocked signature) →
  `Disabled` AND the visible `eprintln` fires.
- Retry: fake direnv exits 1 iff `DIRENV_DIFF` is present in its own env,
  succeeds otherwise → the call recovers via the retry and yields `Patch`.
- Invariant (cross-repo case): fake direnv asserts it received the parent's
  `DIRENV_DIFF` on the FIRST attempt (the happy path inherits, never
  sanitizes).
- Cache: fake direnv writes a counter file; two `repo_env` calls, counter
  is 1. Distinct workspace roots → 2.
- `apply_repo_env` on a `Command`: spawn `sh -c 'echo "$BIOME_PIN"'`
  (portable: the existing test suite already spawns subprocesses), assert the
  var is visible and the `None`-keyed var is absent.
- Git-family strip: canned JSON carrying `GIT_DIR`/`GIT_INDEX_FILE` →
  `apply_repo_env` child sees neither.
- **Real-direnv integration test** (`#[ignore]`d / skipped when `direnv` is
  absent; CI has direnv via the devenv shell, `ci.yml:65`): isolated
  `XDG_DATA_HOME`/`XDG_CONFIG_HOME` tempdir, a plain `export FOO=bar`
  `.envrc` (no nix/devenv needed), exercising the three load-bearing
  states: (a) bare-env export emits `FOO` in the patch; (b) blocked
  `.envrc` (before `direnv allow`) → `Disabled` + visible signal;
  (c) loaded-env → empty diff = `Patch(empty)`. Guards against direnv
  version drift in export behavior (the blocked-output format has already
  changed once across direnv releases). The compass biome repro stays the
  driver's devenv-specific acceptance test.

### T2 — apply the patch at all three worktree spawn sites

Modify `run_subprocess` (`hooks.rs:1000-1003`), `run_steps`
(`setup.rs:150-154`), and `run_hk_validate` (`hooks.rs:303-308`) to call
`repo_env::apply_repo_env(&mut cmd, workspace_root)` before the
`JJ_HOOKS_WORKSPACE` env set. `run_hk_validate` gains a `workspace_root:
&Path` parameter (its one caller at `hooks.rs:873` has it in scope).

Interfaces (changed signatures only):

```rust
// hooks.rs — signature unchanged, body gains the apply call:
fn run_subprocess(argv: &[String], cwd: &Path, workspace_root: &Path,
                  capture: Option<&mut String>) -> Result<bool>;

// hooks.rs — gains workspace_root:
fn run_hk_validate(argv: &[String], cwd: &Path, workspace_root: &Path) -> bool;

// setup.rs — signature unchanged, body gains the apply call:
pub fn run_steps(steps: &[SetupStep], worktree: &Path,
                 workspace_root: &Path) -> Result<String>;
```

Test cycle: end-to-end with the fake direnv shim — a fixture repo whose
workspace root carries `.envrc`, hook argv is `sh -c 'command -v pinned-tool'`
style: assert the child resolves the shim-injected PATH entry first (red
before T2: child sees only parent PATH). Setup-step twin test through
`run_steps`. Capture-purity test: with capture on and the fake direnv writing
to its own stderr, assert the captured buffer contains no direnv output.
Git-family strip end-to-end: patch carrying `GIT_DIR` → the hook child's
`GIT_DIR` is unset inside the worktree.

### T3 (DEFERRED — follow-up, not in this release) — runner resolution falls back to the repo-env PATH

What it would add: in `resolve_runner_argv` layer 4 (`runner.rs:273-340`),
on a parent-PATH `which` miss, retry the walk over the repo-env patch's
`PATH` — making a repo whose *runner* (hk/prek/…) is only devenv-pinned
resolvable at the pre-check, with a friendlier `RunnerNotFound` message
(`error.rs:26-35`) in that case.

Why it is out of the first release (Fork 5): layer 4 returns a bare name
(`runner.rs:332-335`) that the child resolves against its merged PATH via
`execvp`, so T1+T2 already fix every observed manifestation and T3 only
affects the cosmetic pre-check message; and via the `Runner::Prek` probe
feeding `prefer_prek_when_available` (`hooks.rs:830-838`) it would silently
switch runner selection for repos that devenv-pin prek. If pursued in the
follow-up, the fallback must be gated out of the `prefer_prek` probe.

First release = T1 + T2 + T4 + T5. Tracked as a follow-up issue after Matt
rules on the T3 Open Question.

### T4 — opt-out: config key and env var, as a population parameter

The opt-outs are read **once at the batch entrypoints** and passed into the
eager `repo_env()` call as `enabled` — never stored in an ambient
process-global flag (a keyless global would race in-process test harnesses
and silently miss entries cached before it was set). Order:
`JJ_HOOKS_NO_REPO_ENV` env var (any non-empty value → disabled), then jj
config `jj-hooks.repo-env` (`"off"` → disabled; unset or `"auto"` →
automatic), read via the same `jj config get` pattern as
`read_runner_bin_config` (`runner.rs:354-384`). The entrypoints
(`run_for_update` / parallel / sequential) have a `JjCli` in scope; the
spawn sites never consult config.

Interfaces:

```rust
// repo_env.rs — no new seam: T4 is the `enabled` parameter on the eager
// population call defined in T1:
pub fn repo_env(workspace_root: &Path, enabled: bool) -> std::sync::Arc<EnvPatch>;
```

Test cycle: `enabled = false` (env var or config `"off"` at the entrypoint)
→ `Disabled` even with `.envrc` + fake direnv present; default → `Patch`.
No process isolation required — the opt-out is per-call, not ambient.

### T5 — docs + release

Update `tools/jj-hooks/README.md` (mechanism, fallback semantics, the two
opt-outs); version bump per the repo's release flow — the checklist touches
BOTH the crate version in `tools/jj-hooks/Cargo.toml` AND the root
`Cargo.toml:43` internal path-dep pin (`jj-hooks = { version = "0.3.7",
path = "tools/jj-hooks" }`), or the release auto-rewrite drifts; changelog
entry referencing #289. Driver-run acceptance: the compass biome repro
(temp worktree resolves devenv biome 2.5.4, not system 2.4.16) and a push
from a bare (non-direnv-loaded) shell in zireael passing `moon ci` with
live proto pins.

Test cycle: `cargo fmt -p jj-hooks -- --check`, `cargo clippy -p jj-hooks
--all-targets -- -D warnings`, full `cargo nextest` for the crate, plus
markdownlint on touched docs — all green before submit
(`rule://pre-finish-checks`).

## Tasks

First release ships T1 + T2 + T4 + T5. T3 is deferred to a follow-up issue
(see the T3 Open Question).

- [ ] T1 — `repo_env.rs`: detection, eager `direnv export json` once with
  cleared-`DIRENV_*` failure retry, JSON parse (empty diff = success),
  blocked-`.envrc` visible signal, git-family strip, process-global cache;
  `serde_json` dep; fake-direnv tests + real-direnv integration test
- [ ] T2 — apply patch in `run_subprocess`, `run_steps`, `run_hk_validate`;
  end-to-end child-env tests + capture-purity + git-family strip tests
- [ ] T4 — opt-outs (`JJ_HOOKS_NO_REPO_ENV`, `jj-hooks.repo-env = "off"`)
  as the `enabled` population parameter; opt-out tests
- [ ] T5 — README + changelog + version bump (crate + root `Cargo.toml:43`
  path-dep pin); driver acceptance (compass biome repro, bare-shell
  zireael push)
- [ ] (deferred) T3 — runner-resolution fallback to the repo-env PATH;
  follow-up issue after Matt rules on the Open Question

## Open Questions

- **[decision for Matt — headline] Defer T3 (runner resolution via repo-env
  PATH) from the first release.** Recommendation: **defer T3 to a follow-up
  issue; first release = T1+T2+T4+T5.** Two independent reasons: (1) after
  T1+T2 the runner binary resolves via `execvp` against the child's merged
  PATH at spawn time (layer 4 returns a bare name, `runner.rs:332-335`), so
  T1+T2 already fix ALL observed fleet manifestations (biome, proto,
  nilaway) and T3 would only improve the cosmetic pre-check
  `RunnerNotFound` message in an unobserved devenv-only-runner case;
  (2) T3 silently changes runner SELECTION via `prefer_prek_when_available`
  (`hooks.rs:830-838`) — a repo that devenv-pins prek where the user never
  installed it globally would flip from pre-commit to prek, an unwanted
  behavior change on a released tool. The design is frozen against the
  assumption T3 is deferred; Matt rules at design-PR review.
- **[load-bearing] Git repo-location propagation remainder.** The
  git-family strip is now a hard requirement (Global Constraints). The
  remainder for Matt: does any repo rely on the OPPOSITE — a `.envrc`-set
  `GIT_DIR` that hook children inside the temp worktree are supposed to
  see? Assumption: no; worktree spawns must see the worktree's own git
  context.
- **[non-load-bearing] Config key name.** `jj-hooks.repo-env` with values
  `auto`/`off` is assumed; any rename is a find-replace in T4 before it
  lands.

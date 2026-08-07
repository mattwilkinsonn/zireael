# jj-hooks

## Overview

`jj-hooks` runs git-style hooks (`pre-commit`, `pre-push`) for
[jj](https://jj-vcs.github.io), which has no native hook system. jj
never invokes `.git/hooks/*` the way `git commit` / `git push` do, so
this tool reconstructs the gate: for every bookmark a push would
update, it materializes an **ephemeral detached git worktree** at the
target commit and invokes the configured hook runner there
(`src/hooks.rs:1-16` — *"Per-bookmark hook execution pipeline … Create
an ephemeral detached worktree at the new commit. Run the configured
hook backend …"*). The user's working copy is never touched, and the
same path works from primary and secondary jj workspaces
(`README.md:34-37` — *"`jj-hooks` sidesteps this entirely by running
every hook in a fresh `git worktree add --detach` checkout of the
target commit."*).

Citations below use paths under `tools/jj-hooks/` (e.g.
`src/runner.rs:14-19`); cross-tool references are given from the repo
root (e.g. `tools/jj-gt/...`).

The supported runners are `pre-commit`, `prek`, `lefthook`, and `hk`
(`src/runner.rs:14-19` — `pub enum Runner { PreCommit, Prek, Lefthook,
Hk, }`).

The hook machinery is exposed as a library (`crate jj_hooks`) so other
tools gate their own pipelines on it without shelling out
(`src/lib.rs:331-333` — *"Exposed as a library entrypoint so other
tools (e.g. `jj-gt`) can gate their own pipelines on the same hook
machinery without shelling out to the `jj-hp` binary."*). `jj-gt`'s
hook layer is *"Thin wrapper around `jj_hooks::hooks::run_for_update`
and its batch siblings"* (`tools/jj-gt/src/hooks.rs:1-2`) and drives the
pre-push gate through `jj_hooks::hooks::run_for_partitioned_updates_parallel`
(`tools/jj-gt/src/hooks.rs:208`), `run_for_update`
(`tools/jj-gt/src/hooks.rs:83`), and `run_for_updates_sequential`
(`tools/jj-gt/src/hooks.rs:233`).

## Binaries and entrypoint

Two binaries ship from one crate — `jj-hooks` (canonical) and `jj-hp`
(shorter alias the `jj push` alias routes through):

| Binary | Source | Cargo target |
| --- | --- | --- |
| `jj-hooks` | `src/main.rs` | `Cargo.toml:30-32` — `name = "jj-hooks"` |
| `jj-hp` | `src/bin/jj-hp.rs` | `Cargo.toml:34-36` — `name = "jj-hp"` |

Both are trivial wrappers: `fn main() -> std::process::ExitCode {
jj_hooks::run() }` (`src/main.rs:1-3`, `src/bin/jj-hp.rs:1-3`). They
are identical (`README.md:8-10` — *"they're identical"*). `run()`
swaps clap's command name to `argv[0]`'s basename so `jj-hp --version`
prints the right identifier (`src/lib.rs:48-65` — *"Dispatch CLI
parsing through a command whose `name` matches the invoked binary
name (argv[0]'s file_name)."*).

On any error the process prints `jj-hooks: {e}` to stderr and exits 1
(`src/lib.rs:78-81`).

## CLI surface

Two global flags apply to every subcommand (`src/cli.rs:15-26`):

| Flag | Default / env | Meaning |
| --- | --- | --- |
| `--runner <R>` | env `JJ_HOOKS_RUNNER` | Override runner autodetect (`src/cli.rs:16-18`). |
| `--log-level <L>` | `warn`, env `JJ_HOOKS_LOG` | `tracing` filter (`src/cli.rs:21-22`). |

Subcommands (`src/cli.rs:29-119`):

| Subcommand | Key flags | Behavior |
| --- | --- | --- |
| `push` | `--advance-bookmarks`, `--stage <pre-commit\|pre-push>` (default `pre-push`), `--dry-run`, `--no-retry-after-fixup`, + `PushArgs` | Run hooks, then push or abort (`src/cli.rs:32-56`). |
| `run [REVSET]` | `--stage` (default `pre-commit`), `REVSET` (default `@`), `--no-retry-after-fixup`, `--all-files` | Run hooks against a revset without pushing (`src/cli.rs:59-83`). |
| `push-tags [TAGS…]` | `--all`, `-f/--force`, `-n/--dry-run`, `--remote <R>` (default `origin`) | Export refs and `git push refs/tags/<tag>` (`src/cli.rs:88-107`). |
| `init` | — | Interactive setup: install `jj push` alias + defaults (`src/cli.rs:109-110`). |
| `completions <SHELL>` | shell name | Print a dynamic completion registration script (`src/cli.rs:112-118`). |

`PushArgs` mirrors `jj git push` selection flags
(`src/cli.rs:124-165`):

| Flag | Field | Notes |
| --- | --- | --- |
| `-b/--bookmark` | `bookmark: Vec<String>` | Repeatable (`src/cli.rs:127-133`). |
| `-r/--revision` | `revision: Vec<String>` | Repeatable (`src/cli.rs:136-137`). |
| `-c/--change` | `change: Vec<String>` | Repeatable (`src/cli.rs:140-141`). |
| `--remote` | `remote: Option<String>` | (`src/cli.rs:144-148`). |
| `--all` | `all: bool` | Push all bookmarks (`src/cli.rs:151-152`). |
| `--tracked` | `tracked: bool` | (`src/cli.rs:155-156`). |
| `--deleted` | `deleted: bool` | (`src/cli.rs:159-160`). |
| `-- <args…>` | `passthrough: Vec<String>` | Forwarded verbatim (`src/cli.rs:162-164`). |

`push_argv` rebuilds the `jj git push` argv from these fields, emitting
each known flag's canonical form and appending `passthrough` at the end
(`src/cli.rs:170-204`).

## Supported hook runners

`Runner` is the closed enum `PreCommit | Prek | Lefthook | Hk`
(`src/runner.rs:14-19`); `bin()` maps each to its executable name
(`src/runner.rs:22-29` — `Runner::PreCommit => "pre-commit"`,
`Runner::Prek => "prek"`, `Runner::Lefthook => "lefthook"`,
`Runner::Hk => "hk"`). `RunnerArg` is the clap value-enum mirror with a
`From<RunnerArg>` conversion (`src/cli.rs:206-223`).

`Stage` is `PreCommit | PrePush` (`src/runner.rs:85-88`), rendered as
`"pre-commit"` / `"pre-push"` (`src/runner.rs:91-96`).

### Autodetection

When `--runner` is absent the runner is detected from config files
present at the worktree root (`src/runner.rs:33-81`):

| Runner | Config files |
| --- | --- |
| `Hk` | `hk.pkl` (`src/runner.rs:35`) |
| `Lefthook` | `lefthook.yml` / `.yaml` / `.toml`, `.lefthook.yml` / `.yaml` / `.toml` (`src/runner.rs:37-45`) |
| `PreCommit` | `.pre-commit-config.yaml` / `.yml` (`src/runner.rs:46-47`) |
| `Prek` | `prek.toml`, `.prek.toml` (`src/runner.rs:49-55`) |

Two or more *distinct* families found at one root is an error
(`src/runner.rs:76-79` — *"multiple hook-runner configs found at
workspace root … Use --runner to pick one."*). `prek` + `pre-commit`
are not ambiguous — they collapse to `Prek` since prek consumes both
config shapes (`src/runner.rs:69-71` — `found.retain(|r| *r !=
Runner::PreCommit);`). On an autodetected `PreCommit`, `prek` is
preferred when resolvable since it is a faster drop-in
(`src/runner.rs:207-218`, applied at `src/hooks.rs:821-838`). No config
present is a silent skip (`src/hooks.rs:810-820` — *"no hook-runner
config in target commit; skipping hooks"*).

### Per-runner command shapes

| Runner | Diff-range invocation | All-files invocation |
| --- | --- | --- |
| `PreCommit` / `Prek` | `<bin> run --hook-stage <stage> --from-ref <from> --to-ref <to>` (`src/runner.rs:110-119`) | `<bin> run --hook-stage <stage> --all-files` (`src/runner.rs:166-172`) |
| `Hk` | `hk run <stage> --from-ref <from> --to-ref <to>` (`src/runner.rs:120-128`) | `hk run <stage> --glob '*'` (`src/runner.rs:173-179`) |
| `Lefthook` | `lefthook run <stage> --file <path>…` (`src/runner.rs:139-146`) | `lefthook run <stage> --all-files` (`src/runner.rs:198-205`) |

`hk` *needs* `--from-ref`/`--to-ref` inside an ephemeral worktree,
otherwise it tries to resolve `refs/remotes/origin/HEAD` and fails
(`src/runner.rs:101-105`). `lefthook` takes a file list, not ref
bounds — passing it to `hook_command` panics
(`src/runner.rs:129-131`). For all-files mode `hk` uses `--glob '*'`
because its own `-a/--all` does not override stage-hook ref bounds
(`src/runner.rs:155-159`, verified against hk 1.45.0).

### Runner binary resolution

Element 0 of the built argv (the bare binary name) is replaced by the
prefix from `resolve_runner_argv`, which tries four layers, first hit
wins (`src/runner.rs:243-272`, spliced at `src/hooks.rs:855-856` +
`splice_runner_prefix` `src/hooks.rs:696-711`):

1. `jj-hooks.runner-bin.<runner>` config — explicit override, string
   or array, relative paths resolved against `workspace_root`
   (`src/runner.rs:245-249`).
2. The path baked into `primary_git_dir/hooks/<stage>` by `prek
   install` / `pre-commit install` (`src/runner.rs:250-260`).
3. `uv run --` when `workspace_root/uv.lock` and `uv` both exist, for
   pre-commit/prek only (`src/runner.rs:261-265`).
4. Plain `$PATH` (`src/runner.rs:266-267`).

All four empty returns `RunnerNotFound` (`src/runner.rs:269-272`).

## Hook execution pipeline

The per-attempt core is `run_once` (`src/hooks.rs:731-982`). One pass:
create the worktree, run user setup steps, resolve + warm the runner,
run hooks, then synthesize a fixup commit if the tree drifted.

### Requirement: Per-bookmark ephemeral worktree isolation

Each hook attempt SHALL run inside a fresh, detached git worktree at
the target commit, never the user's working copy, and the worktree
SHALL be removed when the attempt ends.

#### Scenario: Worktree created at the target commit

- **Given** a bookmark update whose new commit is `target_commit`,
- **When** `run_once` begins,
- **Then** it MUST create the worktree via `let wt =
  Worktree::create(primary_git_dir, target_commit)?;`
  (`src/hooks.rs:754`), which runs `git --git-dir=<primary>
  worktree add --detach --quiet <tempdir> <commit>`
  (`src/worktree.rs:50-55`),
- **And** the worktree directory MUST be a fresh `tempfile::TempDir`
  prefixed `jj-hooks-worktree-` (`src/worktree.rs:41`).

#### Scenario: Worktree removed on drop

- **Given** a live `Worktree`,
- **When** it is dropped,
- **Then** `Drop` MUST call `remove` (`src/worktree.rs:112-121`),
  running `git --git-dir=<primary> worktree remove --force <dir>`
  (`src/worktree.rs:93-97`); a removal failure is logged, not panicked
  (`src/worktree.rs:114-119`).

#### Scenario: Worktree create/remove serialized against a process lock

- **Given** concurrent per-bookmark threads,
- **When** any thread runs `git worktree add` or `git worktree
  remove`,
- **Then** it MUST hold the process-wide `WORKTREE_CREATE_LOCK`
  (`src/worktree.rs:30` — `static WORKTREE_CREATE_LOCK: Mutex<()> =
  Mutex::new(());`) across that one git call only
  (`src/worktree.rs:47-56`, `src/worktree.rs:90-98`),
- **And** hook *execution* inside the worktree MUST still run in
  parallel — the lock covers only worktree creation/removal, which is
  fast (`src/worktree.rs:24-29`).

The rationale is a macOS/APFS race where concurrent `git worktree add
--detach` reads a partially-initialized `commondir`
(`src/worktree.rs:15-23`).

### Diff range and all-files modes

In the default (diff-range) path, `run_once` iterates `from_refs` — one
diff base per ancestor on the remote — building the hook argv per base
and accumulating modifications in the shared worktree
(`src/hooks.rs:915-943`). `resolve_from_refs` computes the bases
(`src/hooks.rs:1031-1066`):

- An existing bookmark uses its old commit: `return
  Ok(vec![old.clone()]);` (`src/hooks.rs:1032-1033`).
- A new bookmark uses `heads(::<new> &
  ::remote_bookmarks(remote=exact:<remote>))` so each already-on-remote
  ancestor is its own base (`src/hooks.rs:1037-1040`).
- A new bookmark on a fresh remote falls back to the parent:
  `vec![format!("{new}^")]` (`src/hooks.rs:1059-1062`).

In `--all-files` mode the diff range is ignored and the runner's
own all-files command runs once (`src/hooks.rs:900-914`); `from_refs`
is meaningless there (`src/hooks.rs:878-881`).

### Fixup-commit synthesis and retry-after-fixup

After the runner(s), `maybe_build_fixup_commit` stages the worktree,
hashes the resulting tree, and returns a fixup commit **only when the
tree actually differs** from the target's tree — content-addressed
gating so a dirty index that produced no content change is not a false
positive (`src/hooks.rs:1090-1144`). When a fixup is produced,
`jj git import --ignore-working-copy` makes jj aware of it, then the
temporary `jj-hooks-fixup/<bookmark>` jj bookmark and underlying
`refs/heads/jj-hooks-fixup/<bookmark>` ref are cleaned up immediately
(`src/hooks.rs:948-973`; ref names from `fixup_ref` /
`fixup_bookmark`, `src/hooks.rs:1148-1155`).

`RunOpts.retry_after_fixup` (`src/hooks.rs:111-119`) controls the
healing retry in `run_for_update_with_cancel`
(`src/hooks.rs:181-295`): when the initial run failed **and** produced
a fixup, hooks re-run against the fixup
(`src/hooks.rs:226-255`); a clean re-run reports `success` with
`initial_failure = true` (`src/hooks.rs:260`, `src/hooks.rs:279`,
`src/hooks.rs:290-291`). The default `RunOpts` is no-retry
(`src/hooks.rs:108-110`); both CLI paths enable it unless
`--no-retry-after-fixup` is passed (`src/lib.rs:113-114`,
`src/lib.rs:210-211`).

User-declared `[[jj-hooks.setup]]` steps run inside the worktree
*before* the runner so hooks have install-time resources like
`node_modules` (`src/setup.rs:1-27`); a non-zero step exit aborts the
attempt before the runner is invoked and returns `success: false`
(`src/hooks.rs:775-801`). Setup output is always captured and is silent
on success (`src/setup.rs:116-131`); `JJ_HOOKS_WORKSPACE` exposes the
invocation workspace to each step (`src/setup.rs:111-115`).

## Parallel batch execution

The single-bookmark CLI path is sequential: `jj-hp push` loops
`run_for_update` over each non-delete update (`src/push.rs:72-88`) with
`capture_output: false` so the runner's progress streams live
(`src/lib.rs:118-122`). Parallel fan-out is the library batch API
consumed by other tools.

### Requirement: Parallel fan-out with fail-fast cancellation

`run_for_updates_parallel` SHALL run one thread per update and SHALL
flip a shared cancellation token as soon as any sibling fails, so
remaining siblings short-circuit between subprocesses.

#### Scenario: One thread per update, results in input order

- **Given** N bookmark updates,
- **When** `run_for_updates_parallel` runs,
- **Then** `run_updates_parallel_core` MUST spawn one scoped thread
  per update inside `std::thread::scope`
  (`src/hooks.rs:488-502`),
- **And** it MUST return outcomes in **input order**, not completion
  order (`src/hooks.rs:403-404`; collected from per-index slots at
  `src/hooks.rs:504-512`).

#### Scenario: First failure cancels remaining siblings

- **Given** a batch sharing one `Cancel` token (`src/hooks.rs:485`),
- **When** a thread observes `!o.success && !o.cancelled`,
- **Then** it MUST call `cancel_ref.cancel();`
  (`src/hooks.rs:494-496`),
- **And** siblings MUST check the token between subprocess invocations
  and skip the rest, marking `cancelled` (`src/hooks.rs:916-927`;
  short-circuit also checked at entry `src/hooks.rs:746-753`),
- **And** the token never kills an in-flight subprocess — it skips the
  next one (`src/hooks.rs:176-179`).

#### Scenario: Capture is mandatory for parallel runs

- **Given** a call to a parallel entrypoint,
- **When** `opts.capture_output` is `false`,
- **Then** the core MUST panic: `assert!(capture_output,
  "run_for_updates_parallel requires capture_output=true; parallel runs
  without capture garble the terminal")` (`src/hooks.rs:473-476`).

#### Scenario: Partitions cancel independently

- **Given** `run_for_partitioned_updates_parallel` with multiple
  partitions (e.g. `-b X -b Y` for two unrelated stacks),
- **When** any update in partition X fails,
- **Then** only X's `Cancel` (created per partition,
  `src/hooks.rs:609`) is flipped (`src/hooks.rs:615-618`),
- **And** partition Y MUST run to completion — partitions are
  independent (`src/hooks.rs:515-518`).

The sequential opt-out is `run_for_updates_sequential`: same in-order
contract, no thread fan-out, live output by default
(`src/hooks.rs:642-676`).

### Requirement: Pkl-cache warm-once before concurrent hk runs

When the runner is `hk`, each ephemeral worktree's Pkl config cache
SHALL be warmed by a single serialized `hk validate` before any
concurrent `hk run` reads that worktree's cache, so cold-cache writes
never race.

`hk` caches each worktree's resolved config separately, keyed by the
config's *path* (`~/.cache/hk/configs/<path-keyed>.json`), and every
ephemeral worktree has a unique `/tmp` path, so warming is per-worktree
(`src/hooks.rs:326-335`). On a cold cache, concurrent per-bookmark
evaluations race the non-atomic cache write and abort with a
nondeterministic `field not found` error (`src/hooks.rs:858-864`). The
documented invariant is the *"warm-once-before-fan-out invariant"*
(`src/hooks.rs:457-458`) — *"each distinct config is warmed once,
serially, from its own target worktree"* (`src/hooks.rs:550-552`).

#### Scenario: Each batch shares one warm cache

- **Given** `run_for_updates_parallel` /
  `run_for_partitioned_updates_parallel`,
- **When** the batch starts,
- **Then** exactly one `PklWarmCache::default()` MUST be created and
  shared across every per-bookmark thread (`src/hooks.rs:433-434`,
  `src/hooks.rs:553-554`), threaded in as `Some(warm)`
  (`src/hooks.rs:448`, `src/hooks.rs:568`),
- **And** the sequential / single-bookmark paths MUST pass no cache —
  no concurrency, no race (`src/hooks.rs:164-166`,
  `src/hooks.rs:333-335`).

#### Scenario: First touch of a worktree validates under lock

- **Given** a worker whose runner is `hk` and a warm cache is present,
- **When** it reaches the warm gate
  (`src/hooks.rs:869-876` — `Some(warm) if runner == Runner::Hk =>`),
- **Then** it MUST call `warm.warm_once(wt.path(), || run_hk_validate(
  &validate_argv, wt.path()))` (`src/hooks.rs:873`) where
  `validate_argv` is the resolved prefix spliced over
  `hk_validate_command()` = `vec![Runner::Hk.bin().into(),
  "validate".into()]` (`src/runner.rs:191-193`, spliced at
  `src/hooks.rs:871-872`),
- **And** `warm_once` MUST hold the `PklWarmCache` mutex across the
  `validate` call so concurrent callers block, returning early for an
  already-warmed path (`src/hooks.rs:359-362`).

#### Scenario: Warm is best-effort; a failed warm keeps the cold run serialized

- **Given** `hk validate` fails or cannot spawn,
- **When** `run_hk_validate` returns,
- **Then** it MUST swallow the failure (log only) and report `false`
  (`src/hooks.rs:303-324`) — a real config error resurfaces through
  the per-bookmark run that follows,
- **And** `warm_once` MUST hand back the held `MutexGuard` (rather than
  inserting the path), which the worker keeps for the duration of its
  own `hk run`, serializing that still-cold run against other workers
  (`src/hooks.rs:353-368`, guard bound at `src/hooks.rs:869`).

### Requirement: Output capture and completion-order replay

In capture mode, every hook subprocess's output SHALL be folded into
`HookOutcome.captured_output` in order, and the parallel entrypoint
SHALL surface each update's block to the caller as it finishes
(completion order), while still returning the aggregate in input order.

#### Scenario: Capture buffers per-update output in order

- **Given** `RunOpts.capture_output == true`,
- **When** a hook subprocess runs,
- **Then** `run_subprocess` MUST capture child stdout+stderr into the
  buffer instead of inheriting the parent's stdio
  (`src/hooks.rs:984-1025`),
- **And** the per-update buffer MUST be seeded with the captured setup
  output (`src/hooks.rs:894-898`) and carried on
  `HookOutcome.captured_output` (`src/hooks.rs:91-98`),
- **And** with capture off, output MUST stream live to the terminal —
  `captured_output` is `None` (`src/hooks.rs:92-94`,
  `src/hooks.rs:134-136`).

#### Scenario: Completion-order progress, input-order aggregate

- **Given** a parallel batch,
- **When** a thread finishes an update,
- **Then** the `progress(idx, update, o)` callback MUST fire on that
  finishing thread (`src/hooks.rs:497`; partitioned at
  `src/hooks.rs:619`) — completion order — so the caller replays each
  block as it lands (`src/hooks.rs:130-132`),
- **And** the returned `Vec<HookOutcome>` MUST still be in input order
  (`src/hooks.rs:403-404`, `src/hooks.rs:504-512`).

## Push pipeline

`jj-hp push` is a drop-in for `jj git push`: probe which bookmarks
would update, run hooks per bookmark, then push or abort
(`README.md:14-22`). `run_checks` orchestrates the probe + per-bookmark
hooks (`src/push.rs:39-94`); `src/lib.rs:89-196` is the dispatch that
reports, advances, and gates.

### Requirement: Pre-push gating

A push SHALL proceed only when no bookmark failed hooks and no bookmark
produced an un-squashed fixup commit; the final `jj git push` SHALL
run with `--ignore-working-copy`.

#### Scenario: Resolve the bookmark set via a dry-run probe

- **Given** `push` selection flags,
- **When** `run_checks` starts,
- **Then** it MUST run the probe `jj git push --dry-run
  --ignore-working-copy <select_argv>` (`src/push.rs:156-165`,
  `dry_run_updates` at `src/push.rs:141-150`) and parse its stderr
  into `BookmarkUpdate`s,
- **And** an empty result MUST skip hooks (`skipped: true`,
  `src/push.rs:49-55`),
- **And** a result of **only** deletes MUST skip hooks
  (`src/push.rs:57-68`); pure deletes never get a hook attempt
  (`src/hooks.rs:192-202`).

#### Scenario: Diff range is the whole bookmark, base..tip

- **Given** a bookmark moving from `old_commit` to `new_commit`,
- **When** hooks run,
- **Then** the runner MUST see `--from-ref <old> --to-ref <new>`
  (`src/hooks.rs:1032-1033` feeding `hook_command`,
  `src/runner.rs:108-128`) — the full bookmark diff, same as `git push
  origin <bookmark>` would push.
- **And** for the revset entrypoint, the *"to"* is `heads(<revset>)`
  (limit 1) and the *"from"* is `roots(<revset>)-`, so `main..tip`
  resolves its base to `main` and hooks see the entire stack
  (`src/lib.rs:386-397`; resolved at `src/lib.rs:418-450`, synthesized
  update at `src/lib.rs:452-458`).

#### Scenario: Abort on failure or fixup

- **Given** the per-bookmark report,
- **When** dispatch evaluates the gate,
- **Then** the push MUST abort with exit 1 when `report.any_failure()
  || report.any_fixup()` (`src/lib.rs:189-192`; predicates at
  `src/push.rs:21-31`) — a healed retry still leaves a `fixup_commit`,
  so it correctly aborts (`src/lib.rs:184-188`),
- **And** otherwise the final push MUST run `jj git push
  --ignore-working-copy <push_argv>` (`src/lib.rs:194`,
  `execute_push_argv` at `src/push.rs:191-198`),
- **And** when hooks were skipped, dispatch MUST fall through to the
  same push directly (`src/lib.rs:134-137`).

`--ignore-working-copy` on the dry-run probe, the final push, the
fixup `jj git import`, and `jj bookmark set` is what keeps `jj-hp push`
safe to run while another `jj` process holds the op lock
(`src/push.rs:184-190`, `src/push.rs:152-156`, `src/hooks.rs:949-952`).

### Bookmark advancement

`--advance-bookmarks` (or `jj-hooks.advance-bookmarks = true` in
config, `src/lib.rs:321-328`) points each local bookmark with a fixup
commit at that fixup (`src/lib.rs:178-182`). `maybe_advance_bookmarks`
runs `jj bookmark set <bookmark> -r <commit> --allow-backwards
--ignore-working-copy` per fixup (`src/push.rs:101-122`,
`advance_bookmark_argv` at `src/push.rs:129-139`).

## Bookmark-update parsing

`parse_git_push_dry_run` turns `jj git push --dry-run` text into a
`HashSet<BookmarkUpdate>` (`src/bookmark_updates.rs:124-159`). It
tracks the current remote from `^Changes to push to (.+?):`
(`src/bookmark_updates.rs:119-122`) and matches each bookmark line
against a per-`UpdateType` pattern set
(`src/bookmark_updates.rs:51-117`). A bookmark line seen before any
remote line is a hard error (`src/bookmark_updates.rs:139-143` — *"saw
a bookmark line before any `Changes to push to <remote>:` line"*).

`UpdateType` is `MoveForward | MoveBackward | MoveSideways | Add |
Delete` (`src/bookmark_updates.rs:8-15`); each carries optional
`old_commit` / `new_commit` (`src/bookmark_updates.rs:17-24`). Two
regex shapes per type are supported — the prose form (e.g. `Move
forward bookmark <b> from <old> to <new>`) and the structured
`bookmark: <b> [move forward from <old> to <new>]` form
(`src/bookmark_updates.rs:55-67`). `Add` carries only `new_commit`
(`src/bookmark_updates.rs:94-103`); `Delete` carries only `old_commit`
(`src/bookmark_updates.rs:104-114`). `Display` renders e.g. `Move
forward main from <8-char> to <8-char>`
(`src/bookmark_updates.rs:26-43`).

## push-tags

jj has no native `jj git push --tag`, so `push-tags` exports refs to
the colocated git repo and shells out per tag
(`src/push_tags.rs:1-11`, `src/cli.rs:85-87`). `run`
(`src/push_tags.rs:26-82`):

1. `jj git export --ignore-working-copy` to sync local refs into the
   git ref store; best-effort (`src/push_tags.rs:33`).
2. Tag list = every local tag when `--all` (`jj --ignore-working-copy
   tag list -T 'name ++ "\n"'`, `src/push_tags.rs:86-100`), else the
   positional `tags` (`src/push_tags.rs:35-39`); empty prints a notice
   and returns (`src/push_tags.rs:41-44`).
3. Per tag: refuse one that doesn't exist locally via `git rev-parse
   --verify --quiet refs/tags/<tag>` (`src/push_tags.rs:46-54`,
   `git_tag_exists` at `src/push_tags.rs:105-118`).
4. Run `git push [--force] <remote> refs/tags/<tag>`
   (`src/push_tags.rs:56-61`); `--dry-run` prints the command and skips
   (`src/push_tags.rs:63-70`); a non-zero `git push` aborts the loop
   (`src/push_tags.rs:72-78`).

`--all` and positional `tags` are mutually exclusive
(`src/cli.rs:93-94`); remote defaults to `origin`
(`src/cli.rs:105-106`).

## Workspace resolution

`JjCli` wraps the `jj` CLI rooted at a directory
(`src/jj.rs:10-19`); `run` inherits stderr on success while
`run_capture_stderr` always captures it (the dry-run probe writes to
stderr) (`src/jj.rs:25-36`). `primary_git_dir` resolves the shared git
dir for both layouts: a primary workspace's `.jj/repo/` directory vs a
secondary workspace's `.jj/repo` *file* pointing back at the primary
(`src/jj.rs:79-108`), which is why the worktree gate works from either
(`README.md:27-37`).

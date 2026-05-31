//! Per-bookmark hook execution pipeline.
//!
//! For each bookmark update being pushed:
//! 1. Resolve one or more `from_ref` commits (the ancestors on the remote).
//! 2. Create an ephemeral detached worktree at the new commit.
//! 3. Run the configured hook backend against each `from_ref` in turn.
//!    Modifications accumulate in the same worktree.
//! 4. If the worktree ended up with modifications, build a fixup commit
//!    via `git commit-tree`, anchor it under `refs/jj-hooks/fixup/<bookmark>`,
//!    and `jj git import` so jj sees it.
//! 5. Optionally re-run the hook backend against the fixup commit; if
//!    the re-run is clean, the overall outcome is reported as success
//!    with `initial_failure = true` so callers can surface the
//!    transient failure. See [`RunOpts::retry_after_fixup`].
//! 6. Optionally advance the bookmark to the fixup commit.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::bookmark_updates::BookmarkUpdate;
use crate::error::{JjHooksError, Result};
use crate::jj::JjCli;
use crate::runner::{
    Runner, Stage, hook_command, hook_command_all_files, lefthook_command,
    lefthook_command_all_files,
};
use crate::setup::{self, SetupStep};
use crate::worktree::Worktree;

#[derive(Debug, Clone)]
pub struct HookOutcome {
    /// Final success for this bookmark — `true` iff every hook run we
    /// took into account exited 0. When `retry_after_fixup` is enabled
    /// and a retry on the fixup commit was clean, this reports `true`
    /// even though the initial run failed.
    pub success: bool,
    /// Commit id of the fixup commit if the hook(s) modified files.
    /// `Some(_)` means the caller's tree is stale relative to what the
    /// hooks want.
    pub fixup_commit: Option<String>,
    /// `true` iff we re-ran hooks against the fixup commit after the
    /// initial run reported failure-with-fixup.
    pub retried: bool,
    /// `true` iff the initial hook run exited non-zero, regardless of
    /// whether a subsequent retry healed the outcome. CLI uses this to
    /// warn the user that something was racy even when the final state
    /// is OK.
    pub initial_failure: bool,
    /// Captured stdout/stderr from every hook subprocess invoked for
    /// this update, in order. `None` when [`RunOpts::capture_output`]
    /// is false (the default — hook output streams straight to the
    /// parent's terminal so the user sees runner progress live).
    /// `Some(buf)` when the caller asked for capture so it can
    /// multiplex N parallel runs into ordered output blocks. See
    /// [`run_for_updates_parallel`] for the canonical consumer.
    pub captured_output: Option<String>,
}

/// Inputs that control how [`run_for_update`] behaves. Defaults match
/// pre-0.3.0 behavior (no retry).
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOpts {
    /// When the initial hook run produces a fixup commit AND reports
    /// failure, re-run the hooks against the fixup commit. If the
    /// re-run is clean, the overall outcome is reported as success
    /// with `initial_failure = true`. Use this to recover from
    /// transient races (e.g. hk's intra-bookmark step parallelism
    /// fighting for `.git/index.lock` while one step legitimately
    /// auto-fixes files).
    pub retry_after_fixup: bool,
    /// Run hooks against every tracked file in the worktree rather
    /// than the diff range. Each runner gets its own all-files flag
    /// (see [`crate::runner::hook_command_all_files`]). Currently
    /// surfaced via `jj-hp run --all-files`; `push` always uses the
    /// diff range since the bookmark's ref bounds are the whole
    /// point.
    pub all_files: bool,
    /// Capture hook subprocess stdout/stderr into the returned
    /// [`HookOutcome::captured_output`] instead of letting it stream
    /// straight to the parent's terminal. Required for parallel
    /// per-bookmark hook runs (see [`run_for_updates_parallel`]) so
    /// N concurrent runs don't garble the terminal; the caller
    /// replays the captured blocks in completion order.
    ///
    /// Default is `false` — sequential single-bookmark runs (the
    /// `jj-hp push` path) want the live runner progress bar.
    pub capture_output: bool,
}

/// Run hooks for one bookmark update. Returns the outcome (success +
/// optional fixup commit + retry metadata).
///
/// `cli_runner` is the user's `--runner` override (or `None` for autodetect).
/// When `None`, runner detection happens inside the ephemeral worktree at the
/// target commit — so a commit that migrated runners (e.g. `lefthook → hk`)
/// is gated by the runner the *target* commits to, not the runner the user's
/// primary workspace currently has on disk.
pub fn run_for_update(
    jj: &JjCli,
    primary_git_dir: &Path,
    workspace_root: &Path,
    cli_runner: Option<Runner>,
    stage: Stage,
    update: &BookmarkUpdate,
    opts: RunOpts,
) -> Result<HookOutcome> {
    let Some(new_commit) = update.new_commit.as_ref() else {
        // Pure delete — nothing to check.
        return Ok(HookOutcome {
            success: true,
            fixup_commit: None,
            retried: false,
            initial_failure: false,
            captured_output: None,
        });
    };

    let from_refs = resolve_from_refs(jj, update)?;
    let setup_steps = setup::load_steps(jj)?;

    let initial = run_once(
        jj,
        primary_git_dir,
        workspace_root,
        cli_runner,
        stage,
        update,
        new_commit,
        &from_refs,
        &setup_steps,
        opts.all_files,
        opts.capture_output,
    )?;

    // Initial run was clean OR caller opted out of retry OR there's nothing
    // to retry against — return as-is. (No fixup means the caller's tree is
    // already what the hooks would produce; nothing to re-check.)
    if !opts.retry_after_fixup || initial.success || initial.fixup_commit.is_none() {
        return Ok(HookOutcome {
            success: initial.success,
            fixup_commit: initial.fixup_commit,
            retried: false,
            initial_failure: !initial.success,
            captured_output: initial.captured_output,
        });
    }

    let fixup = initial.fixup_commit.as_ref().expect("checked Some above");
    tracing::info!(
        "{update}: re-running hooks against fixup commit {fixup} to check for transient failure"
    );
    let retry = run_once(
        jj,
        primary_git_dir,
        workspace_root,
        cli_runner,
        stage,
        update,
        fixup,
        &from_refs,
        &setup_steps,
        opts.all_files,
        opts.capture_output,
    )?;

    // The retry should be clean (no failure, no new fixup) for the
    // "healed by retry" verdict. Any further fixup means the tree is
    // still drifting; bail with the original failure semantics.
    let healed = retry.success && retry.fixup_commit.is_none();
    // Concatenate initial + retry captured output so the caller sees
    // both passes in order. Only relevant when capture_output is on;
    // when off, both are None.
    let captured_output = match (initial.captured_output, retry.captured_output) {
        (Some(mut a), Some(b)) => {
            a.push_str(&b);
            Some(a)
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    Ok(HookOutcome {
        // If the retry healed it, report success and surface the fixup
        // so the user knows to advance their bookmark. If the retry
        // *also* failed, success is whatever the retry reported and
        // the fixup is whichever one the retry produced (which may
        // differ from the initial one).
        success: if healed { true } else { retry.success },
        fixup_commit: if healed {
            initial.fixup_commit
        } else {
            // The retry pass either produced a fresh fixup (chain of
            // autofixes) or none at all (just a hard failure). Prefer
            // the retry's fixup when it has one so the user advances
            // their bookmark to the most recent good state; fall back
            // to the initial fixup so we don't drop information.
            retry.fixup_commit.or(initial.fixup_commit)
        },
        retried: true,
        initial_failure: true,
        captured_output,
    })
}

/// Batch entrypoint: run hooks for N bookmark updates in parallel.
///
/// One thread per update. Each thread runs the full
/// [`run_for_update`] pipeline against its own ephemeral worktree —
/// the worktrees are filesystem-isolated and don't share index locks,
/// so per-bookmark hook backends (cargo, hk, etc.) can run truly
/// concurrently. The shared `.git/objects/` directory is read-mostly
/// during hook execution; the per-bookmark `jj git import` invoked at
/// the end of `run_for_update` (if a fixup was produced) relies on
/// jj's own concurrent-op reconciliation.
///
/// Mandatory call-site invariant: `opts.capture_output` MUST be true.
/// Letting N hook backends stream live to the same terminal garbles
/// the user's view. The function asserts on this — passing
/// `capture_output: false` is a programmer error.
///
/// Returns results in the same order as `updates` (not completion
/// order). `progress` is invoked once per update on the thread that
/// finished it, with the update's index + outcome — the caller uses
/// this to print captured output in completion order rather than
/// input order.
///
/// First subprocess error (spawn failure, etc.) aborts before
/// returning; per-update non-zero exits are reported via
/// [`HookOutcome::success`], not as `Err`.
#[allow(clippy::too_many_arguments)]
pub fn run_for_updates_parallel<F>(
    jj: &JjCli,
    primary_git_dir: &Path,
    workspace_root: &Path,
    cli_runner: Option<Runner>,
    stage: Stage,
    updates: &[BookmarkUpdate],
    opts: RunOpts,
    progress: F,
) -> Result<Vec<HookOutcome>>
where
    F: Fn(usize, &BookmarkUpdate, &HookOutcome) + Send + Sync,
{
    assert!(
        opts.capture_output,
        "run_for_updates_parallel requires capture_output=true; parallel runs without capture garble the terminal",
    );

    use std::sync::Mutex;
    let progress = &progress;
    let results: Vec<Mutex<Option<Result<HookOutcome>>>> =
        (0..updates.len()).map(|_| Mutex::new(None)).collect();
    let results_ref = &results;

    std::thread::scope(|s| {
        for (idx, update) in updates.iter().enumerate() {
            s.spawn(move || {
                let outcome = run_for_update(
                    jj,
                    primary_git_dir,
                    workspace_root,
                    cli_runner,
                    stage,
                    update,
                    opts,
                );
                if let Ok(o) = &outcome {
                    progress(idx, update, o);
                }
                *results_ref[idx].lock().unwrap() = Some(outcome);
            });
        }
    });

    let mut out = Vec::with_capacity(updates.len());
    for slot in results {
        let result = slot
            .into_inner()
            .unwrap()
            .expect("thread::scope joined all threads but a slot is still None");
        out.push(result?);
    }
    Ok(out)
}

/// Sequential counterpart to [`run_for_updates_parallel`] for the
/// `--hooks-sequential` opt-out path. Same contract (per-bookmark
/// `run_for_update`, in-order results) but no thread fan-out. Output
/// streams live by default — `opts.capture_output` is honored if
/// set, but unlike the parallel variant it isn't required.
#[allow(clippy::too_many_arguments)]
pub fn run_for_updates_sequential<F>(
    jj: &JjCli,
    primary_git_dir: &Path,
    workspace_root: &Path,
    cli_runner: Option<Runner>,
    stage: Stage,
    updates: &[BookmarkUpdate],
    opts: RunOpts,
    progress: F,
) -> Result<Vec<HookOutcome>>
where
    F: Fn(usize, &BookmarkUpdate, &HookOutcome),
{
    let mut out = Vec::with_capacity(updates.len());
    for (idx, update) in updates.iter().enumerate() {
        let outcome = run_for_update(
            jj,
            primary_git_dir,
            workspace_root,
            cli_runner,
            stage,
            update,
            opts,
        )?;
        progress(idx, update, &outcome);
        out.push(outcome);
    }
    Ok(out)
}

/// Internal shape returned by [`run_once`]: a single hook run plus the
/// fixup commit (if any) it produced. This is the per-attempt building
/// block used by [`run_for_update`] to layer retry-after-fixup logic.
struct OnceOutcome {
    success: bool,
    fixup_commit: Option<String>,
    /// `Some(buf)` iff the caller asked for capture (see
    /// [`RunOpts::capture_output`]). Carries the concatenated
    /// stdout+stderr of every subprocess invoked during this pass.
    captured_output: Option<String>,
}

/// Replace element 0 of `command_argv` (the bare runner binary name
/// produced by `hook_command{,_all_files}` / `lefthook_command{,_all_files}`)
/// with the resolved argv prefix from [`crate::runner::resolve_runner_argv`].
///
/// For the common case the prefix is a single element (an absolute path
/// or just the bare name found on $PATH), so this is a near-no-op. For
/// the `uv run --` wrapper case the prefix is multiple elements; we
/// drop the placeholder name and splice in the wrapper.
fn splice_runner_prefix(prefix: &[String], command_argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(prefix.len() + command_argv.len().saturating_sub(1));
    out.extend(prefix.iter().cloned());
    if command_argv.len() > 1 {
        out.extend(command_argv[1..].iter().cloned());
    }
    out
}

/// One pass through the hook pipeline against a specific target commit.
///
/// Builds a fresh worktree at `target_commit`, runs the hook backend
/// against each entry in `from_refs`, and, if the worktree's tree
/// differs from `target_commit`'s tree at the end, builds a fixup
/// commit + cleans up the temp ref / bookmark that `jj git import`
/// creates.
///
/// When `capture_output` is true, every subprocess's stdout+stderr
/// gets folded into the returned `OnceOutcome::captured_output`
/// instead of streaming to the parent terminal. The trade is no live
/// progress bar — the caller (typically [`run_for_updates_parallel`])
/// replays the captured block when the pass finishes.
///
/// Callers (currently [`run_for_update`] and the batch entrypoint)
/// decide whether to retry based on the returned `success` /
/// `fixup_commit`.
#[allow(clippy::too_many_arguments)]
fn run_once(
    jj: &JjCli,
    primary_git_dir: &Path,
    workspace_root: &Path,
    cli_runner: Option<Runner>,
    stage: Stage,
    update: &BookmarkUpdate,
    target_commit: &str,
    from_refs: &[String],
    setup_steps: &[SetupStep],
    all_files: bool,
    capture_output: bool,
) -> Result<OnceOutcome> {
    let wt = Worktree::create(primary_git_dir, target_commit)?;

    // User-declared setup commands (e.g. `bun install`) run inside
    // the worktree before the runner so hooks have install-time
    // resources (`node_modules`, `.venv`, etc.) available. A
    // non-zero exit aborts before the runner is invoked — the
    // worktree is unhealthy and there's no point asking the
    // runner to grade it.
    setup::run_steps(setup_steps, wt.path(), workspace_root)?;

    // Resolve the runner from the target commit's tree, not the primary
    // workspace. `--runner` overrides; otherwise autodetect against the
    // worktree we just checked out. If autodetect comes up empty, the
    // commit doesn't have a hook config — silent-skip with an info log.
    let runner = match cli_runner {
        Some(r) => r,
        None => {
            let Some(r) = Runner::autodetect(wt.path())? else {
                eprintln!(
                    "jj-hooks: {update}: no hook-runner config in target commit; skipping hooks"
                );
                return Ok(OnceOutcome {
                    success: true,
                    fixup_commit: None,
                    captured_output: None,
                });
            };
            // prek is a faster drop-in for pre-commit; prefer it when
            // present. The override path already skips this so an explicit
            // `--runner pre-commit` keeps the slower binary.
            //
            // "Present" here means resolvable through any of the layers
            // in [`resolve_runner_argv`], not just $PATH — a prek
            // installed only inside a venv (the issue #17 scenario) is
            // still preferable to the pre-commit on $PATH if the user
            // bothered to `prek install` the shim or set the config.
            let prek_present = crate::runner::resolve_runner_argv(
                Runner::Prek,
                jj,
                workspace_root,
                primary_git_dir,
                stage,
            )
            .is_ok();
            crate::runner::prefer_prek_when_available(r, prek_present)
        }
    };

    // Pre-check that the runner binary is on PATH. Without this, the
    // `Command::status()` call below surfaces a libc-level
    // `posix_spawn: No such file or directory (os error 2)` with no
    // indication of *which* binary couldn't be found. The common case
    // for prek users is that prek is installed only inside a Python
    // venv — jj-hooks runs in a clean ephemeral worktree and doesn't
    // inherit the venv's PATH, so the user sees the cryptic error
    // and has no idea it was prek that was missing.
    //
    // Resolution order is (1) explicit config, (2) the path baked into
    // the `.git/hooks/<stage>` shim by `prek install` / `pre-commit
    // install`, (3) `uv run` when uv.lock + uv are both present,
    // (4) plain $PATH. See `resolve_runner_argv` for details.
    let runner_argv =
        crate::runner::resolve_runner_argv(runner, jj, workspace_root, primary_git_dir, stage)?;

    // all_files: ignore the diff range and run each runner's
    // "lint every tracked file" command exactly once. from_refs
    // is meaningless here — the runner sees no --from-ref/--to-ref.
    //
    // Default path: iterate from_refs (one per ancestor on the
    // remote) so multi-ancestor pushes still get the full set of
    // diff bases. Each iteration accumulates modifications in the
    // shared worktree, mirroring how the standard pre-push pipeline
    // builds up its fixup.
    let mut success = true;
    let mut captured = if capture_output {
        Some(String::new())
    } else {
        None
    };
    if all_files {
        let argv = match runner {
            Runner::Lefthook => lefthook_command_all_files(stage),
            _ => hook_command_all_files(runner, stage),
        };
        let argv = splice_runner_prefix(&runner_argv, &argv);
        tracing::info!("running (--all-files): {:?}", argv);
        let ok = run_subprocess(&argv, wt.path(), workspace_root, captured.as_mut())?;
        if !ok {
            success = false;
        }
    } else {
        for from_ref in from_refs {
            let argv = match runner {
                Runner::Lefthook => {
                    let files = changed_files(wt.path(), from_ref, target_commit)?;
                    lefthook_command(stage, &files)
                }
                _ => hook_command(runner, stage, from_ref, target_commit),
            };
            let argv = splice_runner_prefix(&runner_argv, &argv);

            tracing::info!("running: {:?}", argv);
            let ok = run_subprocess(&argv, wt.path(), workspace_root, captured.as_mut())?;
            if !ok {
                success = false;
            }
        }
    }

    let fixup_commit =
        maybe_build_fixup_commit(primary_git_dir, wt.path(), target_commit, &update.bookmark)?;

    if fixup_commit.is_some() {
        // Make jj aware of the new commit. --ignore-working-copy keeps
        // this import from racing against any concurrent `jj` process
        // (same lock-contention rationale as in push.rs).
        jj.run(&["git", "import", "--ignore-working-copy"])?;

        // jj git import created a `jj-hooks-fixup/<bookmark>` jj bookmark
        // from the underlying refs/heads/jj-hooks-fixup/<bookmark> ref.
        // Clean both up immediately — the user almost always wants to
        // either squash the fixup into the parent or move their bookmark
        // forward themselves, not have a stale temp bookmark lying
        // around. The commit stays addressable by hash via `jj log`,
        // `jj show`, `jj squash --from <hash>` etc. since jj tracks it
        // in its own commit graph independent of the ref.
        let temp_bookmark = fixup_bookmark(&update.bookmark);
        // `jj bookmark forget` removes the jj bookmark, but in a
        // secondary workspace it leaves the underlying refs/heads/<name>
        // ref alive in the primary's git dir. Explicitly delete the
        // git ref ourselves so the cleanup is uniform.
        let _ = jj.run(&[
            "bookmark",
            "forget",
            &temp_bookmark,
            "--ignore-working-copy",
        ]);
        let _ = delete_git_ref(primary_git_dir, &fixup_ref(&update.bookmark));
    }

    Ok(OnceOutcome {
        success,
        fixup_commit,
        captured_output: captured,
    })
}

/// Run a hook subprocess. When `capture` is `Some`, the child's
/// stdout+stderr are captured into the buffer (chronological order is
/// approximated by concatenating stdout then stderr — the runner CLIs
/// we wrap mostly print failures to stderr so this preserves the
/// signal even though it's not a true byte-level interleave). When
/// `capture` is `None`, the child inherits stdio so the user sees the
/// runner's progress bar live.
///
/// Returns `Ok(true)` on a zero exit, `Ok(false)` on any non-zero
/// exit. IO errors (spawn failure, etc.) propagate as `Err`.
fn run_subprocess(
    argv: &[String],
    cwd: &Path,
    workspace_root: &Path,
    capture: Option<&mut String>,
) -> Result<bool> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(cwd)
        .env("JJ_HOOKS_WORKSPACE", workspace_root);
    match capture {
        None => {
            let status = cmd.status()?;
            Ok(status.success())
        }
        Some(buf) => {
            let output = cmd.output()?;
            // Tag the captured block with the argv so the user can
            // see which subprocess produced each chunk when N hook
            // backends are multiplexed.
            buf.push_str(&format!("$ {}\n", argv.join(" ")));
            buf.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                buf.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            if !buf.ends_with('\n') {
                buf.push('\n');
            }
            Ok(output.status.success())
        }
    }
}

/// Resolve the `from_ref` commits to diff against. For an existing
/// bookmark update we just use the old commit; for a new bookmark we
/// find the heads of `::new & ::remote_bookmarks(remote)` so each
/// already-on-remote ancestor becomes its own diff base.
fn resolve_from_refs(jj: &JjCli, update: &BookmarkUpdate) -> Result<Vec<String>> {
    if let Some(old) = update.old_commit.as_ref() {
        return Ok(vec![old.clone()]);
    }

    let new = update.new_commit.as_ref().expect("not a delete here");
    let revset = format!(
        "heads(::{new} & ::remote_bookmarks(remote=exact:{}))",
        update.remote
    );

    let template = r#"commit_id ++ "\n""#;
    let out = jj.run(&[
        "log",
        "--no-graph",
        "-r",
        &revset,
        "-T",
        template,
        "--ignore-working-copy",
    ])?;

    let refs: Vec<String> = out
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();

    if refs.is_empty() {
        // New bookmark on a totally fresh remote — no ancestors on the
        // remote at all. Use the parent of new as the diff base.
        return Ok(vec![format!("{new}^")]);
    }

    Ok(refs)
}

fn changed_files(worktree: &Path, from: &str, to: &str) -> Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=ACMR"])
        .arg(format!("{from}..{to}"))
        .current_dir(worktree)
        .output()?;
    if !out.status.success() {
        return Err(JjHooksError::JjFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: format!(
                "git diff --name-only failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| PathBuf::from(l.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect())
}

/// Stage everything in the worktree, hash the resulting tree, and
/// compare against the parent commit's tree. Returns a fixup commit
/// only when the trees actually differ — `git status --porcelain`
/// can report a worktree as dirty (e.g. when a hook runner touched
/// the index without changing file content; hk's auto-stage path
/// does this even on check-only steps), but the resulting tree is
/// often identical to the parent and an empty fixup commit is just
/// noise that pins the bookmark to a content-equivalent revision
/// and aborts the push.
///
/// Content-addressed gating eliminates the false positive: if the
/// hooks didn't actually change any file, the write-tree OID equals
/// the parent's tree OID and we return `None`.
fn maybe_build_fixup_commit(
    primary_git_dir: &Path,
    worktree: &Path,
    parent: &str,
    bookmark: &str,
) -> Result<Option<String>> {
    // Stage everything (tracked + untracked) and hash the tree.
    // Both are cheap on a clean checkout — `git add -A` is a no-op
    // when nothing changed; `git write-tree` is hashing-only.
    run_git(worktree, &["add", "-A"])?;
    let tree = run_git_capture(worktree, &["write-tree"])?;

    // Parent's tree as a content reference. `<commit>^{tree}` is
    // the standard rev-parse spelling.
    let parent_tree_spec = format!("{parent}^{{tree}}");
    let parent_tree = run_git_capture(worktree, &["rev-parse", &parent_tree_spec])?;

    if tree == parent_tree {
        return Ok(None);
    }

    // Build the commit object via the *primary* git dir so the resulting
    // commit lives in the shared object database.
    let message = format!("jj-hooks: autofixes for {bookmark}");
    let commit = run_git_capture_with_git_dir(
        primary_git_dir,
        worktree,
        &["commit-tree", &tree, "-p", parent, "-m", &message],
    )?;

    // Anchor under refs/heads/ so `jj git import` will pick it up as a
    // bookmark. (Refs outside refs/heads/ and refs/remotes/ are invisible
    // to jj's git import logic.)
    let ref_name = fixup_ref(bookmark);
    run_git_capture_with_git_dir(
        primary_git_dir,
        worktree,
        &["update-ref", &ref_name, &commit],
    )?;

    Ok(Some(commit))
}

/// The git ref where a fixup commit gets anchored for a given bookmark.
/// Lives under `refs/heads/` so `jj git import` picks it up as a bookmark.
pub fn fixup_ref(bookmark: &str) -> String {
    format!("refs/heads/jj-hooks-fixup/{}", sanitize_for_ref(bookmark))
}

/// The jj bookmark name corresponding to `fixup_ref`.
pub fn fixup_bookmark(bookmark: &str) -> String {
    format!("jj-hooks-fixup/{}", sanitize_for_ref(bookmark))
}

/// Replace characters that git rejects in ref names (per git-check-ref-format)
/// with `_`. Real bookmark names like `main` or `feature/foo` pass through
/// unchanged; synthesized names like `revset:@` (used by `jj-hp run @`) get
/// scrubbed so the resulting `refs/heads/jj-hooks-fixup/<name>` is valid.
fn sanitize_for_ref(s: &str) -> String {
    // Per-character offenders first; then collapse multi-char sequences
    // and trim the position-sensitive ones (leading `-`/`.`, trailing
    // `.`/`.lock`/`/`, internal `//`).
    let mut out: String = s
        .chars()
        .map(|c| match c {
            ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\x7f' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    while out.contains("..") {
        out = out.replace("..", "__");
    }
    while out.contains("@{") {
        out = out.replace("@{", "@_");
    }
    if out.starts_with('-') {
        out.replace_range(0..1, "_");
    }
    if out.starts_with('.') {
        out.replace_range(0..1, "_");
    }
    if out.ends_with('.') {
        let n = out.len();
        out.replace_range(n - 1..n, "_");
    }
    if out.ends_with(".lock") {
        let n = out.len();
        out.replace_range(n - 5..n - 4, "_");
    }
    if out.ends_with('/') {
        let n = out.len();
        out.replace_range(n - 1..n, "_");
    }
    while out.contains("//") {
        out = out.replace("//", "/_");
    }
    if out.is_empty() {
        return "_".into();
    }
    out
}

/// Delete a git ref in the given git dir, ignoring "ref doesn't exist"
/// failures. Used to clean up the temp `refs/heads/jj-hooks-fixup/<name>`
/// after `jj git import` + `jj bookmark forget` from a secondary
/// workspace (where forget leaves the underlying ref alive).
fn delete_git_ref(git_dir: &Path, ref_name: &str) -> Result<()> {
    let out = Command::new("git")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(["update-ref", "-d", ref_name])
        .output()?;
    if !out.status.success() {
        // Treat any failure as best-effort: if the ref didn't exist,
        // that's the desired state already.
        tracing::debug!(
            "git update-ref -d {ref_name} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git").args(args).current_dir(cwd).output()?;
    if !out.status.success() {
        return Err(JjHooksError::JjFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }
    Ok(())
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(cwd).output()?;
    if !out.status.success() {
        return Err(JjHooksError::JjFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn run_git_capture_with_git_dir(git_dir: &Path, cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(args)
        .current_dir(cwd)
        .output()?;
    if !out.status.success() {
        return Err(JjHooksError::JjFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: format!(
                "git --git-dir={} {args:?} failed: {}",
                git_dir.display(),
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixup_ref_for_plain_bookmark() {
        assert_eq!(fixup_ref("main"), "refs/heads/jj-hooks-fixup/main");
    }

    #[test]
    fn fixup_ref_keeps_internal_slash() {
        // jj bookmark names commonly contain `/` (e.g. `feature/foo`) and
        // git accepts them as path separators inside a ref.
        assert_eq!(
            fixup_ref("feature/foo"),
            "refs/heads/jj-hooks-fixup/feature/foo"
        );
    }

    #[test]
    fn fixup_ref_scrubs_colon() {
        // The bug from issue #1: `jj-hp run @` synthesizes `revset:@`.
        // Without sanitization, git rejects the ref with "bad name".
        assert_eq!(fixup_ref("revset:@"), "refs/heads/jj-hooks-fixup/revset_@");
    }

    #[test]
    fn sanitize_replaces_each_invalid_char() {
        // One probe per character class git-check-ref-format rejects.
        assert_eq!(sanitize_for_ref("a:b"), "a_b");
        assert_eq!(sanitize_for_ref("a~b"), "a_b");
        assert_eq!(sanitize_for_ref("a^b"), "a_b");
        assert_eq!(sanitize_for_ref("a?b"), "a_b");
        assert_eq!(sanitize_for_ref("a*b"), "a_b");
        assert_eq!(sanitize_for_ref("a[b"), "a_b");
        assert_eq!(sanitize_for_ref("a\\b"), "a_b");
        assert_eq!(sanitize_for_ref("a b"), "a_b");
        assert_eq!(sanitize_for_ref("a\tb"), "a_b");
        assert_eq!(sanitize_for_ref("a\x7fb"), "a_b");
    }

    #[test]
    fn sanitize_collapses_double_dot() {
        assert_eq!(sanitize_for_ref("a..b"), "a__b");
        // `..` replacement is non-overlapping: `a...b` becomes `a__.b`
        // (first `..` matches at positions 1-2 and gets replaced; the
        // remaining `.` is harmless mid-string).
        assert_eq!(sanitize_for_ref("a...b"), "a__.b");
        assert!(!sanitize_for_ref("a....b").contains(".."));
    }

    #[test]
    fn sanitize_collapses_at_brace() {
        assert_eq!(sanitize_for_ref("a@{b"), "a@_b");
    }

    #[test]
    fn sanitize_strips_leading_dash() {
        assert_eq!(sanitize_for_ref("-foo"), "_foo");
    }

    #[test]
    fn sanitize_strips_leading_dot() {
        assert_eq!(sanitize_for_ref(".foo"), "_foo");
    }

    #[test]
    fn sanitize_strips_trailing_dot() {
        assert_eq!(sanitize_for_ref("foo."), "foo_");
    }

    #[test]
    fn sanitize_strips_trailing_dot_lock() {
        assert_eq!(sanitize_for_ref("foo.lock"), "foo_lock");
    }

    #[test]
    fn sanitize_strips_trailing_slash() {
        assert_eq!(sanitize_for_ref("foo/"), "foo_");
    }

    #[test]
    fn sanitize_collapses_double_slash() {
        assert_eq!(sanitize_for_ref("a//b"), "a/_b");
    }

    #[test]
    fn sanitize_empty_becomes_underscore() {
        // Defensive: if the input is empty after some external transform,
        // emit a single underscore so the joined ref isn't dangling.
        assert_eq!(sanitize_for_ref(""), "_");
    }

    #[test]
    fn fixup_bookmark_uses_same_sanitizer() {
        // fixup_bookmark feeds `jj bookmark forget` which is also strict
        // about colon (jj rejects bookmark names with `:` in them).
        assert_eq!(fixup_bookmark("revset:@"), "jj-hooks-fixup/revset_@");
    }
}

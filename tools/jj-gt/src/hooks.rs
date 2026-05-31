//! Thin wrapper around [`jj_hooks::hooks::run_for_update`] and its
//! batch siblings.
//!
//! We deliberately don't go through `jj_hooks::run_for_revset_outcome`.
//! That entrypoint synthesizes a [`BookmarkUpdate`] from a revset
//! string and was historically a bug magnet — early versions
//! truncated the range to the tip slice (`--limit 1`), so a 3-commit
//! stack only got its top commit's delta checked. We sidestep the
//! synthesis layer by building the BookmarkUpdate ourselves with real
//! commit ids the caller already has on hand — exactly the same
//! shape `jj-hp push` builds from its `jj git push --dry-run` parse.

use std::path::Path;

use jj_hooks::bookmark_updates::{BookmarkUpdate, UpdateType};
use jj_hooks::hooks::{HookOutcome, RunOpts};
use jj_hooks::jj::{self, JjCli};
use jj_hooks::runner::{Runner, Stage};

use crate::error::{JjGtError, Result};

#[derive(Debug, Clone, Default)]
pub struct HookOpts {
    /// Override the autodetected hook runner. `None` means "let
    /// jj_hooks autodetect from the target commit's tree".
    pub runner_override: Option<Runner>,
}

/// Build a [`BookmarkUpdate`] for the standard "push the bookmark at
/// `tip_commit` over `trunk_commit`" shape.
///
/// `old_commit = trunk_commit` is the from-ref hooks diff against
/// (the merge-base of the stack and trunk); `new_commit = tip_commit`
/// is the bookmark tip, the worktree hooks actually run inside.
/// `jj_hooks::hooks::run_for_update` takes that and runs the
/// configured hook backend with `--from-ref <trunk> --to-ref <tip>`
/// so every file changed across the bookmark's diff is in scope —
/// same contract as `git push origin <bookmark>` would produce.
fn build_update(
    remote: &str,
    bookmark: &str,
    trunk_commit: &str,
    tip_commit: &str,
) -> BookmarkUpdate {
    BookmarkUpdate {
        remote: remote.to_owned(),
        bookmark: bookmark.to_owned(),
        update_type: UpdateType::MoveForward,
        old_commit: Some(trunk_commit.to_owned()),
        new_commit: Some(tip_commit.to_owned()),
    }
}

/// Run pre-push hooks against the full diff range from `trunk_commit`
/// to `tip_commit` for `bookmark` on `remote`. Used by the
/// `--hooks-tip-only` opt-out path and (transitively) by callers
/// that want to gate a single bookmark.
///
/// Output streams live to the parent terminal — the user sees hk's
/// progress bar etc. For the multi-bookmark case use
/// [`run_pre_push_stack`] instead.
pub fn run_pre_push(
    jj: &JjCli,
    workspace_root: &Path,
    remote: &str,
    bookmark: &str,
    trunk_commit: &str,
    tip_commit: &str,
    opts: &HookOpts,
) -> Result<()> {
    if trunk_commit == tip_commit {
        tracing::info!("pre-push: bookmark `{bookmark}` is already at trunk; skipping hooks");
        return Ok(());
    }

    let update = build_update(remote, bookmark, trunk_commit, tip_commit);
    let primary_git_dir = jj::primary_git_dir(workspace_root).map_err(JjGtError::Hooks)?;
    let run_opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        capture_output: false,
    };
    let outcome = jj_hooks::hooks::run_for_update(
        jj,
        &primary_git_dir,
        workspace_root,
        opts.runner_override,
        Stage::PrePush,
        &update,
        run_opts,
    )
    .map_err(JjGtError::Hooks)?;
    interpret_outcome(&outcome, bookmark, trunk_commit, tip_commit)
}

/// Per-bookmark pre-push gate over N independent stacks. Each
/// `partitions` entry is one stack (a `Vec<(bookmark_name,
/// tip_commit)>`, bottom→top).
///
/// Within a partition, bookmark `i`'s diff-base is bookmark
/// `i-1`'s tip (and bookmark 0's is `trunk_commit`), matching the
/// per-bookmark slicing introduced in PR-B. Across partitions,
/// nothing is shared — partitions are independent stacks rooted at
/// trunk.
///
/// Fail-fast scope is per-partition. If bookmark `mid` in stack A
/// fails fmt, stack A's `head` cancels its remaining hk steps
/// — but stack B keeps running to completion. The user's mental
/// model "those two `-b` tips were independent" matches what the
/// pipeline actually does.
///
/// When `parallel` is true, runs all partitions concurrently via
/// [`jj_hooks::hooks::run_for_partitioned_updates_parallel`] with
/// output capture; when false, runs everything sequentially with
/// live output (no fail-fast — `--hooks-sequential` users want to
/// see the runner's progress bar end-to-end).
pub fn run_pre_push_stack(
    jj: &JjCli,
    workspace_root: &Path,
    remote: &str,
    trunk_commit: &str,
    partitions: &[Vec<(String, String)>],
    parallel: bool,
    opts: &HookOpts,
) -> Result<()> {
    if partitions.is_empty() || partitions.iter().all(|p| p.is_empty()) {
        return Ok(());
    }
    let primary_git_dir = jj::primary_git_dir(workspace_root).map_err(JjGtError::Hooks)?;

    // Build per-partition `Vec<BookmarkUpdate>`s. Each partition's
    // first update has from-ref = trunk; subsequent updates use the
    // previous update's tip as their from-ref. Empty ranges (a
    // bookmark sitting at the same commit as its parent) are
    // skipped but `prev_tip` still advances.
    let mut update_partitions: Vec<Vec<BookmarkUpdate>> = Vec::with_capacity(partitions.len());
    for partition in partitions {
        let mut updates: Vec<BookmarkUpdate> = Vec::with_capacity(partition.len());
        let mut prev_tip = trunk_commit.to_owned();
        for (name, tip) in partition {
            if &prev_tip == tip {
                tracing::info!(
                    "pre-push: bookmark `{name}` is at the same commit as its parent; skipping hooks"
                );
            } else {
                updates.push(build_update(remote, name, &prev_tip, tip));
            }
            prev_tip = tip.clone();
        }
        update_partitions.push(updates);
    }

    if update_partitions.iter().all(|p| p.is_empty()) {
        return Ok(());
    }

    let run_opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        capture_output: parallel,
    };

    let outcomes: Vec<Vec<HookOutcome>> = if parallel {
        let progress =
            |_p_idx: usize, _u_idx: usize, update: &BookmarkUpdate, outcome: &HookOutcome| {
                // Print the captured block as soon as each thread
                // finishes so the user gets feedback in completion
                // order. The block is self-contained — argv-prefixed
                // subprocess invocations + their output. Cancelled
                // siblings get a one-line "cancelled" marker so the
                // user knows they didn't run, not that they passed.
                if outcome.cancelled {
                    eprintln!(
                        "--- pre-push hooks: {} (cancelled — sibling failed) ---",
                        update.bookmark
                    );
                    return;
                }
                let header = if outcome.success {
                    format!("--- pre-push hooks: {} (clean) ---", update.bookmark)
                } else {
                    format!("--- pre-push hooks: {} (FAILED) ---", update.bookmark)
                };
                eprintln!("{header}");
                if let Some(buf) = outcome.captured_output.as_ref() {
                    eprint!("{buf}");
                    if !buf.ends_with('\n') {
                        eprintln!();
                    }
                }
                eprintln!("--- end {} ---", update.bookmark);
            };
        jj_hooks::hooks::run_for_partitioned_updates_parallel(
            jj,
            &primary_git_dir,
            workspace_root,
            opts.runner_override,
            Stage::PrePush,
            &update_partitions,
            run_opts,
            progress,
        )
        .map_err(JjGtError::Hooks)?
    } else {
        // Sequential path: no fail-fast (the user opted into
        // serial execution explicitly; aborting a 5-bookmark
        // run on bookmark 2 would surprise them). Flatten the
        // partitions for the sequential entrypoint and rebuild
        // the partitioned outcome shape on the way back.
        let progress = |_idx: usize, _update: &BookmarkUpdate, _outcome: &HookOutcome| {};
        let flat: Vec<BookmarkUpdate> = update_partitions
            .iter()
            .flat_map(|p| p.iter().cloned())
            .collect();
        let flat_outcomes = jj_hooks::hooks::run_for_updates_sequential(
            jj,
            &primary_git_dir,
            workspace_root,
            opts.runner_override,
            Stage::PrePush,
            &flat,
            run_opts,
            progress,
        )
        .map_err(JjGtError::Hooks)?;
        let mut iter = flat_outcomes.into_iter();
        update_partitions
            .iter()
            .map(|p| (0..p.len()).filter_map(|_| iter.next()).collect())
            .collect()
    };

    // Surface the first non-cancelled failure. Outcomes mirror the
    // input partition shape; cancelled entries just mean a sibling
    // already failed and we'll report THAT one.
    for (partition_outcomes, partition_updates) in outcomes.iter().zip(update_partitions.iter()) {
        for (outcome, update) in partition_outcomes.iter().zip(partition_updates.iter()) {
            if outcome.cancelled {
                continue;
            }
            let trunk_for_this = update.old_commit.clone().unwrap_or_default();
            let tip_for_this = update.new_commit.clone().unwrap_or_default();
            interpret_outcome(outcome, &update.bookmark, &trunk_for_this, &tip_for_this)?;
        }
    }
    Ok(())
}

/// Translate a [`HookOutcome`] into the success/failure shape
/// `jj-gt` callers expect: Ok on a clean pass, Err with a
/// descriptive message on either a hook failure or a hook autofix
/// (so the user can squash the fixup into the stack and re-submit).
///
/// `retry_after_fixup` is on in every jj-gt entrypoint so a
/// transient race that healed on retry surfaces as a fixup commit
/// with the `retried` flag set; the message names that case
/// explicitly.
fn interpret_outcome(
    outcome: &HookOutcome,
    bookmark: &str,
    trunk_commit: &str,
    tip_commit: &str,
) -> Result<()> {
    if outcome.success && outcome.fixup_commit.is_none() {
        return Ok(());
    }
    if !outcome.success {
        return Err(JjGtError::Invalid(format!(
            "pre-push hooks failed for `{bookmark}` ({trunk_commit}..{tip_commit})"
        )));
    }
    let commit = outcome
        .fixup_commit
        .clone()
        .unwrap_or_else(|| "<unknown>".into());
    let retry_hint = if outcome.retried {
        " (re-run on fixup commit was clean — initial failure was transient)"
    } else {
        ""
    };
    Err(JjGtError::Invalid(format!(
        "pre-push hooks modified files for `{bookmark}` (fixup commit {commit}){retry_hint}; \
         squash it into the relevant bookmark and re-run `jj-gt submit`"
    )))
}

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
                // One-line live status per completed bookmark so
                // the user sees progress in completion order. Full
                // failure detail is deferred to the post-run replay
                // (see below) so the actionable content lands at
                // the bottom of the screen — right above the final
                // error — rather than scrolled away mid-run.
                if outcome.cancelled {
                    eprintln!("  cancelled  {} (sibling failed)", update.bookmark);
                } else if outcome.success {
                    eprintln!("  passed     {}", update.bookmark);
                } else {
                    eprintln!("  FAILED     {} (full output below)", update.bookmark);
                }
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

    // Classify each outcome into a structured per-bookmark result.
    // Used for both the post-run summary table and the error
    // selection below.
    let mut classified: Vec<(String, BookmarkResult)> = Vec::new();
    for (partition_outcomes, partition_updates) in outcomes.iter().zip(update_partitions.iter()) {
        for (outcome, update) in partition_outcomes.iter().zip(partition_updates.iter()) {
            if outcome.cancelled {
                classified.push((update.bookmark.clone(), BookmarkResult::Cancelled));
                continue;
            }
            let trunk_for_this = update.old_commit.clone().unwrap_or_default();
            let tip_for_this = update.new_commit.clone().unwrap_or_default();
            match interpret_outcome(outcome, &update.bookmark, &trunk_for_this, &tip_for_this) {
                Ok(()) => {
                    classified.push((update.bookmark.clone(), BookmarkResult::Passed));
                }
                Err(JjGtError::Invalid(msg)) if outcome.fixup_commit.is_some() => {
                    classified.push((
                        update.bookmark.clone(),
                        BookmarkResult::Fixup {
                            message: msg,
                            captured: outcome.captured_output.clone(),
                        },
                    ));
                }
                Err(JjGtError::Invalid(msg)) => {
                    classified.push((
                        update.bookmark.clone(),
                        BookmarkResult::Failed {
                            message: msg,
                            captured: outcome.captured_output.clone(),
                        },
                    ));
                }
                Err(other) => {
                    // Non-Invalid error (subprocess spawn, etc.) —
                    // propagate untouched; the user gets the raw
                    // error message rather than a summary.
                    return Err(other);
                }
            }
        }
    }

    // Anything classified as Failed or Fixup means the submit
    // can't proceed. Render the per-screen summary + replay the
    // captured output of just the failed bookmarks at the bottom
    // (the user is already looking there), then return a single
    // error naming the first failure.
    let bad_count = classified
        .iter()
        .filter(|(_, r)| {
            matches!(
                r,
                BookmarkResult::Failed { .. } | BookmarkResult::Fixup { .. }
            )
        })
        .count();

    if bad_count == 0 {
        return Ok(());
    }

    let mut stderr = std::io::stderr().lock();
    render_failure_report(&classified, &mut stderr).map_err(JjGtError::Io)?;

    // Return the first Failed/Fixup as the canonical error message
    // so the caller's existing `step.fail(&format!("{e}"))` line
    // surfaces something coherent. The summary above gives the
    // full picture; this one-liner is the "exit code reason."
    for (_name, result) in &classified {
        match result {
            BookmarkResult::Failed { message, .. } | BookmarkResult::Fixup { message, .. } => {
                return Err(JjGtError::Invalid(message.clone()));
            }
            _ => continue,
        }
    }

    // Unreachable: bad_count > 0 guarantees at least one
    // Failed/Fixup above.
    Ok(())
}

/// Structured per-bookmark hook result. Public to the module so
/// `render_failure_report` can be tested without spinning up real
/// hook invocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BookmarkResult {
    Passed,
    Failed {
        message: String,
        captured: Option<String>,
    },
    Fixup {
        message: String,
        captured: Option<String>,
    },
    Cancelled,
}

/// Render the per-screen failure summary + replay each
/// Failed/Fixup bookmark's captured output. Pure function over a
/// classified result list; used by `run_pre_push_stack` after a
/// multi-bookmark run produces at least one failure, and unit-
/// tested via this surface so the shape is pinned without
/// requiring real hook subprocesses.
///
/// Writes to `out` (typically `stderr`). Returns the underlying
/// IO error if writes fail; in production that's basically never
/// (stderr to a terminal doesn't fail).
pub(crate) fn render_failure_report(
    classified: &[(String, BookmarkResult)],
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    writeln!(out)?;
    writeln!(out, "Pre-push hook results:")?;
    for (name, result) in classified {
        match result {
            BookmarkResult::Passed => writeln!(out, "  passed     {name}")?,
            BookmarkResult::Failed { .. } => writeln!(out, "  FAILED     {name}")?,
            BookmarkResult::Fixup { .. } => writeln!(out, "  AUTOFIX    {name}")?,
            BookmarkResult::Cancelled => writeln!(out, "  cancelled  {name}")?,
        }
    }
    writeln!(out)?;

    for (name, result) in classified {
        let (captured, kind) = match result {
            BookmarkResult::Failed { captured, .. } => (captured, "failure output"),
            BookmarkResult::Fixup { captured, .. } => (captured, "autofix output"),
            _ => continue,
        };
        writeln!(out, "--- {name} ({kind}) ---")?;
        if let Some(buf) = captured {
            write!(out, "{buf}")?;
            if !buf.ends_with('\n') {
                writeln!(out)?;
            }
        } else {
            writeln!(
                out,
                "(no captured output — sequential run, see live stream above)"
            )?;
        }
        writeln!(out, "--- end {name} ---")?;
        writeln!(out)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn classified(items: &[(&str, BookmarkResult)]) -> Vec<(String, BookmarkResult)> {
        items
            .iter()
            .map(|(n, r)| ((*n).into(), r.clone()))
            .collect()
    }

    fn render(items: &[(&str, BookmarkResult)]) -> String {
        let mut buf: Vec<u8> = Vec::new();
        render_failure_report(&classified(items), &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn report_summary_lists_every_bookmark_status() {
        // The summary table should list every bookmark with its
        // status word — passed / FAILED / cancelled / AUTOFIX —
        // in input order so the user can map back to their
        // -b ordering at a glance.
        let out = render(&[
            (
                "sea-559",
                BookmarkResult::Failed {
                    message: "boom".into(),
                    captured: Some("fmt diff\n".into()),
                },
            ),
            ("sea-561", BookmarkResult::Cancelled),
            ("sea-695", BookmarkResult::Passed),
        ]);
        assert!(out.contains("Pre-push hook results:"), "got:\n{out}");
        assert!(out.contains("FAILED     sea-559"), "got:\n{out}");
        assert!(out.contains("cancelled  sea-561"), "got:\n{out}");
        assert!(out.contains("passed     sea-695"), "got:\n{out}");
    }

    #[test]
    fn report_replays_captured_output_of_failed_bookmark() {
        // The whole point of this PR: actionable content (the fmt
        // diff, the clippy warning) lands right above the final
        // error so the user doesn't have to scroll.
        let out = render(&[(
            "sea-559",
            BookmarkResult::Failed {
                message: "boom".into(),
                captured: Some("Diff in foo.rs:399:\n-extra\n".into()),
            },
        )]);
        assert!(
            out.contains("--- sea-559 (failure output) ---"),
            "got:\n{out}"
        );
        assert!(out.contains("Diff in foo.rs:399:"), "got:\n{out}");
        assert!(out.contains("--- end sea-559 ---"), "got:\n{out}");
    }

    #[test]
    fn report_replays_captured_output_of_autofix_bookmark() {
        let out = render(&[(
            "sea-559",
            BookmarkResult::Fixup {
                message: "fixup".into(),
                captured: Some("autofix wrote 2 files\n".into()),
            },
        )]);
        assert!(out.contains("AUTOFIX    sea-559"), "got:\n{out}");
        assert!(
            out.contains("--- sea-559 (autofix output) ---"),
            "got:\n{out}"
        );
        assert!(out.contains("autofix wrote 2 files"), "got:\n{out}");
    }

    #[test]
    fn report_skips_replay_for_passed_and_cancelled() {
        // Passed + Cancelled bookmarks show up in the summary
        // table but NOT in the captured-output replay section —
        // they have nothing actionable.
        let out = render(&[
            (
                "sea-559",
                BookmarkResult::Failed {
                    message: "boom".into(),
                    captured: Some("fmt diff content\n".into()),
                },
            ),
            ("sea-561", BookmarkResult::Cancelled),
            ("sea-695", BookmarkResult::Passed),
        ]);
        // sea-561 and sea-695 appear in the summary header...
        assert!(out.contains("cancelled  sea-561"));
        assert!(out.contains("passed     sea-695"));
        // ...but not as a replay block header.
        assert!(
            !out.contains("--- sea-561 ("),
            "got unexpected sea-561 replay:\n{out}"
        );
        assert!(
            !out.contains("--- sea-695 ("),
            "got unexpected sea-695 replay:\n{out}"
        );
    }

    #[test]
    fn report_handles_missing_captured_output_gracefully() {
        // Sequential (`--hooks-sequential`) runs don't capture
        // because output streamed live. The replay block must
        // not panic — it just notes that the live stream was
        // upstream.
        let out = render(&[(
            "sea-559",
            BookmarkResult::Failed {
                message: "boom".into(),
                captured: None,
            },
        )]);
        assert!(out.contains("--- sea-559 (failure output) ---"));
        assert!(out.contains("(no captured output"));
    }

    #[test]
    fn report_appends_trailing_newline_to_unterminated_capture() {
        // Captured buffers may or may not end with '\n'; the
        // replay should always end the block cleanly so the
        // following separator doesn't run-on.
        let out = render(&[(
            "sea-559",
            BookmarkResult::Failed {
                message: "boom".into(),
                captured: Some("no trailing newline".into()),
            },
        )]);
        // Find the failure block and check there's a newline
        // before the "--- end" marker.
        let pos = out.find("--- end sea-559 ---").expect("end marker missing");
        let before = &out[..pos];
        assert!(
            before.ends_with('\n'),
            "expected newline before end marker, got `{}`",
            before.chars().rev().take(20).collect::<String>(),
        );
    }
}

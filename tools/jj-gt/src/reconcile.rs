//! Reconciliation between jj's view of bookmark graph state and gt's
//! tracking metadata + remote refs.
//!
//! Closes #6 (umbrella for #4 and #5). The reconciliation has two
//! independent steps that callers can run separately or together:
//!
//! - [`retrack_adjacent_diverged`]: re-issues `gt track` for local
//!   bookmarks tracked on `remote` whose jj-derived parent differs
//!   from what gt has recorded. Closes #4.
//!
//! - [`push_rebased_tips`]: `jj git push --bookmark <name>` for each
//!   bookmark in the input set. jj's push is force-with-lease by
//!   default so locally-rebased bookmarks land on the remote without
//!   tripping gt's "branch updated remotely" check, while a genuine
//!   collaborator-side race surfaces as a normal jj refusal. Closes
//!   #5.
//!
//! `submit_cmd` calls both as pre-submit reconciliation steps. The
//! `jj-gt reconcile` subcommand exposes the same shapes for manual
//! reconciliation when a previous submit was interrupted.

use std::path::Path;

use crate::error::{JjGtError, Result};
use crate::{gt, jj, stack, ui};

/// Configuration shared by the reconciliation steps.
#[derive(Debug, Clone)]
pub struct ReconcileOpts {
    pub remote: String,
    pub trunk: String,
    pub dry_run: bool,
}

/// Per-step summary aggregated by [`reconcile`]. Returned to the
/// caller so the `submit_cmd` integration can fold the numbers into
/// its own status renderer, and the standalone `jj-gt reconcile`
/// subcommand can print them in its top-level output.
#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
    pub adjacent_retracked: usize,
    pub adjacent_errors: Vec<String>,
    pub push_summary: PushSummary,
}

/// Filter the tracked-bookmark set down to "adjacent bookmarks
/// reconcile should consider re-tracking." Two exclusions:
///
/// - Trunk itself. `gt track <trunk> --parent <trunk>` errors with
///   "Cannot set parent of <trunk> to itself!" because gt's
///   tracking metadata classifies trunk as the root of every
///   stack, not a tracked branch. Including it would abort the
///   reconcile step on every submit (regression from PR-G that hit
///   in the wild).
/// - The submit-stack `skip` set. The in-stack track loop already
///   handled those; re-running here would be redundant work.
///
/// Pure function so the test suite can pin the filter without
/// spinning up a workspace.
pub fn filter_adjacent_targets(
    tracked: &std::collections::BTreeSet<String>,
    trunk: &str,
    skip: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    tracked
        .iter()
        .filter(|name| name.as_str() != trunk && !skip.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Re-track local bookmarks whose jj-derived parent differs from
/// gt's recorded parent. Closes #4.
///
/// `skip` is the set of bookmarks the caller has already handled
/// (typically the in-stack track loop that already ran during
/// `submit_cmd`). They're filtered out so we don't issue redundant
/// `gt track` calls.
///
/// Only bookmarks that gt is ALREADY tracking are considered. A
/// brand-new local bookmark unrelated to any stack — say, another
/// engineer's PR the user pulled down to review — shouldn't get
/// silently registered with gt by a reconciliation that didn't
/// mention it. First-time tracking flows through the explicit
/// `jj-gt submit` / `jj-gt track` paths instead.
///
/// Returns `(count_retracked, per_bookmark_errors)`. Per-bookmark
/// `gt track` failures are collected but don't abort the call.
pub fn retrack_adjacent_diverged(
    jj: &jj::JjCli,
    workspace_root: &Path,
    opts: &ReconcileOpts,
    candidates: &std::collections::BTreeSet<String>,
    skip: &std::collections::BTreeSet<String>,
) -> Result<(usize, Vec<String>)> {
    // Refine the caller's `candidates` against "the user has
    // actually opted into gt for this branch" — `gt log short`'s
    // enumeration. Bookmarks pulled down purely to review someone
    // else's PR shouldn't end up registered with graphite just
    // because they happened to live in the candidate pool.
    let gt_known = gt::list_tracked_branches(workspace_root).unwrap_or_else(|e| {
        tracing::warn!(
            "jj-gt: couldn't enumerate gt-tracked branches ({e}); skipping adjacent re-track"
        );
        std::collections::BTreeSet::new()
    });
    let scoped: std::collections::BTreeSet<String> =
        candidates.intersection(&gt_known).cloned().collect();
    let adjacent = filter_adjacent_targets(&scoped, &opts.trunk, skip);

    if adjacent.is_empty() {
        return Ok((0, Vec::new()));
    }

    // Lossy: we enumerated every tracked bookmark on the remote.
    // If one of them is mid-deletion (merged-PR cleanup not yet
    // exported), don't abort the whole reconcile — the user wants
    // their actual submit-stack tracked, not blocked by an
    // unrelated zombie.
    let derived = stack::derive_parents_lossy(jj, &adjacent, &opts.trunk);
    let sorted = stack::sort_for_tracking(&derived);

    let mut count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for sb in &sorted {
        let parent = sb.parent.as_branch_name(&opts.trunk);
        if opts.dry_run {
            tracing::info!("dry-run: would gt track {} --parent {}", sb.name, parent);
            count += 1;
            continue;
        }
        match gt::track(workspace_root, &sb.name, parent) {
            Ok(()) => count += 1,
            Err(e) => errors.push(format!("{} (parent: {parent}): {e}", sb.name)),
        }
    }
    Ok((count, errors))
}

/// Per-bookmark counts produced by [`push_rebased_tips`]. Each
/// bookmark falls into exactly one bucket per `jj git push`
/// classification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushSummary {
    /// `jj git push --bookmark <name>` exited 0 and jj reported
    /// "Add bookmark" — bookmark was new on the remote.
    pub added: usize,
    /// Existing remote bookmark moved (force-with-lease covered it).
    pub moved: usize,
    /// jj reported the bookmark was already in sync — no work to
    /// do. This is the signal that often hints at a stale local
    /// bookmark (user forgot `jj bookmark set -r @` after editing).
    pub already_in_sync: usize,
    /// Per-bookmark `jj git push` errors (`name: <stderr text>`).
    pub errors: Vec<String>,
}

impl PushSummary {
    /// Total number of bookmarks that produced real remote-side
    /// work (added + moved). Used to decide whether to show the
    /// "all already in sync" hint.
    pub fn newly_pushed_count(&self) -> usize {
        self.added + self.moved
    }

    /// Total number of bookmarks processed without error
    /// (newly-pushed + already-in-sync).
    pub fn processed_count(&self) -> usize {
        self.newly_pushed_count() + self.already_in_sync
    }

    /// One-line summary suitable for the `Syncing rebased
    /// bookmarks` step row. Shape changes based on the bucket
    /// counts so the message is always self-explanatory:
    ///
    ///   "2 pushed, 1 already in sync"
    ///   "3 already in sync"           // all no-ops
    ///   "1 added, 2 moved"            // no in-sync
    ///   "1 added"                     // just one new
    pub fn summary_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.added > 0 {
            parts.push(format!("{} added", self.added));
        }
        if self.moved > 0 {
            parts.push(format!("{} moved", self.moved));
        }
        if self.already_in_sync > 0 {
            parts.push(format!("{} already in sync", self.already_in_sync));
        }
        if !self.errors.is_empty() {
            parts.push(format!("{} error(s)", self.errors.len()));
        }
        if parts.is_empty() {
            "no bookmarks pushed".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

/// `jj git push --bookmark` over each entry in `bookmarks`. Closes
/// #5. Uses jj's default force-with-lease semantics so locally-
/// rebased bookmarks land on the remote without bypassing legitimate
/// collaborator-race detection.
///
/// Per-bookmark failures are non-fatal: collected into the returned
/// summary's `errors` list. The counts in `PushSummary` distinguish
/// "real work happened" (added/moved) from "already in sync" so the
/// summary row tells the user what actually changed on the remote.
pub fn push_rebased_tips(
    jj: &jj::JjCli,
    bookmarks: &[String],
    opts: &ReconcileOpts,
) -> Result<PushSummary> {
    let mut summary = PushSummary::default();
    if bookmarks.is_empty() {
        return Ok(summary);
    }
    for name in bookmarks {
        if opts.dry_run {
            tracing::info!("dry-run: would jj git push --bookmark {name}");
            // Dry-run can't classify what real jj would do
            // without actually invoking it. Count as `moved` so
            // the summary doesn't claim "already in sync" for a
            // case we don't actually know.
            summary.moved += 1;
            continue;
        }
        match jj::git_push_bookmark(jj, &opts.remote, name) {
            Ok(jj::PushOutcome::Added) => summary.added += 1,
            Ok(jj::PushOutcome::Moved) => summary.moved += 1,
            Ok(jj::PushOutcome::AlreadyInSync) => summary.already_in_sync += 1,
            Err(e) => summary.errors.push(format!("{name}: {e}")),
        }
    }
    Ok(summary)
}

/// Run the full reconciliation: adjacent re-track + push rebased
/// tips. Used by both `submit_cmd` (pre-submit step) and the
/// standalone `jj-gt reconcile` subcommand.
///
/// `stack_bookmarks` is the set the caller is operating on. Used
/// as the `skip` filter for the adjacent step — these are the
/// bookmarks the caller has already handled (typically via an
/// in-stack `gt track` loop) and shouldn't be re-tracked.
///
/// `adjacent_candidates` is the pool the adjacent step pulls
/// from. For the submit path it's `list_tracked_bookmarks_on_remote`
/// (every tracked bookmark on the remote — when the user submits a
/// stack, we also want to re-track any unrelated bookmarks that
/// drifted). For the standalone `jj-gt reconcile` it's the
/// focused active stack — reconcile shouldn't touch unrelated
/// stacks the user didn't ask about. When `candidates - skip` is
/// empty the adjacent step no-ops.
///
/// `push_bookmarks` lets the caller seed the set of bookmarks to
/// `jj git push`. For the submit path it's `stacked_sorted` (push
/// the user's submit stack). For the standalone command it's
/// empty by default — pushing arbitrary remote refs out of band is
/// outside reconciliation's scope.
pub fn reconcile(
    jj: &jj::JjCli,
    workspace_root: &Path,
    opts: &ReconcileOpts,
    stack_bookmarks: &[String],
    adjacent_candidates: &std::collections::BTreeSet<String>,
    push_bookmarks: &[String],
    verbosity: ui::Verbosity,
) -> Result<ReconcileReport> {
    let skip: std::collections::BTreeSet<String> = stack_bookmarks.iter().cloned().collect();

    let adjacent_step = ui::Step::start("Re-tracking adjacent diverged bookmarks", verbosity);
    let (adjacent_retracked, adjacent_errors) =
        match retrack_adjacent_diverged(jj, workspace_root, opts, adjacent_candidates, &skip) {
            Ok(out) => out,
            Err(e) => {
                adjacent_step.fail(&format!("{e}"), None);
                return Err(e);
            }
        };
    let adjacent_summary = match (adjacent_retracked, adjacent_errors.is_empty()) {
        (0, true) => "none diverged".to_owned(),
        (n, true) => format!("{n} re-tracked"),
        (n, false) => format!("{} re-tracked, {} error(s)", n, adjacent_errors.len()),
    };
    if adjacent_errors.is_empty() {
        adjacent_step.success(&adjacent_summary, None);
    } else {
        adjacent_step.warn(&adjacent_summary, Some(&adjacent_errors.join("\n")));
    }

    let push_step = ui::Step::start(
        &format!("Syncing rebased bookmarks to {}", opts.remote),
        verbosity,
    );
    let push_summary = match push_rebased_tips(jj, push_bookmarks, opts) {
        Ok(out) => out,
        Err(e) => {
            push_step.fail(&format!("{e}"), None);
            return Err(e);
        }
    };
    let push_line = if push_bookmarks.is_empty() {
        "no bookmarks to push".to_owned()
    } else {
        push_summary.summary_line()
    };
    if push_summary.errors.is_empty() {
        push_step.success(&push_line, None);
    } else {
        push_step.warn(&push_line, Some(&push_summary.errors.join("\n")));
    }

    // Hint when ALL bookmarks the caller asked us to push came back
    // already-in-sync — this is the "did you forget `jj bookmark
    // set <name> -r @` after amending?" trap. Skip the hint when
    // the push set was empty (nothing meaningful to say) or when
    // it was a single bookmark (the user can see the no-op for
    // themselves; the hint reads as preachy).
    if !push_bookmarks.is_empty()
        && push_bookmarks.len() > 1
        && push_summary.errors.is_empty()
        && push_summary.newly_pushed_count() == 0
        && push_summary.already_in_sync == push_bookmarks.len()
    {
        eprintln!(
            "  hint: all local bookmarks already match remote — did you forget \
             `jj bookmark set <name> -r @` after amending?"
        );
    }

    Ok(ReconcileReport {
        adjacent_retracked,
        adjacent_errors,
        push_summary,
    })
}

/// Top-level `jj-gt reconcile` subcommand entry. Resolves the local
/// stack (using the same `--all` default as `jj-gt log`) so the
/// adjacent re-track filter knows which bookmarks the user is
/// actively working on, then runs the full reconciliation.
///
/// Wrap this from `lib.rs::dispatch`; not called directly from
/// tests.
pub fn run_reconcile_subcommand(
    jj: &jj::JjCli,
    remote: String,
    trunk: Option<String>,
    push: bool,
    dry_run: bool,
    verbosity: ui::Verbosity,
) -> Result<()> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = crate::status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    // Step 0a: catch up the workspace before any subprocess that
    // would otherwise fail with "working copy is stale" — see
    // `crate::maybe_catch_up_workspace` for the rationale.
    crate::maybe_catch_up_workspace(jj, verbosity, dry_run)?;

    let opts = ReconcileOpts {
        remote,
        trunk: trunk.clone(),
        dry_run,
    };

    // Resolve the "active" stack — bookmarks on the @-ancestor
    // chain between trunk and @. Same shape `jj-gt log` uses.
    //
    // Reconcile is by definition a focused-stack operation: it
    // re-tracks parents adjacent to the active stack and (with
    // `--push`) pushes that stack to the remote. The 2026-06
    // `--all` broadening made `BookmarkArgs { all: true, ... }`
    // mean "every stack in the repo," which would silently widen
    // reconcile to unrelated stacks. The bareword default
    // (`all: false`, no `-b`/`-r`/`-c`) is the correct
    // focused-stack selector for resolve_bookmarks.
    let args = crate::cli::BookmarkArgs {
        remote: opts.remote.clone(),
        ..crate::cli::BookmarkArgs::default()
    };
    let selected = crate::select::resolve_bookmarks(jj, &args, &trunk)?;

    ui::section(&format!("Reconciling against {}", opts.remote));

    // Standalone reconcile is focused: the adjacent step's
    // candidate pool is the focused stack itself, NOT every
    // tracked bookmark on the remote. The submit path passes
    // the broader tracked pool because submit is by definition a
    // multi-stack operation.
    //
    // `stack_bookmarks` (the `skip` set inside `reconcile`) MUST
    // be empty here even though the standalone command does
    // "know" the focused stack: the standalone path has no
    // in-stack `gt track` loop preceding `reconcile`, so nothing
    // has been handled yet. Passing `selected` as both
    // `stack_bookmarks` AND `adjacent_candidates` collapses
    // `skip == candidates`, which empties the adjacent
    // re-track target set and silently no-ops the focused
    // stack's parent-metadata repair.
    let adjacent_candidates: std::collections::BTreeSet<String> =
        selected.iter().cloned().collect();

    // Push the active stack only when the user opted in. Reconcile
    // without `--push` is the "just re-track parents" shape; with
    // `--push` it also force-with-leases rebased SHAs to the
    // remote.
    let push_set: Vec<String> = if push { selected.clone() } else { Vec::new() };

    let _ = reconcile(
        jj,
        &workspace_root,
        &opts,
        &[],
        &adjacent_candidates,
        &push_set,
        verbosity,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> std::collections::BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn filter_adjacent_excludes_trunk() {
        // Regression: gt's `list_tracked_bookmarks_on_remote`
        // returns the trunk bookmark when it's tracked on the
        // remote. We must skip it — `gt track main --parent main`
        // errors with "Cannot set parent of main to itself!".
        let tracked = names(&["main", "feature-a", "feature-b"]);
        let out = filter_adjacent_targets(&tracked, "main", &names(&[]));
        assert!(
            !out.contains(&"main".to_owned()),
            "trunk should not be in adjacent set, got {out:?}",
        );
        // The non-trunk entries should be retained.
        assert!(out.contains(&"feature-a".to_owned()));
        assert!(out.contains(&"feature-b".to_owned()));
    }

    #[test]
    fn filter_adjacent_excludes_skip_set() {
        // The submit-stack bookmarks ARE in the tracked set (the
        // in-stack track loop just put them there), but reconcile
        // shouldn't re-process them.
        let tracked = names(&["main", "feature-tip", "feature-mid", "other-feature"]);
        let skip = names(&["feature-tip", "feature-mid"]);
        let out = filter_adjacent_targets(&tracked, "main", &skip);
        // Only "other-feature" — main is trunk, the others are
        // in skip.
        assert_eq!(out, vec!["other-feature".to_owned()]);
    }

    #[test]
    fn filter_adjacent_excludes_trunk_under_alternate_name() {
        // Pin that the trunk filter is name-based, not hardcoded
        // to `main`. A repo whose default branch is `master` or
        // `trunk` should still be excluded.
        let tracked = names(&["master", "feature"]);
        let out = filter_adjacent_targets(&tracked, "master", &names(&[]));
        assert_eq!(out, vec!["feature".to_owned()]);
    }

    #[test]
    fn filter_adjacent_no_overlap_returns_everything() {
        let tracked = names(&["a", "b", "c"]);
        let out = filter_adjacent_targets(&tracked, "main", &names(&[]));
        // Order is BTreeSet alphabetical (since we collect from
        // the input set's iter).
        assert_eq!(out, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    }

    #[test]
    fn filter_adjacent_empty_input_returns_empty() {
        let out = filter_adjacent_targets(&names(&[]), "main", &names(&[]));
        assert!(out.is_empty());
    }

    fn summary(added: usize, moved: usize, in_sync: usize, errs: usize) -> PushSummary {
        PushSummary {
            added,
            moved,
            already_in_sync: in_sync,
            errors: (0..errs).map(|i| format!("err{i}")).collect(),
        }
    }

    #[test]
    fn push_summary_line_all_categories() {
        let s = summary(1, 2, 3, 0);
        assert_eq!(s.summary_line(), "1 added, 2 moved, 3 already in sync");
    }

    #[test]
    fn push_summary_line_added_only() {
        let s = summary(2, 0, 0, 0);
        assert_eq!(s.summary_line(), "2 added");
    }

    #[test]
    fn push_summary_line_moved_only() {
        let s = summary(0, 3, 0, 0);
        assert_eq!(s.summary_line(), "3 moved");
    }

    #[test]
    fn push_summary_line_all_in_sync() {
        // The case that triggered this PR: every bookmark was
        // already in sync. The summary makes that obvious instead
        // of saying "1 pushed".
        let s = summary(0, 0, 3, 0);
        assert_eq!(s.summary_line(), "3 already in sync");
    }

    #[test]
    fn push_summary_line_with_errors() {
        let s = summary(1, 0, 0, 2);
        assert_eq!(s.summary_line(), "1 added, 2 error(s)");
    }

    #[test]
    fn push_summary_line_empty_fallback() {
        let s = summary(0, 0, 0, 0);
        assert_eq!(s.summary_line(), "no bookmarks pushed");
    }

    #[test]
    fn push_summary_newly_pushed_excludes_in_sync_and_errors() {
        let s = summary(2, 3, 5, 1);
        assert_eq!(s.newly_pushed_count(), 5);
        // processed_count is newly-pushed + in-sync (excludes errors).
        assert_eq!(s.processed_count(), 10);
    }
}

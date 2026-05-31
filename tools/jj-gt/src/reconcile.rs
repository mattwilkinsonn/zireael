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
    pub bookmarks_pushed: usize,
    pub push_errors: Vec<String>,
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
/// Only bookmarks already tracked on `remote` are considered — a
/// brand-new local bookmark unrelated to any stack shouldn't get
/// auto-tracked by a reconciliation that didn't mention it.
///
/// Returns `(count_retracked, per_bookmark_errors)`. Per-bookmark
/// `gt track` failures are collected but don't abort the call.
pub fn retrack_adjacent_diverged(
    jj: &jj::JjCli,
    workspace_root: &Path,
    opts: &ReconcileOpts,
    skip: &std::collections::BTreeSet<String>,
) -> Result<(usize, Vec<String>)> {
    let tracked = jj::list_tracked_bookmarks_on_remote(jj, &opts.remote).unwrap_or_default();
    let adjacent = filter_adjacent_targets(&tracked, &opts.trunk, skip);

    if adjacent.is_empty() {
        return Ok((0, Vec::new()));
    }

    let derived = stack::derive_parents(jj, &adjacent, &opts.trunk)?;
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

/// `jj git push --bookmark` over each entry in `bookmarks`. Closes
/// #5. Uses jj's default force-with-lease semantics so locally-
/// rebased bookmarks land on the remote without bypassing legitimate
/// collaborator-race detection.
///
/// Per-bookmark failures are non-fatal: collected into the returned
/// error list, the count still reflects successful pushes. Callers
/// that need fail-fast semantics can check `errors.is_empty()` after.
pub fn push_rebased_tips(
    jj: &jj::JjCli,
    bookmarks: &[String],
    opts: &ReconcileOpts,
) -> Result<(usize, Vec<String>)> {
    if bookmarks.is_empty() {
        return Ok((0, Vec::new()));
    }
    let mut pushed = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for name in bookmarks {
        if opts.dry_run {
            tracing::info!("dry-run: would jj git push --bookmark {name}");
            pushed += 1;
            continue;
        }
        match jj::git_push_bookmark(jj, &opts.remote, name) {
            Ok(()) => pushed += 1,
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Ok((pushed, errors))
}

/// Run the full reconciliation: adjacent re-track + push rebased
/// tips. Used by both `submit_cmd` (pre-submit step) and the
/// standalone `jj-gt reconcile` subcommand.
///
/// `stack_bookmarks` lets the caller seed the "already handled"
/// set for the adjacent step. For the submit path it's
/// `stacked_sorted`; for the standalone `jj-gt reconcile` it's
/// empty (process every tracked bookmark).
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
    push_bookmarks: &[String],
    verbosity: ui::Verbosity,
) -> Result<ReconcileReport> {
    let skip: std::collections::BTreeSet<String> = stack_bookmarks.iter().cloned().collect();

    let adjacent_step = ui::Step::start("Re-tracking adjacent diverged bookmarks", verbosity);
    let (adjacent_retracked, adjacent_errors) =
        match retrack_adjacent_diverged(jj, workspace_root, opts, &skip) {
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
    let (bookmarks_pushed, push_errors) = match push_rebased_tips(jj, push_bookmarks, opts) {
        Ok(out) => out,
        Err(e) => {
            push_step.fail(&format!("{e}"), None);
            return Err(e);
        }
    };
    let push_summary = match (bookmarks_pushed, push_errors.is_empty()) {
        (0, true) if push_bookmarks.is_empty() => "no bookmarks to push".to_owned(),
        (0, true) => "all bookmarks already in sync".to_owned(),
        (n, true) => format!("{n} pushed"),
        (n, false) => format!("{n} pushed, {} error(s)", push_errors.len()),
    };
    if push_errors.is_empty() {
        push_step.success(&push_summary, None);
    } else {
        push_step.warn(&push_summary, Some(&push_errors.join("\n")));
    }

    Ok(ReconcileReport {
        adjacent_retracked,
        adjacent_errors,
        bookmarks_pushed,
        push_errors,
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

    let opts = ReconcileOpts {
        remote,
        trunk: trunk.clone(),
        dry_run,
    };

    // Resolve the "active" stack — bookmarks on the @-ancestor
    // chain between trunk and @. Same shape `jj-gt log` uses.
    let args = crate::cli::BookmarkArgs {
        all: true,
        remote: opts.remote.clone(),
        ..crate::cli::BookmarkArgs::default()
    };
    let selected = crate::select::resolve_bookmarks(jj, &args, &trunk)?;

    ui::section(&format!("Reconciling against {}", opts.remote));

    // Push the active stack only when the user opted in. Reconcile
    // without `--push` is the "just re-track parents" shape; with
    // `--push` it also force-with-leases rebased SHAs to the
    // remote.
    let push_set: Vec<String> = if push { selected.clone() } else { Vec::new() };

    let _ = reconcile(jj, &workspace_root, &opts, &selected, &push_set, verbosity)?;
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
}

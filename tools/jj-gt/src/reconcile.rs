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
    let adjacent: Vec<String> = tracked
        .into_iter()
        .filter(|name| !skip.contains(name))
        .collect();

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

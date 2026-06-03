//! Library entrypoint for the `jj-gt` binary.
//!
//! Exposes the same surface as a library so downstream tools could
//! depend on jj-gt's stack derivation + cleanup classifier without
//! shelling out (mirror of how jj-gt itself depends on `jj_hooks`).

pub mod cleanup;
pub mod cli;
pub mod completions;
pub mod error;
pub mod gh;
pub mod gt;
pub mod hooks;
pub mod init;
pub mod jj;
pub mod lock;
pub mod progress;
pub mod reconcile;
pub mod restack;
pub mod select;
pub mod stack;
pub mod status;
pub mod ui;

// Re-export the runner enum so downstream consumers can construct a
// HookOpts without taking a transitive `jj_hooks` dep.
pub use jj_hooks::runner::Runner;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::error::JjGtError;
use crate::jj::JjCli;

/// Parse CLI args, dispatch to a subcommand, and return the process
/// exit code. `bin/jj-gt` is a trivial wrapper around this function.
pub fn run() -> ExitCode {
    // Handle dynamic completion requests *before* anything else.
    use clap::CommandFactory;
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level)),
        )
        .with_target(false)
        .without_time()
        .try_init();

    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("jj-gt: {e}");
            ExitCode::from(1)
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode, JjGtError> {
    let jj = JjCli::new(std::env::current_dir()?);
    let verbosity = ui::Verbosity::from_flag(cli.verbose);

    match cli.command {
        Command::Submit {
            bookmarks,
            submit,
            trunk,
            no_export,
            no_restore_cwc,
            no_hooks,
            hooks_tip_only,
            hooks_sequential,
            hk_runner,
        } => submit_cmd(
            &jj,
            bookmarks,
            submit,
            trunk,
            no_export,
            no_restore_cwc,
            no_hooks,
            hooks_tip_only,
            hooks_sequential,
            hk_runner,
            verbosity,
        ),

        Command::Track {
            bookmarks,
            trunk,
            no_export,
            parent,
            dry_run,
        } => track_cmd(&jj, bookmarks, trunk, no_export, parent, dry_run, verbosity),

        Command::Fetch {
            remote,
            trunk,
            no_backfill,
            no_rebase,
            no_gtmq_prune,
            gtmq_prefix,
            auto,
            no_export,
            dry_run,
        } => fetch_cmd(
            &jj,
            remote,
            trunk,
            no_backfill,
            no_rebase,
            no_gtmq_prune,
            gtmq_prefix,
            auto,
            no_export,
            dry_run,
            verbosity,
        ),

        Command::Status {
            bookmarks,
            trunk,
            json,
        } => status_cmd(&jj, bookmarks, trunk, json, verbosity),

        Command::Log { trunk } => log_cmd(&jj, trunk, verbosity),

        Command::Reconcile {
            remote,
            trunk,
            push,
            dry_run,
        } => {
            reconcile::run_reconcile_subcommand(&jj, remote, trunk, push, dry_run, verbosity)?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Restack {
            bookmark,
            trunk,
            remote,
            stop_on_conflict,
            dry_run,
        } => restack_cmd(
            &jj,
            bookmark,
            trunk,
            remote,
            stop_on_conflict,
            dry_run,
            verbosity,
        ),

        Command::Init { print_only } => {
            init::print_setup_reminders();
            if !print_only {
                let mut prompter = init::InteractivePrompter;
                let plan = init::plan(&mut prompter)?;
                let outcome = init::apply(&plan, None)?;
                let jjui = outcome.jjui_actions_added;
                if jjui.added_submit
                    || jjui.added_submit_selected
                    || jjui.added_fetch
                    || jjui.added_track
                    || jjui.added_track_selected
                    || jjui.added_reconcile
                    || jjui.added_restack
                    || jjui.added_binding_submit
                    || jjui.added_binding_submit_selected
                    || jjui.added_binding_fetch
                    || jjui.added_binding_track
                    || jjui.added_binding_track_selected
                    || jjui.added_binding_reconcile
                    || jjui.added_binding_restack
                {
                    eprintln!("jj-gt: merged jjui actions/bindings into jjui config");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Completions { shell } => completions_cmd(shell),
    }
}

/// Step 0 for `fetch` / `submit` / `restack` / `reconcile`: run
/// `jj workspace update-stale` so the current workspace catches up
/// with any op-log moves made by sibling workspaces sharing the
/// same `.jj/`.
///
/// Surfaces the outcome as a structured step so the user can see
/// when `@` moved out from under them (silently swallowing that
/// would erase the "another workspace mutated something" signal
/// jj's default stale-warning preserves).
///
/// Skipped when `JJ_GT_SKIP_UPDATE_STALE=1` is set — escape hatch
/// for the rare debug case where you specifically want jj's
/// staleness error to fire.
///
/// Skipped when `dry_run` is true. `jj workspace update-stale`
/// is a mutation (it moves `@` from the stale snapshot to the
/// new one), and dry-run callers (`jj-gt submit --dry-run`,
/// `jj-gt fetch --dry-run`, `jj-gt restack --dry-run`) promise
/// not to mutate state. The trade-off: a dry-run run on a stale
/// workspace will hit jj's "working copy is stale" error from
/// the first subsequent `jj` invocation — but that's the
/// correct surface for "your workspace needs a real run to
/// reconcile," and the alternative (silently mutating during
/// dry-run) breaks the dry-run contract more deeply than the
/// error message is worth.
/// What [`maybe_catch_up_workspace`] did on a given call. Returned
/// so callers (and tests) can tell whether the helper actually
/// ran `jj workspace update-stale` or skipped it for one of the
/// documented reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpOutcome {
    /// `dry_run` was true → no jj invocation happened. Skipped
    /// to preserve the dry-run contract.
    SkippedDryRun,
    /// `JJ_GT_SKIP_UPDATE_STALE=1` was set → no jj invocation
    /// happened. Skipped to preserve the debug escape hatch.
    SkippedEnvVar,
    /// `jj workspace update-stale` ran and reported the
    /// workspace was already current.
    AlreadyCurrent,
    /// `jj workspace update-stale` ran and `@` moved from
    /// `from` to `to`. Sibling workspace advanced the op log.
    Updated {
        from_change_id: String,
        to_change_id: String,
    },
    /// `jj workspace update-stale` ran but the before/after
    /// probe couldn't read `@`. We can't verify whether
    /// anything moved.
    CouldNotVerify,
    /// `jj workspace update-stale` errored. Logged as a warn
    /// step; the caller treats this as non-fatal.
    UpdateStaleFailed { message: String },
}

pub fn maybe_catch_up_workspace(
    jj: &JjCli,
    verbosity: ui::Verbosity,
    dry_run: bool,
) -> Result<CatchUpOutcome, JjGtError> {
    // Honor the dry-run contract first. The update-stale call
    // would otherwise advance `@` from one change to another,
    // which is a mutation and dry-run callers explicitly promise
    // not to make. Surface a short skip line so the user sees
    // we deliberately bypassed catch-up.
    if dry_run {
        ui::Step::start(
            "Catching up workspace (jj workspace update-stale)",
            verbosity,
        )
        .skip(
            "skipped (--dry-run mutates nothing; run without --dry-run if `@` is stale)",
            None,
        );
        return Ok(CatchUpOutcome::SkippedDryRun);
    }
    // Honor the escape hatch BEFORE rendering the step. Without
    // this early-return the user gets a "ran update-stale →
    // already current" line in the per-step log even though we
    // never shelled out, which is confusing when they set the
    // var to debug a staleness interaction. The same env-var
    // check inside `jj::ensure_workspace_current` is kept as a
    // defense-in-depth for direct callers of the helper.
    if std::env::var("JJ_GT_SKIP_UPDATE_STALE").as_deref() == Ok("1") {
        return Ok(CatchUpOutcome::SkippedEnvVar);
    }
    let step = ui::Step::start(
        "Catching up workspace (jj workspace update-stale)",
        verbosity,
    );
    match jj::ensure_workspace_current(jj) {
        Ok(jj::UpdateStaleOutcome::NotStale) => {
            step.skip("already current", None);
            Ok(CatchUpOutcome::AlreadyCurrent)
        }
        Ok(jj::UpdateStaleOutcome::Updated {
            from_change_id,
            to_change_id,
        }) => {
            // Trim to short ids for the log line so the message
            // doesn't dominate the terminal width.
            let short = |id: &str| -> String { id.chars().take(12).collect() };
            step.success(
                &format!(
                    "@ moved from {} to {} (sibling workspace advanced the op log)",
                    short(&from_change_id),
                    short(&to_change_id),
                ),
                None,
            );
            Ok(CatchUpOutcome::Updated {
                from_change_id,
                to_change_id,
            })
        }
        Ok(jj::UpdateStaleOutcome::CouldNotVerify) => {
            // `update-stale` succeeded but the before/after
            // change-id probe couldn't read `@`. Surface as a
            // warning rather than "already current" — the latter
            // would lie about us having verified the workspace
            // state. The next jj command will surface a clearer
            // error if the workspace is still stale; this is
            // just a "we tried, the signal was ambiguous" log.
            step.warn(
                "update-stale ran but couldn't verify @ moved (probe failed)",
                None,
            );
            Ok(CatchUpOutcome::CouldNotVerify)
        }
        Err(e) => {
            // Don't hard-error the run when update-stale itself
            // misbehaves — the next jj command will surface a
            // clearer error if the workspace is still actually
            // stale. Log a warning so the user knows we tried.
            let message = format!("{e}");
            step.warn(&format!("update-stale failed: {message}"), None);
            Ok(CatchUpOutcome::UpdateStaleFailed { message })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_cmd(
    jj: &JjCli,
    bookmarks: cli::BookmarkArgs,
    submit: cli::SubmitArgs,
    trunk: Option<String>,
    no_export: bool,
    no_restore_cwc: bool,
    no_hooks: bool,
    hooks_tip_only: bool,
    hooks_sequential: bool,
    hk_runner: Option<cli::RunnerArg>,
    verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    // Step 0a: catch up the workspace before any subprocess that
    // would otherwise fail with "working copy is stale" — see
    // `maybe_catch_up_workspace` for the rationale.
    maybe_catch_up_workspace(jj, verbosity, submit.dry_run)?;

    // Resolve bookmark selection + derive parents up front (these
    // are fast, no subprocess noise to report). Errors here abort
    // before any progress output appears.
    let selected = select::resolve_bookmarks(jj, &bookmarks, &trunk)?;
    if selected.is_empty() {
        return Err(JjGtError::NoBookmarksSelected);
    }
    // Expand each tip's ancestor chain so `gt track <child> --parent
    // <parent>` doesn't error on an untracked intermediate. See
    // `select::expand_ancestors_for_submit` for the rationale.
    let expanded = select::expand_ancestors_for_submit(jj, &selected, &trunk)?;
    let stacked = stack::derive_parents(jj, &expanded, &trunk)?;
    // Partition into independent stacks. One tip → one stack; N
    // unrelated tips → N stacks, each fully expanded back to trunk.
    // Each partition is bottom→top sorted ready for `gt track`.
    let partitions = stack::partition_stacks(&stacked)?;
    if partitions.is_empty() {
        return Err(JjGtError::NoBookmarksSelected);
    }
    // Flat union of every bookmark across all partitions — used for
    // the shared pre-push hook batch and the reconcile step (both
    // are workspace-global concerns, not per-stack).
    let all_sorted: Vec<stack::StackedBookmark> =
        partitions.iter().flat_map(|p| p.iter().cloned()).collect();
    let stack_count = partitions.len();
    let tips: Vec<String> = partitions
        .iter()
        .filter_map(|p| p.last().map(|sb| sb.name.clone()))
        .collect();

    if stack_count == 1 {
        ui::section(&format!(
            "Submitting stack to {} (tip: {})",
            bookmarks.remote, tips[0]
        ));
    } else {
        ui::section(&format!(
            "Submitting {} independent stacks to {} (tips: {})",
            stack_count,
            bookmarks.remote,
            tips.join(", "),
        ));
    }

    // 0. Shelter any pending working-copy edits behind a fresh
    // empty `@` before we start mutating refs. Same hazard model as
    // fetch (issue #1): the upcoming `jj git export` + downstream
    // `gt submit --stack` can race with the user's concurrent jj
    // operations in another shell, and a pending working-copy edit
    // is the easiest thing to silently lose. `jj new @` snapshots
    // those edits into the (now-frozen) old `@` and leaves us
    // operating on a clean empty `@` above them.
    //
    // Skip when `@` is already empty (nothing to shelter) and in
    // dry-run mode (dry-run promises zero workspace mutations — a
    // `jj new @` would create a fresh change-id even if it's
    // semantically harmless, breaking that promise and surprising
    // anyone using `--dry-run` to preview state).
    if !submit.dry_run && jj::has_uncommitted_changes(jj)? {
        let step = ui::Step::start("Sheltering uncommitted edits (jj new @)", verbosity);
        match jj::shelter_uncommitted_edits(jj) {
            Ok(()) => step.success("old @ now holds your edits as a real change", None),
            Err(e) => {
                step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
    }

    // 1. Export jj bookmarks → git refs (idempotent, shared).
    if !no_export {
        let step = ui::Step::start("Exporting jj bookmarks to git refs", verbosity);
        match jj::git_export(jj) {
            Ok(()) => step.success("done", None),
            Err(e) => {
                step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
    }

    // 2. Hook gate across ALL bookmarks in all partitions. Sharing
    // the batch keeps the parallel runner's worktree pool warm
    // across the whole stack set rather than spinning fresh
    // worktrees per partition; correctness is unaffected because
    // each BookmarkUpdate is independent.
    //
    // `hooks_tip_only` collapses to one run per stack tip (the
    // ancestor merge-base is each stack's trunk-rooted ancestor).
    // Otherwise per-bookmark BookmarkUpdates fan out.
    if !no_hooks {
        let trunk_commit = jj::resolve_commit_id(jj, &trunk)?;
        let opts = hooks::HookOpts {
            runner_override: hk_runner.map(Into::into),
        };

        if hooks_tip_only {
            // One run per stack tip (trunk..tip range each), still
            // captures intermediate-commit failures when there's
            // only one bookmark per stack.
            for partition in &partitions {
                let Some(tip_sb) = partition.last() else {
                    continue;
                };
                let tip_commit = jj::resolve_commit_id(jj, &tip_sb.name)?;
                let label = format!(
                    "Running pre-push hooks against {trunk}..{} (tip-only)",
                    tip_sb.name
                );
                let step = ui::Step::start(&label, verbosity);
                match hooks::run_pre_push(
                    jj,
                    &workspace_root,
                    &bookmarks.remote,
                    &tip_sb.name,
                    &trunk_commit,
                    &tip_commit,
                    &opts,
                ) {
                    Ok(()) => step.success("clean", None),
                    Err(e) => {
                        step.fail(&format!("{e}"), None);
                        return Err(e);
                    }
                }
            }
        } else if all_sorted.len() == 1 {
            // Single bookmark, single stack — use the unbatched
            // path so the user sees the runner's live progress
            // bar (the batch API forces capture, which is the
            // right trade for N>1 but unnecessary for N=1).
            let tip_sb = &all_sorted[0];
            let tip_commit = jj::resolve_commit_id(jj, &tip_sb.name)?;
            let label = format!("Running pre-push hooks against {trunk}..{}", tip_sb.name);
            let step = ui::Step::start(&label, verbosity);
            match hooks::run_pre_push(
                jj,
                &workspace_root,
                &bookmarks.remote,
                &tip_sb.name,
                &trunk_commit,
                &tip_commit,
                &opts,
            ) {
                Ok(()) => step.success("clean", None),
                Err(e) => {
                    step.fail(&format!("{e}"), None);
                    return Err(e);
                }
            }
        } else {
            // Per-bookmark gate, grouped by partition so the
            // fail-fast cancellation token is scoped to each
            // independent stack (sibling failures inside stack A
            // don't cancel stack B).
            let mut partition_tips: Vec<Vec<(String, String)>> =
                Vec::with_capacity(partitions.len());
            let mut total_bookmarks = 0usize;
            for partition in &partitions {
                let mut this_partition: Vec<(String, String)> = Vec::with_capacity(partition.len());
                for sb in partition {
                    let tip_oid = jj::resolve_commit_id(jj, &sb.name)?;
                    this_partition.push((sb.name.clone(), tip_oid));
                }
                total_bookmarks += this_partition.len();
                partition_tips.push(this_partition);
            }
            let parallel = !hooks_sequential;
            let label = if parallel {
                format!(
                    "Running pre-push hooks per-bookmark (parallel, {total_bookmarks} bookmarks across {stack_count} stack(s))",
                )
            } else {
                format!(
                    "Running pre-push hooks per-bookmark (sequential, {total_bookmarks} bookmarks across {stack_count} stack(s))",
                )
            };
            let step = ui::Step::start(&label, verbosity);
            match hooks::run_pre_push_stack(
                jj,
                &workspace_root,
                &bookmarks.remote,
                &trunk_commit,
                &partition_tips,
                parallel,
                &opts,
            ) {
                Ok(()) => step.success("clean", None),
                Err(e) => {
                    step.fail(&format!("{e}"), None);
                    return Err(e);
                }
            }
        }
    }

    // 3. gt track per (bookmark, parent), per partition. Bottom→top
    // ordering within each partition; partitions themselves are
    // independent so ordering between them doesn't matter.
    for (idx, partition) in partitions.iter().enumerate() {
        if stack_count > 1 {
            ui::section(&format!(
                "Stack {}/{stack_count}: tracking {} bookmark(s)",
                idx + 1,
                partition.len(),
            ));
        }
        for sb in partition {
            let parent = sb.parent.as_branch_name(&trunk);
            let step = ui::Step::start(
                &format!("Tracking {} (parent: {})", sb.name, parent),
                verbosity,
            );
            if submit.dry_run {
                step.skip("dry-run", None);
            } else {
                match gt::track(&workspace_root, &sb.name, parent) {
                    Ok(()) => step.success("", None),
                    Err(e) => {
                        step.fail(&format!("{e}"), None);
                        return Err(e);
                    }
                }
            }
        }
    }

    // 4. Record @ for restoration (shared across all submits — gt's
    // git-push triggers a ref-import that can shift @ once, not N
    // times).
    let saved_change_id = if no_restore_cwc {
        None
    } else {
        Some(jj::current_change_id(jj)?)
    };

    // 4.5. Reconcile gt's tracking metadata + push rebased SHAs
    // across ALL stacks in one pass. Closes #4 + #5. Workspace-
    // global by design (adjacent re-track is "every tracked
    // bookmark not in our skip set"); running it once per stack
    // would re-track the same adjacents N times.
    if !submit.dry_run {
        let stack_names: Vec<String> = all_sorted.iter().map(|sb| sb.name.clone()).collect();
        let reconcile_opts = reconcile::ReconcileOpts {
            remote: bookmarks.remote.clone(),
            trunk: trunk.clone(),
            dry_run: submit.dry_run,
        };
        // Submit-path: the adjacent step's candidate pool is every
        // tracked bookmark on the remote. Unrelated stacks whose
        // gt metadata drifted since the last submit get re-tracked
        // alongside the submit set. This matches the pre-2026-06
        // behavior of standalone reconcile (which now scopes to
        // the focused stack); for the submit flow we keep the
        // broad sweep because submit is by definition a
        // multi-stack operation.
        let adjacent_candidates =
            jj::list_tracked_bookmarks_on_remote(jj, &bookmarks.remote).unwrap_or_default();
        if let Err(e) = reconcile::reconcile(
            jj,
            &workspace_root,
            &reconcile_opts,
            &stack_names,
            &adjacent_candidates,
            &stack_names,
            verbosity,
        ) {
            tracing::warn!("jj-gt: reconcile step failed: {e}");
        }
    }

    // 5. gt submit --stack --branch <tip>, once per partition.
    // Aborts on the first failure — partitions already submitted
    // stay submitted (gt has no rollback; we don't either).
    for (idx, partition) in partitions.iter().enumerate() {
        let Some(tip_sb) = partition.last() else {
            continue;
        };
        let tip = &tip_sb.name;
        let label = if stack_count == 1 {
            format!("Submitting stack via `gt submit --stack --branch {tip}`")
        } else {
            format!(
                "Submitting stack {}/{stack_count} via `gt submit --stack --branch {tip}`",
                idx + 1
            )
        };
        let submit_step = ui::Step::start(&label, verbosity);
        let argv = gt::build_submit_argv(tip, &submit);
        if submit.dry_run {
            submit_step.skip(&format!("dry-run: gt {}", argv.join(" ")), None);
        } else {
            match gt::submit(&workspace_root, &argv) {
                Ok(()) => submit_step.success("PRs created/updated", None),
                Err(e) => {
                    submit_step.fail(&format!("{e}"), None);
                    return Err(e);
                }
            }
        }
    }

    // 6. Track each pushed bookmark with jj so the next op doesn't
    // import the remote ref as untracked. See
    // jj::track_bookmark_on_remote for the rationale. Single pass
    // across all stacks — the operation is workspace-global.
    if !submit.dry_run {
        let track_step = ui::Step::start("Tracking pushed bookmarks with jj", verbosity);
        let already_tracked =
            jj::list_tracked_bookmarks_on_remote(jj, &bookmarks.remote).unwrap_or_default();
        let mut tracked = 0usize;
        let mut skipped = 0usize;
        let mut errors: Vec<String> = Vec::new();
        for sb in &all_sorted {
            if already_tracked.contains(&sb.name) {
                skipped += 1;
                continue;
            }
            match jj::track_bookmark_on_remote(jj, &sb.name, &bookmarks.remote) {
                Ok(()) => tracked += 1,
                Err(e) => errors.push(format!("{}@{}: {e}", sb.name, bookmarks.remote)),
            }
        }
        let summary = match (tracked, skipped, errors.is_empty()) {
            (0, _, true) => "all already tracked".to_owned(),
            (n, 0, true) => format!("{n} newly tracked"),
            (n, m, true) => format!("{n} newly tracked, {m} already tracked"),
            (_, _, false) => format!("{} error(s)", errors.len()),
        };
        if errors.is_empty() {
            track_step.success(&summary, None);
        } else {
            track_step.warn(&summary, Some(&errors.join("\n")));
        }
    }

    // 7. Restore @, but only if it actually moved. The restore
    // exists because gt's git-push triggers a jj ref-import that
    // sometimes shifts @; on re-submits the import is a no-op and
    // `jj edit` would just print `Nothing changed.` — pure noise.
    if let Some(saved) = saved_change_id
        && !submit.dry_run
    {
        let current = jj::current_change_id(jj).ok();
        if current.as_deref() != Some(saved.as_str()) {
            let step = ui::Step::start("Restoring working-copy @", verbosity);
            match jj::edit_change(jj, &saved) {
                Ok(()) => step.success(&format!("restored to {saved}"), None),
                Err(e) => step.warn(&format!("could not restore @: {e}"), None),
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn track_cmd(
    jj: &JjCli,
    bookmarks: cli::BookmarkArgs,
    trunk: Option<String>,
    no_export: bool,
    parent_override: Option<String>,
    dry_run: bool,
    verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    let selected = select::resolve_bookmarks(jj, &bookmarks, &trunk)?;
    if selected.is_empty() {
        return Err(JjGtError::NoBookmarksSelected);
    }

    ui::section(&format!("Tracking {} bookmark(s) with gt", selected.len()));

    if !no_export {
        let step = ui::Step::start("Exporting jj bookmarks to git refs", verbosity);
        match jj::git_export(jj) {
            Ok(()) => step.success("done", None),
            Err(e) => {
                step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
    }

    let pairs: Vec<(String, String)> = if let Some(p) = parent_override {
        selected.iter().map(|b| (b.clone(), p.clone())).collect()
    } else {
        let stacked = stack::derive_parents(jj, &selected, &trunk)?;
        // Same ordering requirement as the submit path — gt rejects
        // tracking a child whose parent isn't tracked yet.
        let sorted = stack::sort_for_tracking(&stacked);
        sorted
            .into_iter()
            .map(|sb| {
                let parent = sb.parent.as_branch_name(&trunk).to_owned();
                (sb.name, parent)
            })
            .collect()
    };

    for (bookmark, parent) in pairs {
        let step = ui::Step::start(
            &format!("Tracking {bookmark} (parent: {parent})"),
            verbosity,
        );
        if dry_run {
            step.skip("dry-run", None);
        } else {
            match gt::track(&workspace_root, &bookmark, &parent) {
                Ok(()) => step.success("", None),
                Err(e) => {
                    step.fail(&format!("{e}"), None);
                    return Err(e);
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn fetch_cmd(
    jj: &JjCli,
    remote: String,
    trunk: Option<String>,
    no_backfill: bool,
    no_rebase: bool,
    no_gtmq_prune: bool,
    gtmq_prefix: Vec<String>,
    auto: bool,
    no_export: bool,
    dry_run: bool,
    verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    let prefixes = if gtmq_prefix.is_empty() {
        cleanup::default_gtmq_prefixes_owned()
    } else {
        gtmq_prefix
    };

    let opts = cleanup::FetchOpts {
        remote: remote.clone(),
        trunk,
        no_backfill,
        no_rebase,
        no_gtmq_prune,
        gtmq_prefixes: prefixes,
        auto,
        dry_run,
        no_export,
    };

    ui::section(&format!("Fetching + cleaning up against {remote}"));

    // Step 0a: catch up the workspace before any subprocess that
    // would otherwise fail with "working copy is stale" — see
    // `maybe_catch_up_workspace` for the rationale.
    maybe_catch_up_workspace(jj, verbosity, dry_run)?;

    let actions = cleanup::run_fetch(jj, &workspace_root, &opts, verbosity)?;

    if !actions.is_empty() {
        ui::section("Per-bookmark actions");
        for (bookmark, action) in &actions {
            let (status, msg) = action_to_row(&bookmark.name, action);
            ui::action_row(&bookmark.name, status, &msg);
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn restack_cmd(
    jj: &JjCli,
    bookmark: Option<String>,
    trunk: Option<String>,
    remote: String,
    stop_on_conflict: bool,
    dry_run: bool,
    verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    // Step 0a: catch up the workspace before any subprocess that
    // would otherwise fail with "working copy is stale" — see
    // `maybe_catch_up_workspace` for the rationale.
    maybe_catch_up_workspace(jj, verbosity, dry_run)?;

    // The actual rebase destination is `<trunk>@<remote>` so we
    // catch any post-fetch advances. jj's `<name>@<remote>` syntax
    // does NOT fall back to a local bookmark when the remote
    // tracking ref doesn't exist — so probe first, then fall back
    // to plain `<trunk>` when the remote-tracking form fails to
    // resolve. Covers the case where the user passes `--trunk
    // some-local-bookmark` for a bookmark that exists locally but
    // not on the remote.
    let remote_form = format!("{trunk}@{remote}");
    let destination = if jj::resolve_commit_id(jj, &remote_form).is_ok() {
        remote_form
    } else {
        tracing::info!(
            "jj-gt restack: `{remote_form}` did not resolve, falling back to local `{trunk}`"
        );
        trunk.clone()
    };

    let opts = restack::RestackOpts {
        trunk_destination: destination.clone(),
        stop_on_conflict,
        dry_run,
        only_bookmark: bookmark,
    };

    if let Some(only) = opts.only_bookmark.as_deref() {
        ui::section(&format!(
            "Restacking stack containing `{only}` onto {destination}{}",
            if dry_run { " (dry-run)" } else { "" }
        ));
    } else {
        ui::section(&format!(
            "Restacking all local stacks onto {destination}{}",
            if dry_run { " (dry-run)" } else { "" }
        ));
    }

    let results = restack::run_restack(jj, &opts)?;

    if results.is_empty() {
        ui::section("Nothing to restack");
        eprintln!(
            "  no local bookmarks found above {destination} — already in sync, or no bookmarks authored by current user"
        );
        return Ok(ExitCode::SUCCESS);
    }

    ui::section("Per-stack restack");
    for result in &results {
        let (status, message) = restack::outcome_to_row(&result.outcome);
        ui::action_row(&result.tip, status, &message);
    }

    if restack::any_unresolved(&results) {
        // One or more stacks ended conflicted / failed — exit non-
        // zero so scripts (and the jj-gt-restack jjui action's
        // refresh trigger) can react.
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Map a [`cleanup::CleanupAction`] to the (status, message) tuple
/// the [`ui::action_row`] renderer takes. The match cases mirror
/// what the old `format_action` produced, just split into a
/// glyph-color signal + a text payload so the renderer can paint
/// each row consistently.
fn action_to_row(
    bookmark_name: &str,
    action: &cleanup::CleanupAction,
) -> (ui::ActionStatus, String) {
    use cleanup::CleanupAction;
    match action {
        CleanupAction::GtSyncDeleted => (ui::ActionStatus::Ok, "deleted by gt sync".into()),
        CleanupAction::GtmqPruned { had_pr: Some(n) } => (
            ui::ActionStatus::Ok,
            format!("gtmq pruned (PR #{n} closed)"),
        ),
        CleanupAction::GtmqPruned { had_pr: None } => {
            (ui::ActionStatus::Ok, "gtmq pruned (no PR)".into())
        }
        CleanupAction::GtmqLeftAlone { pr } => (
            ui::ActionStatus::Skipped,
            format!("gtmq left alone (PR #{pr} open)"),
        ),
        CleanupAction::OrphanDeleted {
            pr,
            merge_commit_id,
        } => (
            ui::ActionStatus::Ok,
            format!(
                "orphan deleted (PR #{pr} merged as {})",
                &merge_commit_id[..merge_commit_id.len().min(12)]
            ),
        ),
        CleanupAction::OrphanSkipped { pr, .. } => (
            ui::ActionStatus::Skipped,
            format!("orphan skipped (PR #{pr})"),
        ),
        CleanupAction::SkippedDueToDrift {
            pr,
            local_sha,
            pushed_sha,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "drift detected — skipped (PR #{pr}, local {}, pushed {})",
                &local_sha[..local_sha.len().min(12)],
                &pushed_sha[..pushed_sha.len().min(12)],
            ),
        ),
        CleanupAction::Rebased { onto, prev_parent } => (
            ui::ActionStatus::Ok,
            format!(
                "rebased onto {onto} (parent `{prev_parent}` was removed earlier in fetch/sync cleanup)"
            ),
        ),
        CleanupAction::RebaseDeferredForConflict {
            onto,
            prev_parent,
            message,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "rebase onto {onto} would have conflicted (parent `{prev_parent}` was removed earlier in fetch/sync cleanup) — {message}; deferred to `jj-gt restack` (run manually when you're ready to resolve)"
            ),
        ),
        CleanupAction::RebaseConflicted {
            onto,
            prev_parent,
            message,
        } => (
            ui::ActionStatus::Error,
            format!(
                "rebase onto {onto} produced conflicts (parent `{prev_parent}` was removed earlier in fetch/sync cleanup) — {message}; run `jj resolve` to fix"
            ),
        ),
        CleanupAction::BookmarkConflicted { prev_parent } => (
            ui::ActionStatus::Error,
            format!(
                "bookmark target is conflicted (multiple lineages disagree) — would have been orphan-rebased because parent `{prev_parent}` was removed; resolve with `jj bookmark set {bookmark_name} -r <commit>` and re-run"
            ),
        ),
        CleanupAction::RebasedAfterParentMoved {
            parent_bookmark,
            pre_commit,
            post_commit,
        } => (
            ui::ActionStatus::Ok,
            format!(
                "re-anchored onto `{parent_bookmark}`'s new tip (parent moved {} → {} on remote; Graphite pre-merge rebase or force-push)",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RebaseAfterParentMovedDeferred {
            parent_bookmark,
            pre_commit,
            post_commit,
            message,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "re-anchor onto `{parent_bookmark}`'s new tip would have conflicted (parent moved {} → {} on remote) — {message}; deferred to `jj-gt restack` (run manually when you're ready to resolve)",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RebaseAfterParentMovedConflicted {
            parent_bookmark,
            pre_commit,
            post_commit,
            message,
        } => (
            ui::ActionStatus::Error,
            format!(
                "re-anchor onto `{parent_bookmark}`'s new tip produced conflicts (parent moved {} → {} on remote) — {message}; run `jj resolve` to fix",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RebasedAfterRemoteMoved {
            bookmark,
            pre_commit,
            post_commit,
        } => (
            ui::ActionStatus::Ok,
            format!(
                "re-anchored local commits on `{bookmark}` onto new remote tip ({} → {} on remote)",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RebaseAfterRemoteMovedDeferred {
            bookmark,
            pre_commit,
            post_commit,
            message,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "re-anchor of local commits on `{bookmark}` onto new remote tip would have conflicted ({} → {} on remote) — {message}; deferred to `jj-gt restack`",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RebaseAfterRemoteMovedConflicted {
            bookmark,
            pre_commit,
            post_commit,
            message,
        } => (
            ui::ActionStatus::Error,
            format!(
                "re-anchor of local commits on `{bookmark}` onto new remote tip produced conflicts ({} → {} on remote) — {message}; run `jj resolve` to fix",
                &pre_commit[..pre_commit.len().min(12)],
                &post_commit[..post_commit.len().min(12)],
            ),
        ),
        CleanupAction::RestoredAfterRewind { pre, post } => (
            ui::ActionStatus::Warn,
            format!(
                "restored after gt sync silently rewound (pre {}, gt-sync moved to {})",
                &pre[..pre.len().min(12)],
                &post[..post.len().min(12)],
            ),
        ),
        CleanupAction::DivergedFromRemote { pre, post } => (
            ui::ActionStatus::Warn,
            format!(
                "DIVERGED — local {} restored; remote moved to {}; reconcile manually",
                &pre[..pre.len().min(12)],
                &post[..post.len().min(12)],
            ),
        ),
        CleanupAction::LeftAlone => (ui::ActionStatus::Skipped, "left alone".into()),
    }
}

fn status_cmd(
    jj: &JjCli,
    bookmarks: cli::BookmarkArgs,
    trunk: Option<String>,
    json: bool,
    _verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;

    let selected = select::resolve_bookmarks(jj, &bookmarks, &trunk)?;
    if selected.is_empty() {
        return Err(JjGtError::NoBookmarksSelected);
    }
    let stacked = stack::derive_parents(jj, &selected, &trunk)?;

    let locals = status::collect_local_commits(jj, &selected)?;
    let prs = status::fetch_pr_info(&workspace_root, &selected)?;

    let out = status::build(&trunk, &stacked, &locals, &prs);
    if json {
        println!("{}", status::render_json(&out)?);
    } else {
        print!("{}", status::render_table(&out));
    }
    Ok(ExitCode::SUCCESS)
}

fn log_cmd(
    jj: &JjCli,
    trunk: Option<String>,
    _verbosity: ui::Verbosity,
) -> Result<ExitCode, JjGtError> {
    let workspace_root = jj.workspace_root().map_err(JjGtError::Hooks)?;
    let trunk = status::resolve_trunk(&workspace_root, trunk.as_deref())?;
    let args = cli::BookmarkArgs {
        all: true,
        remote: "origin".into(),
        ..cli::BookmarkArgs::default()
    };
    let selected = select::resolve_bookmarks(jj, &args, &trunk)?;
    let stacked = stack::derive_parents(jj, &selected, &trunk)?;

    println!("trunk: {trunk}");
    println!("stack (top → bottom):");
    for sb in stacked.iter().rev() {
        let parent = sb.parent.as_branch_name(&trunk);
        println!("  ● {}\n    └─ parent: {}", sb.name, parent);
    }
    Ok(ExitCode::SUCCESS)
}

fn completions_cmd(shell: clap_complete::Shell) -> Result<ExitCode, JjGtError> {
    use clap::CommandFactory;
    use clap_complete::env::EnvCompleter;
    use clap_complete::env::{Bash, Elvish, Fish, Powershell, Zsh};

    let cmd = Cli::command();
    let bin_name = std::env::args()
        .next()
        .and_then(|arg0| {
            std::path::Path::new(&arg0)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "jj-gt".into());

    let mut out = std::io::stdout();
    let result = match shell {
        clap_complete::Shell::Bash => {
            Bash.write_registration("COMPLETE", &bin_name, &bin_name, &bin_name, &mut out)
        }
        clap_complete::Shell::Zsh => {
            Zsh.write_registration("COMPLETE", &bin_name, &bin_name, &bin_name, &mut out)
        }
        clap_complete::Shell::Fish => {
            Fish.write_registration("COMPLETE", &bin_name, &bin_name, &bin_name, &mut out)
        }
        clap_complete::Shell::PowerShell => {
            Powershell.write_registration("COMPLETE", &bin_name, &bin_name, &bin_name, &mut out)
        }
        clap_complete::Shell::Elvish => {
            Elvish.write_registration("COMPLETE", &bin_name, &bin_name, &bin_name, &mut out)
        }
        _ => {
            eprintln!("jj-gt: unsupported shell for dynamic completion");
            return Ok(ExitCode::from(2));
        }
    };
    let _ = cmd;
    result.map_err(JjGtError::Io)?;
    Ok(ExitCode::SUCCESS)
}

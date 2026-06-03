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
                    || jjui.added_binding_submit
                    || jjui.added_binding_submit_selected
                    || jjui.added_binding_fetch
                    || jjui.added_binding_track
                    || jjui.added_binding_track_selected
                    || jjui.added_binding_reconcile
                {
                    eprintln!("jj-gt: merged jjui actions/bindings into jjui config");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Completions { shell } => completions_cmd(shell),
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
        if let Err(e) = reconcile::reconcile(
            jj,
            &workspace_root,
            &reconcile_opts,
            &stack_names,
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
        vec!["gtmq_".into()]
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

    let actions = cleanup::run_fetch(jj, &workspace_root, &opts, verbosity)?;

    if !actions.is_empty() {
        ui::section("Per-bookmark actions");
        for (bookmark, action) in &actions {
            let (status, msg) = action_to_row(action);
            ui::action_row(&bookmark.name, status, &msg);
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Map a [`cleanup::CleanupAction`] to the (status, message) tuple
/// the [`ui::action_row`] renderer takes. The match cases mirror
/// what the old `format_action` produced, just split into a
/// glyph-color signal + a text payload so the renderer can paint
/// each row consistently.
fn action_to_row(action: &cleanup::CleanupAction) -> (ui::ActionStatus, String) {
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
            format!("rebased onto {onto} (parent `{prev_parent}` was removed by gt sync)"),
        ),
        CleanupAction::RebaseDeferredForConflict {
            onto,
            prev_parent,
            message,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "rebase onto {onto} would have conflicted (parent `{prev_parent}` was removed by gt sync) — {message}; deferred to `jj-gt restack` (run manually when you're ready to resolve)"
            ),
        ),
        CleanupAction::RebaseConflicted {
            onto,
            prev_parent,
            message,
        } => (
            ui::ActionStatus::Error,
            format!(
                "rebase onto {onto} produced conflicts (parent `{prev_parent}` was removed by gt sync) — {message}; run `jj resolve` to fix"
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

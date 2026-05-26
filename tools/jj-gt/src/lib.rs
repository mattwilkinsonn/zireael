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
            hk_runner,
        } => submit_cmd(
            &jj,
            bookmarks,
            submit,
            trunk,
            no_export,
            no_restore_cwc,
            no_hooks,
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
            dry_run,
            verbosity,
        ),

        Command::Status {
            bookmarks,
            trunk,
            json,
        } => status_cmd(&jj, bookmarks, trunk, json, verbosity),

        Command::Log { trunk } => log_cmd(&jj, trunk, verbosity),

        Command::Init => {
            init::print_init();
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
    let stacked = stack::derive_parents(jj, &selected, &trunk)?;
    let tip = stack::find_tip(&stacked)?;
    let stacked_sorted = stack::sort_for_tracking(&stacked);

    ui::section(&format!(
        "Submitting stack to {} (tip: {tip})",
        bookmarks.remote
    ));

    // 1. Export jj bookmarks → git refs (idempotent).
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

    // 2. Hook gate against the full trunk..tip range. Resolve both
    // ends to commit ids ourselves so we can hand `jj_hooks` a
    // proper BookmarkUpdate (skipping the synthesis layer that was
    // historically a bug magnet for multi-commit revsets).
    if !no_hooks {
        let step = ui::Step::start(
            &format!("Running pre-push hooks against {trunk}..{tip}"),
            verbosity,
        );
        let trunk_commit = jj::resolve_commit_id(jj, &trunk)?;
        let tip_commit = jj::resolve_commit_id(jj, &tip)?;
        let opts = hooks::HookOpts {
            runner_override: hk_runner.map(Into::into),
        };
        match hooks::run_pre_push(
            jj,
            &workspace_root,
            &bookmarks.remote,
            &tip,
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

    // 3. gt track per (bookmark, parent). Must be bottom→top because
    // `gt track <child> --parent <parent>` errors if `<parent>` isn't
    // already tracked.
    for sb in &stacked_sorted {
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

    // 4. Record @ for restoration.
    let saved_change_id = if no_restore_cwc {
        None
    } else {
        Some(jj::current_change_id(jj)?)
    };

    // 5. gt submit --stack --branch <tip>.
    let submit_step = ui::Step::start(
        &format!("Submitting stack via `gt submit --stack --branch {tip}`"),
        verbosity,
    );
    let argv = gt::build_submit_argv(&tip, &submit);
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

    // 6. Track each pushed bookmark so jj's `untracked_remote_bookmarks()`
    // (part of the default `immutable_heads()` revset) doesn't freeze
    // the commits we just submitted. See jj::track_bookmark_on_remote
    // for the full rationale; short version: `gt submit` shells out
    // to raw `git push`, jj never sees that push, and the next jj
    // op imports the new `refs/remotes/<remote>/*` refs as
    // untracked → ancestors flip immutable → users can't amend
    // their just-pushed commits.
    //
    // Skip bookmarks already tracked (typical re-submit case).
    if !submit.dry_run {
        let track_step = ui::Step::start("Tracking pushed bookmarks with jj", verbosity);
        let already_tracked =
            jj::list_tracked_bookmarks_on_remote(jj, &bookmarks.remote).unwrap_or_default();
        let mut tracked = 0usize;
        let mut skipped = 0usize;
        let mut errors: Vec<String> = Vec::new();
        for sb in &stacked_sorted {
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

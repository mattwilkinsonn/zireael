//! Clap argument structs for the `jj-gt` binary.
//!
//! Bookmark-selection flags live in a shared `#[command(flatten)]`
//! [`BookmarkArgs`] so `submit`, `track`, and `status` all share the
//! same surface. Submit-passthrough flags live in a separate
//! [`SubmitArgs`].

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use crate::completions::{bookmark_value_completer, remote_value_completer};

#[derive(Parser, Debug)]
#[command(
    name = "jj-gt",
    about = "Bridge jj bookmark stacks and Graphite (gt) PR stacks",
    version,
    propagate_version = true
)]
pub struct Cli {
    /// Log level filter (e.g. `info`, `debug`, `warn`).
    #[arg(long, global = true, env = "JJ_GT_LOG", default_value = "warn")]
    pub log_level: String,

    /// Print the full subprocess output of every step (jj, gt, gh).
    /// By default jj-gt captures and summarises that output; passing
    /// `-v` dumps it as it comes, useful when something is going
    /// subtly wrong inside one of the subprocess invocations and
    /// the structured summary is hiding the real signal.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Track + submit selected bookmarks as a stack (drives
    /// `gt submit --stack` end-to-end).
    ///
    /// The full ancestor chain from trunk up to each `-b <tip>` is
    /// submitted automatically — there's no separate `--stack` flag
    /// because there's no other mode. `jj-gt submit -b head` walks
    /// back through every bookmark between `head` and trunk,
    /// tracks them with gt in bottom→top order, and pushes the
    /// whole stack via `gt submit --stack`. Multiple `-b` flags
    /// for unrelated tips submit each as its own independent stack
    /// — `jj-gt submit -b a -b b` where `a` and `b` sit on
    /// disjoint chains rooted at trunk fans out to two
    /// `gt submit --stack` calls, one per tip. To submit a subset
    /// of a single stack, name each bookmark with its own `-b` flag.
    Submit {
        #[command(flatten)]
        bookmarks: BookmarkArgs,
        #[command(flatten)]
        submit: SubmitArgs,

        /// Trunk to walk back to. Default: read from gt's repo config
        /// (.git/.graphite_repo_config), then fall back to `main`.
        #[arg(long)]
        trunk: Option<String>,

        /// Skip the `jj git export` step.
        #[arg(long)]
        no_export: bool,

        /// Don't restore `@` after the submit. gt's git-push triggers
        /// a jj ref-import that moves `@` as a side effect — jj-gt
        /// records `@` before submit and restores after by default.
        #[arg(long)]
        no_restore_cwc: bool,

        /// Skip the pre-push hook gate (equivalent to passing
        /// `--no-verify` to `git push`).
        #[arg(long)]
        no_hooks: bool,

        /// Run pre-push hooks against the full trunk..tip range
        /// instead of per-bookmark. Faster — one hook invocation
        /// instead of N — but hides intermediate-commit failures
        /// (a fmt violation in commit B that commit C fixes will
        /// pass when checked against trunk..tip but fail in CI
        /// which builds each commit independently). Recovery flag.
        #[arg(long, conflicts_with = "no_hooks")]
        hooks_tip_only: bool,

        /// Run per-bookmark pre-push hooks sequentially instead of
        /// in parallel. Default is parallel: one ephemeral worktree
        /// per bookmark, all hook runners executing concurrently
        /// (output captured + replayed in completion order). Falls
        /// back to sequential automatically when the stack has only
        /// one bookmark. Use this flag when the parallel runs are
        /// thrashing disk / sccache or you want the live runner
        /// progress bar for a multi-bookmark stack.
        #[arg(long, conflicts_with_all = ["no_hooks", "hooks_tip_only"])]
        hooks_sequential: bool,

        /// Force a specific hook runner. Forwarded as the `runner`
        /// arg of `jj_hooks::run_for_revset`.
        #[arg(long, value_enum)]
        hk_runner: Option<RunnerArg>,
    },

    /// Sync `refs/branch-metadata/*` for the selected bookmarks
    /// without invoking `gt submit` (manual escape hatch).
    Track {
        #[command(flatten)]
        bookmarks: BookmarkArgs,

        #[arg(long)]
        trunk: Option<String>,

        #[arg(long)]
        no_export: bool,

        /// Force every selected bookmark to use this parent,
        /// bypassing the jj-graph-derived parent.
        #[arg(long, add = clap_complete::ArgValueCompleter::new(bookmark_value_completer))]
        parent: Option<String>,

        /// Print the (branch, parent) pairs without writing refs.
        #[arg(long)]
        dry_run: bool,
    },

    /// Graphite-aware replacement for `jj git fetch`.
    Fetch {
        #[arg(long, default_value = "origin", add = clap_complete::ArgValueCompleter::new(remote_value_completer))]
        remote: String,

        #[arg(long)]
        trunk: Option<String>,

        /// Skip the gt-tracking backfill step.
        #[arg(long)]
        no_backfill: bool,

        /// Skip the post-cleanup jj rebase step.
        #[arg(long)]
        no_rebase: bool,

        /// Skip the Graphite queue-branch cleanup.
        #[arg(long)]
        no_gtmq_prune: bool,

        /// Bookmark/branch prefix to treat as Graphite queue-test
        /// scratch (repeatable). Default: `gtmq_`.
        #[arg(long, action = clap::ArgAction::Append)]
        gtmq_prefix: Vec<String>,

        /// Skip y/N prompts for the orphan-bookmark fallback. Treats
        /// every detected orphan as "yes, delete."
        #[arg(long)]
        auto: bool,

        /// Skip the pre-fetch `jj git export` step. Default off:
        /// fetch exports JJ's bookmark view into git refs first so
        /// that bookmark moves made in another workspace (sharing
        /// the same `.jj/`) aren't reverted by `jj git fetch`'s
        /// auto-import. Set this when you specifically want to
        /// observe git's pre-export state.
        #[arg(long)]
        no_export: bool,

        /// Print what would happen at every step.
        #[arg(long)]
        dry_run: bool,
    },

    /// Print stack-wide PR + queue state.
    Status {
        #[command(flatten)]
        bookmarks: BookmarkArgs,

        #[arg(long)]
        trunk: Option<String>,

        /// Emit machine-readable JSON instead of the human table.
        #[arg(long)]
        json: bool,
    },

    /// Print the derived stack as jj-gt sees it (debug).
    Log {
        #[arg(long)]
        trunk: Option<String>,
    },

    /// Reconcile gt's tracking metadata + (optionally) remote refs
    /// with jj's current view of the bookmark graph. Standalone
    /// version of the pre-submit reconciliation step that
    /// `jj-gt submit` runs internally. Useful after an interrupted
    /// submit or a manual `jj rebase` outside the submit flow.
    Reconcile {
        /// Remote to reconcile against. Default: `origin`.
        #[arg(long, default_value = "origin", add = clap_complete::ArgValueCompleter::new(remote_value_completer))]
        remote: String,

        /// Trunk to walk back to. Default: read from gt's repo config
        /// (.git/.graphite_repo_config), then fall back to `main`.
        #[arg(long)]
        trunk: Option<String>,

        /// Also `jj git push --bookmark <name>` each bookmark in the
        /// active stack. Default off — reconcile-only updates gt's
        /// tracking metadata without touching remote refs. Pass this
        /// to bring the remote in sync with locally-rebased SHAs
        /// (jj's force-with-lease default protects against
        /// collaborator-race overrides).
        #[arg(long)]
        push: bool,

        /// Print what would happen at every step.
        #[arg(long)]
        dry_run: bool,
    },

    /// Rebase every local stack onto trunk — the explicit "rewrite
    /// my work" sibling to `fetch`. Conflict-producing rebases land
    /// their conflicts as commit markers (jj's conflict-in-commit
    /// model means rebases always complete); the per-stack summary
    /// surfaces what needs `jj resolve`.
    ///
    /// Default behaviour: discover every local-only bookmark
    /// authored by the current user (`bookmarks() & mine() &
    /// ~::main@origin`), group into independent stacks, rebase each
    /// onto trunk. End-of-run summary lists per-stack status.
    /// Exit non-zero if any stack ended conflicted.
    Restack {
        /// Restack only the stack containing this bookmark. Same
        /// default discovery + rebase otherwise.
        #[arg(long, short = 'b')]
        bookmark: Option<String>,

        /// Trunk to rebase onto. Default: read from gt's repo config
        /// (.git/.graphite_repo_config), then fall back to `main`.
        /// Resolved against the remote — the actual destination is
        /// `<trunk>@<remote>`.
        #[arg(long)]
        trunk: Option<String>,

        /// Remote to rebase against. Default: `origin`.
        #[arg(long, default_value = "origin", add = clap_complete::ArgValueCompleter::new(remote_value_completer))]
        remote: String,

        /// Halt at the first conflicted stack rather than proceeding
        /// through the rest. Cautious mode for users who want
        /// to resolve one conflict before facing the next.
        #[arg(long)]
        stop_on_conflict: bool,

        /// Print what would happen at every step — discover stacks
        /// + report them, don't actually rebase.
        #[arg(long)]
        dry_run: bool,
    },

    /// Interactive setup: prints suggested aliases + setup
    /// reminders, then optionally installs jjui actions/bindings so
    /// `jj-gt submit / fetch / track / reconcile` are reachable
    /// from inside [jjui](https://github.com/idursun/jjui).
    Init {
        /// Skip the prompts; only print the setup reminders.
        /// Useful in scripts and one-shot runs.
        #[arg(long)]
        print_only: bool,
    },

    /// Print a shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Bookmark-selection flags shared by `submit`, `track`, and `status`.
/// Same shape as `jj git push` / `jj-hp push`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct BookmarkArgs {
    /// Operate on this bookmark (repeatable).
    #[arg(
        short = 'b',
        long,
        action = clap::ArgAction::Append,
        add = clap_complete::ArgValueCompleter::new(bookmark_value_completer),
    )]
    pub bookmark: Vec<String>,

    /// Operate on bookmarks pointing to these commits (repeatable).
    #[arg(short = 'r', long, action = clap::ArgAction::Append)]
    pub revision: Vec<String>,

    /// Operate on these commits (creates a bookmark per change if
    /// missing). Mirrors `jj git push -c`.
    #[arg(short = 'c', long, action = clap::ArgAction::Append)]
    pub change: Vec<String>,

    /// Operate on every local bookmark across every stack in the
    /// repo (`bookmarks() & trunk..`), excluding trunk + `gtmq_*`
    /// queue branches. Use this for "submit all my stacks" /
    /// "push all my stacks" — distinct from the bareword default,
    /// which operates only on the @-ancestor stack.
    #[arg(long)]
    pub all: bool,

    /// Operate on every locally-tracked bookmark.
    #[arg(long)]
    pub tracked: bool,

    /// Allow new bookmarks (those without a remote counterpart yet).
    #[arg(long)]
    pub allow_new: bool,

    /// Remote to push to / read tracking from.
    #[arg(
        long,
        default_value = "origin",
        add = clap_complete::ArgValueCompleter::new(remote_value_completer),
    )]
    pub remote: String,
}

/// Flags that map to `gt submit` arguments. Most are simple bool
/// passthroughs; the `--draft` / `--no-publish` pair encodes the
/// default-publish behaviour (see DEFAULT PUBLISH BEHAVIOUR in
/// gt::build_submit_argv).
#[derive(clap::Args, Debug, Clone, Default)]
pub struct SubmitArgs {
    /// Create new PRs as draft AND skip the default `--publish`.
    /// Mutually exclusive with `--no-publish`.
    #[arg(long, conflicts_with = "no_publish")]
    pub draft: bool,

    /// Skip the default `--publish` without flipping to `--draft`.
    #[arg(long)]
    pub no_publish: bool,

    /// Restack branches before submitting.
    #[arg(long)]
    pub restack: bool,

    /// Don't prompt for PR metadata edits (script-friendly).
    #[arg(short = 'n', long)]
    pub no_edit: bool,

    /// AI-generate title + description for new PRs.
    #[arg(long)]
    pub ai: bool,

    /// Disable AI generation.
    #[arg(long)]
    pub no_ai: bool,

    /// Comma-separated reviewers. Repeat the flag for multiple.
    #[arg(short = 'R', long)]
    pub reviewers: Option<String>,

    /// Comma-separated team-reviewer slugs.
    #[arg(short = 't', long)]
    pub team_reviewers: Option<String>,

    /// Only push branches and update PRs for branches that already
    /// have open PRs.
    #[arg(short = 'u', long)]
    pub update_only: bool,

    /// Mark each PR as merge-when-ready.
    #[arg(short = 'm', long)]
    pub merge_when_ready: bool,

    /// Target a non-default trunk on the remote.
    #[arg(long)]
    pub target_trunk: Option<String>,

    /// Open the PR(s) in your browser after submitting.
    #[arg(long)]
    pub view: bool,

    /// Open the PR-metadata editor in a browser instead of CLI.
    #[arg(short = 'w', long)]
    pub web: bool,

    /// Add a comment to each PR.
    #[arg(long)]
    pub comment: Option<String>,

    /// Re-request review from current reviewers.
    #[arg(long)]
    pub rerequest_review: bool,

    /// Skip the default `--always` flag passed to `gt submit`. By
    /// default jj-gt invokes `gt submit --always` so Graphite's
    /// view (PR base refs, stack metadata on web) always reflects
    /// the current jj state, even when gt would otherwise think no
    /// branch has changed. Set this flag to opt out — useful when
    /// you specifically want gt's "skip unchanged" heuristic, e.g.
    /// repeated submits during PR review where the local stack
    /// hasn't moved.
    #[arg(long)]
    pub no_always: bool,

    /// True force-push (overrides force-with-lease default).
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Print what would be submitted; don't push or open PRs.
    #[arg(long)]
    pub dry_run: bool,

    /// Print summary, ask to confirm, then submit.
    #[arg(short = 'C', long)]
    pub confirm: bool,

    /// Append this arg to `gt submit` verbatim. Repeat for multiple.
    #[arg(long = "gt-arg", action = clap::ArgAction::Append)]
    pub gt_arg: Vec<String>,
}

/// Mirror of [`jj_hooks::runner::Runner`] for clap's value-enum
/// requirement. The `From` impl below lets us hand the user's choice
/// straight to the library.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum RunnerArg {
    PreCommit,
    Prek,
    Lefthook,
    Hk,
}

impl From<RunnerArg> for jj_hooks::runner::Runner {
    fn from(value: RunnerArg) -> Self {
        match value {
            RunnerArg::PreCommit => jj_hooks::runner::Runner::PreCommit,
            RunnerArg::Prek => jj_hooks::runner::Runner::Prek,
            RunnerArg::Lefthook => jj_hooks::runner::Runner::Lefthook,
            RunnerArg::Hk => jj_hooks::runner::Runner::Hk,
        }
    }
}

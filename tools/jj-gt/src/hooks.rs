//! Thin wrapper around [`jj_hooks::hooks::run_for_update`].
//!
//! We deliberately don't go through `jj_hooks::run_for_revset_outcome`
//! anymore. That entrypoint synthesizes a [`BookmarkUpdate`] from a
//! revset string and was historically a bug magnet — early versions
//! truncated the range to the tip slice (`--limit 1`), so a 3-commit
//! stack only got its top commit's delta checked. We sidestep the
//! synthesis layer by building the BookmarkUpdate ourselves with real
//! commit ids the caller already has on hand — exactly the same
//! shape `jj-hp push` builds from its `jj git push --dry-run` parse.

use std::path::Path;

use jj_hooks::bookmark_updates::{BookmarkUpdate, UpdateType};
use jj_hooks::hooks::RunOpts;
use jj_hooks::jj::{self, JjCli};
use jj_hooks::runner::{Runner, Stage};

use crate::error::{JjGtError, Result};

#[derive(Debug, Clone, Default)]
pub struct HookOpts {
    /// Override the autodetected hook runner. `None` means "let
    /// jj_hooks autodetect from the target commit's tree".
    pub runner_override: Option<Runner>,
}

/// Run pre-push hooks against the full diff range from `trunk_commit`
/// to `tip_commit` for `bookmark` on `remote`.
///
/// The synthesized [`BookmarkUpdate`] mirrors the shape `jj-hp push`
/// builds for an existing bookmark move:
///
/// - `old_commit = trunk_commit` → the from-ref hooks diff against,
///   which is the merge-base of the stack and trunk.
/// - `new_commit = tip_commit` → the bookmark tip, the worktree
///   hooks actually run inside.
///
/// `jj_hooks::hooks::run_for_update` takes that and runs the
/// configured hook backend (hk / lefthook / pre-commit) with
/// `--from-ref <trunk> --to-ref <tip>` so every file changed across
/// the entire stack is in scope — same contract as `git push origin
/// tip` would produce.
///
/// We pass `retry_after_fixup = true` so jj-hooks transparently
/// recovers from transient races (e.g. hk's intra-bookmark step
/// parallelism fighting for `.git/index.lock` while a separate
/// auto-fix step legitimately modifies files). If the retry on the
/// fixup commit comes back clean, the error we return below names
/// the retry-healed shape explicitly.
///
/// Returns Ok(()) on a clean pass; Err with a descriptive message
/// on either a hook failure or a hook autofix (so the user can
/// squash the fixup into the stack and re-submit).
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
        // Empty stack — bookmark is already on trunk. Nothing to
        // check; gt will figure out there's nothing to submit too.
        tracing::info!("pre-push: bookmark `{bookmark}` is already at trunk; skipping hooks");
        return Ok(());
    }

    let update = BookmarkUpdate {
        remote: remote.to_owned(),
        bookmark: bookmark.to_owned(),
        update_type: UpdateType::MoveForward,
        old_commit: Some(trunk_commit.to_owned()),
        new_commit: Some(tip_commit.to_owned()),
    };

    let primary_git_dir = jj::primary_git_dir(workspace_root).map_err(JjGtError::Hooks)?;
    let run_opts = RunOpts {
        retry_after_fixup: true,
    };
    let outcome = jj_hooks::hooks::run_for_update(
        jj,
        &primary_git_dir,
        opts.runner_override,
        Stage::PrePush,
        &update,
        run_opts,
    )
    .map_err(JjGtError::Hooks)?;

    if outcome.success && outcome.fixup_commit.is_none() {
        return Ok(());
    }
    if !outcome.success {
        return Err(JjGtError::Invalid(format!(
            "pre-push hooks failed for `{bookmark}` ({trunk_commit}..{tip_commit})"
        )));
    }
    // success && fixup_commit.is_some() — hooks auto-fixed
    // something. Make the user re-run after squashing the fixup
    // into the bookmark so we push the fixed code, not the
    // pre-fix code. When `outcome.retried` is true, the initial
    // run failed AND the re-run on the fixup was clean — surface
    // that so the user knows the failure was transient rather
    // than a real hook problem.
    let commit = outcome.fixup_commit.unwrap_or_else(|| "<unknown>".into());
    let retry_hint = if outcome.retried {
        " (re-run on fixup commit was clean — initial failure was transient)"
    } else {
        ""
    };
    Err(JjGtError::Invalid(format!(
        "pre-push hooks modified files (fixup commit {commit}){retry_hint}; \
         squash it into the relevant bookmark and re-run `jj-gt submit`"
    )))
}

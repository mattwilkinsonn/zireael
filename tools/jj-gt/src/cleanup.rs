//! `jj-gt fetch` pipeline + the testable per-bookmark classifier
//! decisions.
//!
//! The pipeline is split into named phase functions whose contracts
//! are individually testable; `run_fetch` is a thin orchestrator on
//! top. The motivation is to make the timing-sensitive bits (in
//! particular [`snapshot_pre_fetch`]) catchable by unit tests so a
//! future refactor can't re-introduce the "derive_parents ran after
//! fetch already deleted the parent bookmarks" bug that prompted
//! this split.
//!
//! Most of the file is the [`classify_local_bookmark`] /
//! [`classify_gtmq_branch`] functions — pure decision logic kept
//! separate from the orchestration so the test suite can exercise
//! every branch without spinning up real `gh` / `gt` / `jj` invocations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::Result;
use crate::gh::{self, PrInfo, PrState};
use crate::gt;
use crate::jj::{self, JjCli, LocalBookmark, list_local_bookmarks};
use crate::stack::{BookmarkOrTrunk, StackedBookmark, derive_parents_lossy};

#[derive(Debug, Clone)]
pub struct FetchOpts {
    pub remote: String,
    pub trunk: String,
    pub no_backfill: bool,
    pub no_rebase: bool,
    pub no_gtmq_prune: bool,
    pub gtmq_prefixes: Vec<String>,
    pub auto: bool,
    pub dry_run: bool,
    /// Skip the pre-fetch `jj git export` step. Default `false`;
    /// the export is what keeps bookmark moves made in one workspace
    /// from being clobbered by `jj git fetch`'s auto-import when
    /// `.jj/` is shared across workspaces. Set to `true` only when
    /// you specifically want to observe git's pre-export state.
    pub no_export: bool,
}

impl Default for FetchOpts {
    fn default() -> Self {
        Self {
            remote: "origin".into(),
            trunk: "main".into(),
            no_backfill: false,
            no_rebase: false,
            no_gtmq_prune: false,
            gtmq_prefixes: default_gtmq_prefixes_owned(),
            auto: false,
            dry_run: false,
            no_export: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupAction {
    /// gt sync deleted this branch — bookmark already gone.
    GtSyncDeleted,
    /// Graphite queue-test branch (gtmq_*) with no open PR; deleted
    /// both locally and on the remote.
    GtmqPruned { had_pr: Option<u32> },
    /// gtmq_* branch with an open PR — left alone (queue actively
    /// running).
    GtmqLeftAlone { pr: u32 },
    /// PR closed and merge marker found on trunk; user confirmed
    /// deletion (or --auto).
    OrphanDeleted { pr: u32, merge_commit_id: String },
    /// PR closed and merge marker found, but user said no.
    OrphanSkipped { pr: u32, merge_commit_id: String },
    /// SHA drift detected — local has changes the PR doesn't.
    SkippedDueToDrift {
        pr: u32,
        local_sha: String,
        pushed_sha: String,
    },
    /// Step 7: this bookmark's tracked parent was removed by `gt
    /// sync` (its PR landed and Graphite cleaned the source branch)
    /// so we rebased its commits onto `dest` to keep the stack
    /// linear. The previous parent name is preserved for the user
    /// to see why the rebase happened.
    Rebased { onto: String, prev_parent: String },
    /// Parent was deleted, but by the time `orphan_rebase_phase`
    /// ran the live bookmark was no longer a descendant of the
    /// deleted parent's commit — the most common cause is
    /// Graphite's pre-merge rebase + the import step pulling
    /// the bookmark forward onto a new trunk tip, but any
    /// mutation that moves the bookmark off the deleted
    /// parent's history (manual `jj bookmark set`, an out-of-
    /// band `gt move`, …) routes here too. Emitted instead of
    /// `Rebased` so the action log distinguishes "we rebased
    /// it" from "we noticed it was already where it should be."
    /// Also acts as a guard rail against the
    /// `parent_commit..bookmark` revset accidentally sweeping
    /// in immutable trunk commits when the deleted parent's
    /// commit is no longer reachable from the bookmark.
    OrphanRebaseNoOpAlreadyAdvancedPastParent { prev_parent: String },
    /// Same trigger as `Rebased`, but the rebase produced
    /// conflicts. We rolled it back via `jj op restore` so the
    /// user's working state isn't littered with conflict markers
    /// from a fetch they didn't ask to mutate. The bookmark is
    /// still anchored to its pre-fetch parent; resolving the
    /// orphan now requires `jj-gt restack` (issue #61) or a
    /// manual `jj rebase -d main@origin`. `message` carries the
    /// jj stderr line that surfaced the conflict.
    RebaseDeferredForConflict {
        onto: String,
        prev_parent: String,
        message: String,
    },
    /// `jj rebase` itself errored (e.g. immutable destination,
    /// nonexistent source). Distinct from the conflict-deferred
    /// path because we can't even attempt the rebase; the
    /// bookmark is left where it was and the error message is
    /// surfaced for diagnosis.
    RebaseConflicted {
        onto: String,
        prev_parent: String,
        message: String,
    },
    /// Issue #68: the candidate bookmark's local name is in
    /// conflict (different op-log lineages disagree on the target
    /// commit). `jj rebase -s <name>` would fail with "Name
    /// `<name>` is conflicted" — that's a name-resolution failure,
    /// not a content conflict, and the user has to pick a side
    /// with `jj bookmark set` before the orphan rebase can run.
    /// `prev_parent` is the parent that triggered the orphan
    /// shape so the message can explain why we *would* have
    /// rebased.
    BookmarkConflicted { prev_parent: String },
    /// Issue #69 (Shape A): the bookmark's parent was rewritten
    /// on the remote (e.g. Graphite pre-merge rebase) and the
    /// local child was re-anchored onto the new parent. `pre`
    /// and `post` are the parent's pre-fetch / post-fetch commit
    /// ids so the user sees the move.
    RebasedAfterParentMoved {
        parent_bookmark: String,
        pre_commit: String,
        post_commit: String,
    },
    /// Issue #69 (Shape A), conflict-defer variant: rebasing the
    /// child onto the new parent commit produced content
    /// conflicts; we rolled back via `jj op restore` and surface
    /// the deferred action so the user can resolve via
    /// `jj-gt restack` or manual rebase. `message` is jj's
    /// conflict stderr line.
    RebaseAfterParentMovedDeferred {
        parent_bookmark: String,
        pre_commit: String,
        post_commit: String,
        message: String,
    },
    /// Issue #69 (Shape A), live-conflicts variant: rebasing the
    /// child onto the new parent produced conflicts AND we
    /// couldn't roll back (either the pre-rebase op snapshot
    /// failed or `jj op restore` itself errored). The user's
    /// workspace now carries conflict markers from a mutation
    /// they didn't ask for and needs to run `jj resolve`
    /// directly — `jj-gt restack` won't help here.
    RebaseAfterParentMovedConflicted {
        parent_bookmark: String,
        pre_commit: String,
        post_commit: String,
        message: String,
    },
    /// Issue #69 (Shape B): the bookmark itself moved sideways
    /// on the remote and we had local commits on top; we rebased
    /// the local-only commits onto the new remote tip.
    RebasedAfterRemoteMoved {
        bookmark: String,
        pre_commit: String,
        post_commit: String,
    },
    /// Issue #69 (Shape B), conflict-defer variant.
    RebaseAfterRemoteMovedDeferred {
        bookmark: String,
        pre_commit: String,
        post_commit: String,
        message: String,
    },
    /// Issue #69 (Shape B), live-conflicts variant: same shape as
    /// [`Self::RebaseAfterParentMovedConflicted`] but for the
    /// bookmark-itself-moved-sideways case.
    RebaseAfterRemoteMovedConflicted {
        bookmark: String,
        pre_commit: String,
        post_commit: String,
        message: String,
    },
    /// PR-D / issue #2: gt sync silently moved this bookmark backward
    /// (local was ahead of remote with un-pushed commits). We
    /// restored the bookmark to its pre-pipeline position. `pre` and
    /// `post` are the bookmark's commit ids before/after sync.
    RestoredAfterRewind { pre: String, post: String },
    /// PR-D / issue #2: bookmark diverged from remote during the
    /// pipeline (neither pre nor post is an ancestor of the other —
    /// rare; typically a concurrent push from another machine). We
    /// restored the local pre-pipeline position; the user needs to
    /// reconcile manually. `pre` and `post` are the commit ids
    /// before/after sync.
    DivergedFromRemote { pre: String, post: String },
    /// Detected that a bookmark's commit_id moved BACKWARD across
    /// the fetch pipeline (post is a strict ancestor of pre) AND
    /// the pre-fetch position carried local-only work (pre != the
    /// pre-fetch `@origin` baseline). We auto-restored the
    /// bookmark to its pre-fetch position. Surfaces as Error so
    /// the user notices: something in the pipeline (most likely
    /// `gt sync --force`, a sibling workspace's plain `jj git
    /// fetch` resolving against a stale view, or an unidentified
    /// race) tried to silently rewind local-only work.
    ///
    /// If the rewind was intentional (the agent ran `jj bookmark
    /// set <name> -r <earlier>` deliberately during fetch), the
    /// `pre != origin` filter wouldn't have fired — they'd have
    /// to re-issue the rewind. That's an acceptable cost for the
    /// "your in-flight commits don't silently vanish" guarantee.
    RewindRestored {
        pre_commit: String,
        post_commit: String,
    },
    /// Same trigger shape as [`Self::RewindRestored`] but the
    /// `jj bookmark set` to restore failed (jj refused, the
    /// commit was GC'd, …). Surfaces the original pre/post
    /// commits plus the restore failure so the user can recover
    /// manually with `jj bookmark set` + op-log lookup.
    RewindDetectedButRestoreFailed {
        pre_commit: String,
        post_commit: String,
        message: String,
    },
    /// Sibling of [`Self::RewindRestored`] for the
    /// deletion-with-local-work case: the bookmark vanished
    /// between pre and post snapshots, but the pre snapshot
    /// carried commits not yet pushed to `@<remote>`. We
    /// recreated the bookmark at the saved pre_commit. There's
    /// no post commit to report — the bookmark didn't exist
    /// after fetch.
    DeletionRestored { pre_commit: String },
    /// Sibling of [`Self::RewindDetectedButRestoreFailed`] for
    /// the deletion-with-local-work case where the recreate
    /// `jj bookmark set` failed. User must recover manually.
    DeletionDetectedButRestoreFailed { pre_commit: String, message: String },
    /// Local bookmark had a matching `@<remote>` ref but wasn't
    /// tracked by jj. The `[deleted] propagate` path jj would
    /// normally apply on the next fetch never fires for
    /// untracked bookmarks, so a merged-PR cleanup leaves them
    /// dangling. The auto-track sweep flips the tracking on so
    /// the standard mechanism takes over going forward — no
    /// destructive action, just a metadata correction. Next
    /// fetch handles the actual deletion when the remote ref
    /// disappears.
    OrphanUntrackedTracked { remote: String, commit_id: String },
    /// PR still open and local matches pushed; leave alone.
    LeftAlone,
}

/// Decide what to do with a non-gtmq local bookmark in the cleanup
/// pass. Pure function — no `jj` / `gh` calls — so the test suite can
/// exhaustively cover the cases.
///
/// `pr` is `None` if `gh pr list` returned no PR for this bookmark.
/// `merge_marker_on_trunk` is `Some(sha)` if the orphan-fallback scan
/// found a `(#N)` marker on trunk for the bookmark's PR.
pub fn classify_local_bookmark(
    local: &LocalBookmark,
    pr: Option<&PrInfo>,
    merge_marker_on_trunk: Option<&str>,
) -> CleanupAction {
    match pr {
        None => match merge_marker_on_trunk {
            Some(_) => CleanupAction::LeftAlone, // no PR + marker is ambiguous, leave it
            None => CleanupAction::LeftAlone,
        },
        Some(pr) => {
            // Drift check: local commit vs PR head OID. We tolerate
            // prefix matches in either direction since the local short
            // ID is 12 chars and gh returns the full 40-char OID.
            let drift = if pr.head_ref_oid.is_empty() {
                false
            } else {
                !pr.head_ref_oid.starts_with(&local.commit_id)
                    && !local.commit_id.starts_with(&pr.head_ref_oid)
            };
            if drift {
                return CleanupAction::SkippedDueToDrift {
                    pr: pr.number,
                    local_sha: local.commit_id.clone(),
                    pushed_sha: pr.head_ref_oid.clone(),
                };
            }

            match pr.state {
                PrState::Merged => match merge_marker_on_trunk {
                    Some(sha) => CleanupAction::OrphanDeleted {
                        pr: pr.number,
                        merge_commit_id: sha.into(),
                    },
                    None => CleanupAction::LeftAlone,
                },
                PrState::Closed => match merge_marker_on_trunk {
                    Some(sha) => CleanupAction::OrphanDeleted {
                        pr: pr.number,
                        merge_commit_id: sha.into(),
                    },
                    None => CleanupAction::LeftAlone,
                },
                PrState::Open | PrState::Unknown => CleanupAction::LeftAlone,
            }
        }
    }
}

/// Decide what to do with a `gtmq_*` queue-test branch given its
/// (optional) PR state.
pub fn classify_gtmq_branch(pr: Option<&PrInfo>) -> CleanupAction {
    match pr {
        Some(pr) if pr.state == PrState::Open => CleanupAction::GtmqLeftAlone { pr: pr.number },
        Some(pr) => CleanupAction::GtmqPruned {
            had_pr: Some(pr.number),
        },
        None => CleanupAction::GtmqPruned { had_pr: None },
    }
}

/// Filter `bookmarks` for those whose name starts with any of the
/// configured `gtmq_` prefixes.
pub fn is_gtmq_branch(name: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| name.starts_with(p))
}

/// Single source of truth for the default `gtmq_*` prefix list.
/// Used by both [`FetchOpts::default`] and
/// `crate::select::resolve_bookmarks` so the two pipelines can't
/// drift on what counts as a Graphite queue-test branch.
///
/// Convert to an owned `Vec<String>` via
/// [`default_gtmq_prefixes_owned`] when you need the runtime-
/// configurable shape `is_gtmq_branch` expects.
pub const DEFAULT_GTMQ_PREFIXES: &[&str] = &["gtmq_"];

/// Owned `Vec<String>` form of [`DEFAULT_GTMQ_PREFIXES`] for
/// callers that need to feed it into APIs taking `&[String]`.
pub fn default_gtmq_prefixes_owned() -> Vec<String> {
    DEFAULT_GTMQ_PREFIXES
        .iter()
        .map(|&s| s.to_owned())
        .collect()
}

/// Three-way classification of a bookmark's pre/post-sync position.
/// Pure function — no jj/git calls — so the test suite can pin every
/// branch without spinning up a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindClassification {
    /// Bookmark position unchanged across the pipeline. No action.
    Unchanged,
    /// Bookmark advanced (pre is an ancestor of post). Fast-forward
    /// the pipeline applied legitimately; leave it.
    FastForward,
    /// Bookmark rewound (post is an ancestor of pre). Local was
    /// ahead of remote with un-pushed commits; gt sync silently
    /// reset it. The fetch pipeline restores pre.
    Rewound,
    /// Bookmark diverged (neither is an ancestor of the other).
    /// Rare; usually a concurrent push from another machine. The
    /// fetch pipeline restores pre and surfaces a warning so the
    /// user can reconcile.
    Diverged,
    /// Bookmark disappeared from post (gt sync deleted it). Two
    /// sub-cases handled separately by the caller:
    ///
    /// - Bookmark had no local-only commits ahead of remote:
    ///   deletion was intentional (merged PR cleanup); leave it
    ///   deleted.
    /// - Bookmark had local-only commits: restore.
    ///
    /// The caller does the "had local-only commits?" check on its
    /// side; this classifier just signals that the bookmark is gone.
    Disappeared,
}

/// Pure classifier for the pre/post snapshot diff. Takes the two
/// commit ids and an `is_ancestor` oracle (so the test suite can
/// substitute a deterministic stub for `git merge-base
/// --is-ancestor`).
///
/// `post` of `None` means "bookmark was deleted during the pipeline."
/// Disambiguation between "intentional merge-cleanup deletion" and
/// "deletion of a local-only-ahead bookmark" is the caller's job —
/// this function just signals "Disappeared" and the caller does the
/// "had local-only commits?" check.
pub fn classify_rewind<F>(
    pre: &str,
    post: Option<&str>,
    is_ancestor: F,
) -> Result<RewindClassification>
where
    F: Fn(&str, &str) -> Result<bool>,
{
    let Some(post) = post else {
        return Ok(RewindClassification::Disappeared);
    };
    if pre == post {
        return Ok(RewindClassification::Unchanged);
    }
    // Order matters: pre ancestor of post = advance; post ancestor
    // of pre = rewind. Check both before concluding diverged.
    if is_ancestor(pre, post)? {
        return Ok(RewindClassification::FastForward);
    }
    if is_ancestor(post, pre)? {
        return Ok(RewindClassification::Rewound);
    }
    Ok(RewindClassification::Diverged)
}

/// Run the full `jj-gt fetch` pipeline. Returns a per-bookmark log of
/// the decisions made for the caller to print.
///
/// Pipeline steps (numbered to match the design doc):
///   1. `jj git fetch <remote>`.
///   2. Backfill `refs/branch-metadata/*` via `gt track --force` for
///      every local bookmark with an open or recently-closed PR.
///   3. SHA-drift check (per bookmark — skip cleanup, warn).
///   4. `gt sync --no-restack --force`.
///   5. `jj git import` to pick up gt sync's branch deletions.
///   6. Prune `gtmq_*` queue-test branches (closed PR or no PR → delete
///      locally + remote).
///   7. `jj rebase` orphaned children onto trunk.
///   8. Orphan-bookmark fallback — for any remaining local bookmark
///      with a CLOSED PR, look for the merge marker on trunk and
///      prompt to delete.
pub fn run_fetch(
    jj: &JjCli,
    workspace_root: &Path,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
) -> Result<Vec<(LocalBookmark, CleanupAction)>> {
    let _lock = acquire_pipeline_lock(workspace_root, opts)?;

    // Snapshot every local non-trunk, non-gtmq_* bookmark's
    // commit_id + change_id + origin baseline BEFORE any
    // mutation step runs. Used by `apply_rewind_protection` at
    // the end of the pipeline to detect + restore any bookmark
    // that got silently rewound during fetch (the wild bug
    // shape: an agent's unpushed fixup commit ends up orphaned
    // because something in the pipeline reverted the bookmark
    // to its remote position).
    //
    // Captured here, after the lock is acquired, so concurrent
    // jj-gt fetch invocations can't race past this snapshot.
    let rewind_snapshots = if opts.dry_run {
        BTreeMap::new()
    } else {
        capture_rewind_snapshots(jj, opts)?
    };

    maybe_shelter(jj, opts, verbosity)?;

    // Export jj's bookmark view into git's loose refs BEFORE the
    // fetch step. Critical when `.jj/` is shared across workspaces
    // (`jj workspace add`): another workspace may have run `jj
    // bookmark set <bm> -r <new>` without exporting, leaving JJ's
    // view at `<new>` but git's `refs/heads/<bm>` still at the
    // older value. `jj git fetch` auto-imports after fetching, so
    // without a pre-fetch export the import would treat git's
    // stale loose ref as canonical and revert the local bookmark
    // to the older commit — silently orphaning the other
    // workspace's work.
    //
    // Skip in dry-run (no ref mutations) and when --no-export was
    // explicitly passed (escape hatch for users who know what
    // they're doing and want to inspect the bookmark state without
    // touching git). Idempotent on the happy path: when JJ's view
    // already matches git's loose refs, export is a no-op.
    maybe_export_before_fetch(jj, opts, verbosity)?;

    // Snapshot the bookmark graph BEFORE `jj git fetch` runs. See
    // `snapshot_pre_fetch` for the timing rationale.
    let pre = snapshot_pre_fetch(jj, workspace_root, opts)?;

    do_fetch(jj, opts, verbosity)?;

    let post = snapshot_post_fetch(jj)?;

    backfill_phase(workspace_root, &pre, &post, opts, verbosity)?;

    let mut actions: Vec<(LocalBookmark, CleanupAction)> = Vec::new();
    classify_phase(jj, &pre, opts, &mut actions)?;
    sync_and_rewind_phase(
        jj,
        workspace_root,
        &pre,
        &post,
        opts,
        verbosity,
        &mut actions,
    )?;
    gtmq_prune_phase(jj, workspace_root, &pre, opts, verbosity, &mut actions)?;

    // Auto-track every local bookmark whose `@<remote>` ref
    // exists but isn't tracked yet. After this phase, the
    // standard jj propagation handles merge/delete cleanup
    // automatically on subsequent fetches. Pre-push WIP
    // bookmarks (no `@<remote>` ref) are skipped — they have
    // nothing to track against.
    //
    // Runs AFTER gt sync (so any deletions gt did get picked up
    // by import first) and BEFORE the orphan-rebase phase (so
    // children of bookmarks that get tracked-then-deleted on
    // the NEXT fetch end up consistent without an extra cycle).
    orphan_untracked_phase(jj, &pre, opts, verbosity, &mut actions)?;

    orphan_rebase_phase(jj, &pre, &post, opts, &mut actions)?;

    // After every pipeline mutation has run, sweep for silent
    // rewinds. See `apply_rewind_protection` for the rule set —
    // restores any bookmark that ended up at an ancestor of its
    // pre-fetch position AND carried local-only work pre-fetch.
    apply_rewind_protection(
        jj,
        workspace_root,
        &rewind_snapshots,
        opts,
        verbosity,
        &mut actions,
    )?;

    Ok(actions)
}

/// 0a. Acquire the cooperative pipeline lock. Blocks other
/// `jj-gt fetch` / `jj-gt submit` invocations from racing this
/// one's gt-sync → jj-import → rebase pipeline. The lock is
/// released on function exit (or panic). Does not block plain
/// `jj` operations in other shells — see crate::lock for the
/// scope rationale.
///
/// Dry-run doesn't mutate refs; no need to fight other invocations
/// for the lock.
fn acquire_pipeline_lock(
    workspace_root: &Path,
    opts: &FetchOpts,
) -> Result<Option<crate::lock::PipelineLock>> {
    if opts.dry_run {
        Ok(None)
    } else {
        Ok(Some(crate::lock::PipelineLock::acquire(workspace_root)?))
    }
}

/// 0b. Shelter any uncommitted working-copy edits behind a fresh
/// empty `@`. Concurrent jj operations during the pipeline (or
/// jj's own snapshotting as a side effect of other commands the
/// user runs from another shell) can produce divergent commits
/// that silently lose pending edits. Pre-2026-06 fetch refused
/// to run when `@` had edits and offered `--force-with-changes`
/// as a bypass; the new model auto-shelters via `jj new @`
/// (which snapshots the edits into the old `@` and creates a
/// fresh empty `@` above) — strictly safer than the bypass and
/// less friction than the refusal. See issue #1 for the damage
/// shape this defends against.
///
/// Skip when `@` is already empty (nothing to shelter) and in
/// dry-run (don't mutate the workspace at all).
fn maybe_shelter(jj: &JjCli, opts: &FetchOpts, verbosity: crate::ui::Verbosity) -> Result<()> {
    if opts.dry_run || !jj::has_uncommitted_changes(jj)? {
        return Ok(());
    }
    let step = crate::ui::Step::start("Sheltering uncommitted edits (jj new @)", verbosity);
    match jj::shelter_uncommitted_edits(jj) {
        Ok(()) => {
            step.success("old @ now holds your edits as a real change", None);
            Ok(())
        }
        Err(e) => {
            step.fail(&format!("{e}"), None);
            Err(e)
        }
    }
}

/// Capture a `RewindSnapshot` per local non-trunk, non-gtmq_*
/// bookmark, plus each bookmark's `@<remote>` commit baseline.
/// Called at the top of `run_fetch` so the post-pipeline rewind
/// detector can compare against this baseline.
///
/// The origin baseline is best-effort: if the bookmark has no
/// remote-tracking ref (newly-created locally, never pushed),
/// `origin_baseline_commit` is `None` and rule 3 of
/// [`classify_rewind_protection`] doesn't fire. That's the safe
/// default for those: a brand-new local bookmark has zero pushed
/// state to compare against, so we treat any backward move as a
/// genuine rewind (which it is — there's no remote we could be
/// reverting "to" in the first place).
pub fn capture_rewind_snapshots(
    jj: &JjCli,
    opts: &FetchOpts,
) -> Result<BTreeMap<String, RewindSnapshot>> {
    let bookmarks = jj::list_local_bookmarks_with_changes(jj)?;
    let mut out = BTreeMap::new();
    for b in bookmarks {
        if b.name == opts.trunk || is_gtmq_branch(&b.name, &opts.gtmq_prefixes) {
            continue;
        }
        // Read the bookmark's `@<remote>` view, truncated to the
        // same `short(12)` form used for `pre_commit` so the
        // rule-3 (`pre == origin_baseline`) equality check
        // compares apples to apples — `resolve_commit_id` returns
        // a full 40-char hash, but the bookmark template emits
        // `commit_id.short(12)`.
        //
        // We only treat "the revset doesn't resolve" as a
        // legitimate `None` (e.g. bookmark never pushed to this
        // remote). Other failures — jj crashed, the `.jj/` is
        // corrupted, the user lost network mid-call — get
        // propagated. Otherwise a real fetch problem would
        // silently turn into "no remote baseline" for every
        // bookmark, which then makes rule 3 dead and biases the
        // classifier toward "rewound — restore."
        let origin_baseline_commit =
            match jj::resolve_commit_id(jj, &format!("{}@{}", b.name, opts.remote)) {
                Ok(c) => Some(c[..c.len().min(12)].to_owned()),
                Err(e) if is_revset_unresolved_error(&e) => None,
                Err(e) => return Err(e),
            };
        out.insert(
            b.name.clone(),
            RewindSnapshot {
                pre_commit: b.commit_id,
                pre_change_id: b.change_id,
                origin_baseline_commit,
            },
        );
    }
    Ok(out)
}

/// True iff `err` is the "the revset resolved to no commits / a
/// revision that doesn't exist" shape — the case
/// `capture_rewind_snapshots` legitimately maps to `None` baseline
/// for a bookmark that's never been pushed to the remote.
///
/// We sniff both `Invalid("resolved to no commits")` (the wrapper
/// in `resolve_commit_id`) and `Hooks(JjFailed { stderr })` whose
/// stderr contains jj's native "Revision `…` doesn't exist" /
/// "No such revision" wording. Other error variants (Io, GtFailed,
/// etc.) are NOT this — those mean something actually went wrong
/// and the caller should bubble out.
pub fn is_revset_unresolved_error(err: &crate::error::JjGtError) -> bool {
    use crate::error::JjGtError;
    match err {
        JjGtError::Invalid(msg) => msg.contains("resolved to no commits"),
        JjGtError::Hooks(jj_hooks::error::JjHooksError::JjFailed { stderr, .. }) => {
            let lc = stderr.to_lowercase();
            lc.contains("doesn't exist") || lc.contains("no such revision")
        }
        _ => false,
    }
}

/// Phase that compares the post-pipeline bookmark state against
/// the entry snapshot captured by [`capture_rewind_snapshots`]
/// and auto-restores any bookmark that got silently rewound.
///
/// Runs at the END of `run_fetch` (success path). Aborted runs
/// don't reach this phase, but that's acceptable: a phase that
/// aborts has already returned an error to the user; the user
/// can re-run fetch (which will re-snapshot + re-check) once
/// they've resolved the abort cause.
///
/// `actions` is the action log being built up by the rest of
/// `run_fetch`; we append `RewindRestored` /
/// `RewindDetectedButRestoreFailed` entries so the user sees
/// the restore as a per-bookmark row in the fetch summary.
pub fn apply_rewind_protection(
    jj: &JjCli,
    workspace_root: &Path,
    snapshots: &BTreeMap<String, RewindSnapshot>,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    if opts.dry_run || snapshots.is_empty() {
        return Ok(());
    }
    let step = crate::ui::Step::start("Checking for silent rewinds across the fetch", verbosity);

    // Read current post-pipeline state once. Anything missing
    // from this list is a deletion (handled by other phases),
    // and we don't try to "restore" it here.
    let post = match jj::list_local_bookmarks_with_changes(jj) {
        Ok(b) => b,
        Err(e) => {
            step.warn(&format!("couldn't read post-fetch bookmarks: {e}"), None);
            return Ok(());
        }
    };
    let post_by_name: BTreeMap<String, &jj::LocalBookmarkWithChange> =
        post.iter().map(|b| (b.name.clone(), b)).collect();

    let mut restored = 0usize;
    let mut warnings: Vec<String> = Vec::new();
    for (name, snap) in snapshots {
        let Some(post_entry) = post_by_name.get(name) else {
            // Bookmark went away during fetch. If the snapshot
            // showed local-only work (pre_commit ≠
            // origin_baseline_commit), the deletion silently lost
            // that work — recreate the bookmark at the snapshot's
            // pre_commit. Otherwise it's a legitimate cleanup
            // (merged PR / gt sync) and we leave it alone.
            let had_local_work = match &snap.origin_baseline_commit {
                Some(baseline) => &snap.pre_commit != baseline,
                // No `@origin` baseline → bookmark was never pushed.
                // Any commit it pointed at was local-only by
                // definition.
                None => true,
            };
            if !had_local_work {
                continue;
            }
            let local_for_action = LocalBookmark {
                name: name.clone(),
                // Bookmark didn't exist after fetch — empty commit_id
                // here just means "currently absent"; the action
                // printer's `DeletionRestored` arm doesn't read it.
                commit_id: String::new(),
            };
            match jj::bookmark_set(jj, name, &snap.pre_commit) {
                Ok(()) => {
                    restored += 1;
                    actions.push((
                        local_for_action,
                        CleanupAction::DeletionRestored {
                            pre_commit: snap.pre_commit.clone(),
                        },
                    ));
                    warnings.push(format!(
                        "{name}: bookmark was deleted with local-only work; recreated at {}",
                        &snap.pre_commit[..snap.pre_commit.len().min(12)],
                    ));
                }
                Err(e) => {
                    warnings.push(format!(
                        "{name}: bookmark was deleted with local-only work AND recreate failed ({e}); run `jj bookmark set {name} -r {}` manually",
                        &snap.pre_commit[..snap.pre_commit.len().min(12)],
                    ));
                    actions.push((
                        local_for_action,
                        CleanupAction::DeletionDetectedButRestoreFailed {
                            pre_commit: snap.pre_commit.clone(),
                            message: format!("{e}"),
                        },
                    ));
                }
            }
            continue;
        };
        let outcome = classify_rewind_protection(
            snap,
            &post_entry.commit_id,
            &post_entry.change_id,
            |a, b| jj::is_ancestor(workspace_root, a, b),
        );
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                warnings.push(format!("{name}: classify failed: {e}"));
                continue;
            }
        };
        match outcome {
            RewindProtectionOutcome::NoChange
            | RewindProtectionOutcome::ForwardMove
            | RewindProtectionOutcome::InPlaceRewrite
            | RewindProtectionOutcome::NoLocalWork => {
                // Nothing to do.
            }
            RewindProtectionOutcome::Rewound | RewindProtectionOutcome::Divergent => {
                // The bookmark carried local-only work and is now
                // pointing at an ancestor (or divergent commit).
                // Restore.
                let local_for_action = LocalBookmark {
                    name: name.clone(),
                    commit_id: post_entry.commit_id.clone(),
                };
                match jj::bookmark_set(jj, name, &snap.pre_commit) {
                    Ok(()) => {
                        restored += 1;
                        actions.push((
                            local_for_action,
                            CleanupAction::RewindRestored {
                                pre_commit: snap.pre_commit.clone(),
                                post_commit: post_entry.commit_id.clone(),
                            },
                        ));
                    }
                    Err(e) => {
                        warnings.push(format!("{name}: restore failed: {e}"));
                        actions.push((
                            local_for_action,
                            CleanupAction::RewindDetectedButRestoreFailed {
                                pre_commit: snap.pre_commit.clone(),
                                post_commit: post_entry.commit_id.clone(),
                                message: format!("{e}"),
                            },
                        ));
                    }
                }
            }
        }
    }

    let summary = match (restored, warnings.is_empty()) {
        (0, true) => "no rewinds detected".to_owned(),
        (n, true) => format!("{n} restored"),
        (n, false) => format!("{n} restored, {} warning(s)", warnings.len()),
    };
    if warnings.is_empty() {
        step.success(&summary, None);
    } else {
        step.warn(&summary, Some(&warnings.join("\n")));
    }
    Ok(())
}

/// Snapshot of the bookmark graph captured BEFORE `jj git fetch`
/// runs. Pre-fetch timing is load-bearing: jj's fetch propagates
/// remote-side `[deleted]` refs to local tracked bookmarks — so
/// any merged-PR bookmark whose remote ref was just deleted by the
/// GitHub-side squash-merge will be gone from the local list by
/// the time we'd otherwise compute the cleanup state.
///
/// Capturing the pre-fetch state lets us still:
///
///   - derive each bookmark's parent (in-stack relationship)
///     while the parent bookmark still exists locally;
///   - report "what happened to your N bookmarks" with full
///     coverage in the per-bookmark actions output;
///   - feed the orphan-rebase logic the full pre-fetch name set so
///     a fetch-deleted parent counts as a deletion just like a
///     sync-deleted one.
#[derive(Debug, Clone)]
pub struct PreFetchSnapshot {
    /// `gtmq_*`-shaped bookmarks (filtered out of the normal flow).
    pub gtmq: Vec<LocalBookmark>,
    /// Non-gtmq bookmarks; the set the rest of the pipeline
    /// operates on.
    pub normal: Vec<LocalBookmark>,
    /// PR info batched for the normal set. `find_prs_for_branches`
    /// returns an empty vec for an empty input, so this is empty
    /// when `normal` is empty.
    pub normal_prs: Vec<PrInfo>,
    /// `(child → parent)` edges derived against the pre-fetch
    /// graph. Critical: derive_parents MUST run while the parent
    /// bookmarks are still local; computing this post-fetch loses
    /// any edge whose parent bookmark was deleted in the
    /// meantime.
    pub stacked: Vec<StackedBookmark>,
}

impl PreFetchSnapshot {
    /// Set of normal bookmark names — convenience for the
    /// orphan-rebase phase that compares before-vs-after for
    /// deleted-during-pipeline detection.
    pub fn normal_names(&self) -> BTreeSet<String> {
        self.normal.iter().map(|b| b.name.clone()).collect()
    }
}

/// 0c. Capture the pre-fetch bookmark graph + PR lookup.
///
/// Calls `gh` once to batch all PR lookups. Tests that want to
/// exercise the rest of the pipeline without `gh` use
/// [`snapshot_pre_fetch_with_prs`] to bypass the lookup.
pub fn snapshot_pre_fetch(
    jj: &JjCli,
    workspace_root: &Path,
    opts: &FetchOpts,
) -> Result<PreFetchSnapshot> {
    let all = list_local_bookmarks(jj)?;
    let (gtmq, normal): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|b| is_gtmq_branch(&b.name, &opts.gtmq_prefixes));

    let normal_prs = if normal.is_empty() {
        Vec::new()
    } else {
        let names: Vec<String> = normal.iter().map(|b| b.name.clone()).collect();
        gh::find_prs_for_branches(workspace_root, &names, 200)?
    };

    snapshot_pre_fetch_with_prs(jj, opts, gtmq, normal, normal_prs)
}

/// `gh`-free variant of [`snapshot_pre_fetch`] — used by unit
/// tests that want to pin the snapshot's derive_parents output
/// without standing up a fake GitHub. Production code goes
/// through [`snapshot_pre_fetch`].
pub fn snapshot_pre_fetch_with_prs(
    jj: &JjCli,
    opts: &FetchOpts,
    gtmq: Vec<LocalBookmark>,
    normal: Vec<LocalBookmark>,
    normal_prs: Vec<PrInfo>,
) -> Result<PreFetchSnapshot> {
    let stacked = derive_parents_lossy(
        jj,
        &normal.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        &opts.trunk,
    );
    Ok(PreFetchSnapshot {
        gtmq,
        normal,
        normal_prs,
        stacked,
    })
}

/// Snapshot of the bookmark graph after `jj git fetch`. The
/// rewind detector uses this as its "before sync" baseline so a
/// legitimate fetch-side move of a tracked bookmark (someone else
/// pushed) doesn't get reverted; the backfill phase uses it to
/// skip `gt track` calls for bookmarks fetch already deleted.
#[derive(Debug, Clone)]
pub struct PostFetchSnapshot {
    pub all: Vec<LocalBookmark>,
}

impl PostFetchSnapshot {
    pub fn names(&self) -> BTreeSet<String> {
        self.all.iter().map(|b| b.name.clone()).collect()
    }
}

pub fn snapshot_post_fetch(jj: &JjCli) -> Result<PostFetchSnapshot> {
    Ok(PostFetchSnapshot {
        all: list_local_bookmarks(jj)?,
    })
}

/// 0c. Export jj's bookmark view into git's loose refs before the
/// `jj git fetch` step. See the call site comment in `run_fetch`
/// for the failure mode this defends against — short version:
/// without this, fetch's auto-import treats git's stale ref values
/// as canonical and clobbers any bookmark moves made in another
/// workspace via `jj bookmark set` that haven't been pushed yet.
///
/// Idempotent — `jj git export` is a no-op when JJ's view already
/// matches git's loose refs. Cheap (no network, no graph walk),
/// so we don't gate it on detecting a divergence.
///
/// Skip in dry-run (no ref mutations allowed) and when `no_export`
/// was explicitly set (caller wants to inspect the un-exported
/// state).
pub fn maybe_export_before_fetch(
    jj: &JjCli,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
) -> Result<()> {
    if opts.dry_run || opts.no_export {
        return Ok(());
    }
    let step = crate::ui::Step::start("Exporting jj bookmarks to git refs", verbosity);
    match jj::git_export(jj) {
        Ok(()) => {
            step.success("done", None);
            Ok(())
        }
        Err(e) => {
            step.fail(&format!("{e}"), None);
            Err(e)
        }
    }
}

/// `jj git fetch`.
fn do_fetch(jj: &JjCli, opts: &FetchOpts, verbosity: crate::ui::Verbosity) -> Result<()> {
    let step = crate::ui::Step::start(&format!("Fetching from {}", opts.remote), verbosity);
    if opts.dry_run {
        step.skip("dry-run", None);
        return Ok(());
    }
    match jj::git_fetch(jj, &opts.remote) {
        Ok(()) => {
            step.success("", None);
            Ok(())
        }
        Err(e) => {
            step.fail(&format!("{e}"), None);
            Err(e)
        }
    }
}

/// Decide whether a single `StackedBookmark` is eligible for the
/// backfill phase's `gt track` call. Three conditions:
///
///   1. has an open PR (`gh` told us about it),
///   2. still exists locally after fetch (gt track on a missing
///      branch errors with "branch not found"),
///   3. its recorded parent ALSO still exists locally after fetch.
///      The pre-fetch snapshot intentionally preserves edges to
///      parents that may disappear during `jj git fetch` (so
///      `orphan_rebase_phase` can detect and repair them
///      downstream), but `gt track` fails hard on a missing parent
///      — and that would abort the whole pipeline before the
///      repair phase ever runs.
///
/// Decide whether a bookmark needs `gt track` re-asserted as
/// part of the post-fetch backfill phase.
///
/// Two cases produce `true`:
///
///   1. **Intact chain** — the bookmark has a PR, both it and
///      its parent are present in `post_names`, and the parent
///      isn't in the fetch-deleted set. Standard backfill.
///   2. **Parent fetch-deleted** — the bookmark has a PR, it's
///      present in `post_names`, but its pre-fetch parent was
///      deleted on the remote (a merged-PR cleanup that fetch
///      propagated). The caller will substitute trunk as the
///      parent for the `gt track` call; mirror what
///      `orphan_rebase_phase` will do to the commit a few steps
///      later. Without this fallback the child's
///      `gt track --parent <deleted-bookmark>` aborts the
///      pipeline before the orphan-rebase repair can run.
///
/// Filters out:
///   - bookmarks without a PR (gt has nothing to bind to)
///   - bookmarks deleted during fetch (gone from `post_names`)
///   - bookmarks whose parent is missing AND not in the deleted
///     set (probably renamed or otherwise reshape — leave to
///     manual intervention rather than guess)
///
/// Pure function so unit tests can exhaustively cover the truth
/// table without spinning up a workspace or stubbing `gt`.
pub fn is_backfill_target(
    sb: &StackedBookmark,
    normal_prs: &[PrInfo],
    post_names: &BTreeSet<String>,
    deleted: &BTreeSet<String>,
) -> bool {
    let has_pr = normal_prs.iter().any(|p| p.head_ref_name == sb.name);
    let child_present = post_names.contains(&sb.name);
    let parent_resolvable = match &sb.parent {
        BookmarkOrTrunk::Trunk => true,
        BookmarkOrTrunk::Bookmark(parent) => {
            // Two ways the parent is resolvable: it's still
            // around post-fetch, or it was deleted on the remote
            // (in which case the caller will substitute trunk
            // for the gt track invocation).
            post_names.contains(parent) || deleted.contains(parent)
        }
    };
    has_pr && child_present && parent_resolvable
}

/// Compute the concrete bookmark name to pass to
/// `gt track --parent` for `sb`. Returns `trunk` when the
/// recorded parent was deleted during fetch (merged PR, post-
/// merge cleanup propagated by `jj git fetch`); returns the
/// recorded parent name otherwise.
///
/// Called only from `backfill_phase`; surfaced here as a named
/// helper so the substitution rule has a single home and the
/// unit tests can pin it.
pub fn effective_backfill_parent(
    sb: &StackedBookmark,
    deleted: &BTreeSet<String>,
    trunk: &str,
) -> String {
    match &sb.parent {
        BookmarkOrTrunk::Trunk => trunk.to_owned(),
        BookmarkOrTrunk::Bookmark(parent) => {
            if deleted.contains(parent) {
                trunk.to_owned()
            } else {
                parent.clone()
            }
        }
    }
}

/// Backfill gt tracking metadata for bookmarks that have a PR.
///
/// Filters to bookmarks (a) with a PR, (b) still present locally
/// after fetch, (c) whose recorded parent is either still present
/// or was deleted in this fetch cycle, AND (d) that gt is already
/// tracking. The deleted-parent case substitutes trunk as the
/// `gt track --parent` argument — see
/// [`effective_backfill_parent`] and [`is_backfill_target`] for
/// the rationale. Without this substitution a child whose merged-
/// PR parent just got cleaned up would abort the pipeline because
/// `gt` rejects `--parent <untracked-bookmark>` before the
/// orphan-rebase phase gets a chance to repair the graph.
///
/// The "gt already tracks it" gate (d) keeps backfill from
/// opportunistically auto-tracking bookmarks the user pulled down
/// to review — e.g. another engineer's PR that happens to have
/// `head:branch` matching the PR-list search. First-time tracking
/// only happens via the explicit `jj-gt submit` / `jj-gt track`
/// flow, which the user opted into by name.
fn backfill_phase(
    workspace_root: &Path,
    pre: &PreFetchSnapshot,
    post: &PostFetchSnapshot,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
) -> Result<()> {
    if opts.no_backfill {
        return Ok(());
    }
    let post_names = post.names();
    // Same `deleted` set the orphan-rebase phase computes. Need
    // it here so the deleted-parent substitution can fire.
    let deleted = compute_deleted_set(&pre.normal_names(), &post_names);
    // Scope auto-tracking to bookmarks gt already knows about. A
    // gt enumeration failure (gt not installed, repo not gt-init'd)
    // is treated as "gt knows nothing" — the backfill loop then
    // skips everything, which is the right conservative answer
    // for a workspace without gt set up.
    let gt_known = gt::list_tracked_branches(workspace_root).unwrap_or_else(|e| {
        tracing::warn!("jj-gt: couldn't enumerate gt-tracked branches ({e}); skipping backfill");
        std::collections::BTreeSet::new()
    });
    let stacked = crate::stack::sort_for_tracking(&pre.stacked);
    let backfill_targets: Vec<_> = stacked
        .iter()
        .filter(|sb| {
            is_backfill_target(sb, &pre.normal_prs, &post_names, &deleted)
                && gt_known.contains(&sb.name)
        })
        .collect();
    if backfill_targets.is_empty() {
        let step = crate::ui::Step::start("Backfilling gt tracking metadata", verbosity);
        step.skip("no bookmarks with PRs that gt is tracking", None);
        return Ok(());
    }
    for sb in backfill_targets {
        let parent = effective_backfill_parent(sb, &deleted, &opts.trunk);
        let step = crate::ui::Step::start(
            &format!("Backfilling gt track for {} (parent: {parent})", sb.name),
            verbosity,
        );
        if opts.dry_run {
            step.skip("dry-run", None);
        } else {
            match gt::track(workspace_root, &sb.name, &parent) {
                Ok(()) => step.success("", None),
                Err(e) => {
                    step.fail(&format!("{e}"), None);
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/// Classify each normal bookmark (drift / cleanup decision).
///
/// Pre-PR classification happens against the pre-fetch snapshot so
/// merged-PR bookmarks that fetch will-or-already-has deleted still
/// show up in the per-bookmark actions output.
fn classify_phase(
    jj: &JjCli,
    pre: &PreFetchSnapshot,
    opts: &FetchOpts,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    for local in &pre.normal {
        let pr = pre
            .normal_prs
            .iter()
            .find(|p| p.head_ref_name == local.name);
        let marker = match pr {
            Some(pr) if pr.state.is_terminal() => {
                jj::find_pr_merge_marker_on_trunk(jj, pr.number, &opts.trunk)?
            }
            _ => None,
        };
        let action = classify_local_bookmark(local, pr, marker.as_deref());
        actions.push((local.clone(), action));
    }
    Ok(())
}

/// Run `gt sync --no-restack`, re-import, and run the
/// rewind detector against a post-fetch baseline so a legitimate
/// fetch-side move of a tracked bookmark isn't classified as
/// "silent rewind" (the detector's scope is strictly "what
/// changed during gt sync").
///
/// Dry-run short-circuits the whole block — no sync, no import,
/// no detection.
fn sync_and_rewind_phase(
    jj: &JjCli,
    workspace_root: &Path,
    pre: &PreFetchSnapshot,
    post: &PostFetchSnapshot,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    use std::collections::BTreeMap;

    let sync_step = crate::ui::Step::start("Running gt sync --no-restack", verbosity);
    if opts.dry_run {
        sync_step.skip("dry-run", None);
        return Ok(());
    }

    // PR-D / issue #2: snapshot every local bookmark's commit id
    // AFTER jj git fetch but before gt sync mutates refs.
    // Compared post-import below to detect silent rewinds (gt
    // sync with --force resets a local ref to remote when local
    // was ahead — no prompt, no warning).
    //
    // The baseline is intentionally post-fetch, not pre-fetch:
    // a legitimate `jj git fetch` of an updated remote moves
    // the tracked local bookmark to match the new remote head.
    // Treating that as "silent rewind" would revert genuine
    // remote progress.
    let pre_snapshot: BTreeMap<String, String> = post
        .all
        .iter()
        .filter(|b| !is_gtmq_branch(&b.name, &opts.gtmq_prefixes))
        .map(|b| (b.name.clone(), b.commit_id.clone()))
        .collect();

    match gt::sync_no_restack(workspace_root) {
        Ok(()) => sync_step.success("", None),
        Err(e) => {
            sync_step.fail(&format!("{e}"), None);
            return Err(e);
        }
    }
    let import_step = crate::ui::Step::start("Importing remote refs into jj", verbosity);
    match jj::git_import(jj) {
        Ok(()) => import_step.success("", None),
        Err(e) => {
            import_step.fail(&format!("{e}"), None);
            return Err(e);
        }
    }

    // Post-snapshot + rewind detection. Only act on bookmarks
    // present in pre_snapshot — anything new in post is a
    // remote-side branch we don't want to touch.
    let post_bookmarks = list_local_bookmarks(jj)?;
    let post_by_name: BTreeMap<String, String> = post_bookmarks
        .iter()
        .map(|b| (b.name.clone(), b.commit_id.clone()))
        .collect();

    let rewind_step = crate::ui::Step::start("Checking for silent rewinds from gt sync", verbosity);
    // PR #77 follow-up: this phase used to also `jj bookmark set`
    // bookmarks classified as `Rewound`/`Diverged` here. That
    // restore preempted the snapshot-based protection in
    // `apply_rewind_protection`, which uses the richer
    // change_id + origin_baseline rules from
    // `classify_rewind_protection` (see Rule 2 In-place rewrite
    // and Rule 3 No-local-work) and is the authoritative
    // restoration path. The classify-by-pre/post logic here
    // would, for example, treat a legitimate remote-side rewrite
    // that landed between fetch and gt sync as a "Rewound" and
    // pin the bookmark back to the stale post-fetch commit —
    // and then `apply_rewind_protection` would see the bookmark
    // back at `pre_commit` (since we just restored it) and
    // wouldn't notice the mistake.
    //
    // The classify_rewind classification is still useful for
    // surfacing what happened, but the destructive restore is
    // now deferred to `apply_rewind_protection`. Surface the
    // classification as informational actions only.
    let mut detected_rewound = 0usize;
    let mut detected_diverged = 0usize;
    let mut rewind_errors: Vec<String> = Vec::new();
    for (name, pre_id) in &pre_snapshot {
        let post_id = post_by_name.get(name).map(|s| s.as_str());
        let classification = match classify_rewind(pre_id, post_id, |a, b| {
            jj::is_ancestor(workspace_root, a, b)
        }) {
            Ok(c) => c,
            Err(e) => {
                rewind_errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        match classification {
            RewindClassification::Unchanged | RewindClassification::FastForward => {}
            RewindClassification::Rewound => {
                detected_rewound += 1;
                let local = pre
                    .normal
                    .iter()
                    .find(|b| &b.name == name)
                    .cloned()
                    .unwrap_or_else(|| LocalBookmark {
                        name: name.clone(),
                        commit_id: pre_id.clone(),
                    });
                actions.push((
                    local,
                    CleanupAction::RestoredAfterRewind {
                        pre: pre_id.clone(),
                        post: post_id.unwrap_or("").to_owned(),
                    },
                ));
            }
            RewindClassification::Diverged => {
                detected_diverged += 1;
                let local = pre
                    .normal
                    .iter()
                    .find(|b| &b.name == name)
                    .cloned()
                    .unwrap_or_else(|| LocalBookmark {
                        name: name.clone(),
                        commit_id: pre_id.clone(),
                    });
                actions.push((
                    local,
                    CleanupAction::DivergedFromRemote {
                        pre: pre_id.clone(),
                        post: post_id.unwrap_or("").to_owned(),
                    },
                ));
            }
            RewindClassification::Disappeared => {
                // gt sync deleted the bookmark. Two sub-cases:
                //
                //   - Intentional cleanup (merged PR): the
                //     orphan-detection phase will either rebase
                //     descendants or leave them alone via its own
                //     classifier. Don't resurrect the bookmark
                //     here — that would undo gt sync's legitimate
                //     work.
                //
                //   - Unintentional (local-only commits ahead of
                //     remote): we can't tell from pre/post alone
                //     without consulting the original remote
                //     ref. The most common shape — local + remote
                //     both at the merge commit pre-sync, gt sync
                //     removes both — is benign; the rare bad
                //     case (local ahead with un-pushed work)
                //     would show up as orphan descendants if any
                //     exist, which the orphan-rebase phase
                //     handles.
                //
                // Leaving Disappeared as a no-op here is the
                // conservative choice; it matches the behavior
                // before PR-D landed and is fixed forward by the
                // orphan-rebase phase.
            }
        }
    }
    let summary = match (
        detected_rewound,
        detected_diverged,
        rewind_errors.is_empty(),
    ) {
        (0, 0, true) => "no rewinds detected".to_owned(),
        (r, 0, true) => format!(
            "{r} bookmark(s) flagged as rewound; restore deferred to apply_rewind_protection"
        ),
        (0, d, true) => format!(
            "{d} bookmark(s) flagged as diverged; restore deferred to apply_rewind_protection"
        ),
        (r, d, true) => {
            format!("{r} rewound, {d} diverged; restore deferred to apply_rewind_protection")
        }
        (_, _, false) => format!("{} error(s)", rewind_errors.len()),
    };
    if rewind_errors.is_empty() {
        rewind_step.success(&summary, None);
    } else {
        rewind_step.warn(&summary, Some(&rewind_errors.join("\n")));
    }
    Ok(())
}

/// gtmq_* pruning.
fn gtmq_prune_phase(
    jj: &JjCli,
    workspace_root: &Path,
    pre: &PreFetchSnapshot,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    if opts.no_gtmq_prune || pre.gtmq.is_empty() {
        return Ok(());
    }
    let gtmq_prs = gh::list_prs_by_head_prefix(workspace_root, &opts.gtmq_prefixes, 500)?;
    for branch in &pre.gtmq {
        let pr = gtmq_prs.iter().find(|p| p.head_ref_name == branch.name);
        let action = classify_gtmq_branch(pr);
        if let CleanupAction::GtmqPruned { .. } = action {
            let step =
                crate::ui::Step::start(&format!("Pruning gtmq branch {}", branch.name), verbosity);
            if opts.dry_run {
                step.skip("dry-run", None);
            } else {
                // Capture both delete results so a partial failure
                // (local deleted, remote refused — or vice versa)
                // surfaces as a warn-level step instead of a
                // success message claiming "deleted local + remote"
                // when only one side actually moved. We do NOT
                // `?`-propagate either error: a gtmq branch
                // failing to delete shouldn't abort the whole
                // fetch pipeline. The user gets the warning, the
                // pipeline continues, and the action is still
                // recorded as `GtmqPruned` (the decision was
                // correct; only the execution was partial).
                let local_err = jj::delete_bookmark(jj, &branch.name).err();
                let remote_err =
                    jj::delete_remote_branch(workspace_root, &opts.remote, &branch.name).err();
                match (local_err, remote_err) {
                    (None, None) => step.success("deleted local + remote", None),
                    (Some(le), None) => step.warn(
                        "remote deleted, local delete failed",
                        Some(&format!("{le}")),
                    ),
                    (None, Some(re)) => step.warn(
                        "local deleted, remote delete failed",
                        Some(&format!("{re}")),
                    ),
                    (Some(le), Some(re)) => step.warn(
                        "both local + remote delete failed",
                        Some(&format!("local: {le}\nremote: {re}")),
                    ),
                }
            }
        }
        actions.push((branch.clone(), action));
    }
    Ok(())
}

/// Track-on-discovery sweep. Catches the wild bug shape where
/// a local bookmark has a matching `@<remote>` ref but isn't yet
/// tracked by jj — meaning the remote-side `[deleted]`
/// propagation jj would normally apply on the next fetch never
/// fires for it. After this phase, every locally-present
/// bookmark whose remote ref exists is also tracked, so the
/// standard jj propagation handles merge/delete cleanup
/// automatically on subsequent fetches.
///
/// The shape this addresses:
///
///   1. Agent (or human) created bookmark `X` and pushed it via
///      `jj git push --bookmark X`. jj does NOT auto-track the
///      remote ref on push, so there's no `X@<remote>` tracking
///      link.
///   2. PR merged via Graphite's merge queue → original branch
///      deleted on origin.
///   3. Next `jj git fetch` logs `[deleted] untracked` for
///      `X@<remote>` but doesn't propagate to local because no
///      tracking link exists.
///   4. `gt sync` skips `X` because it's not in gt's tracked
///      branch set.
///   5. Net: local `X` carries on pointing at the abandoned
///      pre-merge commit forever.
///
/// We address (5) at the SOURCE: track every local bookmark
/// that has a matching `@<remote>` ref. Once tracked, jj's
/// standard `git fetch` → "propagate remote-side deletion to
/// local" path takes over on the next fetch, and the bookmark
/// gets removed naturally — same flow that already works for
/// the bookmarks Matt explicitly tracked via
/// [`crate::jj::track_bookmark_on_remote`] after `jj-gt
/// submit`. No destructive sweep, no risk to pre-push WIP.
///
/// Pre-push WIP bookmarks (local-only, no `@<remote>` ref) are
/// SKIPPED by definition — they wouldn't have a remote ref to
/// track against, so the existence check filters them out
/// before we touch anything. The earlier `forget` design got
/// burned here: it auto-deleted a sibling-stack pre-push
/// bookmark because "no `@<remote>` + not tracked + not on
/// `::@`" matched both the orphan AND the WIP shape.
///
/// Safety filters:
///
///   - Trunk is excluded (already tracked by definition).
///   - `gtmq_*` queue branches are excluded.
///   - Bookmarks that ARE already tracked are skipped (no-op).
///   - Bookmarks whose `@<remote>` ref DOESN'T resolve are
///     skipped — those are pre-push WIP; we have nothing to
///     track them against.
///   - dry-run is a no-op.
pub fn orphan_untracked_phase(
    jj: &JjCli,
    pre: &PreFetchSnapshot,
    opts: &FetchOpts,
    verbosity: crate::ui::Verbosity,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    if opts.dry_run {
        return Ok(());
    }

    let step = crate::ui::Step::start(
        "Auto-tracking untracked bookmarks with remote refs",
        verbosity,
    );

    let tracked: BTreeSet<String> = match jj::list_tracked_bookmarks_on_remote(jj, &opts.remote) {
        Ok(s) => s,
        Err(e) => {
            step.warn(
                &format!(
                    "couldn't enumerate `@{}`-tracked bookmarks ({e}); skipping auto-track sweep",
                    opts.remote
                ),
                None,
            );
            return Ok(());
        }
    };

    // Re-read the live local bookmark set. This phase runs after
    // fetch/sync/prune, so `pre.normal` is potentially stale —
    // bookmarks could have been deleted by an earlier phase.
    // Acting on the stale snapshot would probe / track a name
    // that's no longer local AND log a success row for a
    // bookmark that doesn't exist anymore. Take the intersection
    // with the live list (and use the live commit_id when we
    // record the action) so the sweep stays accurate.
    let live: Vec<jj::LocalBookmark> = match jj::list_local_bookmarks(jj) {
        Ok(v) => v,
        Err(e) => {
            step.warn(
                &format!(
                    "couldn't enumerate live local bookmarks ({e}); skipping auto-track sweep"
                ),
                None,
            );
            return Ok(());
        }
    };
    let live_by_name: BTreeMap<String, &jj::LocalBookmark> =
        live.iter().map(|b| (b.name.clone(), b)).collect();

    let mut tracked_now = 0usize;
    let mut warnings: Vec<String> = Vec::new();
    let mut to_override: BTreeSet<String> = BTreeSet::new();

    for snap in &pre.normal {
        let Some(local) = live_by_name.get(&snap.name) else {
            // Bookmark went away during this fetch pipeline —
            // some earlier phase handled it. Skip; we have
            // nothing live to track or report on.
            continue;
        };
        if local.name == opts.trunk {
            continue;
        }
        if is_gtmq_branch(&local.name, &opts.gtmq_prefixes) {
            continue;
        }
        if tracked.contains(&local.name) {
            continue;
        }

        // Probe whether `@<remote>` exists. If it doesn't, this
        // is a pre-push WIP bookmark — skip; we have nothing to
        // track it against. This is the key guardrail that
        // protects sibling-stack WIP from being touched.
        match jj::resolve_commit_id(jj, &format!("{}@{}", local.name, opts.remote)) {
            Ok(_) => {
                // Remote ref exists — proceed to track.
            }
            Err(e) if is_revset_unresolved_error(&e) => {
                // No remote ref — pre-push WIP. Leave it alone.
                continue;
            }
            Err(e) => {
                warnings.push(format!("{}: probe failed: {e}", local.name));
                continue;
            }
        }

        match jj::track_bookmark_on_remote(jj, &local.name, &opts.remote) {
            Ok(()) => {
                tracked_now += 1;
                to_override.insert(local.name.clone());
                actions.push((
                    (*local).clone(),
                    CleanupAction::OrphanUntrackedTracked {
                        remote: opts.remote.clone(),
                        commit_id: local.commit_id.clone(),
                    },
                ));
            }
            Err(e) => {
                warnings.push(format!("{}: track failed: {e}", local.name));
            }
        }
    }

    // Replace any prior `LeftAlone` action for the newly-tracked
    // bookmarks so the summary table shows the more specific
    // tracking action instead.
    if !to_override.is_empty() {
        actions.retain(|(bm, action)| {
            !(matches!(action, CleanupAction::LeftAlone) && to_override.contains(&bm.name))
        });
    }

    let summary = match (tracked_now, warnings.is_empty()) {
        (0, true) => "none needed tracking".to_owned(),
        (n, true) => format!("{n} newly tracked"),
        (n, false) => format!("{n} newly tracked, {} warning(s)", warnings.len()),
    };
    if warnings.is_empty() {
        step.success(&summary, None);
    } else {
        step.warn(&summary, Some(&warnings.join("\n")));
    }
    Ok(())
}

/// Orphan-restack via `jj rebase` ONLY for bookmarks whose
/// tracked parent disappeared between the pre-fetch snapshot and
/// now.
///
/// The "disappeared" set covers both:
///
///   - Fetch-deletions: jj git fetch propagates remote-side
///     deletions (post-merge cleanup) to tracked local bookmarks.
///     By the time we get here those bookmarks are already gone
///     from the local list.
///   - Sync-deletions: gt sync --force deletes any merged-PR
///     bookmarks the fetch missed.
///
/// We capture `pre.stacked` + `pre.normal` BEFORE jj git fetch
/// (see [`snapshot_pre_fetch`]) precisely so both deletion
/// categories show up in `before - remaining` as one set.
///
/// The naive "rebase every remaining bookmark" approach we used
/// to do here rebases unrelated bookmarks (any time they happened
/// to live on a non-trunk commit) and was the source of the bug
/// where `jj-gt fetch` would surprise-rebase an in-flight
/// unrelated stack entry and sometimes introduce conflicts.
/// 7. Rebase orphaned descendants of removed bookmarks onto
/// trunk so the stack doesn't dangle on a fetch-deleted parent.
///
/// Snapshots the op id before each rebase and rolls back via
/// `jj op restore` if the rebase introduces conflicts — fetch
/// should never leave the user with conflict markers from a
/// mutation they didn't ask for. Conflict-deferred bookmarks
/// surface as `RebaseDeferredForConflict` in the per-bookmark
/// summary table; the user runs `jj-gt restack` when they're
/// ready to resolve.
pub fn orphan_rebase_phase(
    jj: &JjCli,
    pre: &PreFetchSnapshot,
    post: &PostFetchSnapshot,
    opts: &FetchOpts,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    if opts.no_rebase || opts.dry_run {
        return Ok(());
    }
    let remaining = list_local_bookmarks(jj)?;

    // Issue #68: query the set of conflicted bookmarks once up
    // front. If a candidate bookmark turns out to be conflicted,
    // we surface that as a distinct action rather than blowing up
    // mid-rebase with the cryptic "Name `<bm>` is conflicted"
    // error.
    let conflicted = jj::list_conflicted_bookmarks(jj)?;

    // `list_local_bookmarks` filters out conflicted bookmarks
    // (they have no `normal_target()` commit id — see
    // `tools/jj-gt/src/jj.rs` Lines 98-104). If we computed
    // `deleted` from just `remaining_names`, a still-present but
    // conflicted parent would land in `deleted` and its children
    // would get misclassified as orphaned + rebased onto trunk.
    // Fold conflicted names back in before the deleted-set
    // computation so conflict ≠ deletion.
    let remaining_names: BTreeSet<String> = remaining
        .iter()
        .map(|b| b.name.clone())
        .chain(conflicted.iter().cloned())
        .collect();
    let deleted = compute_deleted_set(&pre.normal_names(), &remaining_names);

    // Issue #69 (Shape A): compute the moved-sideways set —
    // bookmarks present in both pre and post-FETCH snapshots whose
    // commit_id changed AND whose post commit-id is NOT a
    // fast-forward of the pre commit-id. A fast-forward isn't a
    // re-anchor signal (existing ancestry still resolves), but a
    // divergent move (e.g. Graphite's pre-merge rebase) leaves
    // any downstream bookmark anchored to a now-orphan commit.
    //
    // We compare `pre.normal` against the post-FETCH snapshot
    // (`post.all`), NOT against `remaining` (which reflects state
    // AFTER `gt sync`, import, and prune have already mutated
    // local refs). If we used `remaining`, a later in-pipeline
    // mutation could mask or fake a remote sideways move and
    // either surprise-rebase the wrong child or skip a genuine
    // re-anchor.
    let workspace_root = jj.cwd();
    let moved_candidates = compute_moved_set(&pre.normal, &post.all);
    let mut sideways = std::collections::BTreeMap::<String, (String, String)>::new();
    for (name, (pre_commit, post_commit)) in &moved_candidates {
        match jj::is_ancestor(workspace_root, pre_commit, post_commit) {
            Ok(true) => {
                // Fast-forward — existing ancestry holds; nothing
                // for the re-anchor logic to do.
            }
            Ok(false) => {
                // Genuine sideways move.
                sideways.insert(name.clone(), (pre_commit.clone(), post_commit.clone()));
            }
            Err(e) => {
                // Couldn't classify (typically a bad/transient git
                // failure or a GC'd SHA). Skip re-anchoring for
                // this candidate — surprise-rebasing children
                // off an ambiguous result is worse than failing
                // to surface a genuine sideways move (the user
                // can run `jj-gt restack` to catch it).
                tracing::warn!(
                    "jj-gt: couldn't classify whether `{name}` moved sideways ({e}); skipping re-anchor detection",
                );
            }
        }
    }

    for sb in &pre.stacked {
        // Issue #68 robustness: check conflict BEFORE
        // `plan_orphan_rebase` so a conflicted bookmark that
        // didn't survive into `remaining_names` (jj's
        // `bookmark list` template emits an empty commit_id for
        // the conflicted target, which our parser filters out;
        // the `@git` line is what usually survives, but not
        // always) still surfaces as `BookmarkConflicted`. The
        // pre-rebase candidate test is "was this an orphan-
        // trigger candidate?" which is the parent-deleted check;
        // the bookmark's own survival is irrelevant to the
        // signal we need to surface.
        let parent_was_deleted = match &sb.parent {
            crate::stack::BookmarkOrTrunk::Bookmark(parent) => deleted.contains(parent),
            crate::stack::BookmarkOrTrunk::Trunk => false,
        };
        if parent_was_deleted && is_bookmark_conflicted(&sb.name, &conflicted) {
            // Construct a synthetic LocalBookmark for the action
            // log when the bookmark didn't survive into
            // `remaining`. Empty commit_id signals "couldn't
            // resolve" — the action printer doesn't render it.
            let local = remaining
                .iter()
                .find(|b| b.name == sb.name)
                .cloned()
                .unwrap_or_else(|| LocalBookmark {
                    name: sb.name.clone(),
                    commit_id: String::new(),
                });
            let prev_parent = match &sb.parent {
                crate::stack::BookmarkOrTrunk::Bookmark(p) => p.clone(),
                crate::stack::BookmarkOrTrunk::Trunk => continue,
            };
            actions.push((local, CleanupAction::BookmarkConflicted { prev_parent }));
            continue;
        }

        let Some(prev_parent) = plan_orphan_rebase(sb, &remaining_names, &deleted) else {
            continue;
        };
        // Confirmed orphan: rebase onto trunk.
        let local = remaining
            .iter()
            .find(|b| b.name == sb.name)
            .cloned()
            .expect("filtered by remaining_names above");

        // Look up the deleted parent's pre-sync commit id so we
        // can use it as the lower bound of the rebase revset.
        // The parent's bookmark is gone, but the commit object
        // sticks around until jj's GC runs.
        let parent_commit = pre
            .normal
            .iter()
            .find(|b| b.name == prev_parent)
            .map(|b| b.commit_id.clone());

        // Late-check: did `gt sync` / the import step already
        // move this bookmark past the deleted parent? When the
        // parent's PR merges, Graphite's pre-merge rebase pushes
        // the child bookmark forward onto the new tip of trunk;
        // the subsequent `jj git fetch` + import pulls that down,
        // so by the time orphan_rebase_phase runs the live
        // bookmark is no longer anchored to the deleted parent.
        // Rebasing it again is at best a no-op and at worst (if
        // the live position is on top of new trunk while the
        // deleted parent is now orphan-history) sweeps in
        // immutable trunk commits via the `parent_commit..bookmark`
        // range — `jj rebase` rejects the whole operation and
        // the action log surfaces a "Commit ... is immutable"
        // error the user has no way to act on.
        //
        // Skip the rebase when the deleted parent isn't an
        // ancestor of the live bookmark anymore.
        if let Some(parent_oid) = parent_commit.as_deref() {
            match jj::is_ancestor(jj.cwd(), parent_oid, &local.commit_id) {
                Ok(true) => { /* still anchored — proceed with rebase */ }
                Ok(false) => {
                    actions.push((
                        local,
                        CleanupAction::OrphanRebaseNoOpAlreadyAdvancedPastParent { prev_parent },
                    ));
                    continue;
                }
                Err(e) => {
                    // is_ancestor failed (transient git error,
                    // GC'd SHA, …). Falling back to the rebase
                    // path is the safe call — better to attempt
                    // and surface the rebase error than to skip
                    // a legitimate orphan rebase. Log the
                    // classification failure for the operator.
                    tracing::warn!(
                        "jj-gt: couldn't classify whether `{name}` was already advanced past deleted parent ({e}); attempting orphan-rebase anyway",
                        name = sb.name,
                    );
                }
            }
        }

        let rebase_revset = match parent_commit.as_deref() {
            Some(commit) => build_orphan_rebase_revset(commit, &sb.name, &opts.trunk),
            None => {
                // Defensive fallback. We snapshotted pre.normal
                // from list_local_bookmarks ourselves so a miss
                // here would mean the parent bookmark exists in
                // the stack graph but not in the bookmark list —
                // shouldn't happen, but if it does, fall back to
                // the bookmark-only revset and accept that multi-
                // commit stacks may only move their tip.
                sb.name.clone()
            }
        };

        // Snapshot the op id BEFORE the rebase so we can roll back
        // cleanly if the rebase produces conflicts. fetch should
        // never leave the user with conflict markers from a
        // mutation they didn't ask for — the deferred path puts the
        // ball back in their court via `jj-gt restack` (#61) or a
        // manual `jj rebase -d main@origin`.
        let pre_rebase_op = match jj::current_op_id(jj) {
            Ok(id) => Some(id),
            Err(e) => {
                // If we can't snapshot the op id we can't safely
                // restore — fall back to the legacy behaviour
                // (apply the rebase and surface conflicts if any).
                // Log + continue with the original codepath; the
                // user gets the same outcome as before the fix.
                tracing::warn!(
                    "jj-gt: couldn't snapshot op id pre-rebase ({e}); deferred-on-conflict disabled for this bookmark"
                );
                None
            }
        };

        match jj::rebase(jj, &rebase_revset, &opts.trunk) {
            Ok(jj::RebaseOutcome::Clean) | Ok(jj::RebaseOutcome::NoOp) => {
                actions.push((
                    local,
                    CleanupAction::Rebased {
                        onto: opts.trunk.clone(),
                        prev_parent,
                    },
                ));
            }
            Ok(jj::RebaseOutcome::Conflicted { message }) => {
                // The rebase succeeded as far as jj was concerned,
                // but the result has conflict markers. Roll back so
                // the user's working state stays untouched, and
                // mark the bookmark as deferred so it shows up in
                // the per-bookmark summary as "needs restack."
                if let Some(op_id) = pre_rebase_op.as_deref() {
                    match jj::op_restore(jj, op_id) {
                        Ok(()) => {
                            actions.push((
                                local,
                                CleanupAction::RebaseDeferredForConflict {
                                    onto: opts.trunk.clone(),
                                    prev_parent,
                                    message,
                                },
                            ));
                        }
                        Err(restore_err) => {
                            // Restore failed — the user is stuck with
                            // the conflicted rebase result. Surface
                            // both the original conflict and the
                            // restore failure so they know what to
                            // clean up.
                            let combined = format!(
                                "{message}; op_restore to roll back also failed: {restore_err}"
                            );
                            actions.push((
                                local,
                                CleanupAction::RebaseConflicted {
                                    onto: opts.trunk.clone(),
                                    prev_parent,
                                    message: combined,
                                },
                            ));
                        }
                    }
                } else {
                    // Pre-rebase op snapshot failed — can't restore,
                    // fall through to the legacy "conflicts left in
                    // working tree" surface.
                    actions.push((
                        local,
                        CleanupAction::RebaseConflicted {
                            onto: opts.trunk.clone(),
                            prev_parent,
                            message,
                        },
                    ));
                }
            }
            Err(e) => {
                // Hard rebase failure (e.g. immutable commit) —
                // surface as a conflicted action so it's at
                // least visible in the output rather than silently
                // swallowed. Nothing to roll back; jj already
                // errored before mutating anything.
                actions.push((
                    local,
                    CleanupAction::RebaseConflicted {
                        onto: opts.trunk.clone(),
                        prev_parent,
                        message: format!("jj rebase failed: {e}"),
                    },
                ));
            }
        }
    }

    // Issue #69 (Shape A): re-anchor children whose parent moved
    // sideways. We do this in a second pass after the deleted-
    // parent loop above so an orphan-rebased child doesn't get
    // double-rebased here. (The deleted-parent path already
    // rebases onto trunk; if the same bookmark also showed up
    // here we'd re-rebase, which is at best wasteful and at
    // worst introduces a fresh conflict.)
    if !sideways.is_empty() {
        reanchor_children_of_moved_parents(jj, pre, actions, &sideways, &conflicted, &opts.trunk)?;
    }

    Ok(())
}

/// Issue #69 (Shape A): re-anchor each `StackedBookmark` whose
/// parent appears in the moved-sideways set. The new rebase
/// destination is the parent's post-fetch commit; the revset is
/// the same `(parent_old, sb.name]` shape the orphan-deleted path
/// uses so multi-commit stacks move as a unit.
///
/// Conflict semantics mirror the deleted-parent path: snapshot
/// the op id, attempt the rebase, op-restore on conflict, emit
/// the corresponding deferred-vs-applied action.
fn reanchor_children_of_moved_parents(
    jj: &JjCli,
    pre: &PreFetchSnapshot,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
    sideways: &std::collections::BTreeMap<String, (String, String)>,
    conflicted: &BTreeSet<String>,
    trunk: &str,
) -> Result<()> {
    // Re-read post state since the deleted-parent loop above may
    // have mutated bookmark positions.
    let remaining = list_local_bookmarks(jj)?;
    // `list_local_bookmarks` deliberately filters out conflicted
    // bookmarks (they have no `normal_target()` commit id — see
    // `tools/jj-gt/src/jj.rs`). Folding `conflicted` back into
    // `remaining_names` here keeps a conflicted child from being
    // silently skipped by the `!remaining_names.contains(&sb.name)`
    // gate below — without the fold, the downstream
    // `is_bookmark_conflicted` check (and its
    // `CleanupAction::BookmarkConflicted` emit) never fires for
    // sideways-moved targets.
    let remaining_names: BTreeSet<String> = remaining
        .iter()
        .map(|b| b.name.clone())
        .chain(conflicted.iter().cloned())
        .collect();

    // Track names we already emitted an action for in *this*
    // call so we don't double-emit if a bookmark has more than
    // one parent edge entry (shouldn't happen given the dedup
    // upstream, but defensive).
    let mut already_emitted = BTreeSet::<String>::new();

    for sb in &pre.stacked {
        // Only consider bookmark→bookmark edges; trunk parents
        // can't move sideways in the same sense.
        let parent_name = match &sb.parent {
            crate::stack::BookmarkOrTrunk::Bookmark(p) => p.clone(),
            crate::stack::BookmarkOrTrunk::Trunk => continue,
        };

        let Some((pre_parent_commit, post_parent_commit)) = sideways.get(&parent_name) else {
            continue;
        };

        // Parent moved sideways on remote AND has since been
        // deleted post-sync (the narrow race window: Graphite's
        // pre-merge rebase pushed a new OID, the merged PR's
        // branch deletion got picked up by the same `gt sync`
        // run). The orphan-rebase loop above has already
        // re-anchored this child onto trunk via the
        // parent-deleted code path; falling through here would
        // fire a SECOND rebase that moves the child back off
        // trunk onto the (now abandoned) sideways commit. Bail
        // — the deleted-parent loop is the right handler for
        // this case.
        if !remaining_names.contains(&parent_name) {
            continue;
        }

        // Skip if the child bookmark isn't around anymore (a
        // previous phase deleted it or it never existed in
        // post-fetch state).
        if !remaining_names.contains(&sb.name) {
            continue;
        }

        // Skip names we already processed (defensive dedup).
        if already_emitted.contains(&sb.name) {
            continue;
        }
        already_emitted.insert(sb.name.clone());

        let local = remaining
            .iter()
            .find(|b| b.name == sb.name)
            .cloned()
            // Conflicted bookmarks live in `remaining_names` (via
            // the chain fold above) but NOT in `remaining` itself
            // (jj's template filters them out). Synthesize a
            // placeholder so the BookmarkConflicted action below
            // still emits — the empty `commit_id` is harmless for
            // the action printer's conflicted arm.
            .unwrap_or_else(|| LocalBookmark {
                name: sb.name.clone(),
                commit_id: String::new(),
            });

        // Issue #68: if the candidate bookmark name itself is
        // conflicted, surface as BookmarkConflicted (same
        // reasoning as the deleted-parent loop).
        if is_bookmark_conflicted(&sb.name, conflicted) {
            actions.push((
                local,
                CleanupAction::BookmarkConflicted {
                    prev_parent: parent_name,
                },
            ));
            continue;
        }

        // Rebase the (old-parent-commit, sb.name] range onto the
        // new parent commit. Using the explicit pre-fetch parent
        // commit as the lower bound — the new commit's history
        // is divergent, so a name-based revset would either
        // include unrelated commits or miss commits the user
        // had on top.
        let rebase_revset = build_orphan_rebase_revset(pre_parent_commit, &sb.name, trunk);
        let dest = post_parent_commit.clone();

        let pre_rebase_op = match jj::current_op_id(jj) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(
                    "jj-gt: couldn't snapshot op id pre-rebase ({e}); deferred-on-conflict disabled for sideways re-anchor of {}",
                    sb.name,
                );
                None
            }
        };

        match jj::rebase(jj, &rebase_revset, &dest) {
            Ok(jj::RebaseOutcome::Clean) | Ok(jj::RebaseOutcome::NoOp) => {
                actions.push((
                    local,
                    CleanupAction::RebasedAfterParentMoved {
                        parent_bookmark: parent_name,
                        pre_commit: pre_parent_commit.clone(),
                        post_commit: post_parent_commit.clone(),
                    },
                ));
            }
            Ok(jj::RebaseOutcome::Conflicted { message }) => {
                if let Some(op_id) = pre_rebase_op.as_deref() {
                    match jj::op_restore(jj, op_id) {
                        Ok(()) => {
                            // Clean rollback — workspace is back
                            // to pre-rebase state. Surface as a
                            // deferred action; user can resolve
                            // via `jj-gt restack`.
                            actions.push((
                                local,
                                CleanupAction::RebaseAfterParentMovedDeferred {
                                    parent_bookmark: parent_name,
                                    pre_commit: pre_parent_commit.clone(),
                                    post_commit: post_parent_commit.clone(),
                                    message,
                                },
                            ));
                        }
                        Err(restore_err) => {
                            // Rollback itself failed — workspace
                            // now has live conflict markers from
                            // the rebase we attempted. Surface as
                            // a live-conflicts action so the user
                            // runs `jj resolve` directly rather
                            // than the `jj-gt restack` path the
                            // Deferred message would suggest.
                            let combined = format!(
                                "{message}; op_restore to roll back also failed: {restore_err}"
                            );
                            actions.push((
                                local,
                                CleanupAction::RebaseAfterParentMovedConflicted {
                                    parent_bookmark: parent_name,
                                    pre_commit: pre_parent_commit.clone(),
                                    post_commit: post_parent_commit.clone(),
                                    message: combined,
                                },
                            ));
                        }
                    }
                } else {
                    // Pre-rebase op snapshot failed — we couldn't
                    // even attempt a rollback, so the workspace
                    // has live conflicts. Same surface as the
                    // restore-failed branch above.
                    actions.push((
                        local,
                        CleanupAction::RebaseAfterParentMovedConflicted {
                            parent_bookmark: parent_name,
                            pre_commit: pre_parent_commit.clone(),
                            post_commit: post_parent_commit.clone(),
                            message,
                        },
                    ));
                }
            }
            Err(e) => {
                // Hard rebase failure — jj errored before
                // mutating anything. No conflicts in the
                // workspace; surface as Deferred so the user can
                // retry via `jj-gt restack`.
                actions.push((
                    local,
                    CleanupAction::RebaseAfterParentMovedDeferred {
                        parent_bookmark: parent_name,
                        pre_commit: pre_parent_commit.clone(),
                        post_commit: post_parent_commit.clone(),
                        message: format!("jj rebase failed: {e}"),
                    },
                ));
            }
        }
    }

    Ok(())
}

/// Pure set-arithmetic helper for the orphan-rebase deleted-set.
/// Returns the set of names present in `before` but absent from
/// `after`. Made a named function so a unit test can pin the
/// semantics — `before - after` covers BOTH fetch-deletions and
/// sync-deletions because `before` is the pre-fetch snapshot and
/// `after` is the post-sync snapshot.
pub fn compute_deleted_set(
    before: &BTreeSet<String>,
    after: &BTreeSet<String>,
) -> BTreeSet<String> {
    before.difference(after).cloned().collect()
}

/// Pure conflict-membership check for the orphan-rebase path
/// (issue #68). Returns true if `name` appears in the
/// `conflicted` set jj reported. Made a named function so the
/// caller's intent is searchable from grep and so a tiny unit
/// test can pin the contract without needing a jj subprocess.
pub fn is_bookmark_conflicted(name: &str, conflicted: &BTreeSet<String>) -> bool {
    conflicted.contains(name)
}

/// Snapshot of a bookmark's state at fetch entry. Used by the
/// pipeline's rewind detector to recover from any step that
/// silently rewinds a bookmark with local-only work — see
/// [`classify_rewind_protection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindSnapshot {
    /// Bookmark's commit_id at the start of fetch.
    pub pre_commit: String,
    /// Bookmark's change_id at the start of fetch. Used to
    /// distinguish in-place rewrites (same change_id, new
    /// commit_id) from genuine rewinds (different change_id,
    /// post is ancestor of pre).
    pub pre_change_id: String,
    /// jj's view of the bookmark's `@<remote>` commit_id at
    /// the start of fetch — i.e. the remote-side baseline jj
    /// thinks the bookmark is tracking. `None` when there's no
    /// remote-tracking ref (e.g. bookmark was created locally
    /// and never pushed).
    pub origin_baseline_commit: Option<String>,
}

/// Classification produced by [`classify_rewind_protection`]
/// describing whether a bookmark needs auto-restore after the
/// fetch pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindProtectionOutcome {
    /// Bookmark commit_id didn't change. No action.
    NoChange,
    /// Bookmark moved forward (post is descendant of pre).
    /// Common case — agent advanced past the snapshot, or fetch
    /// pulled a real remote advance. No action.
    ForwardMove,
    /// Same logical change (change_id matches) — agent ran an
    /// in-place rewrite (jj describe / jj squash). No action.
    InPlaceRewrite,
    /// Pre was at origin's baseline — the bookmark didn't
    /// carry local-only work, so even a "rewind" doesn't lose
    /// anything. No action. Also covers the legitimate sideways
    /// case where origin was force-pushed (Graphite pre-merge
    /// rebase): pre matched the OLD origin, post matches the
    /// NEW origin, and the orphan-rebase phase handles
    /// downstream children separately.
    NoLocalWork,
    /// Post is a strict ancestor of pre AND pre had local-only
    /// work AND change_id differs. Restore.
    Rewound,
    /// Pre and post share no ancestor relationship AND
    /// change_id differs AND pre had local-only work.
    /// Probably divergent move. Restore.
    Divergent,
}

/// Decide whether `pre_snapshot`'s recorded position should be
/// auto-restored over the current `post_commit_id` /
/// `post_change_id`.
///
/// The five rules, in order:
///
///   1. **No change**: post_commit == pre_commit → `NoChange`.
///   2. **In-place rewrite**: post_change_id == pre_change_id
///      → `InPlaceRewrite`. Agent ran `jj describe` / `jj squash`
///      against the same logical change; bookmark legitimately
///      points at the rewritten commit.
///   3. **No local-only work**: pre_commit == origin_baseline →
///      `NoLocalWork`. Bookmark wasn't ahead of origin pre-fetch,
///      so any "rewind" can't lose unpushed work. Avoids fighting
///      with the orphan-rebase / moved-sideways paths over
///      legitimate remote advances.
///   4. **Forward move**: `is_ancestor(pre, post)` returns Ok(true)
///      → `ForwardMove`. Agent advanced past the snapshot or
///      fetch fast-forwarded.
///   5. **Otherwise**: `Rewound` (post strict ancestor of pre) or
///      `Divergent` (neither ancestor). Both restore.
///
/// `is_ancestor` is an injected oracle so the function stays
/// pure for unit tests. Errors propagate.
pub fn classify_rewind_protection<F>(
    pre: &RewindSnapshot,
    post_commit_id: &str,
    post_change_id: &str,
    is_ancestor: F,
) -> Result<RewindProtectionOutcome>
where
    F: Fn(&str, &str) -> Result<bool>,
{
    // Rule 1.
    if post_commit_id == pre.pre_commit {
        return Ok(RewindProtectionOutcome::NoChange);
    }
    // Rule 2.
    if post_change_id == pre.pre_change_id {
        return Ok(RewindProtectionOutcome::InPlaceRewrite);
    }
    // Rule 3.
    if let Some(baseline) = &pre.origin_baseline_commit
        && baseline == &pre.pre_commit
    {
        return Ok(RewindProtectionOutcome::NoLocalWork);
    }
    // Rule 4.
    if is_ancestor(&pre.pre_commit, post_commit_id)? {
        return Ok(RewindProtectionOutcome::ForwardMove);
    }
    // Rule 5. Reverse-direction check distinguishes a strict
    // backward rewind (`post` is an ancestor of `pre`) from a
    // sideways/divergent move (neither direction). Both classify
    // as "restore", but the variant feeds the action emit so the
    // user sees the right framing in the per-bookmark row.
    if is_ancestor(post_commit_id, &pre.pre_commit)? {
        Ok(RewindProtectionOutcome::Rewound)
    } else {
        Ok(RewindProtectionOutcome::Divergent)
    }
}

/// Pure pre-vs-post commit-id comparison for the moved-sideways
/// detector (issue #69). Returns a map from bookmark name to
/// `(pre_commit, post_commit)` for every bookmark present in
/// BOTH snapshots whose commit_id changed. Callers must filter
/// the result further (a fast-forward isn't actually
/// "sideways"); this helper only computes the candidate set.
///
/// We don't decide here whether the move is forward vs sideways
/// — that requires `git merge-base --is-ancestor`, which the
/// callers handle via `jj::is_ancestor`. Keeping this helper
/// pure means it can be exhaustively unit-tested without
/// touching a workspace.
pub fn compute_moved_set(
    pre: &[LocalBookmark],
    post: &[LocalBookmark],
) -> std::collections::BTreeMap<String, (String, String)> {
    let post_by_name: std::collections::BTreeMap<&str, &str> = post
        .iter()
        .map(|b| (b.name.as_str(), b.commit_id.as_str()))
        .collect();
    pre.iter()
        .filter_map(|b| {
            post_by_name.get(b.name.as_str()).and_then(|post_commit| {
                if *post_commit == b.commit_id {
                    None
                } else {
                    Some((
                        b.name.clone(),
                        (b.commit_id.clone(), (*post_commit).to_owned()),
                    ))
                }
            })
        })
        .collect()
}

/// Plan a single orphan rebase: returns `Some((bookmark, parent))`
/// when `sb` is a confirmed orphan (its parent disappeared from the
/// local bookmark set during gt sync) AND the bookmark itself still
/// exists locally. Pure function; no jj/gt calls.
///
/// Returns None when:
/// - the bookmark itself was deleted (nothing to rebase),
/// - the bookmark's parent is trunk (already on trunk's ancestry),
/// - the parent still exists locally (the stack edge is intact).
pub fn plan_orphan_rebase(
    sb: &StackedBookmark,
    remaining_names: &std::collections::BTreeSet<String>,
    deleted_during_sync: &std::collections::BTreeSet<String>,
) -> Option<String> {
    if !remaining_names.contains(&sb.name) {
        return None;
    }
    match &sb.parent {
        BookmarkOrTrunk::Trunk => None,
        BookmarkOrTrunk::Bookmark(parent) => {
            if deleted_during_sync.contains(parent) {
                Some(parent.clone())
            } else {
                None
            }
        }
    }
}

/// Build the `jj rebase -s` revset that captures the *entire* range
/// of commits from above the orphan's deleted parent up through the
/// bookmark tip. Crucial for multi-commit-per-bookmark stacks: the
/// naive `jj rebase -s <bookmark> -d trunk` only moves the tip
/// commit (since the bookmark name resolves to one commit), leaving
/// any unbookmarked parent commits stranded — which then surfaces
/// as a "file appeared from nowhere" rebase conflict when those
/// stranded parents are the ones that created the file.
///
/// `roots((<parent_commit>..<bookmark>) ~ ::<trunk>)` reads as:
/// "find the lowest-level commits in the half-open range
/// (parent_commit, bookmark], EXCLUDING anything already on
/// trunk." The trunk exclusion matters when the deleted parent
/// was a now-orphaned commit (the parent's PR merged + Graphite
/// pre-merge rebased the bookmark onto the new tip of trunk).
/// In that scenario, naive `parent_commit..bookmark` sweeps in
/// every commit between OLD-parent and NEW-trunk-tip — including
/// every immutable merge commit on trunk — and `jj rebase`
/// rejects the whole operation with `Commit ... is immutable`.
/// Subtracting `::trunk` keeps the revset honest: it now resolves
/// to just the commits unique to the local bookmark chain.
///
/// We deliberately use the commit id (not the bookmark name) for
/// the lower bound because the parent's bookmark was deleted by
/// `gt sync` and no longer resolves as a name; the commit object
/// itself remains addressable until jj's garbage collector runs.
pub fn build_orphan_rebase_revset(parent_commit_id: &str, bookmark: &str, trunk: &str) -> String {
    format!("roots(({parent_commit_id}..{bookmark}) ~ ::{trunk})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::{PrInfo, PrState};

    fn local(name: &str, commit: &str) -> LocalBookmark {
        LocalBookmark {
            name: name.into(),
            commit_id: commit.into(),
        }
    }

    fn pr_with(number: u32, branch: &str, head_oid: &str, state: PrState) -> PrInfo {
        PrInfo {
            number,
            head_ref_name: branch.into(),
            head_ref_oid: head_oid.into(),
            state,
            is_draft: false,
            merge_state_status: None,
            labels: Vec::new(),
        }
    }

    #[test]
    fn classify_no_pr_leaves_alone() {
        let b = local("foo", "abc123");
        assert_eq!(
            classify_local_bookmark(&b, None, None),
            CleanupAction::LeftAlone
        );
    }

    #[test]
    fn classify_open_pr_same_sha_leaves_alone() {
        let b = local("foo", "abc123");
        let pr = pr_with(1, "foo", "abc12345678", PrState::Open);
        assert_eq!(
            classify_local_bookmark(&b, Some(&pr), None),
            CleanupAction::LeftAlone
        );
    }

    #[test]
    fn classify_open_pr_different_sha_flags_drift() {
        let b = local("foo", "abc123");
        let pr = pr_with(1, "foo", "deadbeef", PrState::Open);
        let action = classify_local_bookmark(&b, Some(&pr), None);
        assert!(matches!(action, CleanupAction::SkippedDueToDrift { .. }));
    }

    #[test]
    fn classify_merged_pr_with_marker_orphan_deletes() {
        let b = local("foo", "abc123");
        let pr = pr_with(7, "foo", "abc12345678", PrState::Merged);
        let action = classify_local_bookmark(&b, Some(&pr), Some("merge_sha_xyz"));
        assert_eq!(
            action,
            CleanupAction::OrphanDeleted {
                pr: 7,
                merge_commit_id: "merge_sha_xyz".into()
            }
        );
    }

    #[test]
    fn classify_closed_no_marker_leaves_alone() {
        let b = local("foo", "abc123");
        let pr = pr_with(7, "foo", "abc12345678", PrState::Closed);
        let action = classify_local_bookmark(&b, Some(&pr), None);
        assert_eq!(action, CleanupAction::LeftAlone);
    }

    #[test]
    fn classify_drift_short_circuits_marker_check() {
        // If we have drift AND a merge marker, drift wins — we never
        // want to delete a local bookmark that has unpushed work,
        // even if a same-numbered PR happened to land elsewhere.
        let b = local("foo", "abc123");
        let pr = pr_with(7, "foo", "deadbeef", PrState::Merged);
        let action = classify_local_bookmark(&b, Some(&pr), Some("merge_sha_xyz"));
        assert!(matches!(action, CleanupAction::SkippedDueToDrift { .. }));
    }

    #[test]
    fn gtmq_open_pr_left_alone() {
        let pr = pr_with(101, "gtmq_xyz", "x", PrState::Open);
        assert_eq!(
            classify_gtmq_branch(Some(&pr)),
            CleanupAction::GtmqLeftAlone { pr: 101 }
        );
    }

    #[test]
    fn gtmq_closed_pr_pruned() {
        let pr = pr_with(101, "gtmq_xyz", "x", PrState::Closed);
        assert_eq!(
            classify_gtmq_branch(Some(&pr)),
            CleanupAction::GtmqPruned { had_pr: Some(101) }
        );
    }

    #[test]
    fn gtmq_no_pr_pruned() {
        assert_eq!(
            classify_gtmq_branch(None),
            CleanupAction::GtmqPruned { had_pr: None }
        );
    }

    #[test]
    fn is_gtmq_branch_matches_default_prefix() {
        let prefixes = vec!["gtmq_".to_owned()];
        assert!(is_gtmq_branch("gtmq_abc", &prefixes));
        assert!(!is_gtmq_branch("feature/foo", &prefixes));
    }

    #[test]
    fn is_gtmq_branch_matches_extra_prefixes() {
        let prefixes = vec!["gtmq_".to_owned(), "graphite-".to_owned()];
        assert!(is_gtmq_branch("graphite-tmp-1", &prefixes));
        assert!(!is_gtmq_branch("other", &prefixes));
    }

    fn sb(name: &str, parent: BookmarkOrTrunk) -> StackedBookmark {
        StackedBookmark {
            name: name.into(),
            parent,
        }
    }

    fn names(items: &[&str]) -> std::collections::BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn plan_orphan_rebase_skips_when_bookmark_was_deleted() {
        // The bookmark itself disappeared (its own PR landed and gt
        // sync removed it) — nothing to rebase.
        let s = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let remaining = names(&["top"]);
        let deleted = names(&["bottom", "mid"]);
        assert_eq!(plan_orphan_rebase(&s, &remaining, &deleted), None);
    }

    #[test]
    fn plan_orphan_rebase_skips_when_parent_is_trunk() {
        // Bottom of a stack — parent is trunk, already on trunk's
        // ancestry; rebasing onto trunk would be a no-op.
        let s = sb("bottom", BookmarkOrTrunk::Trunk);
        let remaining = names(&["bottom"]);
        let deleted = names(&[]);
        assert_eq!(plan_orphan_rebase(&s, &remaining, &deleted), None);
    }

    #[test]
    fn plan_orphan_rebase_skips_when_parent_still_exists() {
        // The stack edge is intact — bottom→mid→top, gt sync didn't
        // delete bottom, so mid isn't orphaned.
        let s = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let remaining = names(&["bottom", "mid", "top"]);
        let deleted = names(&[]);
        assert_eq!(plan_orphan_rebase(&s, &remaining, &deleted), None);
    }

    #[test]
    fn plan_orphan_rebase_fires_when_parent_was_deleted() {
        // bottom's PR landed → gt sync removed bottom → mid is
        // orphaned and needs rebasing onto trunk.
        let s = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let remaining = names(&["mid", "top"]);
        let deleted = names(&["bottom"]);
        assert_eq!(
            plan_orphan_rebase(&s, &remaining, &deleted),
            Some("bottom".into())
        );
    }

    #[test]
    fn plan_orphan_rebase_fires_when_parent_was_fetch_deleted_not_sync_deleted() {
        // Regression for the wild bug:
        //
        //   - All but the topmost bookmark in a 10-commit stack had
        //     their PRs squash-merged into main.
        //   - `jj git fetch` propagated the remote-side deletions to
        //     LOCAL tracked bookmarks before our cleanup code looked
        //     at them.
        //   - `derive_parents` was running post-fetch, so by the time
        //     it ran, the parent bookmarks were already gone; the
        //     orphan signal evaporated and the stack tip was left
        //     floating on a stale base.
        //
        // The fix is to snapshot bookmark state PRE-fetch (so the
        // parent edges exist in `pre_sync_stacked`) and feed the
        // orphan-rebase logic a deleted set that covers fetch-
        // deletions AND sync-deletions uniformly. From
        // `plan_orphan_rebase`'s perspective, the input is the same
        // — it doesn't care WHO deleted the parent, only that it's
        // in the deleted set. This test pins that semantics.
        let s = sb("top", BookmarkOrTrunk::Bookmark("merged-parent".into()));
        let remaining = names(&["top"]);
        // `merged-parent` was deleted by `jj git fetch` (its remote
        // ref was deleted by the post-merge cleanup; the local
        // bookmark followed because it was tracked). Even though
        // the deletion happened earlier in the pipeline than sync,
        // the orphan-rebase rule fires.
        let deleted_by_fetch_or_sync = names(&["merged-parent"]);
        assert_eq!(
            plan_orphan_rebase(&s, &remaining, &deleted_by_fetch_or_sync),
            Some("merged-parent".into())
        );
    }

    #[test]
    fn plan_orphan_rebase_skips_unrelated_bookmark() {
        // Regression test for the bug we observed in the wild:
        // `sea-501` was unrelated to the bookmark that triggered the
        // fetch (`sea-589`), wasn't a child of anything that got
        // deleted, but the old code rebased it anyway and introduced
        // a conflict. plan_orphan_rebase should return None for it.
        let s = sb(
            "sea-501-sccache-supervisor--thor",
            BookmarkOrTrunk::Bookmark("main".into()),
        );
        let remaining = names(&[
            "main",
            "sea-501-sccache-supervisor--thor",
            "sea-589-grant-self-test--iris",
        ]);
        let deleted = names(&[]);
        assert_eq!(plan_orphan_rebase(&s, &remaining, &deleted), None);
    }

    #[test]
    fn build_orphan_rebase_revset_uses_roots_of_half_open_range() {
        // The revset must include the bookmark name in its tip slot,
        // the parent commit id in its lower-bound slot, and exclude
        // `::trunk` so an orphaned-parent + new-main scenario can't
        // sweep in immutable trunk commits.
        let revset = build_orphan_rebase_revset("abc123def456", "sea-501--thor", "main");
        assert_eq!(revset, "roots((abc123def456..sea-501--thor) ~ ::main)");
    }

    #[test]
    fn build_orphan_rebase_revset_with_full_40_char_oid() {
        // gh and git both produce 40-char OIDs; the revset shouldn't
        // care about length but the test pins that no truncation
        // happens.
        let full_oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let revset = build_orphan_rebase_revset(full_oid, "upper", "main");
        assert_eq!(revset, format!("roots(({full_oid}..upper) ~ ::main)"));
    }

    #[test]
    fn build_orphan_rebase_revset_threads_custom_trunk() {
        // Repos with a `trunk` name other than `main` should still
        // have their trunk excluded — the `opts.trunk` plumbing
        // matters.
        let revset = build_orphan_rebase_revset("deadbeef", "feature", "master");
        assert_eq!(revset, "roots((deadbeef..feature) ~ ::master)");
    }

    /// `is_ancestor` oracle for classify_rewind tests. Pin a small DAG
    /// in test code so each test exercises one classification branch.
    /// The closure form lets us keep the test fixtures inline.
    fn ancestor_oracle(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str, &str) -> Result<bool> + use<> {
        move |a, b| Ok(a == b || pairs.iter().any(|(p, q)| *p == a && *q == b))
    }

    #[test]
    fn classify_rewind_unchanged_when_pre_equals_post() {
        let cls = classify_rewind("abc", Some("abc"), ancestor_oracle(&[])).unwrap();
        assert_eq!(cls, RewindClassification::Unchanged);
    }

    #[test]
    fn classify_rewind_fast_forward_when_pre_is_ancestor_of_post() {
        // pre→post is a fast-forward (remote advanced, local was
        // behind). Pipeline applied the advance correctly.
        let cls = classify_rewind("old", Some("new"), ancestor_oracle(&[("old", "new")])).unwrap();
        assert_eq!(cls, RewindClassification::FastForward);
    }

    #[test]
    fn classify_rewind_rewound_when_post_is_ancestor_of_pre() {
        // The bug case from issue #2: local was ahead of remote, gt
        // sync reset local to remote. post is an ancestor of pre.
        let cls = classify_rewind(
            "local_ahead",
            Some("remote_behind"),
            ancestor_oracle(&[("remote_behind", "local_ahead")]),
        )
        .unwrap();
        assert_eq!(cls, RewindClassification::Rewound);
    }

    #[test]
    fn classify_rewind_diverged_when_neither_is_ancestor() {
        // Neither direction's an ancestor — true divergence (e.g.
        // collaborator pushed a different commit). Restore + warn.
        let cls = classify_rewind("local", Some("remote"), ancestor_oracle(&[])).unwrap();
        assert_eq!(cls, RewindClassification::Diverged);
    }

    #[test]
    fn classify_rewind_disappeared_when_post_is_none() {
        // Bookmark deleted during the pipeline. Classifier signals
        // Disappeared; caller decides whether to resurrect based on
        // local-only-commits-ahead-of-remote check.
        let cls = classify_rewind("abc", None, ancestor_oracle(&[])).unwrap();
        assert_eq!(cls, RewindClassification::Disappeared);
    }

    #[test]
    fn classify_rewind_propagates_oracle_errors() {
        // If the oracle errors (e.g. git merge-base failed on a
        // bad SHA), we propagate so the caller surfaces it rather
        // than silently mis-classifying as Diverged.
        let oracle = |_: &str, _: &str| {
            Err(crate::error::JjGtError::Invalid(
                "synthetic test error".into(),
            ))
        };
        let err = classify_rewind("a", Some("b"), oracle).unwrap_err();
        assert!(format!("{err}").contains("synthetic test error"));
    }

    #[test]
    fn is_backfill_target_accepts_intact_chain() {
        // Baseline: bookmark has a PR, both child and parent are
        // still present locally, parent isn't in the deleted set
        // → eligible.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "bottom", "mid", "top"]);
        let deleted = names(&[]);
        assert!(is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn is_backfill_target_rejects_child_deleted_during_fetch() {
        // bookmark's remote was deleted, propagated to local —
        // `gt track` would fail "branch not found." Skip.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "bottom", "top"]); // mid missing
        let deleted = names(&["mid"]);
        assert!(!is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn is_backfill_target_accepts_when_parent_fetch_deleted() {
        // The wild bug this regression covers: `bottom`'s PR
        // squash-merged and the post-merge cleanup deleted the
        // remote ref, which jj git fetch propagated to local. By
        // the time backfill_phase runs, `bottom` is gone from
        // `post_names` but IS in the deleted set. Previously we
        // skipped `mid` here; now we accept it because the caller
        // substitutes trunk as the gt-track parent (see
        // `effective_backfill_parent`). Without this acceptance
        // the child would not get tracked at all, leaving the
        // graphite metadata refs stale and breaking the next
        // `gt submit --stack` from the new tip.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "mid", "top"]); // bottom missing
        let deleted = names(&["bottom"]); // and in deleted set
        assert!(is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn is_backfill_target_rejects_when_parent_missing_and_not_deleted() {
        // Defensive: parent isn't in post_names AND isn't in the
        // deleted set either (e.g. renamed via some out-of-band
        // mutation). Don't guess — skip the bookmark; the user
        // can `gt track` manually with the correct parent.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "mid", "top"]); // bottom missing
        let deleted = names(&[]); // but not in deleted either
        assert!(!is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn is_backfill_target_accepts_trunk_parent_unconditionally() {
        // A bookmark rooted on trunk has parent=Trunk; trunk is
        // always present locally so the parent check is satisfied
        // by definition.
        let target = sb("bottom", BookmarkOrTrunk::Trunk);
        let prs = vec![pr_with(2, "bottom", "cafebabe", PrState::Open)];
        let post = names(&["main", "bottom"]);
        let deleted = names(&[]);
        assert!(is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn is_backfill_target_rejects_bookmark_without_pr() {
        // No PR → no backfill (gt has nothing to bind to).
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs: Vec<PrInfo> = Vec::new();
        let post = names(&["main", "bottom", "mid"]);
        let deleted = names(&[]);
        assert!(!is_backfill_target(&target, &prs, &post, &deleted));
    }

    #[test]
    fn effective_backfill_parent_returns_trunk_when_parent_deleted() {
        // Companion to the deleted-parent backfill-target test:
        // when the parent went away during fetch, the helper must
        // hand us trunk so the gt-track call has something real
        // to bind to. Pinned as a pure function so a regression
        // here trips without spinning up a fake gt.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let deleted = names(&["bottom"]);
        assert_eq!(effective_backfill_parent(&target, &deleted, "main"), "main",);
    }

    #[test]
    fn effective_backfill_parent_returns_recorded_parent_when_present() {
        // Happy path: parent wasn't deleted, return its name
        // unchanged.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let deleted = names(&[]);
        assert_eq!(
            effective_backfill_parent(&target, &deleted, "main"),
            "bottom",
        );
    }

    #[test]
    fn effective_backfill_parent_returns_trunk_for_trunk_parent() {
        // A bookmark rooted directly on trunk: helper returns
        // the trunk name regardless of the deleted set.
        let target = sb("bottom", BookmarkOrTrunk::Trunk);
        let deleted = names(&["unrelated"]);
        assert_eq!(effective_backfill_parent(&target, &deleted, "main"), "main",);
    }

    #[test]
    fn is_bookmark_conflicted_membership_check() {
        // Pure helper for issue #68: the orphan-rebase phase
        // queries the conflicted-bookmark set up front and short-
        // circuits each candidate. Pin the trivial membership
        // semantics so a future refactor (e.g. case-insensitive
        // matching, prefix matching) trips the test.
        let conflicted = names(&["restack-command", "queue-1"]);
        assert!(is_bookmark_conflicted("restack-command", &conflicted));
        assert!(is_bookmark_conflicted("queue-1", &conflicted));
        assert!(!is_bookmark_conflicted("main", &conflicted));
        assert!(!is_bookmark_conflicted("Restack-Command", &conflicted));
        let empty: BTreeSet<String> = BTreeSet::new();
        assert!(!is_bookmark_conflicted("anything", &empty));
    }

    #[test]
    fn compute_moved_set_filters_unchanged_and_missing_bookmarks() {
        // Pure helper for issue #69. The moved-set candidate
        // shape: pre.normal vs post.normal, returning a
        // (pre_commit, post_commit) per bookmark whose name
        // appears in both and whose commit_id changed. The helper
        // is "candidate"-level — fast-forward vs sideways is the
        // caller's call (needs git merge-base).
        let pre = vec![
            local("main", "aaaaaaaaaaaa"),
            local("bottom", "bbbbbbbbbbbb"),
            local("top", "cccccccccccc"),
            // `deleted-before-fetch` exists only in pre.
            local("deleted-before-fetch", "dddddddddddd"),
        ];
        let post = vec![
            local("main", "aaaaaaaaaaaa"),
            local("bottom", "bbbb22222222"), // moved
            local("top", "cccccccccccc"),    // unchanged
            // `appeared-after-fetch` exists only in post.
            local("appeared-after-fetch", "eeeeeeeeeeee"),
        ];

        let moved = compute_moved_set(&pre, &post);
        // `bottom` moved → present.
        assert_eq!(
            moved.get("bottom"),
            Some(&("bbbbbbbbbbbb".to_owned(), "bbbb22222222".to_owned())),
        );
        // `main` unchanged → absent.
        assert!(!moved.contains_key("main"));
        // `top` unchanged → absent.
        assert!(!moved.contains_key("top"));
        // `deleted-before-fetch` missing in post → absent (the
        // deleted-bookmark codepath handles those).
        assert!(!moved.contains_key("deleted-before-fetch"));
        // `appeared-after-fetch` missing in pre → absent (can't
        // have moved if we didn't see it before).
        assert!(!moved.contains_key("appeared-after-fetch"));
    }

    #[test]
    fn compute_moved_set_empty_on_identical_snapshots() {
        // Defensive: a freshly-fetched workspace where nothing
        // upstream changed → empty set. Most common case in
        // production.
        let pre = vec![
            local("main", "aaaaaaaaaaaa"),
            local("feature", "bbbbbbbbbbbb"),
        ];
        let post = pre.clone();
        let moved = compute_moved_set(&pre, &post);
        assert!(moved.is_empty());
    }

    #[test]
    fn compute_moved_set_empty_on_empty_inputs() {
        // Degenerate inputs — both empty, one empty. Should not
        // panic, should return empty.
        let empty: Vec<LocalBookmark> = Vec::new();
        assert!(compute_moved_set(&empty, &empty).is_empty());
        let pre = vec![local("main", "aaaaaaaaaaaa")];
        assert!(compute_moved_set(&pre, &empty).is_empty());
        assert!(compute_moved_set(&empty, &pre).is_empty());
    }

    fn snap(
        pre_commit: &str,
        pre_change_id: &str,
        origin_baseline: Option<&str>,
    ) -> RewindSnapshot {
        RewindSnapshot {
            pre_commit: pre_commit.into(),
            pre_change_id: pre_change_id.into(),
            origin_baseline_commit: origin_baseline.map(str::to_owned),
        }
    }

    #[test]
    fn classify_rewind_protection_no_change_when_post_equals_pre() {
        // Rule 1: identical commit_id → NoChange.
        let pre = snap("commit_a", "change_a", Some("origin_a"));
        let cls = classify_rewind_protection(&pre, "commit_a", "change_a", |_, _| {
            unreachable!("is_ancestor should not be invoked when commit ids match")
        })
        .unwrap();
        assert_eq!(cls, RewindProtectionOutcome::NoChange);
    }

    #[test]
    fn classify_rewind_protection_in_place_rewrite_keeps_change_id() {
        // Rule 2: same change_id, different commit_id (agent ran
        // `jj describe` / `jj squash` against the same logical
        // change). Don't restore.
        let pre = snap("commit_a", "change_x", Some("origin_a"));
        let cls = classify_rewind_protection(&pre, "commit_a_rewritten", "change_x", |_, _| {
            unreachable!("is_ancestor should not be invoked when change ids match")
        })
        .unwrap();
        assert_eq!(cls, RewindProtectionOutcome::InPlaceRewrite);
    }

    #[test]
    fn classify_rewind_protection_no_local_work_when_pre_matches_origin() {
        // Rule 3: pre matched origin's baseline → no local-only
        // work to protect. Even a "rewind" can't lose anything.
        // Also covers the legitimate sideways-on-origin case
        // (Graphite pre-merge rebase): pre matched the OLD
        // origin, post matches the NEW origin.
        let pre = snap("origin_old", "change_a", Some("origin_old"));
        let cls = classify_rewind_protection(&pre, "origin_new", "change_b", |_, _| {
            unreachable!("is_ancestor should not be invoked when pre matches origin baseline")
        })
        .unwrap();
        assert_eq!(cls, RewindProtectionOutcome::NoLocalWork);
    }

    #[test]
    fn classify_rewind_protection_forward_when_pre_is_ancestor_of_post() {
        // Rule 4: pre is ancestor of post (post is descendant).
        // Agent advanced the bookmark, or fetch fast-forwarded
        // because origin pulled ahead. No action.
        let pre = snap("commit_a", "change_a", Some("origin_a"));
        let oracle = ancestor_oracle(&[("commit_a", "commit_b")]);
        let cls =
            classify_rewind_protection(&pre, "commit_b", "change_b", |a, b| oracle(a, b)).unwrap();
        assert_eq!(cls, RewindProtectionOutcome::ForwardMove);
    }

    #[test]
    fn classify_rewind_protection_rewinds_when_post_is_ancestor_of_pre() {
        // The wild bug: pre was the agent's fixup commit, post
        // got rewound to origin's old position. pre had local-
        // only work (pre != origin_baseline), so restore.
        let pre = snap("commit_b_fixup", "change_fixup", Some("commit_a_origin"));
        // pre is NOT an ancestor of post — but post IS an ancestor
        // of pre, so rule 5 classifies as `Rewound` (not
        // `Divergent`). The reverse-direction probe is what
        // distinguishes a strict rewind from a sideways move.
        let oracle = ancestor_oracle(&[("commit_a_origin", "commit_b_fixup")]);
        let cls =
            classify_rewind_protection(&pre, "commit_a_origin", "change_a", |a, b| oracle(a, b))
                .unwrap();
        assert_eq!(cls, RewindProtectionOutcome::Rewound);
    }

    #[test]
    fn classify_rewind_protection_divergent_when_neither_direction_ancestor() {
        // The sibling case to `rewinds_when_post_is_ancestor_of_pre`:
        // pre had local-only work, fetch landed on a commit that
        // is NEITHER ancestor nor descendant of pre. Rule 5's
        // reverse-direction probe must distinguish this from a
        // strict rewind so the action emit can frame it correctly
        // ("divergent" vs "rewound").
        let pre = snap("commit_b_fixup", "change_fixup", Some("commit_a_origin"));
        // Empty oracle → is_ancestor returns false in both
        // directions (the two commits are unrelated tips).
        let oracle = ancestor_oracle(&[]);
        let cls =
            classify_rewind_protection(&pre, "commit_c_sideways", "change_c", |a, b| oracle(a, b))
                .unwrap();
        assert_eq!(cls, RewindProtectionOutcome::Divergent);
    }

    #[test]
    fn classify_rewind_protection_propagates_oracle_errors() {
        // is_ancestor failures (e.g. one of the commits is GC'd)
        // propagate to the caller — they decide whether to skip
        // restoration or surface it as a warning.
        let pre = snap("commit_a", "change_a", Some("origin_old"));
        let cls = classify_rewind_protection(&pre, "commit_b", "change_b", |_, _| {
            Err(crate::error::JjGtError::Invalid(
                "synthetic oracle failure".into(),
            ))
        });
        let err = cls.unwrap_err();
        assert!(format!("{err}").contains("synthetic oracle failure"));
    }

    #[test]
    fn classify_rewind_protection_handles_missing_origin_baseline() {
        // Bookmark was created locally and never pushed
        // (origin_baseline_commit = None). Rule 3 (no-local-work)
        // can't fire — we don't know what "no local work" looks
        // like for an unpushed bookmark. Treat any backward move
        // as a real rewind.
        let pre = snap("commit_b", "change_b", None);
        // Rule 5 reverse probe: post (commit_a) IS an ancestor of
        // pre (commit_b) in the oracle, so this classifies as
        // Rewound rather than Divergent.
        let oracle = ancestor_oracle(&[("commit_a", "commit_b")]);
        let cls =
            classify_rewind_protection(&pre, "commit_a", "change_a", |a, b| oracle(a, b)).unwrap();
        assert_eq!(cls, RewindProtectionOutcome::Rewound);
    }
}

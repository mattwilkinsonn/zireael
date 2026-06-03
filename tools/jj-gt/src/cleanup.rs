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

use std::collections::BTreeSet;
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
            gtmq_prefixes: vec!["gtmq_".into()],
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
    orphan_rebase_phase(jj, &pre, opts, &mut actions)?;

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
/// `BookmarkOrTrunk::Trunk` is always satisfiable: trunk by
/// definition exists locally.
///
/// Pure function so unit tests can exhaustively cover the truth
/// table without spinning up a workspace or stubbing `gt`.
pub fn is_backfill_target(
    sb: &StackedBookmark,
    normal_prs: &[PrInfo],
    post_names: &BTreeSet<String>,
) -> bool {
    let has_pr = normal_prs.iter().any(|p| p.head_ref_name == sb.name);
    let child_present = post_names.contains(&sb.name);
    let parent_present = match &sb.parent {
        BookmarkOrTrunk::Trunk => true,
        BookmarkOrTrunk::Bookmark(parent) => post_names.contains(parent),
    };
    has_pr && child_present && parent_present
}

/// Backfill gt tracking metadata for bookmarks that have a PR.
///
/// Filters to bookmarks (a) with a PR and (b) still present
/// locally after fetch. A bookmark whose remote was just deleted
/// by the post-merge cleanup is gone from local; gt track against
/// a missing branch errors with "branch not found." Skipping
/// those here is a no-op anyway — sync / orphan-rebase will
/// handle the cleanup.
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
    let stacked = crate::stack::sort_for_tracking(&pre.stacked);
    let backfill_targets: Vec<_> = stacked
        .iter()
        .filter(|sb| is_backfill_target(sb, &pre.normal_prs, &post_names))
        .collect();
    if backfill_targets.is_empty() {
        let step = crate::ui::Step::start("Backfilling gt tracking metadata", verbosity);
        step.skip("no bookmarks with PRs", None);
        return Ok(());
    }
    for sb in backfill_targets {
        let parent = match &sb.parent {
            BookmarkOrTrunk::Bookmark(p) => p.clone(),
            BookmarkOrTrunk::Trunk => opts.trunk.clone(),
        };
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
    let mut restored = 0usize;
    let mut diverged = 0usize;
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
                if let Err(e) = jj::bookmark_set(jj, name, pre_id) {
                    rewind_errors.push(format!("{name}: restore failed: {e}"));
                    continue;
                }
                restored += 1;
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
                if let Err(e) = jj::bookmark_set(jj, name, pre_id) {
                    rewind_errors.push(format!("{name}: restore failed: {e}"));
                    continue;
                }
                diverged += 1;
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
    let summary = match (restored, diverged, rewind_errors.is_empty()) {
        (0, 0, true) => "no rewinds detected".to_owned(),
        (r, 0, true) => format!("{r} bookmark(s) restored from rewind"),
        (0, d, true) => format!("{d} bookmark(s) diverged; local restored"),
        (r, d, true) => format!("{r} restored from rewind, {d} diverged; local restored"),
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
    opts: &FetchOpts,
    actions: &mut Vec<(LocalBookmark, CleanupAction)>,
) -> Result<()> {
    if opts.no_rebase || opts.dry_run {
        return Ok(());
    }
    let remaining = list_local_bookmarks(jj)?;
    let remaining_names: BTreeSet<String> = remaining.iter().map(|b| b.name.clone()).collect();
    let deleted = compute_deleted_set(&pre.normal_names(), &remaining_names);

    for sb in &pre.stacked {
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

        let rebase_revset = match parent_commit.as_deref() {
            Some(commit) => build_orphan_rebase_revset(commit, &sb.name),
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
/// `roots(<parent_commit>..<bookmark>)` reads as: "find the
/// lowest-level commits in the half-open range (parent_commit,
/// bookmark]." Concretely, that's the first commit above the
/// deleted parent. `jj rebase -s <root>` then includes that root
/// plus every descendant up to and including the bookmark — moving
/// the whole stack-entry as one unit.
///
/// We deliberately use the commit id (not the bookmark name) for
/// the lower bound because the parent's bookmark was deleted by
/// `gt sync` and no longer resolves as a name; the commit object
/// itself remains addressable until jj's garbage collector runs.
pub fn build_orphan_rebase_revset(parent_commit_id: &str, bookmark: &str) -> String {
    format!("roots({parent_commit_id}..{bookmark})")
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
        // The revset must include the bookmark name in its tip slot
        // and the parent commit id in its lower-bound slot, wrapped
        // by `roots(...)` so `jj rebase -s` picks up the lowest
        // commit above the deleted parent.
        let revset = build_orphan_rebase_revset("abc123def456", "sea-501--thor");
        assert_eq!(revset, "roots(abc123def456..sea-501--thor)");
    }

    #[test]
    fn build_orphan_rebase_revset_with_full_40_char_oid() {
        // gh and git both produce 40-char OIDs; the revset shouldn't
        // care about length but the test pins that no truncation
        // happens.
        let full_oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let revset = build_orphan_rebase_revset(full_oid, "upper");
        assert_eq!(revset, format!("roots({full_oid}..upper)"));
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
        // still present locally → eligible.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "bottom", "mid", "top"]);
        assert!(is_backfill_target(&target, &prs, &post));
    }

    #[test]
    fn is_backfill_target_rejects_child_deleted_during_fetch() {
        // bookmark's remote was deleted, propagated to local —
        // `gt track` would fail "branch not found." Skip.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "bottom", "top"]); // mid missing
        assert!(!is_backfill_target(&target, &prs, &post));
    }

    #[test]
    fn is_backfill_target_rejects_parent_deleted_during_fetch() {
        // Regression for the wild bug: `bottom`'s PR squash-merged
        // and the post-merge cleanup deleted the remote ref, which
        // jj git fetch propagated to local. By the time
        // backfill_phase runs, `bottom` is gone from `post_names`
        // — calling `gt track mid --parent bottom` would error
        // "branch not found" and abort the pipeline before
        // orphan_rebase_phase repairs the stack.
        //
        // The pre-fetch snapshot intentionally preserved the
        // `mid → bottom` edge (so orphan rebase can detect and
        // fix it), so the orphan signal is downstream. We just
        // need to NOT run gt track here.
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs = vec![pr_with(1, "mid", "deadbeef", PrState::Open)];
        let post = names(&["main", "mid", "top"]); // bottom missing
        assert!(!is_backfill_target(&target, &prs, &post));
    }

    #[test]
    fn is_backfill_target_accepts_trunk_parent_unconditionally() {
        // A bookmark rooted on trunk has parent=Trunk; trunk is
        // always present locally so the parent check is satisfied
        // by definition.
        let target = sb("bottom", BookmarkOrTrunk::Trunk);
        let prs = vec![pr_with(2, "bottom", "cafebabe", PrState::Open)];
        let post = names(&["main", "bottom"]);
        assert!(is_backfill_target(&target, &prs, &post));
    }

    #[test]
    fn is_backfill_target_rejects_bookmark_without_pr() {
        // No PR → no backfill (gt has nothing to bind to).
        let target = sb("mid", BookmarkOrTrunk::Bookmark("bottom".into()));
        let prs: Vec<PrInfo> = Vec::new();
        let post = names(&["main", "bottom", "mid"]);
        assert!(!is_backfill_target(&target, &prs, &post));
    }
}

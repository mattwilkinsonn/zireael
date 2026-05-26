//! `jj-gt fetch` pipeline + the testable per-bookmark classifier
//! decisions.
//!
//! Most of the file is the [`classify_local_bookmark`] /
//! [`classify_gtmq_branch`] functions — pure decision logic kept
//! separate from the orchestration so the test suite can exercise
//! every branch without spinning up real `gh` / `gt` / `jj` invocations.

use std::path::Path;

use crate::error::Result;
use crate::gh::{self, PrInfo, PrState};
use crate::gt;
use crate::jj::{self, JjCli, LocalBookmark, list_local_bookmarks};
use crate::stack::{BookmarkOrTrunk, StackedBookmark, derive_parents};

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
    /// Same trigger as `Rebased`, but `jj rebase` reported the
    /// resulting commit(s) as conflicted. We don't roll back —
    /// jj's conflict markers stay in the working tree and the user
    /// runs `jj resolve` to clean up.
    RebaseConflicted {
        onto: String,
        prev_parent: String,
        message: String,
    },
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
    use crate::ui;

    // 1. Fetch.
    let fetch_step = ui::Step::start(&format!("Fetching from {}", opts.remote), verbosity);
    if opts.dry_run {
        fetch_step.skip("dry-run", None);
    } else {
        match jj::git_fetch(jj, &opts.remote) {
            Ok(()) => fetch_step.success("", None),
            Err(e) => {
                fetch_step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
    }

    // Snapshot the current bookmark list once for the cleanup phase.
    let mut bookmarks = list_local_bookmarks(jj)?;

    // Partition gtmq_* vs everything else.
    let (gtmq, normal): (Vec<_>, Vec<_>) = bookmarks
        .drain(..)
        .partition(|b| is_gtmq_branch(&b.name, &opts.gtmq_prefixes));

    // Look up PR info for all branches in one batched call each. The
    // gh CLI search syntax accepts repeated `head:` clauses.
    let normal_prs = if normal.is_empty() {
        Vec::new()
    } else {
        let names: Vec<String> = normal.iter().map(|b| b.name.clone()).collect();
        gh::find_prs_for_branches(workspace_root, &names, 200)?
    };

    // Derive stack structure once — both the backfill step (step 2)
    // and the orphan-detection step (step 7) need to know
    // (bookmark → parent) pairs derived from the pre-sync world. If
    // we re-derived after gt sync ran, we'd lose the parent edges
    // for any bookmark sync deleted, which is exactly the signal
    // step 7 needs.
    let pre_sync_stacked = derive_parents(
        jj,
        &normal.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        &opts.trunk,
    )?;
    let bookmarks_before_sync = normal.clone();

    // 2. Backfill metadata refs for bookmarks that have a PR. Sort
    // bottom→top so gt accepts each parent reference.
    if !opts.no_backfill {
        let stacked = crate::stack::sort_for_tracking(&pre_sync_stacked);
        let backfill_targets: Vec<_> = stacked
            .iter()
            .filter(|sb| normal_prs.iter().any(|p| p.head_ref_name == sb.name))
            .collect();
        if backfill_targets.is_empty() {
            let step = ui::Step::start("Backfilling gt tracking metadata", verbosity);
            step.skip("no bookmarks with PRs", None);
        } else {
            for sb in backfill_targets {
                let parent = match &sb.parent {
                    BookmarkOrTrunk::Bookmark(p) => p.clone(),
                    BookmarkOrTrunk::Trunk => opts.trunk.clone(),
                };
                let step = ui::Step::start(
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
        }
    }

    // 3 + 4 + 5: classify normal bookmarks (drift / cleanup decisions),
    // run gt sync, then re-import.
    let mut actions: Vec<(LocalBookmark, CleanupAction)> = Vec::new();
    for local in &normal {
        let pr = normal_prs.iter().find(|p| p.head_ref_name == local.name);
        let marker = match pr {
            Some(pr) if pr.state.is_terminal() => {
                jj::find_pr_merge_marker_on_trunk(jj, pr.number, &opts.trunk)?
            }
            _ => None,
        };
        let action = classify_local_bookmark(local, pr, marker.as_deref());
        actions.push((local.clone(), action));
    }

    let sync_step = ui::Step::start("Running gt sync --no-restack", verbosity);
    if opts.dry_run {
        sync_step.skip("dry-run", None);
    } else {
        match gt::sync_no_restack(workspace_root) {
            Ok(()) => sync_step.success("", None),
            Err(e) => {
                sync_step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
        let import_step = ui::Step::start("Importing remote refs into jj", verbosity);
        match jj::git_import(jj) {
            Ok(()) => import_step.success("", None),
            Err(e) => {
                import_step.fail(&format!("{e}"), None);
                return Err(e);
            }
        }
    }

    // 6. gtmq_* pruning.
    if !opts.no_gtmq_prune && !gtmq.is_empty() {
        let gtmq_prs = gh::list_prs_by_head_prefix(workspace_root, &opts.gtmq_prefixes, 500)?;
        for branch in &gtmq {
            let pr = gtmq_prs.iter().find(|p| p.head_ref_name == branch.name);
            let action = classify_gtmq_branch(pr);
            if let CleanupAction::GtmqPruned { .. } = action {
                let step =
                    ui::Step::start(&format!("Pruning gtmq branch {}", branch.name), verbosity);
                if opts.dry_run {
                    step.skip("dry-run", None);
                } else {
                    let _ = jj::delete_bookmark(jj, &branch.name);
                    let _ = jj::delete_remote_branch(workspace_root, &opts.remote, &branch.name);
                    step.success("deleted local + remote", None);
                }
            }
            actions.push((branch.clone(), action));
        }
    }

    // 7. Orphan-restack via `jj rebase` ONLY for bookmarks whose
    // tracked parent disappeared during step 4 (`gt sync` deleted
    // it because its PR landed). The naive "rebase every remaining
    // bookmark" approach we used to do here rebases unrelated
    // bookmarks (any time they happened to live on a non-trunk
    // commit) and is the source of the bug where `jj-gt fetch`
    // would surprise-rebase an in-flight unrelated stack entry and
    // sometimes introduce conflicts.
    //
    // The orphan signal is: bookmark existed in `bookmarks_before_sync`
    // with parent X, X no longer exists locally after sync+import.
    if !opts.no_rebase && !opts.dry_run {
        let remaining = list_local_bookmarks(jj)?;
        let remaining_names: std::collections::BTreeSet<String> =
            remaining.iter().map(|b| b.name.clone()).collect();
        let before_names: std::collections::BTreeSet<String> = bookmarks_before_sync
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let deleted_during_sync: std::collections::BTreeSet<String> =
            before_names.difference(&remaining_names).cloned().collect();

        for sb in &pre_sync_stacked {
            let Some(prev_parent) = plan_orphan_rebase(sb, &remaining_names, &deleted_during_sync)
            else {
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
            let parent_commit = bookmarks_before_sync
                .iter()
                .find(|b| b.name == prev_parent)
                .map(|b| b.commit_id.clone());

            let rebase_revset = match parent_commit.as_deref() {
                Some(commit) => build_orphan_rebase_revset(commit, &sb.name),
                None => {
                    // Defensive fallback. We snapshotted
                    // bookmarks_before_sync from list_local_bookmarks
                    // ourselves so a miss here would mean the parent
                    // bookmark exists in the stack graph but not in
                    // the bookmark list — shouldn't happen, but if
                    // it does, fall back to the bookmark-only revset
                    // and accept that multi-commit stacks may only
                    // move their tip.
                    sb.name.clone()
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
                    actions.push((
                        local,
                        CleanupAction::RebaseConflicted {
                            onto: opts.trunk.clone(),
                            prev_parent,
                            message,
                        },
                    ));
                }
                Err(e) => {
                    // Hard rebase failure (e.g. immutable commit) —
                    // surface as a conflicted action so it's at
                    // least visible in the output rather than silently
                    // swallowed.
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
    }

    Ok(actions)
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
}

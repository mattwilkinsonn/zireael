//! `jj-gt restack` — rebase every local stack onto the current trunk.
//!
//! This is the explicit "rewrite my work" command, sibling to
//! `jj-gt fetch` (which is "make local view match remote, plus
//! the obvious cleanups — no surprises"). Conflict-producing
//! rebases are surfaced in the summary table; jj's conflict-in-
//! commit model means rebases always complete even when they
//! conflict, so the user can resolve at their own pace.
//!
//! # Discovery
//!
//! "Local stacks" = bookmarks authored by the current user whose
//! commit is NOT already an ancestor of `main@origin`. Bookmarks
//! that are merged (i.e. their commit IS reachable from
//! `main@origin`) get skipped — there's nothing to restack.
//!
//! Discovered bookmarks are partitioned into independent stacks
//! via [`stack::partition_stacks`]; one rebase invocation per stack
//! root, walking the whole chain via the same `jj rebase -s <root>
//! -d main@origin` shape the orphan-rebase phase of fetch uses.
//!
//! # Default behaviour
//!
//! Do all stacks, summary at the end. Even when one stack
//! conflicts, the others proceed — jj rebases land their conflicts
//! as commit markers rather than blocking subsequent operations,
//! so there's no "intermediate state" the user has to clean up
//! before the next rebase can run.
//!
//! `--stop-on-conflict` (legacy-gt-style) halts at the first
//! conflicted stack and tells the user to resolve manually.
//!
//! # Out of scope
//!
//! - Restacking gt's tracking metadata. The auto-untrack+track in
//!   [`crate::gt::track`] handles that on the next `jj-gt submit`.
//! - Cross-workspace discovery. Restack only sees bookmarks visible
//!   to its own workspace's jj invocations.
//! - Touching trunk itself or `gtmq_*` queue branches.

use crate::error::{JjGtError, Result};
use crate::jj::{self, JjCli, RebaseOutcome};
use crate::stack::{self, StackedBookmark};
use crate::ui;

/// Per-stack outcome reported in the summary table at the end of
/// a `restack` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackOutcome {
    /// Stack's root was rebased cleanly onto trunk. `commits` is
    /// the number of commits in the rebased range (1 for a single-
    /// commit bookmark, N for a multi-commit stack).
    Rebased { onto: String, commits: usize },
    /// Stack was already on trunk; no rebase needed. Reported as
    /// a separate variant from `Rebased` so the summary distinguishes
    /// "we did work" from "we noticed nothing was needed."
    AlreadyCurrent,
    /// Stack was skipped because `--stop-on-conflict` was set and an
    /// earlier stack conflicted. The user resolves the earlier stack
    /// and re-runs; this one will be retried.
    Skipped { reason: String },
    /// Rebase landed conflicts in the resulting commits. Unlike the
    /// fetch path (which rolls back conflicts), restack is explicitly
    /// the "yes please rewrite" command — conflicts stay in the
    /// working tree. `commits` is the count of conflicted commits in
    /// the rebased range.
    Conflicted {
        onto: String,
        commits: usize,
        message: String,
    },
    /// Rebase command itself errored (e.g. immutable destination,
    /// nonexistent source). Surfaced so the user sees what went wrong
    /// rather than a silent skip.
    Failed { message: String },
}

/// Status of a stack from the discovery scan, before any rebase
/// has been attempted. Captures the tip bookmark, the chain shape,
/// and the commit-id of the chain root (used as the lower bound
/// of the rebase revset).
#[derive(Debug, Clone)]
pub struct DiscoveredStack {
    /// Tip bookmark — the deepest commit in the chain — used as the
    /// human-readable label in the summary table.
    pub tip: String,
    /// Every bookmark in the stack, bottom→top. Sorted via
    /// [`stack::sort_for_tracking`].
    pub bookmarks: Vec<StackedBookmark>,
    /// Commit-id of the stack root — the lowest commit in the chain
    /// whose parent is on trunk's ancestry. Used as `-s <root>` for
    /// the rebase. `None` when the root couldn't be resolved (defensive
    /// fallback; production callers should never hit this).
    pub root_commit: Option<String>,
}

/// Options for [`run_restack`].
#[derive(Debug, Clone)]
pub struct RestackOpts {
    /// The destination of every rebase — typically `main@origin`.
    /// Resolved by the caller via [`crate::status::resolve_trunk`]
    /// + the user's `--remote` flag.
    pub trunk_destination: String,
    /// `--stop-on-conflict`: halt at the first conflicted stack
    /// rather than proceeding through the rest.
    pub stop_on_conflict: bool,
    /// `--dry-run`: discover + report, don't actually rebase.
    pub dry_run: bool,
    /// `--bookmark <name>`: restack only the stack containing this
    /// bookmark. `None` means restack everything.
    pub only_bookmark: Option<String>,
}

/// Per-stack action emitted to the summary table — tip + outcome.
#[derive(Debug, Clone)]
pub struct StackResult {
    pub tip: String,
    pub outcome: StackOutcome,
}

/// Discover every local stack rooted under trunk in the current
/// workspace.
///
/// Revset: `bookmarks() & mine() & ~::<trunk_destination>` — every
/// bookmark authored by the current user whose commit is NOT
/// already reachable from trunk. The negation filters out merged
/// PRs whose bookmarks still hang around locally.
///
/// Returns one [`DiscoveredStack`] per independent partition (per
/// the same partitioning logic `jj-gt submit` uses for unrelated
/// `-b` tips).
///
/// `gtmq_*` branches are filtered out — they're Graphite's
/// queue-test scratch space, not user work to restack.
pub fn discover_stacks(jj: &JjCli, trunk_destination: &str) -> Result<Vec<DiscoveredStack>> {
    // Bookmarks the user has authored that are NOT yet merged into
    // trunk. The `~::<trunk>` clause excludes everything reachable
    // FROM trunk's ancestry — i.e. already merged.
    let revset = format!("bookmarks() & mine() & ~::{trunk_destination}");
    let names = jj::bookmarks_in_revset(jj, &revset)?;

    let filtered: Vec<String> = names
        .into_iter()
        .filter(|n| !is_skippable(n, trunk_destination))
        .collect();
    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    // Re-use `derive_parents` so the per-stack ancestry is consistent
    // with what `submit` and `fetch` see. Lossy variant because a
    // local bookmark whose parent is unbookmarked should still
    // restack — it just means the whole bookmarked range gets
    // rebased onto trunk as a single chain.
    let stacked = stack::derive_parents_lossy(jj, &filtered, trunk_destination);
    let partitions = stack::partition_stacks(&stacked)?;

    let mut discovered: Vec<DiscoveredStack> = Vec::new();
    for partition in partitions {
        // The tip is the top of the bottom-to-top sorted chain.
        let tip = partition
            .last()
            .map(|sb| sb.name.clone())
            .unwrap_or_default();
        // The root is the bottom of the chain — that's the commit we
        // pass to `jj rebase -s <root>` to drag the whole chain onto
        // trunk. Resolve to a commit id so the rebase revset is
        // stable across in-flight bookmark moves.
        let root_commit = partition
            .first()
            .and_then(|sb| jj::resolve_commit_id(jj, &sb.name).ok());
        discovered.push(DiscoveredStack {
            tip,
            bookmarks: partition,
            root_commit,
        });
    }
    Ok(discovered)
}

/// Filter bookmarks that the discovery scan should ignore by name.
/// Two reasons to skip:
///
/// - trunk itself (we never want to rebase trunk).
/// - `gtmq_*` queue branches (Graphite's queue scratch, not user
///   work).
fn is_skippable(name: &str, trunk: &str) -> bool {
    name == trunk || name.starts_with("gtmq_")
}

/// Narrow a discovery result to only the stack containing the named
/// bookmark. Returns an empty vec if no stack matched (caller should
/// surface an error so the user notices their `--bookmark` flag
/// didn't land).
pub fn filter_by_bookmark(
    discovered: Vec<DiscoveredStack>,
    bookmark: &str,
) -> Vec<DiscoveredStack> {
    discovered
        .into_iter()
        .filter(|ds| ds.bookmarks.iter().any(|sb| sb.name == bookmark))
        .collect()
}

/// Rebase a single discovered stack onto `trunk_destination`.
///
/// Mirrors the orphan-rebase shape from `cleanup::orphan_rebase_phase`
/// but doesn't roll back on conflict — restack is the explicit
/// "rewrite my work" command; conflicts stay in the commits and the
/// user resolves them with `jj resolve`.
pub fn rebase_one(
    jj: &JjCli,
    discovered: &DiscoveredStack,
    trunk_destination: &str,
) -> StackOutcome {
    // No root commit means we couldn't resolve the stack's root —
    // surface as a Failed action rather than silently no-op'ing.
    let Some(root_commit) = discovered.root_commit.as_deref() else {
        return StackOutcome::Failed {
            message: format!(
                "couldn't resolve stack root for `{}`; restack skipped",
                discovered.tip
            ),
        };
    };

    let count = discovered.bookmarks.len();
    let revset = format!("{root_commit}::");

    match jj::rebase(jj, &revset, trunk_destination) {
        Ok(RebaseOutcome::Clean) => StackOutcome::Rebased {
            onto: trunk_destination.to_owned(),
            commits: count,
        },
        Ok(RebaseOutcome::NoOp) => StackOutcome::AlreadyCurrent,
        Ok(RebaseOutcome::Conflicted { message }) => {
            // Count the conflicted commits in the rebased range so
            // the summary shows the user the magnitude of the
            // resolution work. Errors here fall back to "unknown
            // count" rather than blocking the report.
            let conflicted_commits = jj::count_conflicts_in(jj, &revset).unwrap_or(0);
            StackOutcome::Conflicted {
                onto: trunk_destination.to_owned(),
                commits: conflicted_commits,
                message,
            }
        }
        Err(e) => StackOutcome::Failed {
            message: format!("jj rebase failed: {e}"),
        },
    }
}

/// Drive the full discover→rebase→summarize pipeline. Returns the
/// per-stack results in discovery order. The caller renders the
/// summary + returns a process exit code based on whether any
/// stacks ended conflicted.
pub fn run_restack(jj: &JjCli, opts: &RestackOpts) -> Result<Vec<StackResult>> {
    let mut discovered = discover_stacks(jj, &opts.trunk_destination)?;

    if let Some(bookmark) = opts.only_bookmark.as_deref() {
        discovered = filter_by_bookmark(discovered, bookmark);
        if discovered.is_empty() {
            return Err(JjGtError::Invalid(format!(
                "no stack contains bookmark `{bookmark}`; nothing to restack"
            )));
        }
    }

    let mut results: Vec<StackResult> = Vec::new();
    let mut hit_conflict = false;
    for ds in &discovered {
        if hit_conflict && opts.stop_on_conflict {
            results.push(StackResult {
                tip: ds.tip.clone(),
                outcome: StackOutcome::Skipped {
                    reason: "earlier stack conflicted; --stop-on-conflict set".into(),
                },
            });
            continue;
        }
        if opts.dry_run {
            results.push(StackResult {
                tip: ds.tip.clone(),
                outcome: StackOutcome::Rebased {
                    onto: opts.trunk_destination.clone(),
                    commits: ds.bookmarks.len(),
                },
            });
            continue;
        }
        let outcome = rebase_one(jj, ds, &opts.trunk_destination);
        if matches!(outcome, StackOutcome::Conflicted { .. }) {
            hit_conflict = true;
        }
        results.push(StackResult {
            tip: ds.tip.clone(),
            outcome,
        });
    }
    Ok(results)
}

/// Map a [`StackOutcome`] to the (status, message) tuple
/// [`ui::action_row`] takes, mirroring the shape `cleanup`'s
/// `action_to_row` uses for fetch results.
pub fn outcome_to_row(outcome: &StackOutcome) -> (ui::ActionStatus, String) {
    match outcome {
        StackOutcome::Rebased { onto, commits } => (
            ui::ActionStatus::Ok,
            format!(
                "rebased onto {onto} ({commits} commit{})",
                if *commits == 1 { "" } else { "s" }
            ),
        ),
        StackOutcome::AlreadyCurrent => (ui::ActionStatus::Skipped, "already on trunk".into()),
        StackOutcome::Skipped { reason } => (ui::ActionStatus::Skipped, reason.clone()),
        StackOutcome::Conflicted {
            onto,
            commits,
            message,
        } => (
            ui::ActionStatus::Warn,
            format!(
                "rebased onto {onto} with {commits} conflicted commit{} — {message}; run `jj resolve` to fix",
                if *commits == 1 { "" } else { "s" }
            ),
        ),
        StackOutcome::Failed { message } => (ui::ActionStatus::Error, message.clone()),
    }
}

/// `true` when any stack result is `Conflicted` or `Failed` — the
/// caller uses this to decide between exit 0 and exit 1.
pub fn any_unresolved(results: &[StackResult]) -> bool {
    results.iter().any(|r| {
        matches!(
            r.outcome,
            StackOutcome::Conflicted { .. } | StackOutcome::Failed { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::{BookmarkOrTrunk, StackedBookmark};

    fn sb(name: &str, parent: BookmarkOrTrunk) -> StackedBookmark {
        StackedBookmark {
            name: name.into(),
            parent,
        }
    }

    #[test]
    fn filter_by_bookmark_keeps_matching_stack_drops_others() {
        let s1 = DiscoveredStack {
            tip: "top-a".into(),
            bookmarks: vec![
                sb("bottom-a", BookmarkOrTrunk::Trunk),
                sb("top-a", BookmarkOrTrunk::Bookmark("bottom-a".into())),
            ],
            root_commit: Some("aaa111".into()),
        };
        let s2 = DiscoveredStack {
            tip: "top-b".into(),
            bookmarks: vec![sb("top-b", BookmarkOrTrunk::Trunk)],
            root_commit: Some("bbb222".into()),
        };

        let filtered = filter_by_bookmark(vec![s1.clone(), s2.clone()], "bottom-a");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tip, "top-a");

        let filtered = filter_by_bookmark(vec![s1.clone(), s2.clone()], "top-b");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tip, "top-b");

        let none = filter_by_bookmark(vec![s1, s2], "not-a-real-bookmark");
        assert!(none.is_empty());
    }

    #[test]
    fn any_unresolved_reports_conflicts_and_failures() {
        let clean = vec![StackResult {
            tip: "a".into(),
            outcome: StackOutcome::Rebased {
                onto: "main".into(),
                commits: 1,
            },
        }];
        assert!(!any_unresolved(&clean));

        let with_conflict = vec![StackResult {
            tip: "a".into(),
            outcome: StackOutcome::Conflicted {
                onto: "main".into(),
                commits: 1,
                message: "boom".into(),
            },
        }];
        assert!(any_unresolved(&with_conflict));

        let with_failure = vec![StackResult {
            tip: "a".into(),
            outcome: StackOutcome::Failed {
                message: "boom".into(),
            },
        }];
        assert!(any_unresolved(&with_failure));

        let skip_only = vec![StackResult {
            tip: "a".into(),
            outcome: StackOutcome::Skipped {
                reason: "earlier conflict".into(),
            },
        }];
        // Skipped on its own doesn't count as unresolved — the
        // underlying issue is on a different stack and gets caught
        // there.
        assert!(!any_unresolved(&skip_only));
    }

    #[test]
    fn outcome_to_row_pluralizes_commits() {
        let single = outcome_to_row(&StackOutcome::Rebased {
            onto: "main".into(),
            commits: 1,
        });
        assert!(single.1.contains("1 commit)"), "got: {}", single.1);
        let plural = outcome_to_row(&StackOutcome::Rebased {
            onto: "main".into(),
            commits: 3,
        });
        assert!(plural.1.contains("3 commits)"), "got: {}", plural.1);
    }

    #[test]
    fn outcome_to_row_maps_status_correctly() {
        let (status, _) = outcome_to_row(&StackOutcome::Rebased {
            onto: "main".into(),
            commits: 1,
        });
        assert!(matches!(status, ui::ActionStatus::Ok));

        let (status, _) = outcome_to_row(&StackOutcome::AlreadyCurrent);
        assert!(matches!(status, ui::ActionStatus::Skipped));

        let (status, _) = outcome_to_row(&StackOutcome::Conflicted {
            onto: "main".into(),
            commits: 2,
            message: "boom".into(),
        });
        assert!(matches!(status, ui::ActionStatus::Warn));

        let (status, _) = outcome_to_row(&StackOutcome::Failed {
            message: "boom".into(),
        });
        assert!(matches!(status, ui::ActionStatus::Error));
    }

    #[test]
    fn is_skippable_filters_trunk_and_gtmq() {
        assert!(is_skippable("main", "main"));
        assert!(is_skippable("gtmq_test_123", "main"));
        assert!(is_skippable("gtmq_anything", "main"));
        assert!(!is_skippable("feature-x", "main"));
        assert!(!is_skippable("bottom", "main"));
    }
}

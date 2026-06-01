//! Stack derivation: walk jj's revset graph to figure out (bookmark,
//! parent_bookmark) pairs for a set of selected bookmarks.
//!
//! The heuristic is one revset per selected bookmark:
//!
//! ```text
//! jj log -r 'heads(::<B> & bookmarks() ~ <B> ~ ::<trunk>)' \
//!        -T 'bookmarks.map(|b| b.name()).join("\n") ++ "\n"' \
//!        --no-graph
//! ```
//!
//! Reads as: "find the head commit(s) of bookmarks that are ancestors
//! of `<B>`, excluding `<B>` itself, and also excluding everything
//! that's already an ancestor of trunk." The output is the parent
//! bookmark name(s) — usually one, zero if `<B>` sits directly on
//! trunk, more than one for merge commits (jj-gt punts on those).

use crate::error::{JjGtError, Result};
use crate::jj::{JjCli, bookmarks_in_revset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackedBookmark {
    pub name: String,
    pub parent: BookmarkOrTrunk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkOrTrunk {
    Bookmark(String),
    Trunk,
}

impl BookmarkOrTrunk {
    pub fn as_branch_name<'a>(&'a self, trunk: &'a str) -> &'a str {
        match self {
            BookmarkOrTrunk::Bookmark(name) => name.as_str(),
            BookmarkOrTrunk::Trunk => trunk,
        }
    }
}

/// For each input bookmark, derive its parent bookmark (or trunk) by
/// querying jj's revset graph.
///
/// Strict — any per-bookmark revset failure aborts the whole call.
/// Callers like `submit_cmd` want this because the user explicitly
/// named the bookmark and "we silently dropped it" would be the
/// wrong UX. Callers that enumerate every local bookmark
/// (`cleanup::run_fetch`) should use [`derive_parents_lossy`]
/// instead so a single zombie bookmark doesn't wedge the whole
/// pipeline.
pub fn derive_parents(
    jj: &JjCli,
    bookmarks: &[String],
    trunk: &str,
) -> Result<Vec<StackedBookmark>> {
    let mut out = Vec::with_capacity(bookmarks.len());
    for b in bookmarks {
        out.push(derive_one(jj, b, trunk)?);
    }
    Ok(out)
}

/// Like [`derive_parents`] but logs and skips per-bookmark failures
/// instead of aborting. Returns the subset of bookmarks whose parent
/// resolved cleanly.
///
/// Use this when the caller enumerated every local bookmark and
/// wants the pipeline to make progress on the ones that ARE in good
/// shape — typically `cleanup::run_fetch` after a merged PR's
/// bookmark has been marked-deleted-on-remote but not yet exported
/// to git refs (so the name doesn't resolve to a commit, and the
/// revset for it fails with "Revision X doesn't exist").
pub fn derive_parents_lossy(jj: &JjCli, bookmarks: &[String], trunk: &str) -> Vec<StackedBookmark> {
    let mut out = Vec::with_capacity(bookmarks.len());
    for b in bookmarks {
        match derive_one(jj, b, trunk) {
            Ok(sb) => out.push(sb),
            Err(e) => {
                tracing::warn!(
                    "derive_parents_lossy: skipping `{b}` — could not resolve parent: {e}"
                );
            }
        }
    }
    out
}

fn derive_one(jj: &JjCli, bookmark: &str, trunk: &str) -> Result<StackedBookmark> {
    let revset = format!(
        "heads(::{b} & bookmarks() ~ {b} ~ ::{trunk})",
        b = bookmark,
        trunk = trunk,
    );
    let parents = bookmarks_in_revset(jj, &revset)?;
    match parents.as_slice() {
        [] => Ok(StackedBookmark {
            name: bookmark.into(),
            parent: BookmarkOrTrunk::Trunk,
        }),
        [one] => Ok(StackedBookmark {
            name: bookmark.into(),
            parent: BookmarkOrTrunk::Bookmark(one.clone()),
        }),
        many => Err(JjGtError::ParentDerivation {
            bookmark: bookmark.into(),
            reason: format!(
                "multiple parent bookmarks found ({many:?}) — likely a merge commit. \
                 Specify --parent manually."
            ),
        }),
    }
}

/// Identify the tip of a linear stack from a derived parent list.
///
/// The tip is the bookmark that has no other selected bookmark as a
/// descendant — i.e. the unique bookmark in the selection that is not
/// the parent of any other selected bookmark.
///
/// Returns [`JjGtError::NonLinearStack`] when the selection isn't
/// linear (two or more terminal bookmarks).
pub fn find_tip(stacked: &[StackedBookmark]) -> Result<String> {
    if stacked.is_empty() {
        return Err(JjGtError::NoBookmarksSelected);
    }

    let selected: std::collections::BTreeSet<&str> =
        stacked.iter().map(|s| s.name.as_str()).collect();

    // A bookmark X is a parent if any other selected bookmark Y has
    // parent == X. Tips are the ones that aren't parents of anyone
    // in the selection.
    let parent_names: std::collections::BTreeSet<&str> = stacked
        .iter()
        .filter_map(|s| match &s.parent {
            BookmarkOrTrunk::Bookmark(p) if selected.contains(p.as_str()) => Some(p.as_str()),
            _ => None,
        })
        .collect();

    let tips: Vec<&str> = selected.difference(&parent_names).copied().collect();
    match tips.as_slice() {
        [] => Err(JjGtError::NonLinearStack(
            "every bookmark is the parent of another — selection has a cycle?".into(),
        )),
        [one] => Ok((*one).to_owned()),
        many => Err(JjGtError::NonLinearStack(format!(
            "multiple stack tips in selection: {many:?}. Submit each tip separately."
        ))),
    }
}

/// Partition a derived bookmark list into independent stacks — one
/// entry per stack tip, each entry containing the full bottom→top
/// chain that leads to that tip.
///
/// The single-stack `find_tip` rejects multi-tip selections so the
/// caller (typically `submit_cmd`) can fan out instead. Independent
/// stacks arise legitimately when the user passes multiple unrelated
/// `-b` tips: each one's ancestor-expansion produces a disjoint
/// sub-DAG rooted at trunk.
///
/// The output partitions are in the same order their tips were first
/// discovered in `stacked`; within each partition the chain is
/// sorted via [`sort_for_tracking`] (bottom→top) so callers can feed
/// each partition straight into the `gt track` loop.
///
/// Returns an error if the union DAG is malformed — e.g. has a cycle
/// (impossible from `derive_parents` over a real jj graph but
/// defensive against synthesized inputs in tests).
pub fn partition_stacks(stacked: &[StackedBookmark]) -> Result<Vec<Vec<StackedBookmark>>> {
    if stacked.is_empty() {
        return Ok(Vec::new());
    }

    let by_name: std::collections::BTreeMap<&str, &StackedBookmark> =
        stacked.iter().map(|s| (s.name.as_str(), s)).collect();
    let selected: std::collections::BTreeSet<&str> = by_name.keys().copied().collect();

    let parent_names: std::collections::BTreeSet<&str> = stacked
        .iter()
        .filter_map(|s| match &s.parent {
            BookmarkOrTrunk::Bookmark(p) if selected.contains(p.as_str()) => Some(p.as_str()),
            _ => None,
        })
        .collect();

    // Tip set = bookmarks not pointed at by any other selected
    // bookmark. With one tip, this is the single-stack case; with N
    // tips, the input encodes N independent stacks rooted at trunk
    // (or at unselected ancestor bookmarks).
    let mut tips: Vec<&str> = stacked
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| !parent_names.contains(n))
        .collect();
    // Preserve first-appearance order rather than alphabetical so
    // the partitions in the output line up with the user's `-b`
    // ordering.
    tips.sort_by_key(|t| {
        stacked
            .iter()
            .position(|s| s.name == *t)
            .unwrap_or(usize::MAX)
    });

    if tips.is_empty() {
        return Err(JjGtError::NonLinearStack(
            "every bookmark is the parent of another — selection has a cycle?".into(),
        ));
    }

    let mut partitions: Vec<Vec<StackedBookmark>> = Vec::with_capacity(tips.len());
    for tip in tips {
        // Walk back through `selected`-internal parent edges to
        // gather every ancestor bookmark on this tip's chain. Stop
        // when the parent is trunk or an unselected name.
        let mut chain: Vec<StackedBookmark> = Vec::new();
        let mut cursor: Option<&str> = Some(tip);
        let mut visited: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        while let Some(name) = cursor {
            if !visited.insert(name) {
                return Err(JjGtError::NonLinearStack(format!(
                    "cycle detected at `{name}` while walking stack"
                )));
            }
            let Some(sb) = by_name.get(name).copied() else {
                break;
            };
            chain.push(sb.clone());
            cursor = match &sb.parent {
                BookmarkOrTrunk::Bookmark(p) if selected.contains(p.as_str()) => {
                    Some(by_name.get(p.as_str()).unwrap().name.as_str())
                }
                _ => None,
            };
        }
        // sort_for_tracking expects the bookmarks-list shape and
        // produces bottom→top. We have top→bottom from the walk;
        // hand it the same data and let sort_for_tracking do the
        // canonical ordering.
        partitions.push(sort_for_tracking(&chain));
    }

    Ok(partitions)
}

/// Topologically sort a derived stack so parents come before children.
/// Required because `gt track <child> --parent <parent>` errors out
/// if `<parent>` isn't already tracked. The user's bookmark-selection
/// order (`-b top -b bottom`) isn't necessarily bottom-up, so we
/// always sort before invoking gt.
///
/// Bookmarks whose parent isn't in the input list (e.g. their parent
/// is trunk, or an unselected ancestor bookmark) sort to the front.
/// Cycles are impossible in a derived stack (jj's commit graph is a
/// DAG); a defensive cycle check would just fall back to the input
/// order, which gt would then reject.
pub fn sort_for_tracking(stacked: &[StackedBookmark]) -> Vec<StackedBookmark> {
    let selected: std::collections::BTreeSet<&str> =
        stacked.iter().map(|s| s.name.as_str()).collect();

    let by_name: std::collections::BTreeMap<&str, &StackedBookmark> =
        stacked.iter().map(|s| (s.name.as_str(), s)).collect();

    let mut out: Vec<StackedBookmark> = Vec::with_capacity(stacked.len());
    let mut emitted: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();

    fn visit<'a>(
        name: &'a str,
        by_name: &std::collections::BTreeMap<&'a str, &'a StackedBookmark>,
        selected: &std::collections::BTreeSet<&'a str>,
        emitted: &mut std::collections::BTreeSet<&'a str>,
        out: &mut Vec<StackedBookmark>,
    ) {
        if emitted.contains(name) {
            return;
        }
        let sb = by_name.get(name).copied();
        if let Some(sb) = sb {
            if let BookmarkOrTrunk::Bookmark(parent) = &sb.parent
                && selected.contains(parent.as_str())
            {
                visit(parent.as_str(), by_name, selected, emitted, out);
            }
            emitted.insert(name);
            out.push(sb.clone());
        }
    }

    for sb in stacked {
        visit(
            sb.name.as_str(),
            &by_name,
            &selected,
            &mut emitted,
            &mut out,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb(name: &str, parent: BookmarkOrTrunk) -> StackedBookmark {
        StackedBookmark {
            name: name.into(),
            parent,
        }
    }

    #[test]
    fn find_tip_linear_three() {
        let stack = vec![
            sb("bottom", BookmarkOrTrunk::Trunk),
            sb("mid", BookmarkOrTrunk::Bookmark("bottom".into())),
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
        ];
        assert_eq!(find_tip(&stack).unwrap(), "top");
    }

    #[test]
    fn find_tip_single_on_trunk() {
        let stack = vec![sb("solo", BookmarkOrTrunk::Trunk)];
        assert_eq!(find_tip(&stack).unwrap(), "solo");
    }

    #[test]
    fn find_tip_two_parallel_errors() {
        let stack = vec![
            sb("branch_a", BookmarkOrTrunk::Trunk),
            sb("branch_b", BookmarkOrTrunk::Trunk),
        ];
        let err = find_tip(&stack).unwrap_err();
        assert!(matches!(err, JjGtError::NonLinearStack(_)));
    }

    #[test]
    fn find_tip_empty_errors() {
        assert!(matches!(
            find_tip(&[]).unwrap_err(),
            JjGtError::NoBookmarksSelected
        ));
    }

    #[test]
    fn find_tip_external_parent_treated_as_tip_origin() {
        // If a bookmark's parent is NOT in the selection (e.g. only the
        // top two of a three-deep stack were selected), the bookmark
        // with the external parent is still a candidate root — not the
        // tip. The bookmark not pointed at by anyone else is the tip.
        let stack = vec![
            sb(
                "mid",
                BookmarkOrTrunk::Bookmark("bottom_not_selected".into()),
            ),
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
        ];
        assert_eq!(find_tip(&stack).unwrap(), "top");
    }

    #[test]
    fn sort_for_tracking_preserves_already_ordered_stack() {
        let stack = vec![
            sb("bottom", BookmarkOrTrunk::Trunk),
            sb("mid", BookmarkOrTrunk::Bookmark("bottom".into())),
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
        ];
        let sorted = sort_for_tracking(&stack);
        assert_eq!(
            sorted.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["bottom", "mid", "top"],
        );
    }

    #[test]
    fn sort_for_tracking_reorders_top_first_input() {
        // User passed `-b top -b bottom -b mid`. We must emit
        // bottom→mid→top because gt requires parents tracked first.
        let stack = vec![
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
            sb("bottom", BookmarkOrTrunk::Trunk),
            sb("mid", BookmarkOrTrunk::Bookmark("bottom".into())),
        ];
        let sorted = sort_for_tracking(&stack);
        assert_eq!(
            sorted.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["bottom", "mid", "top"],
        );
    }

    #[test]
    fn sort_for_tracking_external_parent_emits_at_front() {
        // mid's parent is an unselected bookmark — we treat mid as a
        // root for ordering purposes.
        let stack = vec![
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
            sb(
                "mid",
                BookmarkOrTrunk::Bookmark("bottom_not_selected".into()),
            ),
        ];
        let sorted = sort_for_tracking(&stack);
        assert_eq!(
            sorted.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["mid", "top"],
        );
    }

    #[test]
    fn sort_for_tracking_single_bookmark_returns_as_is() {
        let stack = vec![sb("solo", BookmarkOrTrunk::Trunk)];
        let sorted = sort_for_tracking(&stack);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].name, "solo");
    }

    #[test]
    fn sort_for_tracking_empty_input() {
        let sorted = sort_for_tracking(&[]);
        assert!(sorted.is_empty());
    }

    #[test]
    fn partition_stacks_empty_input_returns_empty() {
        let partitions = partition_stacks(&[]).unwrap();
        assert!(partitions.is_empty());
    }

    #[test]
    fn partition_stacks_single_linear_chain_is_one_partition() {
        // The PR-A common case: one tip, ancestor chain expanded.
        // Partition output is a single bottom→top sorted partition.
        let stack = vec![
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
            sb("bottom", BookmarkOrTrunk::Trunk),
            sb("mid", BookmarkOrTrunk::Bookmark("bottom".into())),
        ];
        let partitions = partition_stacks(&stack).unwrap();
        assert_eq!(partitions.len(), 1);
        let names: Vec<&str> = partitions[0].iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["bottom", "mid", "top"]);
    }

    #[test]
    fn partition_stacks_two_independent_chains() {
        // The issue we're fixing: user passes `-b a-tip -b b-tip`
        // where the two tips sit on unrelated stacks both rooted
        // at trunk. Each ancestor expansion produces a disjoint
        // sub-DAG; partition_stacks should produce two
        // partitions, each its own bottom→top chain.
        let stack = vec![
            sb("a-tip", BookmarkOrTrunk::Bookmark("a-mid".into())),
            sb("a-mid", BookmarkOrTrunk::Bookmark("a-bottom".into())),
            sb("a-bottom", BookmarkOrTrunk::Trunk),
            sb("b-tip", BookmarkOrTrunk::Bookmark("b-bottom".into())),
            sb("b-bottom", BookmarkOrTrunk::Trunk),
        ];
        let partitions = partition_stacks(&stack).unwrap();
        assert_eq!(partitions.len(), 2);

        // First partition contains the a-* chain, second contains
        // the b-* chain. Within each, the order is bottom→top.
        let a_names: Vec<&str> = partitions[0].iter().map(|s| s.name.as_str()).collect();
        let b_names: Vec<&str> = partitions[1].iter().map(|s| s.name.as_str()).collect();
        assert_eq!(a_names, vec!["a-bottom", "a-mid", "a-tip"]);
        assert_eq!(b_names, vec!["b-bottom", "b-tip"]);
    }

    #[test]
    fn partition_stacks_two_single_bookmarks_on_trunk() {
        // `jj-gt submit -b feature-x -b feature-y` where both are
        // single bookmarks sitting directly on trunk. Two
        // partitions, one bookmark each.
        let stack = vec![
            sb("feature-x", BookmarkOrTrunk::Trunk),
            sb("feature-y", BookmarkOrTrunk::Trunk),
        ];
        let partitions = partition_stacks(&stack).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].len(), 1);
        assert_eq!(partitions[0][0].name, "feature-x");
        assert_eq!(partitions[1].len(), 1);
        assert_eq!(partitions[1][0].name, "feature-y");
    }

    #[test]
    fn partition_stacks_preserves_first_appearance_order() {
        // Tip order in the output should match first-appearance
        // order in the input rather than alphabetical so the
        // partitions line up with the user's `-b` ordering.
        let stack = vec![
            sb("zulu", BookmarkOrTrunk::Trunk),
            sb("alpha", BookmarkOrTrunk::Trunk),
        ];
        let partitions = partition_stacks(&stack).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0][0].name, "zulu");
        assert_eq!(partitions[1][0].name, "alpha");
    }

    #[test]
    fn partition_stacks_partial_selection_shared_root() {
        // Two tips that share an unselected ancestor (root not
        // included in the selection). Each tip is its own
        // partition; the shared ancestor isn't in either.
        let stack = vec![
            sb(
                "a-mid",
                BookmarkOrTrunk::Bookmark("root_not_selected".into()),
            ),
            sb(
                "b-mid",
                BookmarkOrTrunk::Bookmark("root_not_selected".into()),
            ),
        ];
        let partitions = partition_stacks(&stack).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0][0].name, "a-mid");
        assert_eq!(partitions[1][0].name, "b-mid");
    }
}

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
}

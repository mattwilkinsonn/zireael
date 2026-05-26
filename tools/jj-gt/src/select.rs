//! Bookmark-selection resolution.
//!
//! Implements the same precedence as `jj git push` so users can mix
//! `-b NAME`, `-r REVSET`, `-c REVSET`, `--all`, `--tracked` without
//! surprise.

use std::collections::BTreeSet;

use crate::cli::BookmarkArgs;
use crate::error::Result;
use crate::jj::{JjCli, bookmarks_in_revset};

/// Resolve the user's bookmark-selection flags into a concrete list of
/// local bookmark names.
///
/// Precedence (matches `jj git push`):
///
/// 1. `--all`: every local bookmark in `bookmarks() & ::@ & trunk..`.
/// 2. `--tracked`: every local bookmark with a remote counterpart on
///    `--remote`.
/// 3. Otherwise, the union of `-b` literals + `-r` / `-c` revset
///    expansions.
/// 4. If the result is empty AND no flag was given, fall back to
///    `jj git push`'s default: bookmarks at `@` or its ancestors that
///    need pushing.
pub fn resolve_bookmarks(jj: &JjCli, args: &BookmarkArgs, trunk: &str) -> Result<Vec<String>> {
    if args.all {
        let revset = format!("bookmarks() & ::@ & {trunk}..");
        return Ok(dedup_in_order(bookmarks_in_revset(jj, &revset)?));
    }

    if args.tracked {
        let revset = format!(
            "bookmarks() & remote_bookmarks(remote=exact:{})",
            args.remote
        );
        return Ok(dedup_in_order(bookmarks_in_revset(jj, &revset)?));
    }

    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for b in &args.bookmark {
        if seen.insert(b.clone()) {
            out.push(b.clone());
        }
    }

    for r in args.revision.iter().chain(args.change.iter()) {
        let expanded = bookmarks_in_revset(jj, &format!("bookmarks() & ({r})"))?;
        for name in expanded {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }

    if out.is_empty() && !any_flag_set(args) {
        // `jj git push` default: bookmarks at @ or its ancestors that
        // haven't been pushed yet. We approximate as "bookmarks on the
        // @-ancestor chain between trunk and @" — that's the most
        // common interpretation and matches what `--all` does.
        let revset = format!("bookmarks() & ::@ & {trunk}..");
        let fallback = bookmarks_in_revset(jj, &revset)?;
        return Ok(dedup_in_order(fallback));
    }

    Ok(out)
}

fn any_flag_set(args: &BookmarkArgs) -> bool {
    !args.bookmark.is_empty() || !args.revision.is_empty() || !args.change.is_empty()
}

fn dedup_in_order(items: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::BookmarkArgs;

    fn args() -> BookmarkArgs {
        BookmarkArgs {
            remote: "origin".into(),
            ..BookmarkArgs::default()
        }
    }

    #[test]
    fn no_flags_set_returns_false() {
        assert!(!any_flag_set(&args()));
    }

    #[test]
    fn bookmark_flag_set_returns_true() {
        let mut a = args();
        a.bookmark = vec!["foo".into()];
        assert!(any_flag_set(&a));
    }

    #[test]
    fn dedup_in_order_preserves_first_seen() {
        let items = vec!["a", "b", "a", "c", "b"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(dedup_in_order(items), vec!["a", "b", "c"]);
    }
}

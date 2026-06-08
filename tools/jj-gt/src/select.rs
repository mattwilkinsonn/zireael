//! Bookmark-selection resolution.
//!
//! Implements the same precedence as `jj git push` so users can mix
//! `-b NAME`, `-r REVSET`, `-c REVSET`, `--all`, `--tracked` without
//! surprise.

use std::collections::BTreeSet;

use crate::cleanup::{default_gtmq_prefixes_owned, is_gtmq_branch};
use crate::cli::BookmarkArgs;
use crate::error::Result;
use crate::jj::{JjCli, bookmarks_in_revset};

/// Resolve the user's bookmark-selection flags into a concrete list of
/// local bookmark names.
///
/// Precedence (matches `jj git push` shape, with one deliberate
/// divergence on `--all` for the jjui-cursor workflow — see below):
///
/// 1. `--all`: every local bookmark on ANY stack across the repo
///    (`bookmarks() & {trunk}..`), with trunk + `gtmq_*` queue
///    branches filtered out. This is the "submit every stack I'm
///    working on" shape `x S` in jjui wants — pre-jjui code anchored
///    `--all` at `@`, but jjui's cursor moves freely without `@`, so
///    an `@`-anchored "all" no longer matches user intent.
/// 2. `--tracked`: every local bookmark with a remote counterpart on
///    `--remote`.
/// 3. Otherwise, the union of `-b` literals + `-r` / `-c` revset
///    expansions.
/// 4. If the result is empty AND no flag was given, fall back to
///    `jj git push`'s default: bookmarks at `@` or its ancestors that
///    need pushing. (This is what `--all` used to mean and is the
///    natural "operate on the focused stack" shape `jj-gt submit`
///    bareword still provides.)
pub fn resolve_bookmarks(jj: &JjCli, args: &BookmarkArgs, trunk: &str) -> Result<Vec<String>> {
    if args.all {
        // Every local bookmark on a non-trunk commit, across every
        // stack in the repo. Filter trunk + gtmq_* queue branches so
        // the caller gets the "real" set of work bookmarks.
        let revset = format!("bookmarks() & {trunk}..");
        let raw = bookmarks_in_revset(jj, &revset)?;
        let gtmq_prefixes = default_gtmq_prefixes_owned();
        let filtered = raw
            .into_iter()
            .filter(|name| !is_gtmq_branch(name, &gtmq_prefixes))
            .collect();
        return Ok(dedup_in_order(filtered));
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
        // @-ancestor chain between trunk and @" — the focused-stack
        // shape `x S` used to do (before `--all` got broadened to
        // cover every stack).
        let revset = format!("bookmarks() & ::@ & {trunk}..");
        let fallback = bookmarks_in_revset(jj, &revset)?;
        return Ok(dedup_in_order(fallback));
    }

    Ok(out)
}

/// Expand a selected bookmark set to include every bookmark on the
/// ancestor chain between `trunk` and each selected tip.
///
/// Closes the `submit -b <tip>` UX gap: `gt track <child> --parent
/// <parent>` errors out if `<parent>` isn't already tracked, but the
/// user shouldn't have to enumerate every bookmark in their stack
/// when they just want to ship the tip. `resolve_bookmarks` parses
/// the literal `-b` selection (that's its contract); this helper
/// layers submit-shape semantics on top by walking each tip's
/// ancestor chain via `bookmarks() & ::<tip> & {trunk}..` and
/// returning the union, deduped, in the order jj emits them
/// (bottom→top per chain).
///
/// Called only from `submit_cmd`. `track_cmd`, `status_cmd`, and
/// `log_cmd` keep using `resolve_bookmarks` directly because they
/// don't have the same ordering constraint and the user typically
/// wants them to act on the literal selection.
pub fn expand_ancestors_for_submit(
    jj: &JjCli,
    selected: &[String],
    trunk: &str,
) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for tip in selected {
        let revset = format!("bookmarks() & ::{tip} & {trunk}..");
        let chain = bookmarks_in_revset(jj, &revset)?;
        if chain.is_empty() {
            // Defensive: tip sits exactly on trunk (no commits
            // between trunk and tip). The revset returns the empty
            // set but the user still wants the tip itself in the
            // output. Fall back to a literal one-element entry —
            // derive_parents will classify it as parent=Trunk and
            // the tracker will accept it.
            if seen.insert(tip.clone()) {
                out.push(tip.clone());
            }
            continue;
        }
        for name in chain {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
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

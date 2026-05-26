//! Subprocess wrapper around the `jj` CLI for jj-gt's needs.
//!
//! We re-export `jj_hooks::jj::JjCli` so callers can pass the same handle
//! into [`jj_hooks::run_for_revset`] without juggling two CLI wrappers.
//! Everything jj-gt-specific (revset queries for stack derivation,
//! `jj rebase`, `jj bookmark delete`, etc.) hangs off helper functions
//! that take a `&JjCli`.

use std::path::Path;
use std::process::Command;

use crate::error::{JjGtError, Result};

pub use jj_hooks::jj::JjCli;
pub use jj_hooks::jj::primary_git_dir;

/// A local jj bookmark + its commit id (resolved short hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBookmark {
    pub name: String,
    pub commit_id: String,
}

/// `jj log -r '<revset>' -T 'bookmarks.map(|b| b.name()).join("\n") ++ "\n"'
///        --no-graph --ignore-working-copy`
///
/// Returns deduplicated bookmark names. Used for stack-parent derivation
/// and revset→bookmark expansion in the selection layer.
pub fn bookmarks_in_revset(jj: &JjCli, revset: &str) -> Result<Vec<String>> {
    let out = jj_run(
        jj,
        &[
            "log",
            "--no-graph",
            "-r",
            revset,
            "-T",
            r#"bookmarks.map(|b| b.name()).join("\n") ++ "\n""#,
            "--ignore-working-copy",
        ],
    )?;
    Ok(unique_lines(&out))
}

/// `jj bookmark list --ignore-working-copy
///   -T 'name ++ " " ++ if(self.normal_target(),
///                          self.normal_target().commit_id().short(12),
///                          "") ++ "\n"'`
///
/// Why the if-guard: bookmark templates expose `normal_target()` as
/// an `Option<Commit>` — `None` for conflicted or pure-deletion
/// entries. Unwrapping it directly would template-error on the
/// conflict case, so we fall through to an empty commit-id string
/// and skip the entry below in the parser. There's no top-level
/// `commit_id` keyword in the bookmark scope (that exists on the
/// commit scope used by `jj log` templates).
pub fn list_local_bookmarks(jj: &JjCli) -> Result<Vec<LocalBookmark>> {
    let out = jj_run(
        jj,
        &[
            "bookmark",
            "list",
            "-T",
            r#"name ++ " " ++ if(self.normal_target(), self.normal_target().commit_id().short(12), "") ++ "\n""#,
            "--ignore-working-copy",
        ],
    )?;
    let mut bookmarks = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(commit_id)) = (parts.next(), parts.next()) else {
            // Conflict-target bookmark (name printed, commit_id
            // empty). Skip — callers don't need to act on conflicts
            // here; jj will surface the conflict via its own UI on
            // any operation that actually depends on the target.
            continue;
        };
        bookmarks.push(LocalBookmark {
            name: name.to_owned(),
            commit_id: commit_id.to_owned(),
        });
    }
    Ok(bookmarks)
}

/// `jj git export --ignore-working-copy` — idempotent sync of jj
/// bookmarks into git refs. In colocated repos jj exports on most
/// operations, but running it explicitly is cheap and ensures gt
/// sees the same world jj does.
///
/// `--ignore-working-copy` matters because jj-gt invokes this in
/// the middle of pipelines that already manipulate refs (fetch,
/// submit). Letting jj auto-snapshot here would have it re-anchor
/// `@` against the just-imported refs, producing an extra empty
/// commit on every invocation. We don't need the working-copy
/// state for an export; it's a pure refs operation.
pub fn git_export(jj: &JjCli) -> Result<()> {
    let _ = jj_run(jj, &["git", "export", "--ignore-working-copy"])?;
    Ok(())
}

/// `jj git fetch --remote <remote> --ignore-working-copy`.
///
/// Same `--ignore-working-copy` rationale as `git_export`: fetch
/// updates `refs/remotes/<remote>/*` and can trigger jj's
/// "abandon-and-rebase descendants" logic on `@` if the remote's
/// view of trunk advanced past the local one. Pre-fix, calling
/// this in jj-gt's fetch pipeline produced an empty `@` commit
/// rebased onto the new trunk; combined with the multiple
/// import-style ops downstream, users ended up with 3-5 empty
/// floating commits per `jj-gt fetch`.
pub fn git_fetch(jj: &JjCli, remote: &str) -> Result<()> {
    let _ = jj_run(
        jj,
        &["git", "fetch", "--remote", remote, "--ignore-working-copy"],
    )?;
    Ok(())
}

/// `jj git import --ignore-working-copy`. Run after external tooling
/// (gt sync) mutates refs on the git side so jj's view catches up.
pub fn git_import(jj: &JjCli) -> Result<()> {
    let _ = jj_run(jj, &["git", "import", "--ignore-working-copy"])?;
    Ok(())
}

/// `jj log -r @ --no-graph -T change_id`. Captured before gt submit so
/// we can restore `@` after — gt's git-push triggers a jj ref-import
/// that moves `@` as a side effect.
pub fn current_change_id(jj: &JjCli) -> Result<String> {
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            "change_id",
            "--ignore-working-copy",
        ],
    )?;
    Ok(out.trim().to_owned())
}

/// `jj log -r <revset> --no-graph -T commit_id --limit 1`.
///
/// Cheap point query for resolving a revset (e.g. a bookmark name or
/// trunk name) down to its full commit id. Used by the submit
/// pipeline to build a real BookmarkUpdate for `jj_hooks` instead
/// of relying on the revset-string synthesis layer in
/// `run_for_revset_outcome`.
///
/// Errors when the revset resolves to zero commits — callers
/// generally want a hard failure in that case rather than an empty
/// Option to thread through, because they're already certain the
/// bookmark / trunk exists by the time they ask.
pub fn resolve_commit_id(jj: &JjCli, revset: &str) -> Result<String> {
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            revset,
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Err(JjGtError::Invalid(format!(
            "revset `{revset}` resolved to no commits"
        )));
    }
    Ok(trimmed.to_owned())
}

/// `jj edit <change_id>`. Restores `@` to a previously-recorded
/// change id.
pub fn edit_change(jj: &JjCli, change_id: &str) -> Result<()> {
    let _ = jj_run(jj, &["edit", change_id])?;
    Ok(())
}

/// `jj bookmark track <name> --remote <remote>` (idempotent — succeeds if
/// already tracked).
///
/// Why jj-gt has to do this: when gt pushes a bookmark via raw
/// `git push`, jj has no idea the push happened. The next
/// `jj git import` (or fetch) sees the new `refs/remotes/<remote>/<name>`
/// ref but treats it as an externally-created bookmark — no
/// tracking link to the local bookmark of the same name. The
/// bookmark lands in `untracked_remote_bookmarks()`, which is part
/// of jj's default `immutable_heads()` revset, which freezes every
/// commit in the bookmark's ancestry.
///
/// `jj git push` doesn't have this problem because jj sets up the
/// tracking relationship as part of its own push. We replicate
/// that explicitly after `gt submit` so jj-gt users get the same
/// "amend in place + force-push" workflow they'd have on the
/// vanilla `jj git push` path.
pub fn track_bookmark_on_remote(jj: &JjCli, bookmark: &str, remote: &str) -> Result<()> {
    let _ = jj_run(
        jj,
        &[
            "bookmark",
            "track",
            bookmark,
            "--remote",
            remote,
            "--ignore-working-copy",
        ],
    )?;
    Ok(())
}

/// `jj bookmark list --tracked --remote <remote> -T 'name ++ "\n"'`.
///
/// Returns the set of bookmark names that are already tracked on
/// `remote`. Used by the post-submit pipeline to skip re-tracking
/// (jj warns "Remote bookmark already tracked" for every redundant
/// call, which clutters submit output without doing any work).
///
/// `--ignore-working-copy` matches the rest of this module — keeps
/// the call cheap and doesn't snapshot the working tree.
pub fn list_tracked_bookmarks_on_remote(
    jj: &JjCli,
    remote: &str,
) -> Result<std::collections::BTreeSet<String>> {
    let out = jj_run(
        jj,
        &[
            "bookmark",
            "list",
            "--tracked",
            "--remote",
            remote,
            "-T",
            r#"name ++ "\n""#,
            "--ignore-working-copy",
        ],
    )?;
    Ok(out
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Outcome of a `jj rebase` invocation that exits 0 — broken out
/// because jj treats "rebased successfully but the result contains
/// conflict markers" as a success exit code, and the only signal is
/// stderr text like `New conflicts appeared in N commits`. Callers
/// (cleanup step 7) need to surface that to the user as a distinct
/// CleanupAction, not silently log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// Clean rebase; no conflict markers introduced.
    Clean,
    /// jj rebased without erroring but the result has new conflicts.
    /// `message` carries the relevant stderr line(s) so the caller
    /// can echo them back to the user.
    Conflicted { message: String },
    /// jj decided nothing needed to change (already-in-place).
    NoOp,
}

/// `jj rebase -s <source_revset> -d <dest>`. Used by `jj-gt fetch`'s
/// orphan-restack step in place of `gt restack` (whose git-rebase
/// rewrites jj-tracked SHAs and confuses jj's ref reconciliation).
///
/// Returns `RebaseOutcome::Conflicted` when jj exits 0 *but* its
/// stderr mentions newly-introduced conflicts — jj's CLI doesn't
/// surface this as a non-zero exit, and the only way to detect it
/// from a subprocess is by sniffing the human message. Failing
/// loudly here is the difference between the user noticing a broken
/// stack now vs noticing it three commits later.
pub fn rebase(jj: &JjCli, source_revset: &str, dest: &str) -> Result<RebaseOutcome> {
    let combined = jj
        .run_capture_stderr(&[
            "rebase",
            "-s",
            source_revset,
            "-d",
            dest,
            "--ignore-working-copy",
        ])
        .map_err(JjGtError::Hooks)?;

    if combined.contains("Nothing changed.") || combined.contains("Skipped rebase") {
        // jj prints "Nothing changed." when the rebase is a no-op
        // (the bookmark is already on dest's ancestry). Treat as
        // NoOp so cleanup doesn't claim it rebased something it
        // didn't.
        return Ok(RebaseOutcome::NoOp);
    }
    if combined.contains("New conflicts appeared") {
        // Carry the most relevant stderr line back to the caller so
        // the printed CleanupAction includes the same wording jj
        // itself used.
        let message = combined
            .lines()
            .find(|l| l.contains("New conflicts appeared"))
            .unwrap_or("New conflicts appeared in rebased commits")
            .to_owned();
        return Ok(RebaseOutcome::Conflicted { message });
    }
    Ok(RebaseOutcome::Clean)
}

/// `jj bookmark delete <name> --ignore-working-copy`.
pub fn delete_bookmark(jj: &JjCli, name: &str) -> Result<()> {
    let _ = jj_run(jj, &["bookmark", "delete", name, "--ignore-working-copy"])?;
    Ok(())
}

/// Shell out to `git push --delete <remote> <branch>` from within the
/// workspace. We use git rather than `jj git push --bookmark <name>
/// --deleted` because the queue-test branches we prune via this path
/// (`gtmq_*`) are typically bot-created and never tracked by jj as
/// local bookmarks — `jj git push --deleted` only deletes branches
/// that have local bookmark entries.
pub fn delete_remote_branch(workspace_root: &Path, remote: &str, branch: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["push", "--delete", remote, branch])
        .current_dir(workspace_root)
        .output()?;
    if !out.status.success() {
        return Err(JjGtError::JjFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(())
}

/// Search the commits between `<trunk>..` (descendants of trunk on the
/// local graph) for one whose description ends in `(#<pr_number>)` —
/// the squash-merge suffix github writes when a queued PR lands on
/// trunk. Returns the commit id of the merge marker if found.
///
/// We deliberately exclude `Revert "..."` subjects so a revert commit
/// that mentions `(#N)` doesn't false-positive as a merge marker.
pub fn find_pr_merge_marker_on_trunk(
    jj: &JjCli,
    pr_number: u32,
    trunk: &str,
) -> Result<Option<String>> {
    // Match on the trunk ancestry only, with a generous descendant cap.
    let revset = format!(
        r#"description(regex:"\\(#{n}\\)\\s*$") & ::{trunk} ~ description(glob:"Revert \"*")"#,
        n = pr_number,
        trunk = trunk,
    );
    let out = jj_run(
        jj,
        &[
            "log",
            "--no-graph",
            "-r",
            &revset,
            "-T",
            r#"commit_id ++ "\n""#,
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )?;
    let first = out.lines().next().map(|l| l.trim().to_owned());
    Ok(first.filter(|s| !s.is_empty()))
}

/// Run a jj subcommand, capturing stdout. Stderr is propagated to the
/// user's terminal on success and folded into the error on failure.
fn jj_run(jj: &JjCli, args: &[&str]) -> Result<String> {
    jj.run(args).map_err(JjGtError::Hooks)
}

fn unique_lines(s: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if seen.insert(line.to_owned()) {
            out.push(line.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unique_lines;

    #[test]
    fn unique_lines_dedups_in_order() {
        let input = "a\nb\nA\nb\nc\n";
        assert_eq!(unique_lines(input), vec!["a", "b", "A", "c"]);
    }

    #[test]
    fn unique_lines_skips_blanks_and_trims() {
        let input = "  a  \n\n  b\n\n a\n";
        assert_eq!(unique_lines(input), vec!["a", "b"]);
    }
}

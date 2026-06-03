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

/// `jj log -r '<revset>' -T 'local_bookmarks.map(|b| b.name()).join("\n") ++ "\n"'
///        --no-graph --ignore-working-copy`
///
/// Returns deduplicated bookmark names. Used for stack-parent derivation
/// and revset→bookmark expansion in the selection layer.
///
/// Uses `local_bookmarks` (not the umbrella `bookmarks` keyword) so
/// remote-only refs — graphite's `graphite-base/<N>@origin` markers,
/// stale `<branch>@<remote>` entries that haven't been pruned — don't
/// leak into the candidate list. A `derive_parents` query that hits
/// `graphite-base/43@origin` and a real local bookmark sitting on the
/// same commit was tripping the "multiple parent bookmarks found"
/// error before this filter; local-only is the right scope for every
/// caller (stack derivation, selection expansion — none of them care
/// about remote-only refs).
pub fn bookmarks_in_revset(jj: &JjCli, revset: &str) -> Result<Vec<String>> {
    let out = jj_run(
        jj,
        &[
            "log",
            "--no-graph",
            "-r",
            revset,
            "-T",
            r#"local_bookmarks.map(|b| b.name()).join("\n") ++ "\n""#,
            "--ignore-working-copy",
        ],
    )?;
    Ok(unique_lines(&out))
}

/// `jj bookmark list --ignore-working-copy
///   -T 'if(self.present(),
///          name ++ " " ++ if(self.normal_target(),
///                            self.normal_target().commit_id().short(12),
///                            ""),
///          "") ++ "\n"'`
///
/// Why the `present()` guard: a bookmark whose remote ref has been
/// deleted (a merged PR + post-merge cleanup, say) still appears in
/// `jj bookmark list` until the next `jj git export` writes the
/// deletion to the underlying git refs, but its target is a
/// "pending-deletion" sentinel. Any revset that names it
/// (`heads(::sea-559 & ...)`) fails with
/// "Revision `sea-559` doesn't exist" because the name no longer
/// resolves to a commit. We pre-filter via `present()` so the
/// fetch pipeline's `derive_parents` call never sees these zombie
/// names.
///
/// Why the inner `normal_target()` guard: bookmark templates expose
/// `normal_target()` as an `Option<Commit>` — `None` for conflicted
/// or pure-deletion entries. Unwrapping it directly would
/// template-error on the conflict case, so we fall through to an
/// empty commit-id string and skip the entry below in the parser.
/// There's no top-level `commit_id` keyword in the bookmark scope
/// (that exists on the commit scope used by `jj log` templates).
pub fn list_local_bookmarks(jj: &JjCli) -> Result<Vec<LocalBookmark>> {
    let out = jj_run(
        jj,
        &[
            "bookmark",
            "list",
            "-T",
            r#"if(self.present(), name ++ " " ++ if(self.normal_target(), self.normal_target().commit_id().short(12), ""), "") ++ "\n""#,
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

/// Return the set of local bookmark names whose target is
/// conflicted (the `??` notation in `jj bookmark list` —
/// different op-log lineages have moved the bookmark to different
/// commits and jj can't pick one without a user decision).
///
/// jj's bookmark template exposes `self.conflict()` as a bool
/// (true when the bookmark target is in conflict). We filter for
/// `present()` first so pending-deletion zombies — gone on the
/// remote but not yet exported to git — don't show up as
/// "conflicted" when they're really just deleted.
///
/// Used by [`crate::cleanup::orphan_rebase_phase`] (issue #68):
/// rebasing against a conflicted bookmark name fails with
/// "Name `<bm>` is conflicted" rather than a content-conflict
/// error. We detect the divergence up-front and emit a
/// `BookmarkConflicted` action instead so the per-bookmark
/// summary surfaces the actual problem.
pub fn list_conflicted_bookmarks(jj: &JjCli) -> Result<std::collections::BTreeSet<String>> {
    let out = jj_run(
        jj,
        &[
            "bookmark",
            "list",
            "-T",
            r#"if(self.present() && self.conflict(), name ++ "\n", "")"#,
            "--ignore-working-copy",
        ],
    )?;
    let names = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(names)
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

/// Shelter pending working-copy edits by creating a fresh empty
/// change on top of `@`. After this, the previous `@` carries the
/// user's edits as a real committed change and the new `@` is an
/// empty sibling above it — concurrent jj snapshots can't disturb
/// the sheltered edits because future operations target the new
/// (empty) `@`.
///
/// Mechanism: plain `jj new @`. jj snapshots the working copy into
/// the old `@` first (default behavior), creates a new empty child,
/// and switches `@` to that child. The working-copy *files* don't
/// change on disk; what changes is which change_id those files are
/// associated with — and that's exactly the safety property we
/// want.
///
/// We intentionally do NOT pass `--ignore-working-copy`: the
/// snapshot-before-new is the load-bearing step that converts the
/// edits from "pending against `@`" to "committed in `@`."
///
/// Closes part of issue #1 — pairs with `lock::PipelineLock` and
/// supersedes the older `--force-with-changes` refusal flag (which
/// only let the user bypass detection, doing nothing about the
/// underlying hazard).
///
/// The function is silent on its own; user-facing output is the
/// caller's responsibility (typically via the `ui::Step` machinery
/// so the action shows up in the per-step list alongside Fetch /
/// Submit).
pub fn shelter_uncommitted_edits(jj: &JjCli) -> Result<()> {
    let _ = jj_run(jj, &["new", "@"])?;
    Ok(())
}

/// Outcome of [`ensure_workspace_current`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStaleOutcome {
    /// `jj workspace update-stale` ran and reported nothing to do —
    /// `@` was already current relative to the op log.
    NotStale,
    /// `@` moved to catch up with op-log changes made by another
    /// workspace (or by external tooling). Carries the
    /// before/after change-ids so the caller can surface a
    /// one-liner like "caught up: @ moved from <pre> to <post>".
    Updated {
        from_change_id: String,
        to_change_id: String,
    },
    /// The before/after change-id probe couldn't read `@` cleanly,
    /// so we can't tell whether the update moved anything. The
    /// `jj workspace update-stale` call itself succeeded — if it
    /// hadn't, the caller would have seen an `Err`. This variant
    /// is the explicit "we tried but the signal was muddy" answer,
    /// distinct from `NotStale` which means "we know nothing moved."
    CouldNotVerify,
}

/// `jj workspace update-stale` — catch this workspace up with any
/// op-log moves made by sibling workspaces sharing the same
/// `.jj/`. Issue #67 rationale: the agent + supervisor model
/// shares one `.jj/` across N workspaces, so the "working copy
/// stale" check that fires when one workspace's op log advances
/// past another's is friction by default. Auto-running this at
/// the top of every jj-gt command absorbs the friction without
/// silently swallowing the signal — see the caller, which logs
/// the catch-up explicitly.
///
/// `update-stale` is idempotent — when nothing is stale it prints
/// `Attempted recovery, but the working copy is not stale` and
/// exits 0. We detect "did anything move?" by comparing `@`'s
/// change-id before and after the call.
///
/// Set `JJ_GT_SKIP_UPDATE_STALE=1` to bypass the call entirely
/// (escape hatch for the rare debug case where you need jj's
/// staleness error to surface). Any other value — `0`, `false`,
/// empty, `no` — leaves the call enabled, matching the convention
/// the rest of the codebase uses for opt-in env-var flags.
pub fn ensure_workspace_current(jj: &JjCli) -> Result<UpdateStaleOutcome> {
    if std::env::var("JJ_GT_SKIP_UPDATE_STALE").as_deref() == Ok("1") {
        return Ok(UpdateStaleOutcome::NotStale);
    }

    // Capture @ before the update so we can detect a move. Read with
    // --ignore-working-copy so we don't trigger a snapshot pass that
    // would itself try to materialize a stale @.
    let pre = current_change_id_ignore_stale(jj).ok();

    let _ = jj_run(jj, &["workspace", "update-stale"])?;

    let post = current_change_id_ignore_stale(jj).ok();

    match (pre, post) {
        (Some(p), Some(q)) if p == q => Ok(UpdateStaleOutcome::NotStale),
        (Some(p), Some(q)) => Ok(UpdateStaleOutcome::Updated {
            from_change_id: p,
            to_change_id: q,
        }),
        // Either probe failed to read `@`. `update-stale` itself
        // succeeded (we'd have returned `Err` above otherwise), so
        // we don't know whether anything moved. Surface that
        // explicitly via `CouldNotVerify` rather than papering
        // over it with `NotStale`, which the wrapper would render
        // as "already current" — a false-negative for the user.
        _ => Ok(UpdateStaleOutcome::CouldNotVerify),
    }
}

/// Read `@`'s change id with `--ignore-working-copy` so a stale
/// working copy doesn't poison the call. Used only by
/// [`ensure_workspace_current`] for its before/after probe.
fn current_change_id_ignore_stale(jj: &JjCli) -> Result<String> {
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            "change_id",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )?;
    Ok(out.trim().to_owned())
}

/// `jj git import --ignore-working-copy`. Run after external tooling
/// (gt sync) mutates refs on the git side so jj's view catches up.
pub fn git_import(jj: &JjCli) -> Result<()> {
    let _ = jj_run(jj, &["git", "import", "--ignore-working-copy"])?;
    Ok(())
}

/// Returns true if the working copy has uncommitted file changes
/// (added / modified / deleted files relative to `@`'s parent).
///
/// Used by the fetch pipeline to refuse to run when the user has
/// in-progress edits, the companion of [`crate::lock::PipelineLock`]
/// for the "another shell is editing files concurrently with my
/// fetch" hazard. `--ignore-working-copy` matches the rest of this
/// module — we don't want to snapshot the very state we're trying
/// to detect.
///
/// The check is "does `@` have any diff vs its parent" via a
/// template-based check: `self.diff()` returns a `TreeDiff` whose
/// `len()` is zero when the working copy is clean. We use this
/// rather than parsing `jj status` output because the template
/// API is stable across jj versions and produces a single boolean
/// we don't have to parse.
pub fn has_uncommitted_changes(jj: &JjCli) -> Result<bool> {
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            r#"self.diff().files().len()"#,
            "--ignore-working-copy",
        ],
    )?;
    let n: u64 = out.trim().parse().map_err(|e| {
        JjGtError::Invalid(format!(
            "unexpected `jj log -T self.diff().files().len()` output `{}`: {e}",
            out.trim(),
        ))
    })?;
    Ok(n > 0)
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

/// `jj bookmark set <name> -r <revset> --allow-backwards`. Used by the
/// fetch pipeline's rewind detector to restore a bookmark that gt
/// sync silently moved backward.
///
/// `--allow-backwards` is required when the target commit is an
/// ancestor of the bookmark's current position — jj treats that as a
/// rewind by default and refuses without the flag. The rewind
/// detector is the only legitimate caller (gt sync just deleted the
/// bookmark or moved it backward; we're putting it back where it was
/// pre-pipeline).
pub fn bookmark_set(jj: &JjCli, name: &str, revset: &str) -> Result<()> {
    let _ = jj_run(
        jj,
        &[
            "bookmark",
            "set",
            name,
            "-r",
            revset,
            "--allow-backwards",
            "--ignore-working-copy",
        ],
    )?;
    Ok(())
}

/// `git merge-base --is-ancestor <a> <b>` — returns true iff commit
/// `a` is an ancestor of commit `b` (or they're equal). Used by the
/// rewind detector to classify pre/post-sync bookmark position
/// changes:
///
/// - pre is_ancestor_of post → bookmark advanced (fast-forward). OK.
/// - post is_ancestor_of pre → bookmark rewound. Restore.
/// - neither → divergent. Restore + warn.
///
/// We shell out to git rather than asking jj because the commits we're
/// checking may have been deleted from refs (gt sync removed the
/// bookmark) — git's object database still has them addressable by
/// SHA. jj would need a `bookmarks()`-style revset that doesn't
/// require live refs, which is brittle.
pub fn is_ancestor(workspace_root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(workspace_root)
        .status()
        .map_err(JjGtError::Io)?;
    // Exit 0 → is ancestor; exit 1 → is not; exit 128+ → real error
    // (bad SHA, etc.). Treat 0/1 as the answer; anything else
    // propagates as a failure so we don't silently mis-classify.
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => Err(JjGtError::JjFailed {
            status: code,
            stderr: format!(
                "git merge-base --is-ancestor {ancestor} {descendant} exited with code {code}"
            ),
        }),
        None => Err(JjGtError::JjFailed {
            status: -1,
            stderr: format!(
                "git merge-base --is-ancestor {ancestor} {descendant} terminated by signal"
            ),
        }),
    }
}

/// `jj git push --remote <remote> --bookmark <bookmark>
///  --ignore-working-copy`. Used by `submit_cmd` to bring the remote
/// in sync with a locally-rebased stack BEFORE handing off to
/// `gt submit` — gt's "branch updated remotely" check would otherwise
/// abort on the first bookmark whose local SHA differs from remote.
///
/// jj's `git push` is force-with-lease by default: it pushes when the
/// local bookmark has diverged from what jj last fetched, but ONLY if
/// the remote's current state still matches what jj last saw
/// (preventing the "collaborator pushed something I haven't fetched"
/// race). The natural shape we want.
///
/// `--ignore-working-copy` matches the rest of this module — the
/// push is a refs-only operation and we don't want it to snapshot.
/// Classification of a single `jj git push --bookmark <name>` result.
/// Used by `reconcile::push_rebased_tips` to surface a per-bookmark
/// breakdown ("2 newly pushed, 1 moved, 3 already in sync") rather
/// than just a "1 pushed" count that conflates real work with
/// already-synced bookmarks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// `Add bookmark <name> to <oid>` — bookmark didn't exist on
    /// the remote before this push.
    Added,
    /// `Move forward|backward|sideways bookmark <name> from <a> to <b>` —
    /// bookmark existed but its commit changed (the typical "I
    /// rebased my own PR" shape).
    Moved,
    /// jj reported nothing to do for this bookmark (local already
    /// matches remote). gt submit would then see No-op too — this
    /// is the signal worth surfacing because it usually means the
    /// user forgot `jj bookmark set <name> -r @` after amending.
    AlreadyInSync,
}

/// `jj git push --remote <remote> --bookmark <bookmark>
///  --ignore-working-copy`. Used by `submit_cmd` to bring the remote
/// in sync with a locally-rebased stack BEFORE handing off to
/// `gt submit` — gt's "branch updated remotely" check would otherwise
/// abort on the first bookmark whose local SHA differs from remote.
///
/// jj's `git push` is force-with-lease by default: it pushes when the
/// local bookmark has diverged from what jj last fetched, but ONLY if
/// the remote's current state still matches what jj last saw
/// (preventing the "collaborator pushed something I haven't fetched"
/// race). The natural shape we want.
///
/// `--ignore-working-copy` matches the rest of this module — the
/// push is a refs-only operation and we don't want it to snapshot.
///
/// Returns a [`PushOutcome`] classifying the result against jj's
/// stderr output. jj writes its decisions to stderr (not stdout)
/// regardless of exit code, so we read both via
/// [`jj_hooks::jj::JjCli::run_capture_stderr`] and pattern-match the
/// distinguishing lines. When the output doesn't match any known
/// shape, we conservatively report `AlreadyInSync` — the
/// alternative (returning Moved on every unrecognized line) would
/// over-claim work that didn't happen.
pub fn git_push_bookmark(jj: &JjCli, remote: &str, bookmark: &str) -> Result<PushOutcome> {
    let combined = jj
        .run_capture_stderr(&[
            "git",
            "push",
            "--remote",
            remote,
            "--bookmark",
            bookmark,
            "--ignore-working-copy",
        ])
        .map_err(JjGtError::Hooks)?;
    Ok(classify_push_output(&combined))
}

/// Pure classifier for `jj git push` output. Pulled out so the test
/// suite can pin every branch without spinning up real subprocesses.
///
/// `jj git push`'s known shapes (post jj 0.34, which renamed the
/// verbs from `Push bookmark` to `Add bookmark` / `Move bookmark`):
///
///   "Add bookmark <name> to <oid>"
///   "Move forward bookmark <name> from <a> to <b>"
///   "Move backward bookmark <name> from <a> to <b>"
///   "Move sideways bookmark <name> from <a> to <b>"
///   "Force-pushed bookmark <name>" (rare; gt-submit-style force path)
///   "Nothing changed."  // or no output at all when there's nothing
///   "Warning: ..."      // various non-fatal warnings
///
/// We match on the action verbs anywhere in the combined output
/// because jj precedes them with `Changes to push to origin:` and
/// other context lines; the verb is what we care about.
pub fn classify_push_output(combined: &str) -> PushOutcome {
    // Order matters: "Add" wins over "Move" if both somehow appear
    // (multi-bookmark push wouldn't happen since we pass one
    // --bookmark, but defensive). "Force-pushed" is treated as a
    // move because the bookmark did move from the remote's
    // perspective.
    let lower = combined.to_lowercase();
    if lower.contains("add bookmark ") {
        return PushOutcome::Added;
    }
    if lower.contains("move forward bookmark ")
        || lower.contains("move backward bookmark ")
        || lower.contains("move sideways bookmark ")
        || lower.contains("force-pushed bookmark ")
    {
        return PushOutcome::Moved;
    }
    // Default: nothing changed, or the output doesn't match any
    // known verb. Treat as in-sync; the user gets a "no-op" signal
    // for the bookmark.
    PushOutcome::AlreadyInSync
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

/// `jj op log --limit 1 --no-graph -T 'self.id()'` — returns the
/// current operation id, useful as a snapshot before a potentially-
/// destructive mutation that may need rolling back via [`op_restore`].
///
/// Op ids are long hex strings; jj accepts any unambiguous prefix
/// in op-restore, but we keep the full id for safety so a future
/// concurrent op can't collide on a short prefix.
pub fn current_op_id(jj: &JjCli) -> Result<String> {
    let out = jj_run(
        jj,
        &[
            "op",
            "log",
            "--limit",
            "1",
            "--no-graph",
            "-T",
            "self.id()",
            "--ignore-working-copy",
        ],
    )?;
    Ok(out.trim().to_owned())
}

/// `jj op restore <op_id> --ignore-working-copy` — rewinds the
/// repository to the state recorded at `op_id`. Used to roll back
/// a rebase that turned out to introduce conflicts, so the user
/// doesn't have to clean up commits that fetch shouldn't have
/// touched in the first place.
///
/// Note: `op restore` is a global operation across all workspaces
/// sharing `.jj/`. The caller paired with a snapshot taken
/// immediately before the would-be mutation, so the restore
/// window is tight and unlikely to clobber unrelated ops from
/// other workspaces. Still, the caller should only invoke this
/// when they hold the pipeline lock.
pub fn op_restore(jj: &JjCli, op_id: &str) -> Result<()> {
    let _ = jj_run(jj, &["op", "restore", op_id, "--ignore-working-copy"])?;
    Ok(())
}

/// Count of conflicted commits in `revset` — used to detect whether
/// a rebased range now contains conflict markers without parsing
/// human-readable stderr. `jj log -r 'conflicts() & <revset>'
/// --no-graph -T id` emits one line per matching commit; counting
/// lines gives the count without parsing the content.
///
/// Used by the restack code path to enumerate conflicted commits
/// after `jj rebase`, so the per-stack summary can report "5
/// commits conflicted" instead of a generic "rebase produced
/// conflicts." Also held in reserve as a double-confirmation
/// against [`rebase`]'s stderr-parsing heuristic — that heuristic
/// is cheap and correct today, but if jj's CLI output ever drifts,
/// the conflict-defer path in
/// [`crate::cleanup::orphan_rebase_phase`] could call this to
/// validate before rolling back.
///
/// Returns 0 when the revset is empty (no commits) or when none of
/// the matched commits carry conflicts.
pub fn count_conflicts_in(jj: &JjCli, revset: &str) -> Result<usize> {
    let combined_revset = format!("conflicts() & ({revset})");
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            &combined_revset,
            "--no-graph",
            "-T",
            "commit_id ++ \"\\n\"",
            "--ignore-working-copy",
        ],
    )?;
    Ok(out.lines().filter(|l| !l.trim().is_empty()).count())
}

/// Count of commits in `revset`. Used by the restack summary to
/// report "rebased N commits" accurately — bookmark counts
/// undercount stacks with unbookmarked intermediate commits.
///
/// `jj log -r <revset> --no-graph -T commit_id` emits one line per
/// matching commit; counting lines gives the count without parsing
/// content. Returns 0 when the revset is empty.
pub fn count_commits_in(jj: &JjCli, revset: &str) -> Result<usize> {
    let out = jj_run(
        jj,
        &[
            "log",
            "-r",
            revset,
            "--no-graph",
            "-T",
            "commit_id ++ \"\\n\"",
            "--ignore-working-copy",
        ],
    )?;
    Ok(out.lines().filter(|l| !l.trim().is_empty()).count())
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
    use super::{PushOutcome, classify_push_output, unique_lines};

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

    #[test]
    fn classify_added_bookmark() {
        // Real `jj git push --bookmark` output for a brand-new
        // bookmark, captured 2026-05 against jj 0.40.
        let out = "\
Changes to push to origin:
  Add bookmark feature-x to 75897050816e
";
        assert_eq!(classify_push_output(out), PushOutcome::Added);
    }

    #[test]
    fn classify_moved_forward_bookmark() {
        let out = "\
Changes to push to origin:
  Move forward bookmark feature-x from abc12345 to def67890
";
        assert_eq!(classify_push_output(out), PushOutcome::Moved);
    }

    #[test]
    fn classify_moved_sideways_bookmark() {
        // The rebase-style move: SHA changed but it's neither
        // strictly forward nor strictly backward.
        let out = "\
Changes to push to origin:
  Move sideways bookmark feature-x from abc12345 to def67890
";
        assert_eq!(classify_push_output(out), PushOutcome::Moved);
    }

    #[test]
    fn classify_moved_backward_bookmark() {
        let out = "\
Changes to push to origin:
  Move backward bookmark feature-x from def67890 to abc12345
";
        assert_eq!(classify_push_output(out), PushOutcome::Moved);
    }

    #[test]
    fn classify_force_pushed_bookmark() {
        let out = "Force-pushed bookmark feature-x";
        assert_eq!(classify_push_output(out), PushOutcome::Moved);
    }

    #[test]
    fn classify_nothing_changed() {
        // "Nothing changed." is the canonical no-op shape.
        let out = "Nothing changed.";
        assert_eq!(classify_push_output(out), PushOutcome::AlreadyInSync);
    }

    #[test]
    fn classify_empty_output_treated_as_in_sync() {
        // Some jj versions print nothing at all when there's
        // nothing to push. Don't over-claim work.
        assert_eq!(classify_push_output(""), PushOutcome::AlreadyInSync);
    }

    #[test]
    fn classify_warning_only_is_in_sync() {
        // Warnings without any action verb shouldn't be
        // classified as a move.
        let out = "Warning: some non-fatal thing happened\n";
        assert_eq!(classify_push_output(out), PushOutcome::AlreadyInSync);
    }
}

//! Tests for the fetch pipeline's safety net: cooperative flock +
//! uncommitted-changes refusal (PR-C for issue #1).
//!
//! Skipped silently when `jj` isn't on PATH so the test can live in
//! the default `cargo test` set without forcing a hard dep on jj in
//! CI matrices that haven't installed it yet.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use jj_gt::jj::JjCli;
use jj_gt::lock::PipelineLock;

/// Process-wide lock for tests that mutate environment variables.
/// Cargo test runs tests on multiple threads in one process, so
/// `std::env::set_var` is a process-wide race without
/// serialization. Tests that touch env state acquire this lock
/// for their entire body so concurrent tests observe a stable
/// view.
fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

/// RAII guard that restores a process-wide env var on drop. Pair
/// with `env_lock()` so the cleanup runs even if the body of the
/// test panics — leaking `JJ_GT_SKIP_UPDATE_STALE=1` (or any
/// other jj-gt env knob) into a subsequent test in the same
/// process would silently flip its behavior. The guard captures
/// whatever value the var held before `set()` ran and restores
/// it on drop (or removes the var if it wasn't set), so a test
/// run that already had the var set in its outer env doesn't
/// lose that value across the first guarded test.
struct EnvVarGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    /// Capture the prior value, set `name=value`, and arrange for
    /// the prior value to be restored (or `remove_var(name)` if
    /// the var was unset) on drop.
    /// SAFETY: caller must hold `env_lock()` for the lifetime of
    /// the guard so no other thread observes the mutation.
    unsafe fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(name);
        // SAFETY: documented at the call site — the env_lock guard
        // serializes us against every other env-touching test.
        unsafe {
            std::env::set_var(name, value);
        }
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: same lock invariant as `set` — the caller holds
        // `env_lock()` and therefore so do we.
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

fn jj_available() -> bool {
    Command::new("jj")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn jj(cwd: &Path, args: &[&str]) {
    let out = Command::new("jj")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "jj {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Shell out to `git`. Mirrors [`jj`] so the test fixtures don't
/// repeat the `Command::new("git").args(...).output().unwrap()` +
/// `assert!(status.success(), ...)` boilerplate at every call
/// site.
fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn jj_capture(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("jj")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "jj {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Build a fresh jj workspace with `main` bookmark on the root commit.
/// Used by every test in this file.
fn build_workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    jj(tmp.path(), &["git", "init", "--colocate"]);
    jj(
        tmp.path(),
        &["config", "set", "--repo", "user.email", "test@example.com"],
    );
    jj(
        tmp.path(),
        &["config", "set", "--repo", "user.name", "Tester"],
    );
    jj(tmp.path(), &["describe", "-m", "root"]);
    jj(tmp.path(), &["bookmark", "create", "main", "-r", "@"]);
    // Advance @ to an empty commit so the workspace's @ has a
    // parent — has_uncommitted_changes checks @ vs @-, and a
    // root-only workspace has nothing to diff against.
    jj(tmp.path(), &["new", "-m", "work"]);
    tmp
}

#[test]
fn pipeline_lock_releases_on_drop() {
    // Pin the RAII contract: acquiring + dropping the lock leaves
    // the path in a state where a fresh acquire succeeds. Without
    // this guarantee, a panic mid-pipeline would orphan the lock.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();

    {
        let _lock = PipelineLock::acquire(tmp.path()).unwrap();
        // Lock held inside this scope.
    }

    // Re-acquire — should not block, no contention.
    let start = Instant::now();
    let _second = PipelineLock::acquire(tmp.path()).unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "second acquire took {}ms; lock wasn't released by drop",
        start.elapsed().as_millis(),
    );
}

#[test]
fn pipeline_lock_blocks_concurrent_acquirer() {
    // Two threads racing the same lock. The second one must wait
    // for the first to drop. We use a Barrier to synchronize the
    // attempt timing so the second thread reliably hits contention.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let workspace = tmp.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));

    let workspace_a = workspace.clone();
    let barrier_a = barrier.clone();
    let handle_a = std::thread::spawn(move || {
        let lock = PipelineLock::acquire(&workspace_a).unwrap();
        // Signal that we hold the lock.
        barrier_a.wait();
        // Hold the lock briefly so the second thread sees contention.
        std::thread::sleep(Duration::from_millis(200));
        drop(lock);
    });

    let workspace_b = workspace.clone();
    let barrier_b = barrier.clone();
    let handle_b = std::thread::spawn(move || {
        // Wait until thread A holds the lock.
        barrier_b.wait();
        // A is now holding the lock and sleeping 200ms. Acquiring
        // here must block for ~200ms (the rest of A's hold).
        let start = Instant::now();
        let _lock = PipelineLock::acquire(&workspace_b).unwrap();
        let elapsed = start.elapsed();
        // Allow generous slack — CI is slow. The point is "not
        // instant": if the lock didn't block at all, this would
        // come back in single-digit ms.
        assert!(
            elapsed >= Duration::from_millis(100),
            "second acquire returned in {}ms; expected to block on first lock",
            elapsed.as_millis(),
        );
    });

    handle_a.join().unwrap();
    handle_b.join().unwrap();
}

#[test]
fn pipeline_lock_errors_when_workspace_has_no_jj_dir() {
    // A bare directory (no `.jj/`) isn't a jj workspace; the lock
    // call should surface a structured error rather than silently
    // creating a stray `.jj/jj-gt.lock` somewhere odd.
    let tmp = tempfile::tempdir().unwrap();
    let err = PipelineLock::acquire(tmp.path()).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("no .jj directory"),
        "expected 'no .jj directory' in error, got: {msg}",
    );
}

#[test]
fn has_uncommitted_changes_false_on_clean_workspace() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // The workspace is "clean" — @ is an empty commit with no
    // file changes vs @-.
    assert!(!jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap());
}

#[test]
fn has_uncommitted_changes_true_after_file_edit() {
    // Write a file, snapshot via a no-op jj op so @'s diff
    // includes the change, then check the helper sees it.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    std::fs::write(tmp.path().join("new.txt"), "content\n").unwrap();
    // Snapshot via a benign jj op so the new file becomes part of
    // @'s state without describing it (mimics the typical
    // "user is editing files" state).
    jj(tmp.path(), &["status"]);

    let jj_cli = JjCli::new(tmp.path().to_path_buf());
    assert!(jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap());
}

#[test]
fn shelter_uncommitted_edits_moves_them_into_a_committed_change() {
    // The contract: after `shelter_uncommitted_edits`, the user's
    // file edits are no longer pending against `@` — they're
    // committed in what was previously `@`, and the new `@` is an
    // empty change above that. The file content on disk doesn't
    // change.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();

    // Write a file and snapshot it as a pending edit on @.
    std::fs::write(tmp.path().join("shelter-me.txt"), "edits\n").unwrap();
    jj(tmp.path(), &["status"]);

    let jj_cli = JjCli::new(tmp.path().to_path_buf());
    assert!(
        jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap(),
        "fixture should have set up pending edits before sheltering"
    );

    // Capture @ before sheltering so we can assert what happened to
    // the previous change.
    let before_change_id = jj_gt::jj::current_change_id(&jj_cli).unwrap();

    jj_gt::jj::shelter_uncommitted_edits(&jj_cli).unwrap();

    // The new @ must be empty (no pending file changes).
    assert!(
        !jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap(),
        "new @ should be empty after sheltering"
    );

    // The new @ must have a different change_id (jj new @ creates
    // a fresh child).
    let after_change_id = jj_gt::jj::current_change_id(&jj_cli).unwrap();
    assert_ne!(
        before_change_id, after_change_id,
        "shelter should have moved @ to a new change",
    );

    // The previous change must now carry the file content. Cheapest
    // verification: ask jj for the diff of the old change and
    // confirm the file path appears.
    let diff = jj_capture(
        tmp.path(),
        &[
            "log",
            "-r",
            &before_change_id,
            "--no-graph",
            "-T",
            r#"self.diff().files().map(|f| f.path()).join(",")"#,
            "--ignore-working-copy",
        ],
    );
    assert!(
        diff.contains("shelter-me.txt"),
        "pre-shelter change should carry the sheltered file, got: {diff:?}",
    );

    // File on disk must be untouched — sheltering is a metadata
    // operation, not a checkout.
    let on_disk = std::fs::read_to_string(tmp.path().join("shelter-me.txt")).unwrap();
    assert_eq!(on_disk, "edits\n");
}

#[test]
fn shelter_uncommitted_edits_is_a_noop_on_clean_workspace() {
    // When there's nothing to shelter, calling the helper anyway
    // is still valid — `jj new @` just adds an empty change above
    // an empty change. Cheap to do, cheaper than branching the
    // caller on a clean-workspace fast path.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    assert!(!jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap());
    jj_gt::jj::shelter_uncommitted_edits(&jj_cli).unwrap();
    assert!(!jj_gt::jj::has_uncommitted_changes(&jj_cli).unwrap());
}

#[test]
fn is_ancestor_recognizes_linear_ancestry() {
    // PR-D's rewind detector relies on git merge-base --is-ancestor
    // to classify pre/post snapshot diffs. Pin that the wrapper:
    //   (a) returns true for a commit that's actually an ancestor,
    //   (b) returns false for a commit that isn't,
    //   (c) returns true for a commit compared with itself.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    // Add an explicit non-empty commit + bookmark on top so the
    // git side has a HEAD strictly above main. The build_workspace
    // helper leaves @ as an empty WIP commit which jj doesn't
    // necessarily export to git refs.
    std::fs::write(tmp.path().join("file.txt"), "content\n").unwrap();
    jj(tmp.path(), &["describe", "-m", "tip commit"]);
    jj(tmp.path(), &["bookmark", "create", "tip", "-r", "@"]);
    jj(tmp.path(), &["git", "export"]);

    let main_oid = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let tip_oid = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "tip"])
            .current_dir(tmp.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();

    assert_ne!(main_oid, tip_oid, "fixture didn't advance tip past main");
    // main is an ancestor of tip.
    assert!(jj_gt::jj::is_ancestor(tmp.path(), &main_oid, &tip_oid).unwrap());
    // tip is not an ancestor of main.
    assert!(!jj_gt::jj::is_ancestor(tmp.path(), &tip_oid, &main_oid).unwrap());
    // A commit is its own ancestor (reflexive — git's contract).
    assert!(jj_gt::jj::is_ancestor(tmp.path(), &main_oid, &main_oid).unwrap());
}

/// Build a `main → bottom → mid → top` stack + bare remote, push
/// everything, track all four bookmarks on the remote. Returns the
/// workspace TempDir (caller manages lifetime); the bare remote
/// lives inside the workspace tmp dir (sibling to `.jj/.git`) so it
/// cleans up together.
fn build_tracked_stack_with_bare_remote() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    jj(tmp.path(), &["git", "init", "--colocate"]);
    jj(
        tmp.path(),
        &["config", "set", "--repo", "user.email", "test@example.com"],
    );
    jj(
        tmp.path(),
        &["config", "set", "--repo", "user.name", "Tester"],
    );

    // main → bottom → mid → top, real commits so the OIDs differ.
    // Advance @ off `top` after creating it so subsequent jj
    // operations don't auto-snapshot empty changes back onto `top`
    // (which produces the dreaded "(conflicted) ... behind by 1 commit"
    // shape during track_bookmark_on_remote).
    jj(tmp.path(), &["describe", "-m", "root"]);
    jj(tmp.path(), &["bookmark", "create", "main", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "bottom"]);
    jj(tmp.path(), &["bookmark", "create", "bottom", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "mid"]);
    jj(tmp.path(), &["bookmark", "create", "mid", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "top"]);
    jj(tmp.path(), &["bookmark", "create", "top", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "post-top"]);
    jj(tmp.path(), &["git", "export"]);

    // Bare remote next to the workspace.
    let remote_path = tmp.path().join("remote.git");
    let bare = std::process::Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&remote_path)
        .output()
        .unwrap();
    assert!(bare.status.success());

    // Wire `origin` to the bare remote and push everything.
    let add_remote = std::process::Command::new("git")
        .args(["remote", "add", "origin"])
        .arg(format!("file://{}", remote_path.display()))
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        add_remote.status.success(),
        "git remote add failed: {}",
        String::from_utf8_lossy(&add_remote.stderr)
    );
    for bookmark in ["main", "bottom", "mid", "top"] {
        git(tmp.path(), &["push", "origin", bookmark]);
    }
    // Import so jj's remote-tracking refs are populated.
    jj(tmp.path(), &["git", "import"]);
    // Track each so a future remote-side deletion propagates locally
    // on fetch — the bug condition we're testing.
    for bookmark in ["main", "bottom", "mid", "top"] {
        jj_gt::jj::track_bookmark_on_remote(
            &JjCli::new(tmp.path().to_path_buf()),
            bookmark,
            "origin",
        )
        .unwrap();
    }
    tmp
}

#[test]
fn snapshot_pre_fetch_captures_parent_edges_that_post_fetch_loses() {
    // The orchestration bug this test pins:
    //
    //   1. User has a tracked stack: `main → bottom → mid → top`.
    //   2. Someone merges `bottom`'s PR upstream; the post-merge
    //      cleanup deletes the remote ref.
    //   3. User runs `jj-gt fetch`. `jj git fetch` propagates the
    //      remote-side deletion to the LOCAL `bottom` bookmark
    //      (because it was tracked).
    //   4. By the time the cleanup code derives the stack graph,
    //      `bottom` is gone — so `derive_parents` for `top` /
    //      `mid` returns Trunk (not `bottom`), the orphan-rebase
    //      rule never fires, and `top`+`mid` are left floating
    //      on a stale base.
    //
    // Fix: `snapshot_pre_fetch` captures the parent edges BEFORE
    // fetch runs, so the orphan-rebase phase still sees the
    // pre-fetch edges and can detect that the (now-deleted) parent
    // was an orphan trigger.
    //
    // This test asserts the contract:
    //
    //   - The pre-fetch snapshot, taken while all bookmarks exist
    //     locally, includes the `top → mid → bottom` chain.
    //   - After `jj git fetch` (with remote-deleted-bottom), the
    //     post-fetch snapshot LOSES the `bottom` edge — confirming
    //     the bug condition is reproduced and the snapshot's
    //     timing is load-bearing.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }

    let tmp = build_tracked_stack_with_bare_remote();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };

    // Step 1: snapshot the pre-fetch state. PR list bypassed via
    // _with_prs so we don't need gh; the structural contract is
    // independent of PR info.
    let pre_bookmarks = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let (gtmq, normal): (Vec<_>, Vec<_>) = pre_bookmarks
        .iter()
        .cloned()
        .partition(|b| b.name.starts_with("gtmq_"));
    let pre = jj_gt::cleanup::snapshot_pre_fetch_with_prs(&jj_cli, &opts, gtmq, normal, Vec::new())
        .unwrap();
    let pre_normal_names: std::collections::BTreeSet<&str> =
        pre.normal.iter().map(|b| b.name.as_str()).collect();
    for expected in ["main", "bottom", "mid", "top"] {
        assert!(
            pre_normal_names.contains(expected),
            "pre-fetch normal set missing `{expected}`: {pre_normal_names:?}",
        );
    }
    let pre_edges: std::collections::BTreeMap<String, String> = pre
        .stacked
        .iter()
        .filter_map(|sb| match &sb.parent {
            jj_gt::stack::BookmarkOrTrunk::Bookmark(p) => Some((sb.name.clone(), p.clone())),
            jj_gt::stack::BookmarkOrTrunk::Trunk => None,
        })
        .collect();
    // The full chain must be intact at pre-fetch time.
    assert_eq!(
        pre_edges.get("top").map(String::as_str),
        Some("mid"),
        "expected pre-fetch top→mid edge, got {pre_edges:?}",
    );
    assert_eq!(
        pre_edges.get("mid").map(String::as_str),
        Some("bottom"),
        "expected pre-fetch mid→bottom edge, got {pre_edges:?}",
    );

    // Step 2: delete `bottom` on the bare remote (simulates the
    // post-merge cleanup).
    git(tmp.path(), &["push", "origin", "--delete", "bottom"]);

    // Step 3: `jj git fetch`. Tracked `bottom` should disappear
    // from local.
    jj(tmp.path(), &["git", "fetch", "--remote", "origin"]);

    let post_bookmarks = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let post_names: std::collections::BTreeSet<&str> =
        post_bookmarks.iter().map(|b| b.name.as_str()).collect();
    assert!(
        !post_names.contains("bottom"),
        "fetch should have propagated the bottom deletion to local; \
         got post-fetch bookmarks: {post_names:?}",
    );
    assert!(
        post_names.contains("top") && post_names.contains("mid") && post_names.contains("main"),
        "fetch should have left main/mid/top alone: {post_names:?}",
    );

    // Step 4: re-derive the stacked-edges using the post-fetch
    // bookmark list. This is what the pre-fix buggy code did. We
    // assert the edge LOSS — confirming the bug condition reproduces
    // and the only reason the production fix works is the pre-fetch
    // timing of `snapshot_pre_fetch`.
    let (gtmq_post, normal_post): (Vec<_>, Vec<_>) = post_bookmarks
        .iter()
        .cloned()
        .partition(|b| b.name.starts_with("gtmq_"));
    let post = jj_gt::cleanup::snapshot_pre_fetch_with_prs(
        &jj_cli,
        &opts,
        gtmq_post,
        normal_post,
        Vec::new(),
    )
    .unwrap();
    let post_edges: std::collections::BTreeMap<String, String> = post
        .stacked
        .iter()
        .filter_map(|sb| match &sb.parent {
            jj_gt::stack::BookmarkOrTrunk::Bookmark(p) => Some((sb.name.clone(), p.clone())),
            jj_gt::stack::BookmarkOrTrunk::Trunk => None,
        })
        .collect();
    assert_ne!(
        post_edges.get("mid").map(String::as_str),
        Some("bottom"),
        "post-fetch mid→bottom edge should have been lost when bottom \
         was deleted (this is the bug condition the pre-fetch snapshot \
         fix prevents)",
    );

    // Step 5: regression assertion — the orphan-rebase deleted-set
    // computed from pre vs post correctly identifies `bottom` as
    // disappeared. Using `pre.normal_names()` keeps this test aligned
    // with the helper production callers use; if the encapsulated
    // "what counts as a normal name" semantics change, the test
    // tracks it automatically.
    let pre_names = pre.normal_names();
    let post_names_set: std::collections::BTreeSet<String> =
        post_bookmarks.iter().map(|b| b.name.clone()).collect();
    let deleted = jj_gt::cleanup::compute_deleted_set(&pre_names, &post_names_set);
    assert!(
        deleted.contains("bottom"),
        "compute_deleted_set should report `bottom` as deleted; got {deleted:?}",
    );
}

#[test]
fn maybe_export_before_fetch_runs_jj_git_export() {
    // Regression for the workspace-leak bug: when `jj bookmark set
    // <bm> -r <new>` runs in workspace W2 sharing `.jj/` with W1
    // (where W2 is non-colocated, so no auto-export fires), JJ's
    // view has `<bm> = <new>` but git's `refs/heads/<bm>` is still
    // at the old commit. When W1 later runs `jj-gt fetch`, the
    // `jj git fetch` step auto-imports git refs back into JJ —
    // which sees git's stale ref as canonical and reverts W2's
    // bookmark move.
    //
    // The fix: `run_fetch` calls `maybe_export_before_fetch` BEFORE
    // `jj git fetch`. From W1's perspective (colocated), the export
    // sees its last-export tracking is behind JJ's current view
    // (because W2 made the change without exporting), and updates
    // git's loose refs to match. The subsequent fetch+auto-import
    // is then a no-op for that bookmark.
    //
    // Reliably reproducing the full bug shape in a single-workspace
    // test is awkward because colocated `jj bookmark set` auto-
    // exports immediately and updates the "last exported" tracking
    // — so a follow-up `jj git export` sees no work to do. The
    // shape only manifests with the secondary-workspace timing
    // gap. End-to-end coverage of `run_fetch` lives in
    // `snapshot_pre_fetch_captures_parent_edges_that_post_fetch_loses`.
    //
    // What we pin here is the contract: the helper runs `jj git
    // export` and doesn't error on a clean workspace. That's the
    // minimum the fix has to do — if a future refactor breaks the
    // call wiring, this test fires.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }

    let tmp = build_tracked_stack_with_bare_remote();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };

    // Happy path: succeeds on a clean workspace.
    jj_gt::cleanup::maybe_export_before_fetch(&jj_cli, &opts, jj_gt::ui::Verbosity::Quiet)
        .expect("export should succeed on a clean workspace");

    // dry-run skips the export entirely — return without error.
    let dry_opts = jj_gt::cleanup::FetchOpts {
        dry_run: true,
        ..opts.clone()
    };
    jj_gt::cleanup::maybe_export_before_fetch(&jj_cli, &dry_opts, jj_gt::ui::Verbosity::Quiet)
        .expect("export should no-op in dry-run mode");

    // no_export skips the export entirely — return without error.
    let no_export_opts = jj_gt::cleanup::FetchOpts {
        no_export: true,
        ..opts.clone()
    };
    jj_gt::cleanup::maybe_export_before_fetch(
        &jj_cli,
        &no_export_opts,
        jj_gt::ui::Verbosity::Quiet,
    )
    .expect("export should be skipped when no_export is set");
}

#[test]
fn ensure_workspace_current_returns_not_stale_on_clean_workspace() {
    // The cheap path of `ensure_workspace_current`: when nothing
    // sibling-workspaced has happened, the before/after change-id
    // probe matches and we return NotStale. Pins the contract that
    // the helper is a no-op signal-wise in the single-workspace
    // happy path (the case 99% of jj-gt invocations hit).
    //
    // We hold `env_lock()` for the call because a sibling test in
    // this binary (`ensure_workspace_current_respects_skip_env_var`)
    // toggles `JJ_GT_SKIP_UPDATE_STALE`. Without the lock we could
    // race into the function with the env var set and trivially
    // return `NotStale` via the skip path, hollowing out the
    // before/after change-id probe this test exists to pin.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let _guard = env_lock().lock().unwrap();
    let outcome = jj_gt::jj::ensure_workspace_current(&jj_cli).unwrap();
    assert_eq!(outcome, jj_gt::jj::UpdateStaleOutcome::NotStale);
}

#[test]
fn ensure_workspace_current_respects_skip_env_var() {
    // Escape hatch: `JJ_GT_SKIP_UPDATE_STALE=1` returns NotStale
    // without shelling out. Pin this so users debugging a stale-
    // working-copy interaction can deterministically reproduce
    // jj's native error by setting the env var.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Serialize against any other test in this binary that
    // touches the same env var. `unsafe` is still required by
    // Rust 2024 for the set_var/remove_var calls themselves, but
    // the lock makes the operation safe from a data-race
    // standpoint — no other thread in this process is allowed to
    // read or write the env while we hold the guard.
    let _lock = env_lock().lock().unwrap();
    // SAFETY: held the process-wide env lock above. `EnvVarGuard`
    // captures whatever value the var held before this test (so
    // an outer-env setting survives the test run) and restores
    // it on drop, even if `ensure_workspace_current` panics —
    // without that restore-on-drop, a leaked
    // `JJ_GT_SKIP_UPDATE_STALE=1` would silently flip the next
    // test that consults the same knob.
    let _env = unsafe { EnvVarGuard::set("JJ_GT_SKIP_UPDATE_STALE", "1") };
    let outcome = jj_gt::jj::ensure_workspace_current(&jj_cli);
    assert_eq!(outcome.unwrap(), jj_gt::jj::UpdateStaleOutcome::NotStale);
}

#[test]
fn env_var_guard_restores_prior_value_on_drop() {
    // Regression for the PR #72 review: an earlier version of
    // EnvVarGuard always called `remove_var` on drop, so a test
    // run that started with `JJ_GT_SKIP_UPDATE_STALE=outer-value`
    // would lose that outer value the first time a guarded test
    // ran. Pin both the "had prior value → restore" and the
    // "no prior value → clear" branches.
    let _lock = env_lock().lock().unwrap();
    const NAME: &str = "JJ_GT_ENV_GUARD_TEST";

    // Branch 1: prior value present.
    // SAFETY: we hold env_lock() for the rest of the test.
    unsafe {
        std::env::set_var(NAME, "outer-value");
    }
    {
        // SAFETY: same lock held; the guard captures + restores.
        let _g = unsafe { EnvVarGuard::set(NAME, "inner-value") };
        assert_eq!(std::env::var(NAME).unwrap(), "inner-value");
    }
    assert_eq!(
        std::env::var(NAME).unwrap(),
        "outer-value",
        "guard must restore the prior value on drop",
    );

    // Branch 2: no prior value.
    // SAFETY: still under env_lock().
    unsafe {
        std::env::remove_var(NAME);
    }
    {
        // SAFETY: same lock held.
        let _g = unsafe { EnvVarGuard::set(NAME, "inner-value") };
        assert_eq!(std::env::var(NAME).unwrap(), "inner-value");
    }
    assert!(
        std::env::var_os(NAME).is_none(),
        "guard must clear the var on drop when there was no prior value",
    );
}

#[test]
fn ensure_workspace_current_does_not_error_after_sibling_workspace_advances() {
    // The interesting path: a second workspace sharing the same
    // `.jj/` advances the op log past what this workspace has on
    // disk. The next call to `ensure_workspace_current` from the
    // first workspace should detect the move (returning Updated{
    // from, to }) without erroring.
    //
    // This is the exact friction issue #67 addresses: in the
    // multi-agent / agent-+-supervisor model, two workspaces
    // routinely race the op log. Auto-running update-stale at the
    // top of every jj-gt command absorbs the resulting
    // "working copy is stale" error.
    //
    // The contract this test pins is "doesn't error" — jj's
    // stale-detection has version-dependent edges (the
    // abandoned-change-with-no-descendants case doesn't fire on
    // every jj version), so `NotStale` and `CouldNotVerify` are
    // both acceptable terminal states. The original name claimed
    // the test pinned the `Updated` variant; it never did. The
    // separate `not_stale_on_clean_workspace` test covers the
    // negative case deterministically.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }

    // Primary workspace.
    let tmp = build_workspace();
    let primary = tmp.path();

    // Secondary workspace sharing the same `.jj/`. Created at a
    // sibling path; jj manages the cross-workspace metadata.
    let secondary_dir = tempfile::tempdir().unwrap();
    let secondary = secondary_dir.path();
    jj(
        primary,
        &[
            "workspace",
            "add",
            "--name",
            "secondary",
            secondary.to_str().unwrap(),
        ],
    );

    // The probe must read primary's @ before the secondary mutates
    // anything — establish that baseline.
    let jj_cli_primary = JjCli::new(primary.to_path_buf());
    let primary_at_before = jj_capture(
        primary,
        &["log", "-r", "@", "--no-graph", "-T", "change_id"],
    )
    .trim()
    .to_owned();

    // In the secondary workspace, advance @ to a fresh empty
    // commit. This bumps the op log without touching primary's
    // bookmarks — but primary's working-copy commit (its @) is
    // still in the op log and unchanged, so primary itself is
    // *not* stale.
    //
    // To actually mark primary stale we need to touch primary's
    // working-copy commit FROM the secondary. The standard way is
    // for the secondary to abandon-and-create-new on the same
    // change, or for it to amend a shared ancestor. The simplest
    // shape that reliably triggers staleness for primary: have
    // secondary `jj edit` primary's @ change_id and then `jj
    // abandon` it.
    //
    // jj's stale detection fires when the working-copy commit
    // recorded in the workspace's view points at an operation
    // that's no longer the tip of the op log AND its working-
    // copy commit has been rewritten. So: secondary edits
    // primary's @ change, abandons it, and creates a fresh
    // replacement.
    jj(secondary, &["edit", &primary_at_before]);
    jj(secondary, &["abandon"]);

    // Sanity: secondary's op-log advance should have made
    // primary's @ recorded change-id no longer the canonical
    // working-copy for primary. `ensure_workspace_current` MUST
    // succeed (not return Err) regardless of which terminal
    // variant it picks.
    //
    // Hold `env_lock()` because the skip-env-var sibling test
    // could otherwise race in and make us return NotStale via the
    // fast path. Same rationale as the `not_stale` test above.
    let _guard = env_lock().lock().unwrap();
    let outcome = jj_gt::jj::ensure_workspace_current(&jj_cli_primary).unwrap();
    match outcome {
        jj_gt::jj::UpdateStaleOutcome::Updated {
            from_change_id,
            to_change_id,
        } => {
            assert_ne!(
                from_change_id, to_change_id,
                "Updated variant must carry distinct from/to ids",
            );
        }
        jj_gt::jj::UpdateStaleOutcome::NotStale | jj_gt::jj::UpdateStaleOutcome::CouldNotVerify => {
            // Either is acceptable — see test docstring.
        }
    }
}

#[test]
fn list_conflicted_bookmarks_returns_empty_on_clean_workspace() {
    // The cheap path: no bookmark divergence → empty set. Pins
    // the contract that the helper doesn't false-positive on a
    // normal workspace (the case 99% of jj-gt invocations hit).
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let conflicted = jj_gt::jj::list_conflicted_bookmarks(&jj_cli).unwrap();
    assert!(
        conflicted.is_empty(),
        "clean workspace should report no conflicted bookmarks, got {conflicted:?}"
    );
}

#[test]
fn list_conflicted_bookmarks_detects_divergent_target() {
    // Issue #68 contract: when the same bookmark name has been
    // set to different commits from concurrent op-log lineages,
    // jj's `bookmark list` template `self.conflict()` returns
    // true and `list_conflicted_bookmarks` includes the name.
    //
    // Fixture strategy: create two distinct commits, then race
    // two `jj bookmark set <name>` invocations via `--at-op`
    // pointing at the same parent op. Each invocation produces
    // a divergent op-log branch; jj merges them on the next
    // command and the bookmark target ends up conflicted.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Create two distinct commits to race the bookmark across.
    // `right` branches off `main` (sibling of `left`) so the two
    // are unrelated revisions, not a chain — that way `racy`
    // pointing at one vs the other is a real divergence.
    jj(tmp.path(), &["new", "-m", "left side"]);
    let left = jj_capture(
        tmp.path(),
        &["log", "-r", "@", "--no-graph", "-T", "change_id"],
    )
    .trim()
    .to_owned();
    jj(tmp.path(), &["new", "@-", "-m", "right side"]);
    let right = jj_capture(
        tmp.path(),
        &["log", "-r", "@", "--no-graph", "-T", "change_id"],
    )
    .trim()
    .to_owned();

    // Snapshot the parent op so the two bookmark-set invocations
    // both fork from it (creating two divergent op-log
    // branches).
    let parent_op = jj_capture(
        tmp.path(),
        &["op", "log", "--no-graph", "-T", "id", "--limit", "1"],
    )
    .trim()
    .to_owned();

    // Branch 1: set `racy` to the left commit.
    jj(
        tmp.path(),
        &[
            "--at-op",
            &parent_op,
            "bookmark",
            "set",
            "racy",
            "-r",
            &left,
            "--allow-backwards",
        ],
    );
    // Branch 2: set `racy` to the right commit (same parent op
    // — concurrent with branch 1).
    jj(
        tmp.path(),
        &[
            "--at-op",
            &parent_op,
            "bookmark",
            "set",
            "racy",
            "-r",
            &right,
            "--allow-backwards",
        ],
    );

    // Force jj to merge the divergent op-log branches by running
    // any benign command without --at-op.
    jj(tmp.path(), &["status"]);

    let conflicted = jj_gt::jj::list_conflicted_bookmarks(&jj_cli).unwrap();
    assert!(
        conflicted.contains("racy"),
        "concurrent set should produce a conflicted bookmark; got {conflicted:?}"
    );
    // Sanity: `main` (untouched) shouldn't be flagged.
    assert!(
        !conflicted.contains("main"),
        "untouched bookmark must not be reported as conflicted; got {conflicted:?}"
    );
}

#[test]
fn list_conflicted_bookmarks_filters_out_pending_deletion_zombies() {
    // Regression: jj's bookmark template iterates one entry per
    // (bookmark, remote) pair, INCLUDING entries where the
    // remote ref is gone but the deletion hasn't been exported
    // yet ("zombies"). Those zombies satisfy `self.conflict()`
    // because the local view of the target diverges from the
    // (missing) remote view. The `present()` guard in
    // `list_conflicted_bookmarks` is what keeps them out — pin
    // it so a future template tweak that drops `present()`
    // can't silently regress the cleanup pipeline into
    // misreporting deleted bookmarks as conflicted.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Create + delete a bookmark without exporting. jj keeps the
    // tombstone around until the next `jj git export` propagates
    // the deletion to git refs.
    jj(tmp.path(), &["bookmark", "create", "zombie", "-r", "@"]);
    jj(tmp.path(), &["bookmark", "delete", "zombie"]);
    // Snapshot to flush state. We deliberately DON'T run `jj git
    // export` — that would clear the zombie.

    let conflicted = jj_gt::jj::list_conflicted_bookmarks(&jj_cli).unwrap();
    assert!(
        !conflicted.contains("zombie"),
        "pending-deletion zombie must not be reported as conflicted; got {conflicted:?}"
    );
}

#[test]
fn maybe_catch_up_workspace_does_not_mutate_when_dry_run() {
    // Regression for the PR #72 CR review: `jj-gt submit --dry-run`
    // (and the same flag on fetch / restack / reconcile) promises
    // not to mutate workspace state. Before this fix,
    // `maybe_catch_up_workspace` would still call
    // `jj workspace update-stale` on a stale primary workspace,
    // advancing `@` to a new change.
    //
    // The function returns a `CatchUpOutcome` enum describing
    // which branch fired. Test contract:
    //   - dry_run=true → `SkippedDryRun` (no jj invocation at all)
    //   - dry_run=false on a clean workspace → `AlreadyCurrent`
    //
    // jj's stale-detection has version-dependent edges (the
    // existing
    // `ensure_workspace_current_does_not_error_after_sibling_workspace_advances`
    // test documents that) so we can't reliably trigger the
    // Updated path; we exercise the gate via the enum's
    // distinguishability instead.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }

    let tmp = build_workspace();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Hold env_lock + clear the skip-env-var so the dry-run gate
    // is the ONLY thing that can fire the SkippedDryRun outcome.
    // Without this, a stale `JJ_GT_SKIP_UPDATE_STALE=1` could
    // route us to `SkippedEnvVar` instead.
    let _lock = env_lock().lock().unwrap();
    // SAFETY: env_lock held above.
    let _env = unsafe { EnvVarGuard::set("JJ_GT_SKIP_UPDATE_STALE", "0") };

    // dry_run=true → must report SkippedDryRun.
    let outcome_dry = jj_gt::maybe_catch_up_workspace(&jj_cli, jj_gt::ui::Verbosity::Quiet, true)
        .expect("maybe_catch_up_workspace should not error on dry-run");
    assert_eq!(
        outcome_dry,
        jj_gt::CatchUpOutcome::SkippedDryRun,
        "dry_run=true must short-circuit to SkippedDryRun",
    );

    // dry_run=false on a clean workspace → must NOT be
    // SkippedDryRun (must actually invoke the underlying
    // ensure_workspace_current). On a clean workspace the
    // expected terminal is AlreadyCurrent; if jj is in an odd
    // state CouldNotVerify is also acceptable. The contract
    // we're pinning is "the gate didn't fire for non-dry-run."
    let outcome_wet = jj_gt::maybe_catch_up_workspace(&jj_cli, jj_gt::ui::Verbosity::Quiet, false)
        .expect("maybe_catch_up_workspace should not error on a clean workspace");
    assert_ne!(
        outcome_wet,
        jj_gt::CatchUpOutcome::SkippedDryRun,
        "dry_run=false must reach the update-stale path (not collapse to SkippedDryRun)",
    );
    assert!(
        matches!(
            outcome_wet,
            jj_gt::CatchUpOutcome::AlreadyCurrent | jj_gt::CatchUpOutcome::CouldNotVerify
        ),
        "clean workspace + dry_run=false should report AlreadyCurrent (or CouldNotVerify on jj versions where the probe is flaky); got {outcome_wet:?}",
    );
}

#[test]
fn rewind_protection_restores_locally_advanced_bookmark_after_rewind() {
    // The wild bug shape this layer protects against: an agent
    // advanced a bookmark to a new local commit (unpushed); some
    // step in the fetch pipeline silently rewound it back to its
    // pre-advance position (the @origin baseline). Pre-this-fix,
    // the new commit was orphaned and the user had to recover
    // via op-log. With the rewind detector wired in, the
    // post-pipeline sweep notices and restores the advanced
    // position.
    //
    // We exercise capture_rewind_snapshots + apply_rewind_protection
    // directly here rather than the full run_fetch — the goal is
    // to pin the detect+restore contract in isolation. The
    // end-to-end "run_fetch in the multi-workspace race" test
    // would need a fixture too elaborate to be reliable in CI.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_tracked_stack_with_bare_remote();
    let cwd = tmp.path();
    let jj_cli = JjCli::new(cwd.to_path_buf());

    // Pre-snapshot state: `bottom` is at its pushed origin commit
    // (the fixture pushed everything). Resolve the origin
    // baseline for the test's first assertion.
    let origin_bottom = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom@origin",
            "--no-graph",
            "-T",
            "commit_id.short(12)",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();

    // Agent advances `bottom` to a NEW commit beyond origin
    // (simulating "made a fixup commit, moved the bookmark
    // forward, didn't push yet").
    jj(cwd, &["new", "bottom", "-m", "agent's local fixup"]);
    jj(
        cwd,
        &["bookmark", "set", "bottom", "-r", "@", "--allow-backwards"],
    );
    let pre_fetch_bottom = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom",
            "--no-graph",
            "-T",
            "commit_id.short(12)",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_ne!(
        pre_fetch_bottom, origin_bottom,
        "fixture should have advanced bottom past origin"
    );

    // Snapshot — same call the fetch entry would make.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };
    let snapshots = jj_gt::cleanup::capture_rewind_snapshots(&jj_cli, &opts).unwrap();
    let bottom_snap = snapshots
        .get("bottom")
        .expect("bottom should be in the snapshot");
    assert_eq!(
        bottom_snap.pre_commit, pre_fetch_bottom,
        "snapshot should record the advanced position",
    );
    assert_eq!(
        bottom_snap.origin_baseline_commit.as_deref(),
        Some(origin_bottom.as_str()),
        "snapshot should record the origin baseline so the no-local-work filter has a basis to compare against",
    );

    // Simulate the silent rewind: some step in the pipeline (the
    // exact mechanism varies — could be `gt sync --force`, an
    // inter-workspace race during plain `jj git fetch`, or an
    // export+import dance) reverts the local bookmark to origin.
    jj(
        cwd,
        &[
            "bookmark",
            "set",
            "bottom",
            "-r",
            "bottom@origin",
            "--allow-backwards",
        ],
    );
    let mid_fetch_bottom = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom",
            "--no-graph",
            "-T",
            "commit_id.short(12)",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_eq!(
        mid_fetch_bottom, origin_bottom,
        "fixture should have rewound bottom to origin",
    );

    // Run the rewind protection sweep.
    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::apply_rewind_protection(
        &jj_cli,
        cwd,
        &snapshots,
        &opts,
        jj_gt::ui::Verbosity::Quiet,
        &mut actions,
    )
    .unwrap();

    // bottom must be back at the advanced position.
    let post_protection_bottom = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom",
            "--no-graph",
            "-T",
            "commit_id.short(12)",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_eq!(
        post_protection_bottom, pre_fetch_bottom,
        "apply_rewind_protection should have restored bottom to the pre-fetch (advanced) position",
    );

    // And the action log should record the restore.
    let restore_action = actions
        .iter()
        .find(|(b, _)| b.name == "bottom")
        .map(|(_, a)| a.clone())
        .expect("apply_rewind_protection should emit a RewindRestored action for bottom");
    match restore_action {
        jj_gt::cleanup::CleanupAction::RewindRestored {
            pre_commit,
            post_commit,
        } => {
            assert_eq!(pre_commit, pre_fetch_bottom);
            assert_eq!(post_commit, origin_bottom);
        }
        other => panic!("expected RewindRestored for bottom; got {other:?}"),
    }
}

#[test]
fn rewind_protection_does_not_restore_when_no_local_work_predates_fetch() {
    // Negative case: bookmark equals its origin baseline at fetch
    // start (no local-only work). A subsequent "rewind" is
    // either a no-op or a legitimate fast-forward elsewhere;
    // either way, the protection layer must not fire — that's
    // what the `pre == origin_baseline` filter is for, and what
    // keeps us from fighting the orphan-rebase / moved-sideways
    // path over legitimate remote moves.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_tracked_stack_with_bare_remote();
    let cwd = tmp.path();
    let jj_cli = JjCli::new(cwd.to_path_buf());

    // `bottom` is at @origin (the fixture pushed it). No local-
    // only work.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };
    let snapshots = jj_gt::cleanup::capture_rewind_snapshots(&jj_cli, &opts).unwrap();
    let bottom_snap = snapshots.get("bottom").unwrap();
    assert_eq!(
        Some(bottom_snap.pre_commit.as_str()),
        bottom_snap.origin_baseline_commit.as_deref(),
        "test prereq: bottom snapshot should match its origin baseline",
    );

    // Mutate `bottom` backward (e.g. to main). This wouldn't
    // normally happen in fetch but we want to confirm the filter
    // skips even an obvious "rewind" when pre had no local work.
    jj(
        cwd,
        &[
            "bookmark",
            "set",
            "bottom",
            "-r",
            "main",
            "--allow-backwards",
        ],
    );

    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::apply_rewind_protection(
        &jj_cli,
        cwd,
        &snapshots,
        &opts,
        jj_gt::ui::Verbosity::Quiet,
        &mut actions,
    )
    .unwrap();

    // No RewindRestored action should have been emitted —
    // pre matched origin baseline, so the filter blocked the
    // restore.
    let restored_action = actions.iter().find(|(b, _)| b.name == "bottom");
    assert!(
        restored_action.is_none(),
        "no-local-work bookmark must not be auto-restored; got {restored_action:?}",
    );
}

#[test]
fn orphan_untracked_phase_tracks_local_bookmark_with_matching_remote_ref() {
    // The shape this phase catches in the wild (PR #74 → SEA-732
    // follow-up): an agent created bookmark `X` via
    // `jj bookmark create X -r @` + `jj git push --bookmark X`,
    // never ran `jj bookmark track X@origin`. Without the
    // tracking link, jj's standard "remote ref deleted →
    // propagate to local" path never fires when the PR merges
    // and origin deletes the ref. The result is a dangling
    // local bookmark forever.
    //
    // The phase's contract: find local bookmarks that have a
    // matching `@<remote>` ref but no tracking link, and
    // `jj bookmark track <name>@<remote>` them. After this,
    // jj's normal propagation handles future merge/delete
    // cleanup automatically. Surface as `OrphanUntrackedTracked`
    // action so the user sees the metadata correction.
    //
    // Fixture: minimal `main → orphan` workspace + bare remote.
    // `orphan` is pushed to origin via plain `git push` so the
    // remote ref exists, but we deliberately skip
    // `jj bookmark track orphan@origin` — that's the bug-shape
    // ingredient.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    jj(cwd, &["git", "init", "--colocate"]);
    jj(
        cwd,
        &["config", "set", "--repo", "user.email", "test@example.com"],
    );
    jj(cwd, &["config", "set", "--repo", "user.name", "Tester"]);
    jj(cwd, &["describe", "-m", "root"]);
    jj(cwd, &["bookmark", "create", "main", "-r", "@"]);
    jj(cwd, &["new", "-m", "orphan content"]);
    jj(cwd, &["bookmark", "create", "orphan", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let remote_path = cwd.join("remote.git");
    let bare = std::process::Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&remote_path)
        .output()
        .unwrap();
    assert!(bare.status.success());
    let add_remote = std::process::Command::new("git")
        .args(["remote", "add", "origin"])
        .arg(format!("file://{}", remote_path.display()))
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(add_remote.status.success());

    // Push BOTH main and orphan via plain git. Then `jj git
    // import` so jj's view sees the remote refs, but DON'T
    // track orphan explicitly — that's the bug-shape ingredient.
    // `main` will be tracked separately so we can verify the
    // phase skips the already-tracked case.
    for bookmark in ["main", "orphan"] {
        let push = std::process::Command::new("git")
            .args(["push", "origin", bookmark])
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            push.status.success(),
            "git push {bookmark} failed: {}",
            String::from_utf8_lossy(&push.stderr)
        );
    }
    jj(cwd, &["git", "import"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());
    jj_gt::jj::track_bookmark_on_remote(&jj_cli, "main", "origin").unwrap();

    // Sanity: orphan is locally present, has an @origin ref,
    // but isn't yet tracked.
    let tracked_pre = jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap();
    assert!(
        !tracked_pre.contains("orphan"),
        "fixture should leave `orphan` untracked; got tracked={tracked_pre:?}",
    );

    // Run the phase.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };
    let pre_all = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let (gtmq_pre, normal_pre): (Vec<_>, Vec<_>) = pre_all
        .into_iter()
        .partition(|b| b.name.starts_with("gtmq_"));
    let pre = jj_gt::cleanup::snapshot_pre_fetch_with_prs(
        &jj_cli,
        &opts,
        gtmq_pre,
        normal_pre,
        Vec::new(),
    )
    .unwrap();

    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_untracked_phase(
        &jj_cli,
        &pre,
        &opts,
        jj_gt::ui::Verbosity::Quiet,
        &mut actions,
    )
    .unwrap();

    // The phase must have emitted an `OrphanUntrackedTracked`
    // action for orphan.
    let orphan_action = actions
        .iter()
        .find(|(b, _)| b.name == "orphan")
        .map(|(_, a)| a.clone())
        .expect("phase should emit an action for the orphan bookmark");
    match orphan_action {
        jj_gt::cleanup::CleanupAction::OrphanUntrackedTracked { remote, .. } => {
            assert_eq!(remote, "origin");
        }
        other => panic!("expected OrphanUntrackedTracked; got {other:?}"),
    }

    // And the bookmark must now be tracked.
    let tracked_post = jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap();
    assert!(
        tracked_post.contains("orphan"),
        "orphan bookmark should be tracked post-phase; got tracked={tracked_post:?}",
    );
}

#[test]
fn orphan_untracked_phase_skips_pre_push_wip_bookmark() {
    // The safety guardrail: a local-only WIP bookmark that
    // hasn't been pushed yet has no `@<remote>` ref. The phase
    // MUST skip it — there's nothing to track against, and
    // attempting to track would either error or do nothing.
    //
    // This is the case the earlier "forget" design got burned
    // on: it auto-deleted a sibling-stack pre-push bookmark
    // because the orphan-vs-WIP shapes both matched. The new
    // "track-only-when-remote-ref-exists" rule cleanly
    // distinguishes them.
    //
    // We deliberately set up an EMPTY `origin` remote here so
    // `jj::list_tracked_bookmarks_on_remote("origin")` succeeds
    // with an empty set — without that, the phase's prologue
    // would early-bail on the missing-remote error and the
    // per-bookmark `@origin` probe (the actual guard we're
    // pinning) would never run.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let bare_remote = tempfile::tempdir().unwrap();
    // Initialize a bare git repo as the `origin` remote so jj
    // can resolve it. No bookmarks are pushed; we want
    // `list_tracked_bookmarks_on_remote("origin")` to succeed
    // with an empty set, exercising the per-bookmark probe.
    std::process::Command::new("git")
        .args(["init", "--bare"])
        .arg(bare_remote.path())
        .output()
        .expect("git init --bare must succeed");
    jj(cwd, &["git", "init", "--colocate"]);
    jj(
        cwd,
        &["config", "set", "--repo", "user.email", "test@example.com"],
    );
    jj(cwd, &["config", "set", "--repo", "user.name", "Tester"]);
    jj(
        cwd,
        &[
            "git",
            "remote",
            "add",
            "origin",
            bare_remote.path().to_str().unwrap(),
        ],
    );
    jj(cwd, &["describe", "-m", "root"]);
    jj(cwd, &["bookmark", "create", "main", "-r", "@"]);
    // Create `wip` purely locally — never pushed.
    jj(cwd, &["new", "-m", "wip content"]);
    jj(cwd, &["bookmark", "create", "wip", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());

    // Sanity check: the remote exists and is reachable (so the
    // phase's prologue doesn't early-bail). Whether `main` shows
    // up as tracked already (jj's colocated semantics) doesn't
    // matter — what we need is for `wip` to NOT be in the
    // tracked set so the per-bookmark probe runs.
    let pre_tracked = jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap();
    assert!(
        !pre_tracked.contains("wip"),
        "fixture intent: `wip` must not be tracked yet so the phase reaches the per-bookmark probe; got: {pre_tracked:?}",
    );

    // Run the phase. With no remote bookmarks at all, every
    // local bookmark hits the per-bookmark `@origin` probe;
    // `wip` resolves as unresolved-revset and gets skipped.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        ..jj_gt::cleanup::FetchOpts::default()
    };
    let pre_all = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let (gtmq_pre, normal_pre): (Vec<_>, Vec<_>) = pre_all
        .into_iter()
        .partition(|b| b.name.starts_with("gtmq_"));
    let pre = jj_gt::cleanup::snapshot_pre_fetch_with_prs(
        &jj_cli,
        &opts,
        gtmq_pre,
        normal_pre,
        Vec::new(),
    )
    .unwrap();

    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_untracked_phase(
        &jj_cli,
        &pre,
        &opts,
        jj_gt::ui::Verbosity::Quiet,
        &mut actions,
    )
    .unwrap();

    // No action should have been emitted for `wip` — the
    // per-bookmark `@origin` probe got an unresolved-revset
    // error and the phase bailed on the WIP bookmark.
    let wip_action = actions.iter().find(|(b, _)| b.name == "wip");
    assert!(
        wip_action.is_none(),
        "pre-push WIP bookmark must NOT be touched; got {wip_action:?}",
    );

    // And the bookmark must still be present locally.
    let post_bookmarks = jj_capture(
        cwd,
        &[
            "bookmark",
            "list",
            "-T",
            r#"if(self.present() && self.normal_target() && !remote, name ++ "\n", "")"#,
            "--ignore-working-copy",
        ],
    );
    assert!(
        post_bookmarks.contains("wip"),
        "pre-push WIP bookmark must survive the phase; got: {post_bookmarks:?}",
    );
}

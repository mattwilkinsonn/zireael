//! Tests for the fetch pipeline's safety net: cooperative flock +
//! uncommitted-changes refusal (PR-C for issue #1).
//!
//! Skipped silently when `jj` isn't on PATH so the test can live in
//! the default `cargo test` set without forcing a hard dep on jj in
//! CI matrices that haven't installed it yet.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use jj_gt::jj::JjCli;
use jj_gt::lock::PipelineLock;

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

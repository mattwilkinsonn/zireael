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

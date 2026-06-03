//! Integration tests for the `jj-gt restack` command's discovery
//! and per-stack rebase pipeline.
//!
//! Skipped silently when `jj` isn't on PATH so the test can live in
//! the default `cargo test` set without forcing a hard dep on jj
//! in CI matrices that haven't installed it yet.

use std::path::Path;
use std::process::Command;

use jj_gt::jj::JjCli;

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

/// Two-stack fixture for the restack discovery + rebase tests.
///
/// Topology:
///
/// ```text
///   * upper-tip       <- `upper-tip` bookmark
///   * upper-mid       <- `upper-mid` bookmark
///   |
///   |  * sibling-tip  <- `sibling-tip` bookmark
///   | /
///   * main            <- `main` bookmark
/// ```
///
/// Both `upper-tip` and `sibling-tip` are direct descendants of main
/// in this snapshot. After main advances to a new commit (simulated
/// by creating an empty commit on top of main and moving the
/// bookmark), restack should rebase both onto the new main.
fn build_two_stack_fixture() -> tempfile::TempDir {
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

    // Build a 2-commit stack: upper-mid then upper-tip.
    jj(cwd, &["new", "-m", "upper-mid commit"]);
    std::fs::write(cwd.join("upper-mid.txt"), "from upper-mid\n").unwrap();
    jj(cwd, &["bookmark", "create", "upper-mid", "-r", "@"]);
    jj(cwd, &["new", "-m", "upper-tip commit"]);
    std::fs::write(cwd.join("upper-tip.txt"), "from upper-tip\n").unwrap();
    jj(cwd, &["bookmark", "create", "upper-tip", "-r", "@"]);

    // Build a sibling single-commit stack rooted at main.
    jj(cwd, &["new", "main", "-m", "sibling-tip commit"]);
    std::fs::write(cwd.join("sibling.txt"), "from sibling\n").unwrap();
    jj(cwd, &["bookmark", "create", "sibling-tip", "-r", "@"]);

    // Park `@` somewhere irrelevant so the working copy isn't
    // entangled with either stack tip — matches what jj-gt's shelter
    // step would leave behind.
    jj(cwd, &["new", "main", "-m", "scratch wip"]);

    jj(cwd, &["git", "export"]);

    tmp
}

/// Simulate a `main@origin` advance by creating a fresh empty commit
/// above the current main and moving the `main` bookmark to it.
fn advance_main_one_commit(cwd: &Path) {
    jj(cwd, &["edit", "main"]);
    jj(cwd, &["new", "-m", "post-main work (now-trunk-tip)"]);
    jj(
        cwd,
        &["bookmark", "set", "main", "-r", "@", "--allow-backwards"],
    );
}

#[test]
fn discover_stacks_finds_both_unmerged_stacks() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Discovery uses `~::main` to exclude already-merged commits.
    // Pass plain "main" (no @origin in this fixture — bare jj repo,
    // no remote).
    let discovered = jj_gt::restack::discover_stacks(&jj_cli, "main").unwrap();

    // Two independent stacks: one rooted at upper-mid, one rooted at
    // sibling-tip.
    assert_eq!(discovered.len(), 2, "expected 2 stacks, got {discovered:?}",);

    let tip_names: std::collections::BTreeSet<&str> =
        discovered.iter().map(|ds| ds.tip.as_str()).collect();
    assert!(
        tip_names.contains("upper-tip"),
        "discovered tips should include `upper-tip`; got {tip_names:?}",
    );
    assert!(
        tip_names.contains("sibling-tip"),
        "discovered tips should include `sibling-tip`; got {tip_names:?}",
    );

    // The upper stack should carry both `upper-mid` and `upper-tip`.
    let upper_stack = discovered.iter().find(|ds| ds.tip == "upper-tip").unwrap();
    let upper_names: std::collections::BTreeSet<&str> = upper_stack
        .bookmarks
        .iter()
        .map(|sb| sb.name.as_str())
        .collect();
    assert!(
        upper_names.contains("upper-mid") && upper_names.contains("upper-tip"),
        "upper stack should include both upper-mid and upper-tip; got {upper_names:?}",
    );

    // root_commit should be populated for each stack.
    for ds in &discovered {
        assert!(
            ds.root_commit.is_some(),
            "stack `{}` should resolve a root commit",
            ds.tip
        );
    }
}

#[test]
fn discover_stacks_skips_trunk_and_gtmq_branches() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let cwd = tmp.path();

    // Add a `gtmq_test_123` branch to confirm it's skipped.
    jj(cwd, &["new", "main", "-m", "gtmq scratch"]);
    jj(cwd, &["bookmark", "create", "gtmq_test_123", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());
    let discovered = jj_gt::restack::discover_stacks(&jj_cli, "main").unwrap();

    let names: Vec<String> = discovered.iter().map(|ds| ds.tip.clone()).collect();
    assert!(
        !names.iter().any(|n| n == "main"),
        "discovery should never include trunk itself; got {names:?}",
    );
    assert!(
        !names.iter().any(|n| n.starts_with("gtmq_")),
        "discovery should skip gtmq_* branches; got {names:?}",
    );
}

#[test]
fn filter_by_bookmark_narrows_to_one_stack() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let discovered = jj_gt::restack::discover_stacks(&jj_cli, "main").unwrap();
    assert_eq!(discovered.len(), 2);

    let just_upper = jj_gt::restack::filter_by_bookmark(discovered.clone(), "upper-mid");
    assert_eq!(just_upper.len(), 1);
    assert_eq!(just_upper[0].tip, "upper-tip");

    let just_sibling = jj_gt::restack::filter_by_bookmark(discovered.clone(), "sibling-tip");
    assert_eq!(just_sibling.len(), 1);
    assert_eq!(just_sibling[0].tip, "sibling-tip");

    let none = jj_gt::restack::filter_by_bookmark(discovered, "no-such-bookmark");
    assert!(none.is_empty());
}

#[test]
fn run_restack_rebases_both_stacks_onto_advanced_main() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let cwd = tmp.path();

    // Capture the pre-advance main commit so we can detect the
    // rebase by comparing post-rebase ancestry.
    let jj_cli = JjCli::new(cwd.to_path_buf());
    let pre_main = jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap();

    advance_main_one_commit(cwd);
    let post_main = jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap();
    assert_ne!(
        pre_main, post_main,
        "test prereq: advancing main should change its commit id",
    );

    // Both stacks still point at the pre-advance main — they need
    // to be moved onto the new tip.
    let opts = jj_gt::restack::RestackOpts {
        trunk_destination: "main".into(),
        stop_on_conflict: false,
        dry_run: false,
        only_bookmark: None,
    };
    let results = jj_gt::restack::run_restack(&jj_cli, &opts).unwrap();

    assert_eq!(
        results.len(),
        2,
        "expected 2 per-stack results, got {results:?}"
    );
    for result in &results {
        assert!(
            matches!(result.outcome, jj_gt::restack::StackOutcome::Rebased { .. }),
            "stack `{}` should be cleanly rebased; got {:?}",
            result.tip,
            result.outcome,
        );
    }

    // The rebased bookmarks should now have post_main as an ancestor.
    let upper_tip = jj_gt::jj::resolve_commit_id(&jj_cli, "upper-tip").unwrap();
    let upper_ancestry: std::collections::BTreeSet<String> = Command::new("jj")
        .args([
            "log",
            "-r",
            &format!("::{upper_tip}"),
            "--no-graph",
            "-T",
            "commit_id ++ \"\\n\"",
            "--ignore-working-copy",
        ])
        .current_dir(cwd)
        .output()
        .unwrap()
        .stdout
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .lines()
        .map(|l| l.trim().to_owned())
        .collect();
    assert!(
        upper_ancestry.contains(&post_main),
        "post-rebase: upper-tip's ancestry should include the new main commit `{post_main}`; got {upper_ancestry:?}",
    );

    let sibling_tip = jj_gt::jj::resolve_commit_id(&jj_cli, "sibling-tip").unwrap();
    let sibling_ancestry: std::collections::BTreeSet<String> = Command::new("jj")
        .args([
            "log",
            "-r",
            &format!("::{sibling_tip}"),
            "--no-graph",
            "-T",
            "commit_id ++ \"\\n\"",
            "--ignore-working-copy",
        ])
        .current_dir(cwd)
        .output()
        .unwrap()
        .stdout
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .lines()
        .map(|l| l.trim().to_owned())
        .collect();
    assert!(
        sibling_ancestry.contains(&post_main),
        "post-rebase: sibling-tip's ancestry should include the new main commit; got {sibling_ancestry:?}",
    );

    assert!(
        !jj_gt::restack::any_unresolved(&results),
        "post-rebase: no stacks should be unresolved",
    );
}

#[test]
fn run_restack_already_current_when_main_hasnt_advanced() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Don't advance main. Restack should report AlreadyCurrent for
    // both stacks since the rebase is a no-op.
    let opts = jj_gt::restack::RestackOpts {
        trunk_destination: "main".into(),
        stop_on_conflict: false,
        dry_run: false,
        only_bookmark: None,
    };
    let results = jj_gt::restack::run_restack(&jj_cli, &opts).unwrap();
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(
            matches!(result.outcome, jj_gt::restack::StackOutcome::AlreadyCurrent),
            "stack `{}` should be AlreadyCurrent; got {:?}",
            result.tip,
            result.outcome,
        );
    }
}

#[test]
fn run_restack_dry_run_returns_planned_actions_without_mutating() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let cwd = tmp.path();
    let jj_cli = JjCli::new(cwd.to_path_buf());

    advance_main_one_commit(cwd);
    let post_main = jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap();
    let pre_upper_tip = jj_gt::jj::resolve_commit_id(&jj_cli, "upper-tip").unwrap();
    let pre_sibling_tip = jj_gt::jj::resolve_commit_id(&jj_cli, "sibling-tip").unwrap();

    let opts = jj_gt::restack::RestackOpts {
        trunk_destination: "main".into(),
        stop_on_conflict: false,
        dry_run: true,
        only_bookmark: None,
    };
    let results = jj_gt::restack::run_restack(&jj_cli, &opts).unwrap();
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(
            matches!(result.outcome, jj_gt::restack::StackOutcome::Rebased { .. }),
            "dry-run: stack `{}` should report a planned Rebased; got {:?}",
            result.tip,
            result.outcome,
        );
    }

    // Bookmarks should NOT have moved. Their commit ids should still
    // match what we captured pre-restack, and they should NOT have
    // post_main as an ancestor.
    assert_eq!(
        jj_gt::jj::resolve_commit_id(&jj_cli, "upper-tip").unwrap(),
        pre_upper_tip,
        "dry-run: upper-tip should still be at its pre-restack commit",
    );
    assert_eq!(
        jj_gt::jj::resolve_commit_id(&jj_cli, "sibling-tip").unwrap(),
        pre_sibling_tip,
        "dry-run: sibling-tip should still be at its pre-restack commit",
    );
    let _ = post_main; // referenced for clarity in the comment above
}

#[test]
fn run_restack_only_bookmark_narrows_scope_to_one_stack() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let cwd = tmp.path();
    let jj_cli = JjCli::new(cwd.to_path_buf());

    advance_main_one_commit(cwd);

    let pre_sibling_tip = jj_gt::jj::resolve_commit_id(&jj_cli, "sibling-tip").unwrap();

    let opts = jj_gt::restack::RestackOpts {
        trunk_destination: "main".into(),
        stop_on_conflict: false,
        dry_run: false,
        only_bookmark: Some("upper-mid".into()),
    };
    let results = jj_gt::restack::run_restack(&jj_cli, &opts).unwrap();

    // Only the upper stack should appear in the results.
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tip, "upper-tip");

    // Sibling's commit shouldn't have moved.
    assert_eq!(
        jj_gt::jj::resolve_commit_id(&jj_cli, "sibling-tip").unwrap(),
        pre_sibling_tip,
        "--bookmark scoping: sibling-tip should NOT have been touched",
    );
}

#[test]
fn run_restack_only_bookmark_errors_when_no_stack_matches() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_two_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let opts = jj_gt::restack::RestackOpts {
        trunk_destination: "main".into(),
        stop_on_conflict: false,
        dry_run: false,
        only_bookmark: Some("does-not-exist".into()),
    };
    let err =
        jj_gt::restack::run_restack(&jj_cli, &opts).expect_err("missing bookmark should error");
    let msg = format!("{err}");
    assert!(
        msg.contains("does-not-exist"),
        "error message should name the missing bookmark; got: {msg}",
    );
}

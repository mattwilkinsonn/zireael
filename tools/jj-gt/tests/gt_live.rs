//! Live integration tests against the `gt` (Graphite) CLI.
//!
//! These tests build a colocated jj/git repo in a temp dir, run
//! `gt init --trunk main --no-interactive` to write the Graphite
//! sidecar config, then exercise `jj_gt::gt::track` and assert that
//! the resulting `refs/branch-metadata/<branch>` ref's blob contains
//! the expected `parentBranchName`.
//!
//! No network calls — `gt track` is purely local. Tests skip silently
//! when `gt` (or `jj`) isn't on PATH so they don't fail in
//! environments that haven't installed them yet.

use std::path::Path;
use std::process::Command;

use jj_gt::gt;
use jj_gt::jj::JjCli;
use jj_gt::stack::{derive_parents, sort_for_tracking};

fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Isolate this test's gt user-config from the global
/// `~/.config/graphite/user_config`. nextest spawns each test in its
/// own process, so setting `XDG_CONFIG_HOME` is safe (no thread
/// race) but other parallel tests would otherwise hammer the shared
/// config file — concurrent writes truncate it mid-stream and the
/// next gt invocation errors with "Malformed JSON". The src/gt.rs
/// `run_gt` helper checks for `JJ_GT_TEST_XDG_CONFIG_HOME` and
/// propagates it through to gt so both the test-direct invocations
/// (gt_cli) and the library-level ones (jj_gt::gt::track) end up
/// pointed at the same per-test config dir.
fn isolate_graphite_config(tmp: &Path) {
    let xdg = tmp.join(".xdg-config");
    std::fs::create_dir_all(&xdg).unwrap();
    // Safety: nextest runs each test in its own process, so set_var
    // doesn't race with anything else in the binary.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &xdg);
        std::env::set_var("JJ_GT_TEST_XDG_CONFIG_HOME", &xdg);
    }
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

fn gt_cli(cwd: &Path, args: &[&str]) {
    let out = Command::new("gt")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gt {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn gt_capture(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("gt")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gt {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Build a fixture jj+git repo with a 3-bookmark linear stack on top
/// of `main`. Returns the TempDir guarding the cleanup.
///
/// Layout after this returns:
///
/// ```text
///   * top    bookmark
///   * mid    bookmark
///   * bottom bookmark
///   * main   bookmark (trunk)
///   * root
/// ```
fn build_three_stack_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    isolate_graphite_config(tmp.path());
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
    jj(tmp.path(), &["new", "-m", "bottom change"]);
    jj(tmp.path(), &["bookmark", "create", "bottom", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "mid change"]);
    jj(tmp.path(), &["bookmark", "create", "mid", "-r", "@"]);
    jj(tmp.path(), &["new", "-m", "top change"]);
    jj(tmp.path(), &["bookmark", "create", "top", "-r", "@"]);
    // jj `git init --colocate` keeps the git refs in sync on most ops,
    // but a final `git export` makes the contract explicit.
    jj(tmp.path(), &["git", "export"]);
    tmp
}

fn gt_init(tmp: &Path) {
    gt_cli(tmp, &["init", "--trunk", "main", "--no-interactive"]);
}

/// Ask gt itself which branch is the parent of `branch`. Works
/// uniformly across gt's storage-backend evolution:
///
/// - gt ≤ 1.7.x stores stack metadata as
///   `refs/branch-metadata/<branch>` git refs pointing at JSON
///   blobs.
/// - gt 1.8+ migrated to a SQLite database under
///   `$XDG_CONFIG_HOME/graphite/` — the git refs disappear
///   entirely.
///
/// Either way, `gt log short --no-interactive` renders the stack
/// tree in topological order (top → bottom), one bookmark per
/// line, formatted like:
///
/// ```text
/// ◯  top
/// ◯  mid
/// ◯  bottom
/// ◯  main
/// ```
///
/// The parent of `<branch>` is the next non-empty bookmark line
/// after `<branch>` — for a linear stack this is always the line
/// immediately below. We deliberately don't try to parse the
/// graph-glyph column (which encodes branching in non-linear
/// stacks) because every test fixture in this file builds a
/// linear stack on purpose.
///
/// Returns None when `<branch>` isn't in gt's tracked set or when
/// it sits at the bottom of the output (i.e. has no parent line
/// below it).
fn parent_in_metadata(workspace_root: &Path, branch: &str) -> Option<String> {
    let listing = gt_capture(workspace_root, &["log", "short", "--no-interactive"]);
    let names: Vec<&str> = listing
        .lines()
        .filter_map(parse_gt_log_short_line)
        .collect();
    let idx = names.iter().position(|n| *n == branch).or_else(|| {
        eprintln!(
            "parent_in_metadata: branch `{branch}` not found in `gt log short` output:\n{listing}"
        );
        None
    })?;
    names.get(idx + 1).map(|s| (*s).to_owned()).or_else(|| {
        eprintln!(
            "parent_in_metadata: branch `{branch}` has no parent line below it; full output:\n{listing}"
        );
        None
    })
}

/// Parse a single `gt log short` line to its bookmark name. gt
/// prefixes each tracked branch with a one-character graph glyph
/// (`◯`, `◉`, `●`, `*`, etc. depending on gt version + whether
/// the branch is the current one) followed by whitespace; the
/// remainder is the bookmark name plus optional annotation in
/// parens. We split on whitespace and take the second token, then
/// strip any trailing parenthesized annotation.
///
/// Returns None for blank lines or for lines we can't parse — the
/// caller will just skip them.
fn parse_gt_log_short_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // The graph glyph is always the first non-whitespace token.
    // Some gt versions use ASCII (`*`), others use unicode (`◯`).
    // After the glyph + whitespace, the next token is the branch
    // name. Annotations like `(needs restack)` may follow.
    let after_glyph = trimmed.split_whitespace().nth(1)?;
    Some(after_glyph)
}

#[test]
fn track_writes_metadata_ref_with_expected_parent() {
    if !binary_available("jj") || !binary_available("gt") {
        eprintln!("skipping: jj or gt not on PATH");
        return;
    }
    let tmp = build_three_stack_fixture();
    gt_init(tmp.path());

    gt::track(tmp.path(), "bottom", "main").unwrap();
    gt::track(tmp.path(), "mid", "bottom").unwrap();
    gt::track(tmp.path(), "top", "mid").unwrap();

    assert_eq!(
        parent_in_metadata(tmp.path(), "bottom").as_deref(),
        Some("main")
    );
    assert_eq!(
        parent_in_metadata(tmp.path(), "mid").as_deref(),
        Some("bottom")
    );
    assert_eq!(
        parent_in_metadata(tmp.path(), "top").as_deref(),
        Some("mid")
    );
}

#[test]
fn track_overwrites_existing_parent_on_re_invoke() {
    if !binary_available("jj") || !binary_available("gt") {
        eprintln!("skipping: jj or gt not on PATH");
        return;
    }
    let tmp = build_three_stack_fixture();
    gt_init(tmp.path());

    // Track with parent=main first.
    gt::track(tmp.path(), "bottom", "main").unwrap();
    assert_eq!(
        parent_in_metadata(tmp.path(), "bottom").as_deref(),
        Some("main")
    );

    // Now stand up a sibling trunk-adjacent branch and retrack
    // bottom onto it to verify the metadata gets rewritten.
    jj(
        tmp.path(),
        &["bookmark", "create", "alt_trunk", "-r", "main"],
    );
    jj(tmp.path(), &["git", "export"]);
    gt::track(tmp.path(), "alt_trunk", "main").unwrap();
    gt::track(tmp.path(), "bottom", "alt_trunk").unwrap();
    assert_eq!(
        parent_in_metadata(tmp.path(), "bottom").as_deref(),
        Some("alt_trunk"),
    );
}

#[test]
fn track_rejects_child_before_parent() {
    // gt 1.7.x errors with "Cannot perform this operation on untracked
    // branch X" if you try to track a child before its parent. We want
    // jj-gt to topo-sort and avoid this; the test pins the gt
    // behaviour we're working around.
    if !binary_available("jj") || !binary_available("gt") {
        eprintln!("skipping: jj or gt not on PATH");
        return;
    }
    let tmp = build_three_stack_fixture();
    gt_init(tmp.path());

    let err = gt::track(tmp.path(), "top", "mid");
    assert!(
        err.is_err(),
        "expected gt to reject tracking `top` while `mid` is untracked"
    );
}

#[test]
fn submit_path_orders_track_calls_bottom_to_top() {
    // End-to-end: derive_parents → sort_for_tracking → gt::track in
    // the same order jj-gt submit does. Passing an inverted user
    // order (-b top -b bottom -b mid) should still produce a
    // correctly tracked stack because of the topo sort.
    if !binary_available("jj") || !binary_available("gt") {
        eprintln!("skipping: jj or gt not on PATH");
        return;
    }
    let tmp = build_three_stack_fixture();
    gt_init(tmp.path());

    let jj_cli = JjCli::new(tmp.path().to_path_buf());
    let user_order = vec!["top".to_owned(), "bottom".to_owned(), "mid".to_owned()];
    let stacked = derive_parents(&jj_cli, &user_order, "main").unwrap();
    let sorted = sort_for_tracking(&stacked);

    for sb in &sorted {
        let parent = sb.parent.as_branch_name("main");
        gt::track(tmp.path(), &sb.name, parent).unwrap();
    }

    assert_eq!(
        parent_in_metadata(tmp.path(), "bottom").as_deref(),
        Some("main")
    );
    assert_eq!(
        parent_in_metadata(tmp.path(), "mid").as_deref(),
        Some("bottom")
    );
    assert_eq!(
        parent_in_metadata(tmp.path(), "top").as_deref(),
        Some("mid")
    );
}

#[test]
fn read_repo_config_trunk_round_trips_through_gt_init() {
    if !binary_available("jj") || !binary_available("gt") {
        eprintln!("skipping: jj or gt not on PATH");
        return;
    }
    let tmp = build_three_stack_fixture();
    gt_init(tmp.path());

    let trunk = gt::read_repo_config_trunk(tmp.path()).unwrap();
    assert_eq!(trunk.as_deref(), Some("main"));
}

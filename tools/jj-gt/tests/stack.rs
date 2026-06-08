//! End-to-end test for stack parent derivation against a real `jj`
//! repo. Builds a three-bookmark linear stack and asserts that
//! `derive_parents` walks the revset graph correctly.
//!
//! Skipped silently when `jj` isn't on PATH so the test can live in
//! the default `cargo test` set without forcing a hard dep on jj in
//! CI matrices that haven't installed it yet.

use std::path::Path;
use std::process::Command;

use jj_gt::jj::JjCli;
use jj_gt::stack::{BookmarkOrTrunk, derive_parents, find_tip};

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

/// Build a fixture jj repo with this shape:
///
/// ```text
///   * top    (top bookmark)
///   * mid    (mid bookmark)
///   * bottom (bottom bookmark)
///   * main   (trunk)
///   * root
/// ```
fn build_linear_stack_fixture() -> tempfile::TempDir {
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

    // Commit on the root change so `main` has somewhere real to point.
    jj(tmp.path(), &["describe", "-m", "root commit"]);
    jj(tmp.path(), &["bookmark", "create", "main", "-r", "@"]);

    // Bottom.
    jj(tmp.path(), &["new", "-m", "bottom change"]);
    jj(tmp.path(), &["bookmark", "create", "bottom", "-r", "@"]);

    // Mid.
    jj(tmp.path(), &["new", "-m", "mid change"]);
    jj(tmp.path(), &["bookmark", "create", "mid", "-r", "@"]);

    // Top.
    jj(tmp.path(), &["new", "-m", "top change"]);
    jj(tmp.path(), &["bookmark", "create", "top", "-r", "@"]);

    tmp
}

#[test]
fn derive_parents_linear_three_stack() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let bookmarks: Vec<String> = vec!["bottom".into(), "mid".into(), "top".into()];
    let stacked = derive_parents(&jj_cli, &bookmarks, "main").unwrap();

    let by_name: std::collections::HashMap<String, BookmarkOrTrunk> = stacked
        .iter()
        .map(|s| (s.name.clone(), s.parent.clone()))
        .collect();

    assert_eq!(by_name["bottom"], BookmarkOrTrunk::Trunk);
    assert_eq!(by_name["mid"], BookmarkOrTrunk::Bookmark("bottom".into()));
    assert_eq!(by_name["top"], BookmarkOrTrunk::Bookmark("mid".into()));

    let tip = find_tip(&stacked).unwrap();
    assert_eq!(tip, "top");
}

#[test]
fn bookmark_on_trunk_resolves_to_trunk_parent() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
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
    jj(tmp.path(), &["describe", "-m", "only commit"]);
    jj(tmp.path(), &["bookmark", "create", "main", "-r", "@"]);
    jj(tmp.path(), &["bookmark", "create", "solo", "-r", "@"]);

    let jj_cli = JjCli::new(tmp.path().to_path_buf());
    let stacked = derive_parents(&jj_cli, &["solo".into()], "main").unwrap();

    assert_eq!(stacked.len(), 1);
    assert_eq!(stacked[0].name, "solo");
    // `solo` and `main` point at the same commit. jj-gt treats main
    // as a special trunk name regardless of co-location with another
    // bookmark, so parent should resolve to Trunk.
    //
    // Note: the revset `heads(::solo & bookmarks() ~ solo ~ ::main)`
    // excludes everything reachable from main, so the result is the
    // empty set → Trunk parent.
    assert_eq!(stacked[0].parent, BookmarkOrTrunk::Trunk);
}

#[test]
fn bookmarks_in_revset_excludes_remote_only_refs() {
    // Regression: when a remote-tracking ref (e.g. graphite's
    // `graphite-base/<N>@origin` markers) sits on the same commit
    // as a real local bookmark, `bookmarks_in_revset` used to
    // return BOTH because the template iterated the umbrella
    // `bookmarks` keyword instead of `local_bookmarks`. That
    // turned downstream `derive_parents` calls into spurious
    // "multiple parent bookmarks found" errors on every
    // subsequent `jj-gt submit`.
    //
    // This test fabricates the colliding ref by writing directly
    // into the colocated git remote-ref store, importing it into
    // jj, then asserting only the local name comes back.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Pull the commit id `bottom` resolves to. We'll plant a
    // remote-tracking ref at that same OID so the revset query
    // returns both names.
    let bottom_oid =
        jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").expect("resolve bottom commit id");

    // Fabricate the remote-tracking ref. `git update-ref` is the
    // lowest-level way to do this without an actual remote — the
    // colocated layout puts a `refs/remotes/origin/...` write in
    // the place `jj git import` then reads from.
    let git_dir = tmp.path().join(".git");
    let status = std::process::Command::new("git")
        .args([
            "update-ref",
            "refs/remotes/origin/graphite-base/42",
            &bottom_oid,
        ])
        .env("GIT_DIR", &git_dir)
        .status()
        .expect("git update-ref");
    assert!(status.success(), "git update-ref failed");

    // Make jj see the remote-tracking ref.
    let status = std::process::Command::new("jj")
        .args(["git", "import"])
        .current_dir(tmp.path())
        .status()
        .expect("jj git import");
    assert!(status.success(), "jj git import failed");

    // Sanity check: both names DO show up under the umbrella
    // `bookmarks` template — i.e. the fixture is actually
    // reproducing the original bug shape, not just trivially
    // passing because the remote ref isn't there.
    let umbrella = jj_capture(
        tmp.path(),
        &[
            "log",
            "--no-graph",
            "-r",
            "bookmarks() & bottom",
            "-T",
            r#"bookmarks.map(|b| b.name()).join(",") ++ "\n""#,
            "--ignore-working-copy",
        ],
    );
    assert!(
        umbrella.contains("graphite-base/42") && umbrella.contains("bottom"),
        "fixture didn't reproduce the collision; got: {umbrella:?}",
    );

    // The actual assertion: our `bookmarks_in_revset` (now using
    // `local_bookmarks`) MUST NOT return the remote-only ref.
    let names = jj_gt::jj::bookmarks_in_revset(&jj_cli, "bookmarks() & bottom").unwrap();
    assert!(
        names.iter().any(|n| n == "bottom"),
        "expected `bottom` in {names:?}",
    );
    assert!(
        !names.iter().any(|n| n.starts_with("graphite-base/")),
        "remote-only ref leaked into bookmarks_in_revset: {names:?}",
    );
}

#[test]
fn derive_parents_skips_remote_only_collider() {
    // End-to-end version of the regression: plant a
    // remote-tracking ref on the same commit as a real local
    // bookmark, then run `derive_parents` against the *next*
    // bookmark up the stack. Without the local-only filter this
    // throws "multiple parent bookmarks found (["graphite-base/42",
    // "bottom"])"; with the fix it cleanly picks `bottom`.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let bottom_oid =
        jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").expect("resolve bottom commit id");

    let git_dir = tmp.path().join(".git");
    let status = std::process::Command::new("git")
        .args([
            "update-ref",
            "refs/remotes/origin/graphite-base/42",
            &bottom_oid,
        ])
        .env("GIT_DIR", &git_dir)
        .status()
        .expect("git update-ref");
    assert!(status.success());
    let status = std::process::Command::new("jj")
        .args(["git", "import"])
        .current_dir(tmp.path())
        .status()
        .expect("jj git import");
    assert!(status.success());

    // `mid` sits directly above `bottom`. Its derived parent must
    // be `bottom`, not the remote-only `graphite-base/42`.
    let derived = derive_parents(&jj_cli, &["mid".into()], "main").unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].name, "mid");
    match &derived[0].parent {
        BookmarkOrTrunk::Bookmark(name) => assert_eq!(name, "bottom"),
        other => panic!("expected parent=bottom, got {other:?}"),
    }
}

#[test]
fn bookmarks_in_revset_resolves_names() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // bookmarks() & ::@ should include all three stack bookmarks.
    let names = jj_gt::jj::bookmarks_in_revset(&jj_cli, "bookmarks() & ::@").unwrap();
    let set: std::collections::HashSet<String> = names.into_iter().collect();
    for expected in ["bottom", "mid", "top", "main"] {
        assert!(set.contains(expected), "missing `{expected}` in {set:?}");
    }
}

#[test]
fn current_change_id_round_trips() {
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let id = jj_gt::jj::current_change_id(&jj_cli).unwrap();
    assert!(!id.is_empty(), "expected a non-empty change id");

    // Compare against `jj log -r @` to sanity-check we're reading the
    // same thing.
    let direct = jj_capture(
        tmp.path(),
        &["log", "-r", "@", "--no-graph", "-T", "change_id"],
    );
    assert_eq!(id.trim(), direct.trim());
}

#[test]
fn resolve_commit_id_returns_full_oid_for_bookmark() {
    // The submit hook gate uses resolve_commit_id to build a real
    // BookmarkUpdate for jj_hooks (instead of going through the
    // synthesis layer that was historically buggy). This test pins
    // that the helper returns a non-empty 40-char hex commit id for
    // a bookmark name and for `@`.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    for revset in ["main", "bottom", "mid", "top", "@"] {
        let oid = jj_gt::jj::resolve_commit_id(&jj_cli, revset)
            .unwrap_or_else(|e| panic!("resolve `{revset}`: {e}"));
        assert_eq!(
            oid.len(),
            40,
            "expected 40-char commit id for `{revset}`, got `{oid}`",
        );
        assert!(
            oid.chars().all(|c| c.is_ascii_hexdigit()),
            "expected hex commit id for `{revset}`, got `{oid}`",
        );
    }

    // Empty-revset error path: a revset that resolves to no commits
    // surfaces as a clear error, not a panic or an empty string.
    let err =
        jj_gt::jj::resolve_commit_id(&jj_cli, "description(\"definitely-not-a-real-commit\")");
    assert!(err.is_err(), "expected error for empty revset, got {err:?}");
}

#[test]
fn list_local_bookmarks_returns_name_and_short_commit_id() {
    // Regression test: jj 0.40+ rejected our previous template
    // `name ++ " " ++ commit_id.short(12) ++ "\n"` because the
    // bookmark template scope has no top-level `commit_id` keyword.
    // The fix in src/jj.rs uses `self.normal_target().commit_id()`
    // under an if-guard; this test pins that the template stays
    // valid against whatever jj version we're running on.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let bookmarks = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let by_name: std::collections::HashMap<String, String> = bookmarks
        .into_iter()
        .map(|b| (b.name, b.commit_id))
        .collect();
    for name in ["bottom", "mid", "top", "main"] {
        let commit = by_name
            .get(name)
            .unwrap_or_else(|| panic!("missing bookmark `{name}` in {by_name:?}"));
        assert_eq!(
            commit.len(),
            12,
            "expected 12-char short id, got `{commit}`"
        );
        assert!(
            commit.chars().all(|c| c.is_ascii_hexdigit()),
            "expected hex short id, got `{commit}`",
        );
    }
}

#[test]
fn orphan_rebase_moves_full_multi_commit_range() {
    // Regression test for the sea-501 multi-commit-bookmark
    // conflict bug: when a bookmark holds 2+ commits (the user did
    // `jj new -m A; jj new -m B; jj bookmark create bk`), the
    // naive `jj rebase -s <bookmark> -d trunk` only moves the tip
    // commit (the one the bookmark name resolves to). The earlier
    // commits get stranded on the old parent's branch, and any
    // file they created looks like it "appeared from nowhere" on
    // the rebased tip → 2-sided file-creation conflict.
    //
    // Fixture topology, before the rebase:
    //
    //   * upper_b   <- `upper` bookmark
    //   * upper_a
    //   * bottom_b  <- `bottom` bookmark (the "merged" parent)
    //   * bottom_a
    //   * main
    //
    // We simulate gt sync having deleted `bottom` by passing the
    // pre-deletion commit id to build_orphan_rebase_revset. After
    // the rebase, `upper`'s ancestry chain should be
    // `upper_b -> upper_a -> main` (i.e. both upper_a + upper_b
    // came across), with no stranded bottom_* commits between them.
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

    // Two-commit "bottom" stack entry, holding its own files.
    jj(cwd, &["new", "-m", "bottom commit A: create file"]);
    std::fs::write(cwd.join("bottom.txt"), "from bottom A\n").unwrap();
    jj(cwd, &["new", "-m", "bottom commit B: extend file"]);
    std::fs::write(cwd.join("bottom.txt"), "from bottom A\nfrom bottom B\n").unwrap();
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);

    // Capture bottom's pre-deletion tip commit id — this is what
    // run_fetch reads out of bookmarks_before_sync.
    let bottom_commit = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
        ],
    )
    .trim()
    .to_owned();

    // Two-commit "upper" stack entry. Upper's commits modify upper's
    // OWN file (not bottom's). The point of the test is to verify
    // both upper_a and upper_b come across in the rebase; the
    // file-identity of bottom's content (which a real merge would
    // carry into main via squash) isn't what we're exercising.
    jj(cwd, &["new", "-m", "upper commit A: create upper file"]);
    std::fs::write(cwd.join("upper.txt"), "from upper A\n").unwrap();
    jj(cwd, &["new", "-m", "upper commit B: extend upper file"]);
    std::fs::write(cwd.join("upper.txt"), "from upper A\nfrom upper B\n").unwrap();
    jj(cwd, &["bookmark", "create", "upper", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    // Run the rebase using the same revset jj-gt would build.
    let jj_cli = JjCli::new(cwd.to_path_buf());
    let revset = jj_gt::cleanup::build_orphan_rebase_revset(&bottom_commit, "upper", "main");
    let outcome = jj_gt::jj::rebase(&jj_cli, &revset, "main").unwrap();

    assert!(
        matches!(outcome, jj_gt::jj::RebaseOutcome::Clean),
        "expected clean rebase, got {outcome:?}",
    );

    // Verify upper's ancestry: upper -> upper_a -> main, with no
    // bottom commits in between. The clean way to check: log
    // upper's ancestors that aren't reachable from main.
    // --ignore-working-copy matches jj_gt::jj::rebase's contract;
    // the rebase ran with --ignore-working-copy so the working
    // copy is "stale" by jj's reckoning and a plain `jj log`
    // would refuse to run.
    let ancestry = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "main..upper",
            "--no-graph",
            "-T",
            r#"description.first_line() ++ "\n""#,
            "--ignore-working-copy",
        ],
    );
    let lines: Vec<&str> = ancestry.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 commits in upper's ancestry above main, got: {ancestry}",
    );
    assert!(
        lines.iter().any(|l| l.contains("upper commit B")),
        "missing upper B commit: {ancestry}",
    );
    assert!(
        lines.iter().any(|l| l.contains("upper commit A")),
        "missing upper A commit — the multi-commit-range fix didn't move it: {ancestry}",
    );
    // And critically, NO bottom commits should be there.
    assert!(
        !lines.iter().any(|l| l.contains("bottom commit")),
        "bottom commits shouldn't be in upper's post-rebase ancestry: {ancestry}",
    );
}

#[test]
fn list_tracked_bookmarks_round_trips() {
    // Regression test for the "Warning: Remote bookmark already
    // tracked" spam: jj-gt's submit path uses this query to skip
    // redundant `jj bookmark track` calls when re-submitting a
    // stack where every bookmark is already tracked.
    //
    // No network — uses the linear-stack fixture, which has no
    // remote, so the tracked set should be empty. Once a remote
    // ref is added and tracked, the same call should return it.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Fresh fixture, no remote → tracked set is empty.
    let tracked = jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap();
    assert!(
        tracked.is_empty(),
        "no remote → tracked set should be empty, got {tracked:?}",
    );

    // Manually create a colocated git remote + a remote ref so we
    // can exercise the tracked-set populated path.
    let remote_dir = tempfile::tempdir().unwrap();
    let remote_path = remote_dir.path();
    let bare = std::process::Command::new("git")
        .args(["init", "--bare", "-q"])
        .current_dir(remote_path)
        .output()
        .unwrap();
    assert!(bare.status.success());
    let add_remote = std::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            &format!("file://{}", remote_path.display()),
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        add_remote.status.success(),
        "git remote add failed: {}",
        String::from_utf8_lossy(&add_remote.stderr)
    );

    // Push `top` to the bare remote so origin gets a real ref;
    // then explicitly track it via the wrapper under test.
    let push = std::process::Command::new("git")
        .args(["push", "origin", "top"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        push.status.success(),
        "git push failed: {}",
        String::from_utf8_lossy(&push.stderr)
    );
    jj(tmp.path(), &["git", "import"]);
    jj_gt::jj::track_bookmark_on_remote(&jj_cli, "top", "origin").unwrap();

    let tracked = jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap();
    assert!(
        tracked.contains("top"),
        "expected `top` in tracked set, got {tracked:?}",
    );
}

#[test]
fn expand_ancestors_for_submit_includes_chain_between_trunk_and_tip() {
    // Regression for issue #7: `jj-gt submit -b <tip>` against a
    // stack of unsubmitted bookmarks must expand the selection to
    // include every ancestor bookmark on the chain, otherwise
    // `gt track <tip> --parent <mid>` errors because <mid> isn't
    // tracked yet.
    //
    // We assert content not order — jj's revset emission order
    // varies and `sort_for_tracking` handles the bottom→top
    // ordering needed for `gt track` downstream. What this test
    // pins is "the full chain is in the output."
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let expanded =
        jj_gt::select::expand_ancestors_for_submit(&jj_cli, &["top".into()], "main").unwrap();
    let as_set: std::collections::BTreeSet<String> = expanded.iter().cloned().collect();
    assert_eq!(
        as_set,
        ["bottom", "mid", "top"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<std::collections::BTreeSet<String>>(),
        "expected {{bottom, mid, top}} from `-b top` expansion, got {expanded:?}",
    );

    // The expanded set must round-trip through derive_parents +
    // sort_for_tracking with the right order for `gt track`.
    let stacked = derive_parents(&jj_cli, &expanded, "main").unwrap();
    let sorted = jj_gt::stack::sort_for_tracking(&stacked);
    let order: Vec<&str> = sorted.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(order, vec!["bottom", "mid", "top"]);
    // And the tip should still be `top`.
    assert_eq!(find_tip(&stacked).unwrap(), "top");
}

#[test]
fn expand_ancestors_for_submit_dedupes_across_tips() {
    // Two selected tips sharing a common ancestor chain. The shared
    // ancestor should appear exactly once in the output. (Realistic
    // case: user passes both `-b foo-tip -b bar-tip` where both
    // sit on top of the same `base` bookmark.)
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // `mid` is an ancestor of `top` (chain: bottom → mid → top).
    // Expanding both `mid` and `top` together should yield bottom,
    // mid, top — each exactly once.
    let expanded =
        jj_gt::select::expand_ancestors_for_submit(&jj_cli, &["mid".into(), "top".into()], "main")
            .unwrap();
    assert_eq!(
        expanded.len(),
        3,
        "expected 3 unique entries, got {expanded:?}",
    );
    let as_set: std::collections::BTreeSet<String> = expanded.iter().cloned().collect();
    assert_eq!(as_set.len(), expanded.len(), "duplicates in {expanded:?}");
    assert_eq!(
        as_set,
        ["bottom", "mid", "top"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<std::collections::BTreeSet<String>>(),
        "expected deduped union {{bottom, mid, top}}, got {expanded:?}",
    );
}

#[test]
fn expand_ancestors_for_submit_single_on_trunk_keeps_just_tip() {
    // A bookmark sitting directly on trunk has no intermediate
    // ancestors. The expansion should return just the tip itself so
    // downstream derive_parents/track sees a single-element stack
    // with parent=Trunk.
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
    jj(cwd, &["new", "-m", "feature"]);
    jj(cwd, &["bookmark", "create", "feature", "-r", "@"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());
    let expanded =
        jj_gt::select::expand_ancestors_for_submit(&jj_cli, &["feature".into()], "main").unwrap();
    assert_eq!(expanded, vec!["feature".to_owned()]);
}

#[test]
fn list_local_bookmarks_skips_pending_deletion_bookmark() {
    // Regression for the "Revision X doesn't exist" abort in
    // `jj-gt fetch`: after a remote-deleted bookmark is imported,
    // jj keeps the local entry in `bookmark list` until the next
    // `jj git export`, but its target is a pending-deletion
    // sentinel. The previous template (`name ++ " " ++
    // if(normal_target, ..., "")`) printed the name with an empty
    // commit-id; the parser dropped those by the second-token
    // check, but `derive_parents` was then called with the empty
    // name and the revset failed. The fix uses `self.present()`
    // at the template's outer level so deleted-but-not-exported
    // bookmarks don't even emit a line.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Sanity: all 4 bookmarks present initially.
    let pre = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    assert_eq!(pre.len(), 4, "expected 4 bookmarks, got {pre:?}");

    // Delete one (creates the pending-deletion sentinel that
    // would survive until the next `jj git export`).
    jj(tmp.path(), &["bookmark", "delete", "mid"]);

    let post = jj_gt::jj::list_local_bookmarks(&jj_cli).unwrap();
    let names: std::collections::HashSet<String> = post.into_iter().map(|b| b.name).collect();
    assert!(
        !names.contains("mid"),
        "pending-deletion bookmark `mid` should be filtered, got {names:?}",
    );
    // The other three should still show up.
    for n in ["bottom", "top", "main"] {
        assert!(names.contains(n), "expected `{n}` to remain, got {names:?}");
    }
}

#[test]
fn derive_parents_lossy_skips_unresolved_bookmark_names() {
    // PR-D2 / "Revision X doesn't exist" fix: when the caller
    // enumerated a bookmark that no longer resolves to a commit
    // (deleted-but-not-exported zombie, conflict, etc.),
    // derive_parents_lossy logs+skips it rather than aborting.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    // Mix a real bookmark name with a clearly-bogus one. The
    // bogus revset fails; lossy should return just the real one.
    let names = vec!["bottom".to_owned(), "does-not-exist".to_owned()];
    let out = jj_gt::stack::derive_parents_lossy(&jj_cli, &names, "main");
    assert_eq!(
        out.len(),
        1,
        "expected only the real bookmark to survive, got {out:?}",
    );
    assert_eq!(out[0].name, "bottom");
}

#[test]
fn derive_parents_strict_propagates_revset_error_for_missing_bookmark() {
    // Counterpart: strict mode must still abort the call on a
    // failed revset so `submit_cmd` users get a real error rather
    // than a silent drop.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let tmp = build_linear_stack_fixture();
    let jj_cli = JjCli::new(tmp.path().to_path_buf());

    let names = vec!["bottom".to_owned(), "does-not-exist".to_owned()];
    let err = jj_gt::stack::derive_parents(&jj_cli, &names, "main").unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("does-not-exist") || msg.contains("doesn't exist"),
        "expected error to mention the missing bookmark, got: {msg}",
    );
}

/// Build a tiny one-commit workspace + return the JjCli for it.
/// Helper for the op-id snapshot / restore tests.
fn build_single_commit_workspace() -> (tempfile::TempDir, JjCli) {
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
    jj(cwd, &["new", "-m", "work"]);
    let jj_cli = JjCli::new(cwd.to_path_buf());
    (tmp, jj_cli)
}

#[test]
fn current_op_id_returns_a_hex_id_for_the_latest_op() {
    // Pin the contract: `current_op_id` returns a non-empty hex
    // string corresponding to the most recent op. Used as a
    // snapshot point for `op_restore` in the orphan-rebase
    // conflict-defer path.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let (_tmp, jj_cli) = build_single_commit_workspace();

    let op_id = jj_gt::jj::current_op_id(&jj_cli).unwrap();
    assert!(!op_id.is_empty(), "op id should be non-empty");
    assert!(
        op_id.chars().all(|c| c.is_ascii_hexdigit()),
        "op id should be all hex, got: {op_id:?}",
    );
    assert!(
        op_id.len() >= 12,
        "op id should be long enough to be unambiguous, got: {op_id:?}",
    );
}

#[test]
fn op_restore_rolls_back_a_bookmark_set() {
    // The full round-trip: snapshot op id → mutate → restore →
    // assert the mutation was undone. This is the primitive the
    // orphan-rebase conflict-defer path uses.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let (_tmp, jj_cli) = build_single_commit_workspace();
    let cwd = _tmp.path();

    let main_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap();
    let work_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "@").unwrap();
    assert_ne!(
        main_commit, work_commit,
        "test prereq: main and @ should be different commits",
    );

    // Snapshot. Then move main forward.
    let snapshot = jj_gt::jj::current_op_id(&jj_cli).unwrap();
    jj(cwd, &["bookmark", "set", "main", "-r", "@"]);
    assert_eq!(
        jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap(),
        work_commit,
        "post-mutation: main should be at the work commit",
    );

    // Roll back. main should snap back to its pre-snapshot position.
    jj_gt::jj::op_restore(&jj_cli, &snapshot).unwrap();
    assert_eq!(
        jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap(),
        main_commit,
        "post-restore: main should be back at the original commit",
    );
}

#[test]
fn count_conflicts_in_returns_zero_for_clean_revset() {
    // Cheap baseline test for the conflict-detection helper.
    // Used by the orphan-rebase phase to decide whether the
    // freshly-applied rebase actually conflicted or just looked
    // like it might from jj's stderr.
    if !jj_available() {
        eprintln!("skipping: jj not on PATH");
        return;
    }
    let (_tmp, jj_cli) = build_single_commit_workspace();

    // Clean workspace, no conflicts anywhere.
    assert_eq!(jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(), 0);
    assert_eq!(jj_gt::jj::count_conflicts_in(&jj_cli, "main").unwrap(), 0);
}

#[test]
fn fetch_orphan_rebase_defers_when_rebase_would_conflict() {
    // Regression for issue #60 part 2: `jj-gt fetch`'s orphan-
    // rebase should NOT leave the user with conflict markers when
    // the rebase would conflict. The fix snapshots the op id before
    // the rebase, runs it, and if jj reports conflicts in the
    // rebased range, rolls back via `op restore` and surfaces the
    // bookmark as `RebaseDeferredForConflict`.
    //
    // Fixture topology:
    //
    //   *  upper          <- `upper` bookmark
    //   *  bottom         <- `bottom` bookmark (will be "deleted")
    //   *  main
    //
    // `bottom` creates a file `shared.txt`, `upper` modifies the
    // same file with content that doesn't merge cleanly with what
    // main has (it has nothing). When `bottom` is removed from the
    // chain and `upper` is rebased onto main, jj produces a
    // conflict because upper's change has no base to apply to.
    //
    // The test calls the same `jj::rebase` codepath the orphan-
    // rebase phase uses + asserts the outcome is Conflicted; the
    // higher-level phase logic (which wraps with snapshot/restore)
    // is exercised indirectly via the unit tests above for the
    // helper primitives.
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

    // bottom creates shared.txt with content A.
    jj(cwd, &["new", "-m", "bottom: create shared.txt"]);
    std::fs::write(cwd.join("shared.txt"), "version A\n").unwrap();
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);
    let bottom_commit = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "bottom",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
        ],
    )
    .trim()
    .to_owned();

    // upper modifies shared.txt with content C — diverges from A.
    // When upper is rebased onto main (which has no shared.txt),
    // jj will see "delete the file" (since A is no longer the
    // base) vs "modify it to C" — a conflict.
    jj(cwd, &["new", "-m", "upper: modify shared.txt"]);
    std::fs::write(cwd.join("shared.txt"), "version C\n").unwrap();
    jj(cwd, &["bookmark", "create", "upper", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());

    // Snapshot the op id BEFORE the rebase — matches what
    // `orphan_rebase_phase` does in production.
    let pre_rebase_op = jj_gt::jj::current_op_id(&jj_cli).unwrap();
    let revset = jj_gt::cleanup::build_orphan_rebase_revset(&bottom_commit, "upper", "main");
    let outcome = jj_gt::jj::rebase(&jj_cli, &revset, "main").unwrap();

    // Confirm the rebase produced conflicts (the bug condition the
    // fix defends against).
    assert!(
        matches!(outcome, jj_gt::jj::RebaseOutcome::Conflicted { .. }),
        "fixture should produce a conflicted rebase to exercise the deferred path; got {outcome:?}",
    );
    let upper_after = jj_gt::jj::resolve_commit_id(&jj_cli, "upper").unwrap();
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, &format!("{upper_after}::{upper_after}")).unwrap(),
        1,
        "post-rebase: upper should carry a conflict marker",
    );

    // Roll back via op_restore — same primitive the fix uses.
    jj_gt::jj::op_restore(&jj_cli, &pre_rebase_op).unwrap();

    // The bookmark should be back where it started (pre-rebase).
    // No conflicts in the rebased range anymore.
    let upper_after_restore = jj_gt::jj::resolve_commit_id(&jj_cli, "upper").unwrap();
    assert_ne!(
        upper_after_restore, upper_after,
        "post-restore: upper's commit id should change back from the rebased version",
    );
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(),
        0,
        "post-restore: no conflicts should remain anywhere",
    );

    // Both `jj::rebase` and `jj::op_restore` pass `--ignore-working-
    // copy`, which means the on-disk file state hasn't been
    // synchronized with the repo state. The user-facing experience
    // depends on the NEXT jj command (which won't have that flag)
    // materializing the restored tree onto disk.
    //
    // Trigger materialization by running a plain `jj status` — that
    // command snapshots + updates the working copy by default.
    // Then assert the on-disk shared.txt has the restored content
    // ("version C\n", upper's pre-rebase state) and contains no
    // conflict markers.
    //
    // Without this assertion, the test would pass even if a
    // regression left conflict markers on the user's disk while
    // the repo state was technically clean.
    jj(cwd, &["status"]);
    let on_disk = std::fs::read_to_string(cwd.join("shared.txt"))
        .expect("shared.txt should exist after the working copy is materialized");
    assert_eq!(
        on_disk, "version C\n",
        "post-restore + materialize: shared.txt on disk should match upper's pre-rebase content; got: {on_disk:?}",
    );
    assert!(
        !on_disk.contains("<<<<<<<"),
        "post-restore: shared.txt on disk should not contain conflict markers; got: {on_disk:?}",
    );
    assert!(
        !on_disk.contains("======="),
        "post-restore: shared.txt on disk should not contain conflict markers; got: {on_disk:?}",
    );
    assert!(
        !on_disk.contains(">>>>>>>"),
        "post-restore: shared.txt on disk should not contain conflict markers; got: {on_disk:?}",
    );
}

#[test]
fn orphan_rebase_phase_defers_via_op_restore_on_conflict() {
    // Higher-level integration of the conflict-defer behavior.
    //
    // Where `fetch_orphan_rebase_defers_when_rebase_would_conflict`
    // (above) exercises the `current_op_id + rebase + op_restore`
    // primitives in sequence, this test walks the *real*
    // `orphan_rebase_phase` function — so the snapshot-failed and
    // restore-failed defensive arms have a happy-path baseline to
    // regress against.
    //
    // Fixture topology (identical to the primitive test):
    //
    //   *  upper          <- `upper` bookmark
    //   *  bottom         <- `bottom` bookmark (simulated as deleted
    //                       upstream — present in PreFetchSnapshot,
    //                       absent from list_local_bookmarks output
    //                       at phase time)
    //   *  main
    //
    // We construct a `PreFetchSnapshot` that includes `bottom` as
    // a `StackedBookmark` parent of `upper`, then delete `bottom`
    // locally so the phase sees it in the deleted set. The phase
    // should attempt the rebase, detect the conflict, roll back
    // via op_restore, and emit a `RebaseDeferredForConflict`
    // action — not a `RebaseConflicted`.
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

    // bottom: creates shared.txt with content A.
    jj(cwd, &["new", "-m", "bottom: create shared.txt"]);
    std::fs::write(cwd.join("shared.txt"), "version A\n").unwrap();
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);

    // upper: modifies shared.txt with content C.
    jj(cwd, &["new", "-m", "upper: modify shared.txt"]);
    std::fs::write(cwd.join("shared.txt"), "version C\n").unwrap();
    jj(cwd, &["bookmark", "create", "upper", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());
    let pre_upper = jj_gt::jj::resolve_commit_id(&jj_cli, "upper").unwrap();

    // Build a PreFetchSnapshot the way snapshot_pre_fetch would,
    // but bypass the gh call by passing an empty PR list to
    // snapshot_pre_fetch_with_prs.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        no_backfill: true,
        no_rebase: false,
        no_gtmq_prune: true,
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

    // Sanity check the snapshot captured the upper → bottom edge.
    let upper_parent =
        pre.stacked
            .iter()
            .find(|sb| sb.name == "upper")
            .map(|sb| match &sb.parent {
                jj_gt::stack::BookmarkOrTrunk::Bookmark(p) => p.as_str(),
                jj_gt::stack::BookmarkOrTrunk::Trunk => "main",
            });
    assert_eq!(
        upper_parent,
        Some("bottom"),
        "test prereq: pre-fetch snapshot should record upper → bottom; got {upper_parent:?}",
    );

    // Simulate the post-fetch state where `bottom`'s remote ref was
    // deleted (merged PR) and `jj git fetch`'s auto-import wiped the
    // local bookmark. The orphan-rebase phase reads
    // list_local_bookmarks at phase entry to compute the deleted
    // set; deleting bottom locally produces the exact condition
    // the phase exists to handle.
    jj(cwd, &["bookmark", "delete", "bottom"]);

    // Capture the post-fetch snapshot now (after the delete that
    // simulates the fetch-side ref removal). orphan_rebase_phase
    // uses this to compare against pre.normal for moved-sideways
    // detection — passing a fresh snapshot here mirrors the
    // run_fetch contract (post is captured right after `jj git
    // fetch`, before any other mutation step).
    let post = jj_gt::cleanup::snapshot_post_fetch(&jj_cli).unwrap();

    // Run the phase. The conflict-defer path should fire: snapshot
    // op id → rebase upper onto main → detect conflict → op_restore
    // → emit RebaseDeferredForConflict.
    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_rebase_phase(&jj_cli, &pre, &post, &opts, &mut actions).unwrap();

    // The phase should have produced exactly one action for upper.
    let upper_action = actions
        .iter()
        .find(|(b, _)| b.name == "upper")
        .map(|(_, a)| a.clone())
        .expect("phase should emit an action for upper");
    assert!(
        matches!(
            upper_action,
            jj_gt::cleanup::CleanupAction::RebaseDeferredForConflict { .. }
        ),
        "expected RebaseDeferredForConflict; got {upper_action:?}",
    );

    // The bookmark should be back where it started — op_restore
    // rolled the rebase back. Compare commit ids: post-phase upper
    // should match the pre-phase commit.
    let post_upper = jj_gt::jj::resolve_commit_id(&jj_cli, "upper").unwrap();
    assert_eq!(
        post_upper, pre_upper,
        "post-phase: upper should still point at its pre-rebase commit (rebase was rolled back); got {post_upper}, expected {pre_upper}",
    );

    // And no conflicts should remain in the workspace.
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(),
        0,
        "post-phase: no conflicts should remain anywhere",
    );
}

#[test]
fn orphan_rebase_phase_emits_bookmark_conflicted_when_target_has_divergent_heads() {
    // Issue #68: when the candidate bookmark itself is in
    // conflict (two op-log lineages disagree on the target
    // commit), the rebase invocation fails with "Name `<bm>` is
    // conflicted" — not a content conflict. The phase should
    // detect the divergence up front and emit
    // `BookmarkConflicted` instead of attempting a doomed rebase
    // and surfacing a misleading "produced conflicts" message.
    //
    // Fixture: main → bottom → upper, then simulate bottom's
    // deletion (orphan trigger) AND give `upper` a divergent
    // target by setting it concurrently to two distinct commits
    // via `--at-op`. Run the phase; expect a
    // `BookmarkConflicted { prev_parent: "bottom" }` action.
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
    jj(cwd, &["new", "-m", "bottom"]);
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);
    jj(cwd, &["new", "-m", "upper"]);
    jj(cwd, &["bookmark", "create", "upper", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());

    // Snapshot pre-fetch state so the orphan-rebase phase sees
    // upper → bottom as the relevant edge.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        no_backfill: true,
        no_rebase: false,
        no_gtmq_prune: true,
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

    // Simulate the orphan trigger: bottom's remote ref was
    // deleted upstream and fetch propagated the deletion.
    jj(cwd, &["bookmark", "delete", "bottom"]);

    // Make `upper` conflicted: race two `jj bookmark set` calls
    // via --at-op on the same parent op so jj's auto-merge
    // produces a divergent target. Both moves must change the
    // target — pointing at the current location is a no-op and
    // won't create a divergent op-log branch.
    jj(cwd, &["new", "main", "-m", "alt-upper"]);
    let alt_upper_change = jj_capture(cwd, &["log", "-r", "@", "--no-graph", "-T", "change_id"])
        .trim()
        .to_owned();
    let main_change = jj_capture(cwd, &["log", "-r", "main", "--no-graph", "-T", "change_id"])
        .trim()
        .to_owned();

    let parent_op = jj_capture(
        cwd,
        &["op", "log", "--no-graph", "-T", "id", "--limit", "1"],
    )
    .trim()
    .to_owned();
    // Branch 1: point upper at main.
    jj(
        cwd,
        &[
            "--at-op",
            &parent_op,
            "bookmark",
            "set",
            "upper",
            "-r",
            &main_change,
            "--allow-backwards",
        ],
    );
    // Branch 2: point upper at alt-upper (concurrent with branch 1
    // — same --at-op parent).
    jj(
        cwd,
        &[
            "--at-op",
            &parent_op,
            "bookmark",
            "set",
            "upper",
            "-r",
            &alt_upper_change,
            "--allow-backwards",
        ],
    );
    // Force the op-log merge so the conflict is materialized.
    jj(cwd, &["status"]);

    // Sanity: confirm jj sees `upper` as conflicted.
    let conflicted_now = jj_gt::jj::list_conflicted_bookmarks(&jj_cli).unwrap();
    assert!(
        conflicted_now.contains("upper"),
        "fixture failed to produce a conflicted `upper`: {conflicted_now:?}",
    );

    // Run the phase. The conflict-detect arm should fire.
    let post = jj_gt::cleanup::snapshot_post_fetch(&jj_cli).unwrap();
    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_rebase_phase(&jj_cli, &pre, &post, &opts, &mut actions).unwrap();

    let upper_action = actions
        .iter()
        .find(|(b, _)| b.name == "upper")
        .map(|(_, a)| a.clone())
        .expect("phase should emit an action for upper");
    match upper_action {
        jj_gt::cleanup::CleanupAction::BookmarkConflicted { prev_parent } => {
            assert_eq!(
                prev_parent, "bottom",
                "BookmarkConflicted should preserve the orphan trigger parent",
            );
        }
        other => panic!("expected BookmarkConflicted; got {other:?}"),
    }

    // The phase must NOT have left conflict markers in the
    // workspace — short-circuit means no rebase invocation,
    // which means no `jj resolve` to clean up.
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(),
        0,
        "BookmarkConflicted path must not leave content conflicts behind",
    );
}

#[test]
fn orphan_rebase_phase_reanchors_child_when_parent_moved_sideways_on_remote() {
    // Issue #69 (Shape A): Graphite pre-merge-rebases a PR;
    // origin's `<parent>` ref ends up at a different commit-id
    // with the same content. `jj git fetch` propagates the move
    // to local. The local child is still anchored to the old
    // parent commit (now an orphan from git's view). The phase
    // should detect the sideways move and rebase the child onto
    // the new parent commit, emitting
    // `RebasedAfterParentMoved`.
    //
    // Fixture:
    //   1. Build main → bottom → top, push to a bare remote.
    //   2. Manually rewrite `bottom` on the remote to a NEW
    //      commit (different OID, simulating Graphite's
    //      pre-merge rebase).
    //   3. Capture the pre-fetch snapshot, run `jj git fetch`
    //      to update local `bottom` to the new commit, then
    //      invoke `orphan_rebase_phase`.
    //   4. Assert: `top` was re-anchored, post-phase action is
    //      `RebasedAfterParentMoved`, and `top`'s parent is
    //      the new `bottom` commit.
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
    jj(cwd, &["new", "-m", "bottom"]);
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);
    jj(cwd, &["new", "-m", "top"]);
    jj(cwd, &["bookmark", "create", "top", "-r", "@"]);
    jj(cwd, &["new", "-m", "post-top wip"]);
    jj(cwd, &["git", "export"]);

    // Bare remote next to the workspace.
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
    assert!(
        add_remote.status.success(),
        "git remote add failed: {}",
        String::from_utf8_lossy(&add_remote.stderr)
    );
    for bookmark in ["main", "bottom", "top"] {
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
    for bookmark in ["main", "bottom", "top"] {
        jj_gt::jj::track_bookmark_on_remote(&jj_cli, bookmark, "origin").unwrap();
    }

    let pre_bottom_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").unwrap();
    let pre_top_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "top").unwrap();

    // Simulate Graphite's pre-merge rebase: clone the bare remote
    // into a scratch dir, rewrite `bottom`'s commit message
    // (which mints a new OID), and force-push it back. The
    // content stays effectively the same; what changes is the
    // commit id `bottom` resolves to on origin.
    let scratch = tempfile::tempdir().unwrap();
    let clone = std::process::Command::new("git")
        .args(["clone", "-q"])
        .arg(format!("file://{}", remote_path.display()))
        .arg(scratch.path())
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );
    let scratch_cwd = scratch.path();
    // Reset to the bottom commit, amend the message (allow-empty
    // since the fixture commits are empty), force-push.
    let cmds: &[&[&str]] = &[
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Tester"],
        &["checkout", "-q", "bottom"],
        &[
            "commit",
            "--amend",
            "--allow-empty",
            "-m",
            "bottom (graphite pre-merge rebase)",
        ],
        &["push", "origin", "+bottom"],
    ];
    for args in cmds {
        let out = std::process::Command::new("git")
            .args(*args)
            .current_dir(scratch_cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Pre-fetch snapshot (the production pipeline takes this
    // BEFORE `jj git fetch` so the parent-edge graph captures
    // the world as it was before the remote move).
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        no_backfill: true,
        no_rebase: false,
        no_gtmq_prune: true,
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

    // Now run the fetch — local `bottom` should advance to the
    // new commit, `top` should still be anchored to the old
    // commit.
    jj(cwd, &["git", "fetch", "--remote", "origin"]);

    let post_bottom_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").unwrap();
    assert_ne!(
        pre_bottom_commit, post_bottom_commit,
        "fetch should have advanced local `bottom` to the new commit; bug fixture"
    );

    // Run the phase. The sideways re-anchor should kick in for
    // `top`.
    let post = jj_gt::cleanup::snapshot_post_fetch(&jj_cli).unwrap();
    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_rebase_phase(&jj_cli, &pre, &post, &opts, &mut actions).unwrap();

    let top_action = actions
        .iter()
        .find(|(b, _)| b.name == "top")
        .map(|(_, a)| a.clone())
        .expect("phase should emit an action for top");
    match top_action {
        jj_gt::cleanup::CleanupAction::RebasedAfterParentMoved {
            parent_bookmark,
            pre_commit,
            post_commit,
        } => {
            assert_eq!(parent_bookmark, "bottom");
            assert!(
                pre_bottom_commit.starts_with(&pre_commit)
                    || pre_commit.starts_with(&pre_bottom_commit),
                "pre_commit {pre_commit} should match pre-fetch bottom {pre_bottom_commit}",
            );
            assert!(
                post_bottom_commit.starts_with(&post_commit)
                    || post_commit.starts_with(&post_bottom_commit),
                "post_commit {post_commit} should match post-fetch bottom {post_bottom_commit}",
            );
        }
        other => panic!("expected RebasedAfterParentMoved for top; got {other:?}"),
    }

    // Verify the rebase took effect: top's parent commit should
    // now be the post-fetch bottom commit (the new tip), not
    // the pre-fetch one.
    let new_top_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "top").unwrap();
    assert_ne!(
        pre_top_commit, new_top_commit,
        "rebase should have rewritten `top` onto the new parent",
    );
    // The parent-chain assertion: `top`'s direct parent commit
    // must be the post-fetch `bottom` tip. Without this, a
    // mistaken rebase onto `main` (or any other commit that
    // happens to differ from pre_top_commit) would still pass
    // the `assert_ne!` above and silently regress the contract.
    let top_parent = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "parents(top)",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_eq!(
        top_parent, post_bottom_commit,
        "post-phase: top should now be parented by the fetched `bottom` tip ({post_bottom_commit}); got {top_parent}",
    );

    // No content conflicts should remain — both commits had the
    // same content, just different commit ids.
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(),
        0,
        "Shape-A re-anchor against same-content rebase must not leave conflicts",
    );
}

#[test]
fn orphan_rebase_phase_does_not_double_rebase_when_sideways_parent_also_deleted_post_sync() {
    // Regression for the narrow race window greptile flagged on
    // PR #73: parent appears in BOTH the sideways set (it moved
    // post-fetch — Graphite pre-merge rebase) AND in the deleted
    // set (its branch got cleaned up post-sync because the PR
    // merged in the same `gt sync` run).
    //
    // Without the guard, the orphan-rebase loop first re-anchors
    // the child onto trunk (deleted-parent path), and the
    // sideways loop then fires a SECOND rebase that moves the
    // child off trunk and onto the now-abandoned sideways
    // commit. The user ends up on an orphan ref and sees two
    // confusing actions for one bookmark.
    //
    // The fix is a `remaining_names.contains(&parent_name)`
    // guard inside the sideways loop: when the parent is gone
    // post-sync, the deleted-parent path is the right handler;
    // the sideways branch must bail.
    //
    // Fixture: build the moved-sideways shape (main → bottom →
    // top, push, rewrite `bottom` on origin, fetch). Then,
    // BEFORE running the phase, locally delete `bottom` to
    // simulate `gt sync` having pruned it. Run the phase; assert
    // that:
    //   - top emits ONE action (the deleted-parent re-anchor,
    //     RebasedAfterParentMoved with parent="bottom" but the
    //     destination being trunk — actually emitted as a
    //     `RebasedAfterParentDeleted`/equivalent action; assert
    //     the count and direction);
    //   - top's final parent is `main`, not the abandoned
    //     pre/post sideways commits.
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
    jj(cwd, &["new", "-m", "bottom"]);
    jj(cwd, &["bookmark", "create", "bottom", "-r", "@"]);
    jj(cwd, &["new", "-m", "top"]);
    jj(cwd, &["bookmark", "create", "top", "-r", "@"]);
    jj(cwd, &["new", "-m", "post-top wip"]);
    jj(cwd, &["git", "export"]);

    // Bare remote.
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
    for bookmark in ["main", "bottom", "top"] {
        let push = std::process::Command::new("git")
            .args(["push", "origin", bookmark])
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(push.status.success());
    }
    jj(cwd, &["git", "import"]);
    let jj_cli = JjCli::new(cwd.to_path_buf());
    for bookmark in ["main", "bottom", "top"] {
        jj_gt::jj::track_bookmark_on_remote(&jj_cli, bookmark, "origin").unwrap();
    }

    let pre_bottom_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").unwrap();
    let trunk_commit = jj_gt::jj::resolve_commit_id(&jj_cli, "main").unwrap();

    // Rewrite `bottom` on origin (Graphite pre-merge rebase
    // shape).
    let scratch = tempfile::tempdir().unwrap();
    let clone = std::process::Command::new("git")
        .args(["clone", "-q"])
        .arg(format!("file://{}", remote_path.display()))
        .arg(scratch.path())
        .output()
        .unwrap();
    assert!(clone.status.success());
    let scratch_cwd = scratch.path();
    let cmds: &[&[&str]] = &[
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Tester"],
        &["checkout", "-q", "bottom"],
        &[
            "commit",
            "--amend",
            "--allow-empty",
            "-m",
            "bottom (graphite pre-merge rebase)",
        ],
        &["push", "origin", "+bottom"],
    ];
    for args in cmds {
        let out = std::process::Command::new("git")
            .args(*args)
            .current_dir(scratch_cwd)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    // Pre-fetch snapshot BEFORE the fetch (production
    // run_fetch contract).
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        no_backfill: true,
        no_rebase: false,
        no_gtmq_prune: true,
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

    // Fetch — local `bottom` advances to the new commit.
    jj(cwd, &["git", "fetch", "--remote", "origin"]);
    let post_bottom_commit_before_delete = jj_gt::jj::resolve_commit_id(&jj_cli, "bottom").unwrap();
    assert_ne!(
        pre_bottom_commit, post_bottom_commit_before_delete,
        "fetch should have advanced local `bottom`",
    );

    // Capture the post-fetch snapshot WHILE bottom is still
    // present — this mirrors run_fetch's ordering: post is
    // captured immediately after fetch, before sync/prune
    // mutations. So bottom appears in post.all even though it'll
    // be deleted moments later.
    let post = jj_gt::cleanup::snapshot_post_fetch(&jj_cli).unwrap();

    // Simulate `gt sync` deleting the merged PR's branch from
    // origin (the merged-PR cleanup) and the same sync run's
    // follow-up fetch propagating the deletion locally. By the
    // time orphan_rebase_phase runs, `bottom` is gone from
    // `remaining` AND gone from `remaining_names`, but the
    // sideways detection (built from pre + post) still has it
    // because the snapshot was taken between the first fetch and
    // the deletion.
    let delete_push = std::process::Command::new("git")
        .args(["push", "origin", "--delete", "bottom"])
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        delete_push.status.success(),
        "git push --delete bottom failed: {}",
        String::from_utf8_lossy(&delete_push.stderr)
    );
    jj(cwd, &["git", "fetch", "--remote", "origin"]);

    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_rebase_phase(&jj_cli, &pre, &post, &opts, &mut actions).unwrap();

    // The phase must emit EXACTLY ONE action for `top` — the
    // deleted-parent re-anchor. The sideways branch is supposed
    // to bail (via the parent-still-exists guard); without it,
    // we'd see two actions and the second rebase would move top
    // off trunk.
    let top_actions: Vec<&jj_gt::cleanup::CleanupAction> = actions
        .iter()
        .filter(|(b, _)| b.name == "top")
        .map(|(_, a)| a)
        .collect();
    assert_eq!(
        top_actions.len(),
        1,
        "top should receive exactly one action; got {top_actions:?}",
    );

    // top's final parent must be trunk (`main`), not either of
    // the abandoned `bottom` commits (pre or post).
    let top_parent = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "parents(top)",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_eq!(
        top_parent, trunk_commit,
        "top should be re-anchored on trunk, not on either abandoned bottom commit; \
         got {top_parent}, pre-bottom={pre_bottom_commit}, post-bottom={post_bottom_commit_before_delete}",
    );
}

#[test]
fn orphan_rebase_phase_emits_no_op_when_bookmark_already_advanced_past_deleted_parent() {
    // Regression for the wild bug Matt hit running `jj-gt fetch`
    // right after PR #71 (`jj-gt-67-auto-update-stale`) merged.
    //
    // Sequence:
    //   1. Parent PR merged → its branch deleted on origin, its
    //      content squashed into a new merge commit on trunk.
    //   2. Graphite's pre-merge rebase pushed the child PR's
    //      bookmark forward onto the new tip of trunk, then
    //      `jj git fetch` + `gt sync` + the import step pulled
    //      that down. The local child bookmark is now sitting
    //      on top of trunk (the OLD parent commit is no longer
    //      an ancestor of the child).
    //   3. `orphan_rebase_phase` runs with `pre.normal` still
    //      remembering the OLD parent position. It sees the
    //      parent in `deleted`, fires the orphan-rebase path,
    //      and builds the rebase revset
    //      `roots((OLD-parent..child) ~ ::trunk)`.
    //
    // Before this fix, two things went wrong:
    //   a. The revset was `roots(OLD-parent..child)` without the
    //      `~ ::trunk` carve-out, so it swept in every merge
    //      commit on trunk between OLD-parent and the new tip.
    //   b. Even with the carve-out, attempting the rebase is
    //      semantically a no-op (the bookmark is already on
    //      trunk) and the action log shouldn't surface a
    //      misleading "rebased onto main" message.
    //
    // The fix: late-check via `is_ancestor(OLD-parent, child)`.
    // When false, skip the rebase entirely and emit
    // `OrphanRebaseNoOpAlreadyOnTrunk` so the user sees that
    // the cleanup noticed the situation and chose not to act.
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

    // Build the pre-merge topology: main → OLD-parent → child.
    jj(cwd, &["new", "-m", "OLD-parent (PR that will merge)"]);
    let old_parent_commit = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
        ],
    )
    .trim()
    .to_owned();
    jj(cwd, &["bookmark", "create", "old-parent", "-r", "@"]);
    jj(cwd, &["new", "-m", "child PR commit"]);
    jj(cwd, &["bookmark", "create", "child", "-r", "@"]);
    jj(cwd, &["git", "export"]);

    let jj_cli = JjCli::new(cwd.to_path_buf());

    // Snapshot the pre-fetch state — this remembers OLD-parent
    // at `old_parent_commit` as `child`'s parent.
    let opts = jj_gt::cleanup::FetchOpts {
        remote: "origin".into(),
        trunk: "main".into(),
        no_backfill: true,
        no_rebase: false,
        no_gtmq_prune: true,
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

    // Sanity: the snapshot recorded child → old-parent.
    let child_parent =
        pre.stacked
            .iter()
            .find(|sb| sb.name == "child")
            .map(|sb| match &sb.parent {
                jj_gt::stack::BookmarkOrTrunk::Bookmark(p) => p.as_str(),
                jj_gt::stack::BookmarkOrTrunk::Trunk => "main",
            });
    assert_eq!(
        child_parent,
        Some("old-parent"),
        "test prereq: pre-fetch snapshot should record child → old-parent; got {child_parent:?}",
    );

    // Simulate the post-fetch state:
    //   - main advances to a NEW commit (PR #71 merge + an
    //     unrelated PR #70 commit on top — the immutable trunk
    //     content that the revset bug used to sweep in).
    //   - old-parent gets deleted (its branch is gone on origin).
    //   - child gets moved forward onto the new tip of trunk
    //     (Graphite's pre-merge rebase + the import step).
    //
    // The key shape: `child` is NOT a descendant of
    // `old_parent_commit` anymore. The is_ancestor check should
    // return false → emit OrphanRebaseNoOpAlreadyOnTrunk.

    // Move main to a new commit (simulating PR #71's merge +
    // any commits pushed to main while child was in review).
    jj(
        cwd,
        &["new", "main", "-m", "trunk move (PR #70 + PR #71 merges)"],
    );
    let new_trunk_commit = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
        ],
    )
    .trim()
    .to_owned();
    jj(
        cwd,
        &["bookmark", "set", "main", "-r", "@", "--allow-backwards"],
    );

    // Move child to a new commit on top of new trunk (Graphite
    // pre-merge rebase + import).
    jj(cwd, &["new", "main", "-m", "child PR commit (rebased)"]);
    jj(
        cwd,
        &["bookmark", "set", "child", "-r", "@", "--allow-backwards"],
    );

    // Delete old-parent locally (mirrors the fetch-side deletion).
    jj(cwd, &["bookmark", "delete", "old-parent"]);

    // Verify the fixture matches the bug shape: child is NOT a
    // descendant of old_parent_commit.
    let workspace_root = cwd;
    let is_anc = jj_gt::jj::is_ancestor(
        workspace_root,
        &old_parent_commit,
        &jj_gt::jj::resolve_commit_id(&jj_cli, "child").unwrap(),
    )
    .unwrap();
    assert!(
        !is_anc,
        "fixture intent: child should NOT be a descendant of old_parent_commit after the simulated fetch; \
         {old_parent_commit} is_ancestor of child = true (this means the test isn't reproducing the bug)",
    );

    let post = jj_gt::cleanup::snapshot_post_fetch(&jj_cli).unwrap();

    let mut actions: Vec<(jj_gt::jj::LocalBookmark, jj_gt::cleanup::CleanupAction)> = Vec::new();
    jj_gt::cleanup::orphan_rebase_phase(&jj_cli, &pre, &post, &opts, &mut actions).unwrap();

    // The phase MUST emit the no-op action (not Rebased, not
    // RebaseConflicted, not RebaseDeferredForConflict — the
    // bookmark didn't need touching).
    let child_action = actions
        .iter()
        .find(|(b, _)| b.name == "child")
        .map(|(_, a)| a.clone())
        .expect("phase should emit an action for child");
    assert!(
        matches!(
            child_action,
            jj_gt::cleanup::CleanupAction::OrphanRebaseNoOpAlreadyOnTrunk { .. }
        ),
        "expected OrphanRebaseNoOpAlreadyOnTrunk; got {child_action:?}",
    );

    // And the bookmark should still be on its rebased position
    // (no further mutation happened).
    let post_child = jj_gt::jj::resolve_commit_id(&jj_cli, "child").unwrap();
    let post_child_parent = jj_capture(
        cwd,
        &[
            "log",
            "-r",
            "child-",
            "--no-graph",
            "-T",
            "commit_id",
            "--limit",
            "1",
            "--ignore-working-copy",
        ],
    )
    .trim()
    .to_owned();
    assert_eq!(
        post_child_parent, new_trunk_commit,
        "post-phase: child should still be parented on new_trunk_commit (no rebase happened); \
         got parent {post_child_parent}, expected {new_trunk_commit}",
    );

    // And no conflicts in the workspace.
    assert_eq!(
        jj_gt::jj::count_conflicts_in(&jj_cli, "all()").unwrap(),
        0,
        "post-phase: no conflicts should remain",
    );

    let _ = post_child; // pin variable for clarity
}

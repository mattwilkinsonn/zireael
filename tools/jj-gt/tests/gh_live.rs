//! Live integration tests against the `gh` CLI.
//!
//! Hits real GitHub. Skipped unless **both** of these env vars are set:
//!
//! * `JJ_GT_LIVE_GH=1` — opt-in to live network tests.
//! * `JJ_GT_LIVE_REPO=<owner>/<repo>` — repo to query against. Set
//!   this to whatever repo `scripts/setup-live-test-fixture.sh`
//!   created (default: `<your-gh-user>/jj-gt-live-tests`).
//!
//! The fixture is a single long-lived branch + PR named
//! `fixture/persistent-pr`. The PR is intentionally left in the
//! `CLOSED` state — `gh pr list --state all` returns closed PRs the
//! same as open ones, and a closed fixture keeps the Graphite home
//! page from cluttering up with a never-merging "Do not touch" entry.
//! The setup script + the gt_submit_live cleanup both preserve this
//! invariant. Don't delete the branch or merge the PR.

use std::path::PathBuf;
use std::process::Command;

use jj_gt::gh::{self, PrState};

const FIXTURE_BRANCH: &str = "fixture/persistent-pr";

fn live_repo() -> Option<String> {
    if std::env::var("JJ_GT_LIVE_GH").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("JJ_GT_LIVE_REPO").ok()
}

/// Materialise a workspace that `gh` will treat as belonging to the
/// fixture repo. The gh CLI infers the target repo from the workspace's
/// git remote, so we make a one-commit git repo whose `origin` points
/// at the fixture and use it as `cwd` for the gh subprocess.
///
/// (Alternative: pass `--repo <owner>/<repo>` on every gh call. jj-gt's
/// production code doesn't do that — it relies on the workspace's
/// inferred remote — so the test should exercise the same path.)
fn fixture_workspace(repo: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "--quiet"]);
    run(&[
        "remote",
        "add",
        "origin",
        &format!("https://github.com/{repo}.git"),
    ]);
    tmp
}

fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn find_pr_for_fixture_branch_returns_persistent_pr() {
    let Some(repo) = live_repo() else {
        eprintln!("skipping: JJ_GT_LIVE_GH + JJ_GT_LIVE_REPO not set");
        return;
    };
    if !binary_available("gh") {
        eprintln!("skipping: gh not on PATH");
        return;
    }
    let workspace = fixture_workspace(&repo);

    let pr = gh::find_pr_for_branch(workspace.path(), FIXTURE_BRANCH)
        .expect("gh pr list should succeed");
    let pr = pr.unwrap_or_else(|| {
        panic!(
            "no PR found for fixture branch `{FIXTURE_BRANCH}` in {repo}; \
             did you run scripts/setup-live-test-fixture.sh?"
        )
    });

    assert_eq!(pr.head_ref_name, FIXTURE_BRANCH);
    assert!(pr.number > 0);
    // PR was created non-draft in setup; closing doesn't promote it
    // back to draft, so this should always hold.
    assert!(!pr.is_draft);
    // Fixture is parked in CLOSED state (see module docs). Open is
    // also acceptable — gh pr list returns both equally for our
    // purposes — but Merged would mean somebody accidentally merged
    // the fixture PR, which would invalidate the fixture (the
    // branch sha would no longer match what's referenced).
    assert!(
        !matches!(pr.state, PrState::Merged),
        "fixture PR shouldn't be merged; got state={:?}",
        pr.state
    );
}

#[test]
fn find_prs_for_branches_returns_fixture_in_batched_search() {
    let Some(repo) = live_repo() else {
        eprintln!("skipping: JJ_GT_LIVE_GH + JJ_GT_LIVE_REPO not set");
        return;
    };
    if !binary_available("gh") {
        eprintln!("skipping: gh not on PATH");
        return;
    }
    let workspace = fixture_workspace(&repo);

    // Include a non-existent branch alongside the fixture so we
    // exercise the case where gh returns a partial result.
    let branches = vec![
        FIXTURE_BRANCH.to_owned(),
        "branch-that-does-not-exist".into(),
    ];
    let prs = gh::find_prs_for_branches(workspace.path(), &branches, 50).unwrap();

    assert!(
        prs.iter().any(|p| p.head_ref_name == FIXTURE_BRANCH),
        "batched search didn't return the fixture PR; got: {prs:?}"
    );
}

#[test]
fn empty_branch_list_short_circuits_without_calling_gh() {
    // No env-var gate needed; the empty-list path is in-process.
    let cwd = PathBuf::from(".");
    let prs = gh::find_prs_for_branches(&cwd, &[], 50).unwrap();
    assert!(prs.is_empty());
}

#[test]
fn list_prs_by_head_prefix_empty_prefixes_short_circuits() {
    let cwd = PathBuf::from(".");
    let prs = gh::list_prs_by_head_prefix(&cwd, &[], 50).unwrap();
    assert!(prs.is_empty());
}

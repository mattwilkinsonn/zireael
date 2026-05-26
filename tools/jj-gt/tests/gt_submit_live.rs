//! Live end-to-end test for `jj-gt submit`. Creates real PRs in a
//! real GitHub repo, asserts, then cleans up.
//!
//! Skipped unless **all three** env vars are set:
//!
//! * `JJ_GT_LIVE_SUBMIT=1`
//! * `JJ_GT_LIVE_REPO=<owner>/<repo>` — same fixture repo the gh
//!   tests use (created by `scripts/setup-live-test-fixture.sh`).
//! * `JJ_GT_LIVE_REPO_URL=<git-url>` — fetchable + pushable URL for
//!   that repo (typically `https://github.com/<owner>/<repo>.git`
//!   with `gh auth setup-git` already done).
//!
//! Even with cleanup, every run creates and closes 2 PRs against the
//! fixture repo. That's fine for a low-frequency test; if you find
//! yourself running this on every save, gate it harder.

use std::path::Path;
use std::process::Command;

use jj_gt::cli::SubmitArgs;
use jj_gt::gh;
use jj_gt::gt;
use jj_gt::jj::JjCli;
use jj_gt::stack::{derive_parents, sort_for_tracking};

struct Env {
    repo: String,
    repo_url: String,
}

fn env_or_skip() -> Option<Env> {
    if std::env::var("JJ_GT_LIVE_SUBMIT").ok().as_deref() != Some("1") {
        return None;
    }
    let repo = std::env::var("JJ_GT_LIVE_REPO").ok()?;
    let repo_url = std::env::var("JJ_GT_LIVE_REPO_URL").ok()?;
    Some(Env { repo, repo_url })
}

fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run(cwd: &Path, bin: &str, args: &[&str]) -> std::process::Output {
    Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn run_ok(cwd: &Path, bin: &str, args: &[&str]) {
    let out = run(cwd, bin, args);
    assert!(
        out.status.success(),
        "{bin} {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Best-effort post-test cleanup. We try to close every PR and delete
/// every branch we created; any failure here just gets logged so the
/// next run still has a clean slate to work from (cleanup-on-startup
/// can be added later if this proves flaky).
struct Cleanup {
    repo: String,
    workspace: std::path::PathBuf,
    branches: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for branch in &self.branches {
            // Close any open PR for the branch.
            let close = Command::new("gh")
                .args([
                    "pr",
                    "close",
                    "--repo",
                    &self.repo,
                    "--delete-branch",
                    branch,
                ])
                .output();
            match close {
                Ok(o) if o.status.success() => eprintln!("cleanup: closed PR for {branch}"),
                Ok(o) => eprintln!(
                    "cleanup: gh pr close failed for {branch}: {}",
                    String::from_utf8_lossy(&o.stderr)
                ),
                Err(e) => eprintln!("cleanup: gh pr close errored: {e}"),
            }
            // Defensive: delete the remote branch outside gh too, in
            // case the PR-close-delete-branch path didn't take.
            let _ = Command::new("git")
                .args(["push", "origin", "--delete", branch])
                .current_dir(&self.workspace)
                .output();
        }
    }
}

#[test]
fn submit_creates_real_pr_stack_and_marks_them_ready() {
    let Some(env) = env_or_skip() else {
        eprintln!("skipping: JJ_GT_LIVE_SUBMIT + JJ_GT_LIVE_REPO + JJ_GT_LIVE_REPO_URL not set");
        return;
    };
    for tool in ["jj", "gt", "gh", "git"] {
        if !binary_available(tool) {
            eprintln!("skipping: {tool} not on PATH");
            return;
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();

    // Run-id keeps branch names unique across test invocations so a
    // failed cleanup doesn't poison the next run.
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let bottom = format!("jj-gt-test/{run_id}/bottom");
    let top = format!("jj-gt-test/{run_id}/top");

    // 1. Clone the fixture repo (colocated jj+git).
    eprintln!("cloning {} into {}", env.repo_url, workspace.display());
    let mut clone_cmd = Command::new("git");
    clone_cmd.args(["clone", "--depth", "1", &env.repo_url, "."]);
    clone_cmd.current_dir(workspace);
    let out = clone_cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // git config so commits we make below have valid metadata.
    run_ok(
        workspace,
        "git",
        &["config", "user.email", "jj-gt-live-test@example.com"],
    );
    run_ok(
        workspace,
        "git",
        &["config", "user.name", "jj-gt live test"],
    );

    // 2. Turn it into a jj workspace (`--colocate` so jj and git
    // share .git).
    run_ok(workspace, "jj", &["git", "init", "--colocate"]);

    // 3. `gt init` so the .graphite_repo_config sidecar exists. Safe
    // to re-run on an existing config.
    run_ok(
        workspace,
        "gt",
        &["init", "--trunk", "main", "--no-interactive"],
    );

    // Register cleanup BEFORE any push so we delete branches even if
    // an assertion below panics.
    let _cleanup = Cleanup {
        repo: env.repo.clone(),
        workspace: workspace.to_path_buf(),
        branches: vec![bottom.clone(), top.clone()],
    };

    // 4. Build a 2-bookmark stack on top of main.
    run_ok(
        workspace,
        "jj",
        &["new", "-m", "bottom commit (jj-gt live test)"],
    );
    // Touch a file so this commit is non-empty (gt rejects empty commits).
    std::fs::write(
        workspace.join(format!("fixture-{run_id}-bottom.txt")),
        "bottom\n",
    )
    .unwrap();
    run_ok(workspace, "jj", &["bookmark", "create", &bottom, "-r", "@"]);

    run_ok(
        workspace,
        "jj",
        &["new", "-m", "top commit (jj-gt live test)"],
    );
    std::fs::write(workspace.join(format!("fixture-{run_id}-top.txt")), "top\n").unwrap();
    run_ok(workspace, "jj", &["bookmark", "create", &top, "-r", "@"]);

    run_ok(workspace, "jj", &["git", "export"]);

    // 5. Run the same submit-path the binary does.
    let jj_cli = JjCli::new(workspace.to_path_buf());
    let selected = vec![bottom.clone(), top.clone()];
    let stacked = derive_parents(&jj_cli, &selected, "main").unwrap();
    let sorted = sort_for_tracking(&stacked);

    for sb in &sorted {
        let parent = sb.parent.as_branch_name("main");
        gt::track(workspace, &sb.name, parent).unwrap();
    }

    let submit_args = SubmitArgs {
        no_edit: true, // script-friendly
        ..SubmitArgs::default()
    };
    let argv = gt::build_submit_argv(&top, &submit_args);
    gt::submit(workspace, &argv).expect("gt submit should succeed");

    // 5b. Replicate jj-gt's post-submit bookmark-track pass so the
    // immutable-commits assertion below has the same starting
    // state the real `jj-gt submit` would leave. Mirror production
    // code's skip-if-already-tracked so we exercise both branches.
    let already_tracked =
        jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap_or_default();
    for sb in &sorted {
        if already_tracked.contains(&sb.name) {
            continue;
        }
        jj_gt::jj::track_bookmark_on_remote(&jj_cli, &sb.name, "origin")
            .expect("post-submit bookmark track should succeed");
    }

    // Sanity-check the tracking call actually achieved its goal —
    // re-query and assert both bookmarks now appear in the tracked
    // set. (Catches a future regression where the tracking call
    // silently no-ops.)
    let now_tracked =
        jj_gt::jj::list_tracked_bookmarks_on_remote(&jj_cli, "origin").unwrap_or_default();
    for name in [&bottom, &top] {
        assert!(
            now_tracked.contains(name),
            "expected `{name}` in tracked set after post-submit track; got {now_tracked:?}",
        );
    }

    // 6. Assert the PRs landed on github.
    let bottom_pr = gh::find_pr_for_branch(workspace, &bottom)
        .unwrap()
        .expect("expected an open PR for the bottom branch");
    let top_pr = gh::find_pr_for_branch(workspace, &top)
        .unwrap()
        .expect("expected an open PR for the top branch");

    assert_eq!(bottom_pr.head_ref_name, bottom);
    assert_eq!(top_pr.head_ref_name, top);
    assert!(
        !bottom_pr.is_draft,
        "submit defaults to --publish (non-draft)"
    );
    assert!(!top_pr.is_draft);

    // 7. Assert the pushed bookmarks are TRACKED — not freshly
    // imported as untracked — so the commits stay mutable. This
    // is the core fix the post-submit track step exists for; if
    // it regresses, this test catches it.
    let bookmark_list = run(workspace, "jj", &["bookmark", "list", "--all-remotes"]);
    let stdout = String::from_utf8_lossy(&bookmark_list.stdout);
    assert!(
        bookmark_list.status.success(),
        "jj bookmark list failed: {stdout}",
    );
    for name in [&bottom, &top] {
        // A tracked bookmark renders as a nested `@origin: …` line
        // under the bookmark name. An untracked remote bookmark
        // renders as a standalone `<name>@origin:` line at the
        // top level.
        let untracked_marker = format!("{name}@origin:");
        assert!(
            !stdout.contains(&untracked_marker),
            "bookmark {name} should be tracked after submit, but `{untracked_marker}` \
             appeared in:\n{stdout}",
        );
    }

    // 8. Assert the pushed commits are MUTABLE (not in
    // `immutable_heads()`). The cheapest probe is `jj log -r
    // <bookmark> & immutable_heads()` — empty output means
    // mutable, non-empty means the bookmark got frozen.
    for name in [&bottom, &top] {
        let revset = format!("{name} & immutable_heads()");
        let out = run(
            workspace,
            "jj",
            &[
                "log",
                "-r",
                &revset,
                "--no-graph",
                "-T",
                "commit_id",
                "--ignore-working-copy",
            ],
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "jj log -r `{revset}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.trim().is_empty(),
            "bookmark {name} should be mutable after submit but is in immutable_heads(): {stdout}",
        );
    }
}

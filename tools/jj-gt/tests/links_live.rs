//! Live end-to-end test for jj-gt's magic-word link hoisting
//! (`hoist_links_step` path). Creates a real PR with magic-word
//! references in its commit messages, runs the hoist, and asserts the
//! managed block lands in the PR description verbatim — then adds a
//! commit with a new reference and asserts the block updates.
//!
//! Skipped unless **all three** env vars are set (same gate as
//! `gt_submit_live.rs`):
//!
//! * `JJ_GT_LIVE_SUBMIT=1`
//! * `JJ_GT_LIVE_REPO=<owner>/<repo>`
//! * `JJ_GT_LIVE_REPO_URL=<git-url>`
//!
//! Every run creates + closes one PR against the fixture repo.

use std::path::Path;
use std::process::Command;

use jj_gt::cli::SubmitArgs;
use jj_gt::gh;
use jj_gt::gt;
use jj_gt::jj::{self, JjCli};
use jj_gt::links;
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

struct Cleanup {
    repo: String,
    workspace: std::path::PathBuf,
    branches: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for branch in &self.branches {
            let _ = Command::new("gh")
                .args([
                    "pr",
                    "close",
                    "--repo",
                    &self.repo,
                    "--delete-branch",
                    branch,
                ])
                .output();
            let _ = Command::new("git")
                .args(["push", "origin", "--delete", branch])
                .current_dir(&self.workspace)
                .output();
        }
    }
}

/// Run jj-gt's hoist for a single bookmark: read its commit range,
/// extract references, reconcile the PR body. Mirrors the per-bookmark
/// body of `hoist_links_step` (which is private to the binary).
fn hoist_one(jj_cli: &JjCli, workspace: &Path, parent: &str, bookmark: &str) {
    let messages = jj::commit_messages_in_range(jj_cli, parent, bookmark).unwrap();
    let refs = links::extract_references(&messages);
    let coauthors = links::extract_coauthors(&messages);
    let pr = gh::find_pr_for_branch(workspace, bookmark)
        .unwrap()
        .expect("expected an open PR for the bookmark");
    let body = gh::pr_body(workspace, pr.number).unwrap();
    let new_body = links::reconcile_body(&body, &refs, &coauthors);
    if new_body != body {
        gh::set_pr_body(workspace, pr.number, &new_body).unwrap();
    }
}

#[test]
fn submit_hoists_magic_word_references_into_pr_body() {
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

    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let branch = format!("jj-gt-test/{run_id}/links");

    let mut clone_cmd = Command::new("git");
    clone_cmd.args(["clone", "--depth", "1", &env.repo_url, "."]);
    clone_cmd.current_dir(workspace);
    let out = clone_cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

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
    run_ok(workspace, "jj", &["git", "init", "--colocate"]);
    run_ok(
        workspace,
        "gt",
        &["init", "--trunk", "main", "--no-interactive"],
    );

    let _cleanup = Cleanup {
        repo: env.repo.clone(),
        workspace: workspace.to_path_buf(),
        branches: vec![branch.clone()],
    };

    // Two commits naming two issues with different magic words: a
    // closing `Closes` and a non-closing `Refs`. Each also carries a
    // Co-Authored-By trailer so the hoist's trailer-union path is
    // exercised end-to-end.
    run_ok(
        workspace,
        "jj",
        &[
            "new",
            "-m",
            "feat: first part\n\nCloses SEA-100\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>",
        ],
    );
    std::fs::write(workspace.join(format!("fixture-{run_id}-a.txt")), "a\n").unwrap();
    run_ok(
        workspace,
        "jj",
        &[
            "new",
            "-m",
            "feat: second part\n\nRefs SEA-200\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>",
        ],
    );
    std::fs::write(workspace.join(format!("fixture-{run_id}-b.txt")), "b\n").unwrap();
    run_ok(workspace, "jj", &["bookmark", "create", &branch, "-r", "@"]);
    run_ok(workspace, "jj", &["git", "export"]);

    let jj_cli = JjCli::new(workspace.to_path_buf());
    let stacked = derive_parents(&jj_cli, std::slice::from_ref(&branch), "main").unwrap();
    let sorted = sort_for_tracking(&stacked);
    for sb in &sorted {
        gt::track(workspace, &sb.name, sb.parent.as_branch_name("main")).unwrap();
    }

    let submit_args = SubmitArgs {
        no_edit: true,
        no_ai: true, // deterministic body so the assertion is stable
        ..SubmitArgs::default()
    };
    let argv = gt::build_submit_argv(&branch, &submit_args);
    gt::submit(workspace, &argv).expect("gt submit should succeed");

    // Hoist into the PR body.
    hoist_one(&jj_cli, workspace, "main", &branch);

    let pr = gh::find_pr_for_branch(workspace, &branch).unwrap().unwrap();
    let body = gh::pr_body(workspace, pr.number).unwrap();
    assert!(
        body.contains("Closes SEA-100"),
        "PR body missing `Closes SEA-100`:\n{body}"
    );
    assert!(
        body.contains("Refs SEA-200"),
        "PR body missing `Refs SEA-200`:\n{body}"
    );
    assert!(
        body.contains("<!-- jj-gt:links -->"),
        "PR body missing the managed-block fence:\n{body}"
    );
    assert!(
        body.contains("Co-Authored-By: seal <noreply@sealedsecurity.com>"),
        "PR body missing the hoisted co-author trailer:\n{body}"
    );

    // Add a third commit naming a new issue with a closing word; the
    // bookmark advances. Re-run the hoist and assert the block now
    // also carries SEA-300, with exactly one fence (idempotent).
    run_ok(
        workspace,
        "jj",
        &["new", "-m", "fix: review feedback\n\nFixes SEA-300"],
    );
    std::fs::write(workspace.join(format!("fixture-{run_id}-c.txt")), "c\n").unwrap();
    run_ok(workspace, "jj", &["bookmark", "set", &branch, "-r", "@"]);
    run_ok(workspace, "jj", &["git", "export"]);

    hoist_one(&jj_cli, workspace, "main", &branch);

    let body2 = gh::pr_body(workspace, pr.number).unwrap();
    assert!(
        body2.contains("Closes SEA-300"),
        "re-hoist missing `Closes SEA-300`:\n{body2}"
    );
    assert!(
        body2.contains("Closes SEA-100") && body2.contains("Refs SEA-200"),
        "re-hoist dropped earlier references:\n{body2}"
    );
    assert_eq!(
        body2.matches("<!-- jj-gt:links -->").count(),
        1,
        "managed block must not be duplicated on re-hoist:\n{body2}"
    );
}

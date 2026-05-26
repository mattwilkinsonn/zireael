//! Subprocess wrappers for the `gt` (Graphite) CLI.
//!
//! gt is a Node-based CLI installed via `npm i -g
//! @withgraphite/graphite-cli`. We let PATH resolve it; macOS and Linux
//! behave identically since both end up at the same node entry point.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::cli::SubmitArgs;
use crate::error::{JjGtError, Result};

/// `gt track <branch> --parent <parent> --no-interactive`.
///
/// **Note on `--force` vs `--parent`:** in modern gt (1.7.x and
/// later) the `--force` flag means "auto-pick parent and ignore the
/// `--parent` value" — the opposite of what jj-gt wants. We never
/// pass `--force`; a plain `gt track <branch> --parent <name>`
/// already overwrites an existing metadata ref when re-invoked, so
/// there's no "force" needed.
///
/// `--no-interactive` keeps gt from prompting when invoked from a
/// script and is mandatory in a CI / non-TTY context.
pub fn track(workspace_root: &Path, branch: &str, parent: &str) -> Result<()> {
    run_gt(
        workspace_root,
        &["track", branch, "--parent", parent, "--no-interactive"],
    )
}

/// Build the `gt submit --stack --branch <tip> [...]` argv from a
/// populated [`SubmitArgs`]. Always appends `--publish` unless
/// `submit.draft` or `submit.no_publish` is set — see "DEFAULT PUBLISH
/// BEHAVIOUR" in the design doc.
///
/// Also appends `--no-verify` so gt's internal `git push` doesn't fire
/// the git pre-push hook a second time (we already ran it via
/// `hooks::run_pre_push` against the correct diff range; gt's git-push
/// would re-run it against the empty `origin/main..HEAD` range in a jj
/// workspace and either no-op or fail spuriously).
pub fn build_submit_argv(tip: &str, submit: &SubmitArgs) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "submit".into(),
        "--stack".into(),
        "--branch".into(),
        tip.into(),
    ];

    // Publish vs draft vs no-publish.
    if submit.draft {
        argv.push("--draft".into());
    } else if !submit.no_publish {
        argv.push("--publish".into());
    }

    if submit.restack {
        argv.push("--restack".into());
    }
    if submit.no_edit {
        argv.push("--no-edit".into());
    }
    if submit.ai {
        argv.push("--ai".into());
    }
    if submit.no_ai {
        argv.push("--no-ai".into());
    }
    if let Some(reviewers) = &submit.reviewers {
        argv.push("--reviewers".into());
        argv.push(reviewers.clone());
    }
    if let Some(team) = &submit.team_reviewers {
        argv.push("--team-reviewers".into());
        argv.push(team.clone());
    }
    if submit.update_only {
        argv.push("--update-only".into());
    }
    if submit.merge_when_ready {
        argv.push("--merge-when-ready".into());
    }
    if let Some(trunk) = &submit.target_trunk {
        argv.push("--target-trunk".into());
        argv.push(trunk.clone());
    }
    if submit.view {
        argv.push("--view".into());
    }
    if submit.web {
        argv.push("--web".into());
    }
    if let Some(comment) = &submit.comment {
        argv.push("--comment".into());
        argv.push(comment.clone());
    }
    if submit.rerequest_review {
        argv.push("--rerequest-review".into());
    }
    if submit.always {
        argv.push("--always".into());
    }
    if submit.force {
        argv.push("--force".into());
    }
    if submit.dry_run {
        argv.push("--dry-run".into());
    }
    if submit.confirm {
        argv.push("--confirm".into());
    }

    // Don't let gt re-run pre-push hooks — we ran them already against
    // the right revset. gt forwards `--no-verify` straight through to
    // its internal `git push`.
    argv.push("--no-verify".into());

    argv.extend(submit.gt_arg.iter().cloned());
    argv
}

/// Run `gt <argv>` in `workspace_root`.
pub fn submit(workspace_root: &Path, argv: &[String]) -> Result<()> {
    let str_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    run_gt(workspace_root, &str_refs)
}

/// `gt sync --no-restack --force`. We always pass `--no-restack`
/// because gt's git-rebase restack rewrites jj-tracked SHAs and
/// confuses jj's ref reconciliation — we use `jj rebase` instead.
pub fn sync_no_restack(workspace_root: &Path) -> Result<()> {
    run_gt(workspace_root, &["sync", "--no-restack", "--force"])
}

/// Shape of the JSON gt writes to `.git/.graphite_repo_config`. Only
/// the trunk name is interesting to us right now; everything else is
/// flexible and we don't want to fail the trunk-resolution path if
/// gt adds new fields.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoConfig {
    trunk: Option<String>,
}

/// Read the trunk name from `.git/.graphite_repo_config` if the file
/// exists. Returns `Ok(None)` for a missing file (caller can fall back
/// to a configured default like `"main"`); returns `Err` if the file
/// exists but doesn't parse.
pub fn read_repo_config_trunk(workspace_root: &Path) -> Result<Option<String>> {
    let path = workspace_root.join(".git").join(".graphite_repo_config");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(JjGtError::GtRepoConfig(format!("{}: {e}", path.display()))),
    };
    let cfg: RepoConfig = serde_json::from_str(&raw)
        .map_err(|e| JjGtError::GtRepoConfig(format!("{}: {e}", path.display())))?;
    Ok(cfg.trunk)
}

fn run_gt(workspace_root: &Path, argv: &[&str]) -> Result<()> {
    tracing::info!("running: gt {:?}", argv);
    let mut cmd = Command::new("gt");
    cmd.args(argv).current_dir(workspace_root);
    // Tests-only escape hatch: when JJ_GT_TEST_XDG_CONFIG_HOME is
    // set, propagate it to gt as XDG_CONFIG_HOME so parallel
    // integration tests each get their own user_config and don't
    // race on the shared ~/.config/graphite/user_config file (gt
    // rewrites it on every invocation; concurrent writes have been
    // observed to truncate it mid-write). Production code never
    // sets this variable.
    if let Ok(xdg) = std::env::var("JJ_GT_TEST_XDG_CONFIG_HOME") {
        cmd.env("XDG_CONFIG_HOME", xdg);
    }
    let status = cmd.status().map_err(|e| JjGtError::GtFailed {
        status: -1,
        stderr: format!("failed to spawn gt: {e}"),
    })?;
    if !status.success() {
        return Err(JjGtError::GtFailed {
            status: status.code().unwrap_or(-1),
            stderr: String::new(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SubmitArgs;

    fn args() -> SubmitArgs {
        SubmitArgs::default()
    }

    fn argv(submit: SubmitArgs) -> Vec<String> {
        build_submit_argv("top--athena", &submit)
    }

    #[test]
    fn default_includes_publish_and_no_verify() {
        let out = argv(args());
        assert!(out.contains(&"--publish".to_owned()), "got: {out:?}");
        assert!(out.contains(&"--no-verify".to_owned()), "got: {out:?}");
        assert!(!out.contains(&"--draft".to_owned()), "got: {out:?}");
        assert_eq!(out[0], "submit");
        assert_eq!(out[1], "--stack");
        assert_eq!(out[2], "--branch");
        assert_eq!(out[3], "top--athena");
    }

    #[test]
    fn draft_drops_publish_and_adds_draft() {
        let out = argv(SubmitArgs {
            draft: true,
            ..args()
        });
        assert!(out.contains(&"--draft".to_owned()), "got: {out:?}");
        assert!(!out.contains(&"--publish".to_owned()), "got: {out:?}");
    }

    #[test]
    fn no_publish_drops_both_publish_and_draft() {
        let out = argv(SubmitArgs {
            no_publish: true,
            ..args()
        });
        assert!(!out.contains(&"--publish".to_owned()), "got: {out:?}");
        assert!(!out.contains(&"--draft".to_owned()), "got: {out:?}");
    }

    #[test]
    fn modelled_flags_forwarded() {
        let out = argv(SubmitArgs {
            no_edit: true,
            ai: true,
            update_only: true,
            merge_when_ready: true,
            reviewers: Some("alice,bob".into()),
            team_reviewers: Some("eng".into()),
            comment: Some("ready for review".into()),
            ..args()
        });
        // Cheap structural check — the flag is somewhere in the argv.
        for flag in [
            "--no-edit",
            "--ai",
            "--update-only",
            "--merge-when-ready",
            "--reviewers",
            "--team-reviewers",
            "--comment",
        ] {
            assert!(
                out.iter().any(|s| s == flag),
                "expected flag {flag} in {out:?}"
            );
        }
        assert!(out.iter().any(|s| s == "alice,bob"));
        assert!(out.iter().any(|s| s == "ready for review"));
    }

    #[test]
    fn passthrough_appended_verbatim() {
        let out = argv(SubmitArgs {
            gt_arg: vec!["--some-niche-flag".into(), "value".into()],
            ..args()
        });
        let last_two = &out[out.len() - 2..];
        assert_eq!(last_two, &["--some-niche-flag", "value"]);
    }
}

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
///
/// gt's stderr is captured (not streamed) so a non-zero exit
/// surfaces the actual error message in `JjGtError::GtFailed.stderr`.
/// Errors like "branch X already tracks parent Y" or "parent Z does
/// not exist" are otherwise silently swallowed by the spinner row.
pub fn track(workspace_root: &Path, branch: &str, parent: &str) -> Result<()> {
    // Best-effort untrack first. Stale tracking metadata from a prior
    // submit where the stack was reordered makes plain `gt track`
    // exit non-zero with "branch already tracks parent X" even though
    // we want to overwrite. Untracking first turns the subsequent
    // track into the canonical "set parent" operation regardless of
    // prior state.
    //
    // Ignore the result — gt returns non-zero when the branch isn't
    // tracked yet, which is the common case on a fresh stack. The
    // subsequent track call is the one that actually reports a
    // meaningful failure.
    let _ = run_gt_captured(workspace_root, &["untrack", branch, "--no-interactive"]);
    run_gt_captured(
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
    // Default-on: `gt submit --always` so gt re-evaluates every PR
    // even when it thinks nothing changed. The motivation is the
    // "no-op recovery" trap — gt's diff heuristic decides a PR is
    // up-to-date based on the local branch head, but doesn't notice
    // that GitHub's PR base ref still points at a stale
    // `graphite-base/N` marker from a previous interrupted submit.
    // Forcing `--always` makes gt re-push the base ref alongside
    // the head ref. The cost is one extra round-trip per branch
    // when nothing genuinely changed — acceptable for a tool whose
    // job is "make Graphite's state match jj's state, every time."
    // Opt-out via `--no-always` for users who specifically want
    // gt's skip-unchanged heuristic.
    if !submit.no_always {
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

/// Run `gt <argv>` in `workspace_root`. Inherits stdout / stderr so
/// gt's progress output streams live to the user's terminal — used
/// for `gt submit` where the AI commit-message generation, push
/// progress, and PR-creation messages are exactly what the user
/// wants to see.
pub fn submit(workspace_root: &Path, argv: &[String]) -> Result<()> {
    let str_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    run_gt_streaming(workspace_root, &str_refs)
}

/// `gt sync --no-restack --force`. We always pass `--no-restack`
/// because gt's git-rebase restack rewrites jj-tracked SHAs and
/// confuses jj's ref reconciliation — we use `jj rebase` instead.
///
/// Captured (not streamed) so a sync failure surfaces gt's actual
/// error message rather than `gt exited with status 1: ` with an
/// empty stderr.
pub fn sync_no_restack(workspace_root: &Path) -> Result<()> {
    run_gt_captured(workspace_root, &["sync", "--no-restack", "--force"])
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

/// Apply the test-only `XDG_CONFIG_HOME` redirect so parallel
/// integration tests each get their own gt user_config and don't
/// race on the shared `~/.config/graphite/user_config` file (gt
/// rewrites it on every invocation; concurrent writes have been
/// observed to truncate it mid-write). Production code never sets
/// `JJ_GT_TEST_XDG_CONFIG_HOME`.
fn apply_test_xdg(cmd: &mut Command) {
    if let Ok(xdg) = std::env::var("JJ_GT_TEST_XDG_CONFIG_HOME") {
        cmd.env("XDG_CONFIG_HOME", xdg);
    }
}

/// Run gt with stdout / stderr inherited so the user sees progress
/// output live. Used for `gt submit` where waiting until completion
/// to print anything would make a 30-second push feel hung.
///
/// Trade-off: on failure, gt's stderr is gone (it streamed already)
/// so `JjGtError::GtFailed.stderr` is empty. For commands where we
/// want the error message, use [`run_gt_captured`] instead.
fn run_gt_streaming(workspace_root: &Path, argv: &[&str]) -> Result<()> {
    tracing::info!("running (streaming): gt {:?}", argv);
    let mut cmd = Command::new("gt");
    cmd.args(argv).current_dir(workspace_root);
    apply_test_xdg(&mut cmd);
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

/// Run gt with stdout + stderr captured so a non-zero exit can
/// surface the actual error message in `JjGtError::GtFailed.stderr`.
/// Used for `gt track`, `gt untrack`, and `gt sync` where the call
/// is short-running and progress output isn't load-bearing — but a
/// failure message ("branch already tracks parent X", "parent Y
/// does not exist", "missing graphite config") is critical signal
/// for the user.
///
/// gt sometimes writes the operative error to stdout instead of
/// stderr (depends on the subcommand), so we concatenate both into
/// the surfaced message, with stderr first since that's where most
/// errors land.
fn run_gt_captured(workspace_root: &Path, argv: &[&str]) -> Result<()> {
    tracing::info!("running (captured): gt {:?}", argv);
    let mut cmd = Command::new("gt");
    cmd.args(argv).current_dir(workspace_root);
    apply_test_xdg(&mut cmd);
    let output = cmd.output().map_err(|e| JjGtError::GtFailed {
        status: -1,
        stderr: format!("failed to spawn gt: {e}"),
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let merged = match (stderr.trim().is_empty(), stdout.trim().is_empty()) {
            (true, true) => String::from("(gt produced no output)"),
            (false, true) => stderr.trim_end().to_owned(),
            (true, false) => stdout.trim_end().to_owned(),
            (false, false) => format!("{}\n{}", stderr.trim_end(), stdout.trim_end()),
        };
        return Err(JjGtError::GtFailed {
            status: output.status.code().unwrap_or(-1),
            stderr: merged,
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
        // Default-on `--always` so gt re-pushes base refs even when
        // it thinks the branch is unchanged. The pre-2026-06 default
        // was opt-in `--always`; flipping it changed how gt
        // recovers from stale PR base refs after an interrupted
        // submit.
        assert!(out.contains(&"--always".to_owned()), "got: {out:?}");
        assert!(!out.contains(&"--draft".to_owned()), "got: {out:?}");
        assert_eq!(out[0], "submit");
        assert_eq!(out[1], "--stack");
        assert_eq!(out[2], "--branch");
        assert_eq!(out[3], "top--athena");
    }

    #[test]
    fn no_always_opts_out_of_default_always_flag() {
        // The user wants gt's skip-unchanged heuristic — typically
        // for repeated submits during PR review where the stack
        // hasn't moved.
        let out = argv(SubmitArgs {
            no_always: true,
            ..args()
        });
        assert!(
            !out.contains(&"--always".to_owned()),
            "expected --always to be absent when no_always=true, got: {out:?}",
        );
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

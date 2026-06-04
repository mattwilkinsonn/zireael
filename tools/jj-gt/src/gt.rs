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

/// Run `gt` with `argv` and return its stdout. Same env handling
/// as `run_gt_captured` but returns the output text instead of
/// dropping it.
fn run_gt_capture_stdout(workspace_root: &Path, argv: &[&str]) -> Result<String> {
    tracing::info!("running (capture-stdout): gt {:?}", argv);
    let mut cmd = Command::new("gt");
    cmd.args(argv).current_dir(workspace_root);
    apply_test_xdg(&mut cmd);
    let output = cmd.output().map_err(|e| JjGtError::GtFailed {
        status: -1,
        stderr: format!("failed to spawn gt: {e}"),
    })?;
    if !output.status.success() {
        // gt sometimes writes operative diagnostics to stdout
        // before failing (the same shape `run_gt_captured`
        // already handles). Merge both streams so the caller's
        // error message carries whatever gt actually said,
        // regardless of which fd it landed on.
        let stderr_lossy = String::from_utf8_lossy(&output.stderr);
        let stdout_lossy = String::from_utf8_lossy(&output.stdout);
        let stderr_trim = stderr_lossy.trim_end();
        let stdout_trim = stdout_lossy.trim_end();
        let merged = match (stderr_trim.is_empty(), stdout_trim.is_empty()) {
            (true, true) => String::from("(gt produced no output)"),
            (false, true) => stderr_trim.to_owned(),
            (true, false) => stdout_trim.to_owned(),
            (false, false) => format!("{stderr_trim}\n{stdout_trim}"),
        };
        return Err(JjGtError::GtFailed {
            status: output.status.code().unwrap_or(-1),
            stderr: merged,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Enumerate the branch names gt currently tracks via
/// `gt log short --no-interactive`. Returns the set as a sorted
/// `BTreeSet<String>` so callers can do `O(log n)` membership
/// checks.
///
/// gt's storage backend changed in 1.8 (refs/branch-metadata/* →
/// SQLite) so we can't read git refs directly; `gt log short` is
/// the version-agnostic enumeration shape.
///
/// Used by `cleanup::backfill_phase` and
/// `reconcile::retrack_adjacent_diverged` to scope auto-tracking
/// to bookmarks the user has already opted into via gt. A
/// brand-new local bookmark unrelated to any tracked stack — say,
/// another engineer's PR the user pulled down to review — is left
/// alone; jj-gt only fixes up things gt already cares about.
///
/// On a fresh repo `gt log short` outputs `◯  main` (just trunk),
/// so the empty/trunk-only case parses to `{ "main" }` and the
/// callers naturally short-circuit.
pub fn list_tracked_branches(workspace_root: &Path) -> Result<std::collections::BTreeSet<String>> {
    let out = run_gt_capture_stdout(workspace_root, &["log", "short", "--no-interactive"])?;
    Ok(parse_gt_log_short_branches(&out))
}

/// Pure: parse `gt log short --no-interactive` output into the
/// set of branch names it lists. The format is one branch per
/// line, with a graph glyph + whitespace prefix and an optional
/// trailing `(needs restack)` / `(current)` / `(PR #N)` annotation:
///
/// ```text
/// ◯  top
/// ◯  mid (needs restack)
/// ◉  bottom (current)
/// ◯  main
/// ```
///
/// We grab the first non-whitespace, non-glyph token of each line.
/// gt prints annotations like `(current)` / `(needs restack)` as
/// **separate** whitespace-delimited tokens (`main (current)` →
/// `["main", "(current)"]`), so taking `nth(1)` after the glyph
/// already drops them — no need to chop on `(`. Multi-stack output
/// uses the same one-branch-per-line shape with non-linear graph
/// glyphs (`├`, `│`, etc.); the column-after-glyph parse handles
/// those too.
fn parse_gt_log_short_branches(stdout: &str) -> std::collections::BTreeSet<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Skip the glyph column. `split_whitespace().nth(1)`
            // returns the first real token after the glyph
            // (matches the same parse the gt_live integration
            // test uses). Annotation parens are separate tokens
            // (`nth(2)` and onward) — they get naturally dropped.
            let tok = trimmed.split_whitespace().nth(1)?;
            // Don't chop on `(` — a branch name `foo(bar)` is
            // unusual but legal under git-check-ref-format, and
            // the annotation parens are already a separate token.
            let name = tok.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_owned())
            }
        })
        .collect()
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

    #[test]
    fn parse_gt_log_short_branches_only_trunk() {
        // Fresh repo (or one where gt knows only trunk) — the
        // helper should return a set with just "main".
        let out = "\n◯  main\n";
        let parsed = parse_gt_log_short_branches(out);
        let expected: std::collections::BTreeSet<String> =
            ["main".to_owned()].into_iter().collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_gt_log_short_branches_linear_stack() {
        let out = "\n◯  top\n◯  mid\n◯  bottom\n◯  main\n";
        let parsed = parse_gt_log_short_branches(out);
        let expected: std::collections::BTreeSet<String> = ["main", "bottom", "mid", "top"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_gt_log_short_branches_strips_annotation_parens() {
        // gt may suffix branches with `(needs restack)`,
        // `(current)`, etc. The parser must drop the annotation
        // and keep only the branch name. The annotations are
        // SEPARATE whitespace-delimited tokens, so taking the
        // first non-glyph token already drops them — no `(`
        // chopping required.
        let out = "◯  top (current)\n◯  mid (needs restack)\n◯  bottom\n◯  main\n";
        let parsed = parse_gt_log_short_branches(out);
        let expected: std::collections::BTreeSet<String> = ["main", "bottom", "mid", "top"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_gt_log_short_branches_preserves_parens_in_branch_name() {
        // Regression for the prior `tok.split('(').next()` shape
        // that truncated branch names containing `(`. Branch
        // names like `feature(foo)` are unusual but allowed by
        // git-check-ref-format, and the annotation parens are
        // already separated by whitespace — the parser shouldn't
        // chop on `(` and lose data.
        let out = "◯  feature(foo)\n◯  feature(bar) (current)\n◯  main\n";
        let parsed = parse_gt_log_short_branches(out);
        let expected: std::collections::BTreeSet<String> = ["feature(foo)", "feature(bar)", "main"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_gt_log_short_branches_empty_input() {
        // Defensive: gt could in principle return nothing (e.g.
        // a repo that hasn't been gt-init'd, in which case the
        // command would have errored and we wouldn't get here,
        // but the pure parser should still cope).
        let parsed = parse_gt_log_short_branches("");
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_gt_log_short_branches_handles_ascii_glyph() {
        // Older gt versions render with `*` instead of `◯`.
        // The parser is glyph-agnostic — it just takes the
        // first non-whitespace token after the glyph column.
        let out = "*  top\n*  bottom\n*  main\n";
        let parsed = parse_gt_log_short_branches(out);
        let expected: std::collections::BTreeSet<String> = ["main", "bottom", "top"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(parsed, expected);
    }
}

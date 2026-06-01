//! Smoke tests for the CLI binary — `--version`, `--help`, completions
//! generation. No external network or `gt`/`gh` invocations.

use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    // tests run at target/debug/deps/<name>-<hash>; binary is at
    // target/debug/jj-gt.
    p.pop(); // remove test exe name
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("jj-gt")
}

#[test]
fn version_flag_prints_something() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("jj-gt"), "got: {stdout}");
}

#[test]
fn help_flag_lists_subcommands() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in [
        "submit",
        "track",
        "fetch",
        "status",
        "log",
        "init",
        "completions",
    ] {
        assert!(stdout.contains(cmd), "expected `{cmd}` in:\n{stdout}");
    }
}

#[test]
fn submit_help_documents_publish_default() {
    let out = Command::new(bin())
        .args(["submit", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both passthrough flags should be present.
    assert!(stdout.contains("--draft"));
    assert!(stdout.contains("--no-publish"));
    assert!(stdout.contains("--merge-when-ready"));
}

#[test]
fn submit_help_documents_per_bookmark_hook_flags() {
    // PR-B added --hooks-tip-only and --hooks-sequential as
    // opt-outs from the default per-bookmark parallel hook gate.
    // Pin that both surface in `submit --help` so users discover
    // them.
    let out = Command::new(bin())
        .args(["submit", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--hooks-tip-only"),
        "missing --hooks-tip-only in help:\n{stdout}",
    );
    assert!(
        stdout.contains("--hooks-sequential"),
        "missing --hooks-sequential in help:\n{stdout}",
    );
}

#[test]
fn hooks_tip_only_conflicts_with_no_hooks() {
    // --no-hooks already skips the gate entirely; --hooks-tip-only
    // changes the gate's shape. The combination is meaningless;
    // clap should reject it.
    let out = Command::new(bin())
        .args(["submit", "--no-hooks", "--hooks-tip-only", "-b", "main"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "--no-hooks + --hooks-tip-only should error, but it succeeded",
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflict"),
        "expected conflict error, got: {stderr}",
    );
}

#[test]
fn hooks_sequential_conflicts_with_hooks_tip_only() {
    // tip-only is a single hook run; sequential is about HOW to
    // run N hooks. Combining them is meaningless.
    let out = Command::new(bin())
        .args([
            "submit",
            "--hooks-tip-only",
            "--hooks-sequential",
            "-b",
            "main",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "--hooks-tip-only + --hooks-sequential should error, but it succeeded",
    );
}

#[test]
fn reconcile_subcommand_in_help() {
    // PR-G added `jj-gt reconcile` as the standalone umbrella for
    // the #4 + #5 reconciliation steps. Pin that it surfaces in
    // the top-level help.
    let out = Command::new(bin()).args(["--help"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("reconcile"),
        "missing 'reconcile' subcommand in --help:\n{stdout}",
    );
}

#[test]
fn reconcile_help_documents_push_and_dry_run() {
    let out = Command::new(bin())
        .args(["reconcile", "--help"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--push"), "missing --push:\n{stdout}");
    assert!(stdout.contains("--dry-run"), "missing --dry-run:\n{stdout}");
    assert!(stdout.contains("--remote"), "missing --remote:\n{stdout}");
}

#[test]
fn completions_zsh_emits_script() {
    let out = Command::new(bin())
        .args(["completions", "zsh"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // dynamic env-driven registration scripts mention COMPLETE.
    assert!(stdout.contains("COMPLETE"), "got: {stdout}");
}

#[test]
fn completions_bash_emits_script() {
    let out = Command::new(bin())
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("COMPLETE"));
}

#[test]
fn init_prints_reminders() {
    // `init --print-only` skips the interactive prompt and just
    // dumps the setup reminders (preserves the pre-2026-05 default
    // for tests / scripts).
    let out = Command::new(bin())
        .args(["init", "--print-only"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("jj-gt"));
    assert!(stdout.contains("completions"));
}

#[test]
fn draft_and_no_publish_conflict() {
    // clap should reject `--draft --no-publish` since we marked them
    // `conflicts_with`.
    let out = Command::new(bin())
        .args(["submit", "--all", "--draft", "--no-publish"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("conflict") || stderr.contains("cannot be used"),
        "expected a conflict message; got: {stderr}"
    );
}

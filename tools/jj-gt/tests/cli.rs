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
    let out = Command::new(bin()).arg("init").output().unwrap();
    assert!(out.status.success());
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

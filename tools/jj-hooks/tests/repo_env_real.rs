//! Real-`direnv` integration tests for `repo_env`.
//!
//! The unit tests in `src/repo_env.rs` use a hermetic fake `direnv` shim to
//! prove the detection / retry / cache / apply logic. These tests exercise the
//! real `direnv export json` contract against a plain `export FOO=bar` `.envrc`
//! (no nix/devenv needed), guarding against direnv version drift in the export
//! and blocked-`.envrc` output formats (the blocked format has changed once
//! across direnv releases).
//!
//! Each test gates on `direnv` being present on the parent PATH; if it isn't,
//! the test early-returns (skips) rather than fails — mirroring
//! `runner_resolution_real.rs`. nextest runs each test in its own process, so
//! the in-process `XDG_*` overrides below are isolated per test.

use std::path::Path;
use std::process::Command;

use jj_hooks::repo_env::{EnvPatch, repo_env};

/// Skip-helper: true when `direnv` isn't installed on the parent PATH.
fn skip_if_no_direnv() -> bool {
    let missing = std::env::var_os("PATH")
        .map(|path| !std::env::split_paths(&path).any(|dir| dir.join("direnv").is_file()))
        .unwrap_or(true);
    if missing {
        eprintln!("test skipped: `direnv` not on parent PATH");
    }
    missing
}

/// Point direnv's allow-list + config at a tempdir so `direnv allow` state is
/// hermetic and doesn't touch the host's real direnv data.
fn isolate_direnv_state(home: &Path) {
    // SAFETY: nextest runs each test in its own process, so no sibling test
    // observes these mutations.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        // Ensure no stale DIRENV_* from the harness's own shell leaks in.
        std::env::remove_var("DIRENV_DIR");
        std::env::remove_var("DIRENV_FILE");
        std::env::remove_var("DIRENV_DIFF");
        std::env::remove_var("DIRENV_WATCHES");
    }
    std::fs::create_dir_all(home.join("data")).unwrap();
    std::fs::create_dir_all(home.join("config")).unwrap();
}

fn write_envrc(ws: &Path, body: &str) {
    std::fs::write(ws.join(".envrc"), body).unwrap();
}

fn direnv_allow(ws: &Path) {
    let out = Command::new("direnv")
        .arg("allow")
        .arg(ws.join(".envrc"))
        .current_dir(ws)
        .output()
        .expect("spawn direnv allow");
    assert!(
        out.status.success(),
        "direnv allow failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn blocked_envrc_before_allow_is_disabled() {
    if skip_if_no_direnv() {
        return;
    }
    let home = tempfile::TempDir::new().unwrap();
    isolate_direnv_state(home.path());
    let ws = tempfile::TempDir::new().unwrap();
    write_envrc(ws.path(), "export FOO=bar\n");
    // No `direnv allow` — the `.envrc` is blocked. repo_env degrades to
    // Disabled (and emits the visible eprintln, checked in unit tests).
    let patch = repo_env(ws.path(), true);
    assert_eq!(*patch, EnvPatch::Disabled);
}

#[test]
fn bare_env_export_emits_the_var() {
    if skip_if_no_direnv() {
        return;
    }
    let home = tempfile::TempDir::new().unwrap();
    isolate_direnv_state(home.path());
    let ws = tempfile::TempDir::new().unwrap();
    write_envrc(ws.path(), "export FOO=bar\n");
    direnv_allow(ws.path());
    // The harness process has no repo env loaded (bare), so the diff sets FOO.
    let patch = repo_env(ws.path(), true);
    let EnvPatch::Patch(map) = &*patch else {
        panic!("expected Patch, got {patch:?}");
    };
    assert_eq!(map.get("FOO"), Some(&Some("bar".to_string())));
}

#[test]
fn loaded_env_is_empty_patch() {
    if skip_if_no_direnv() {
        return;
    }
    let home = tempfile::TempDir::new().unwrap();
    isolate_direnv_state(home.path());
    let ws = tempfile::TempDir::new().unwrap();
    write_envrc(ws.path(), "export FOO=bar\n");
    direnv_allow(ws.path());

    // Simulate a shell that already has THIS repo's env loaded: run
    // `direnv export json` once to obtain the DIRENV_* state, apply it to the
    // process env, then repo_env's export should see no changes → empty diff.
    let out = Command::new("direnv")
        .arg("export")
        .arg("json")
        .current_dir(ws.path())
        .env("DIRENV_LOG_FORMAT", "")
        .output()
        .expect("spawn direnv export json");
    assert!(out.status.success());
    let diff: std::collections::HashMap<String, Option<String>> =
        serde_json::from_slice(&out.stdout).expect("parse direnv export");
    // SAFETY: per-process (nextest) isolation, see isolate_direnv_state.
    unsafe {
        for (k, v) in &diff {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    let patch = repo_env(ws.path(), true);
    let EnvPatch::Patch(map) = &*patch else {
        panic!("expected Patch (empty), got {patch:?}");
    };
    assert!(
        map.is_empty(),
        "already-loaded env should yield an empty diff, got: {map:?}"
    );
}

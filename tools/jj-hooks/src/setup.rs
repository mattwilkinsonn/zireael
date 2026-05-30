//! Pre-hook setup steps run inside the ephemeral worktree.
//!
//! When the user declares `jj-hooks.setup` in jj config (user- or
//! repo-level), each step's command runs inside the freshly-created
//! `git worktree add --detach` checkout before the hook runner
//! fires. The motivating use case (issue #9) is `node_modules`:
//! hooks like `tsc` need install-time resources that aren't in the
//! committed tree, so the worktree starts without them and the
//! hook fails with `command not found`. A user-declared setup
//! command (`bun install`, `pnpm install --frozen-lockfile`, etc.)
//! restores them.
//!
//! Config shape (matches the precedent set by pre-commit / hk
//! per-step tables):
//!
//! ```toml
//! [[jj-hooks.setup]]
//! name = "install deps"          # optional; falls back to run[0]
//! run = ["bun", "install", "--frozen-lockfile"]
//!
//! [[jj-hooks.setup]]
//! run = ["bun", "run", "prepare"]
//! ```
//!
//! A non-zero exit from any step aborts the pipeline before the
//! hook runner is invoked — `run_once` propagates the failure
//! through the same path as a failing hook.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::{JjHooksError, Result};
use crate::jj::JjCli;

/// One entry in `jj-hooks.setup`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SetupStep {
    /// Optional label used in failure messages. Falls back to
    /// `run[0]` (the bare program name) when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// argv list — `Command::new(run[0]).args(&run[1..])`. We use
    /// argv (not a shell string) so quoting rules can't bite. For
    /// chained commands the user runs `bash -c "..."` explicitly.
    pub run: Vec<String>,
}

impl SetupStep {
    /// Label to print in failure messages — `name` if set,
    /// otherwise the bare program name.
    pub fn label(&self) -> &str {
        if let Some(name) = self.name.as_deref() {
            return name;
        }
        self.run.first().map_or("<empty>", String::as_str)
    }
}

/// Parse `[[jj-hooks.setup]]` array-of-tables from a TOML fragment.
///
/// Input is the value `jj config get jj-hooks.setup` prints, which
/// for an array-of-tables comes back as a TOML *array of inline
/// tables* on a single line, e.g.
/// `[{ run = ["bun", "install"] }, { run = ["echo", "done"] }]`.
pub fn parse_steps(toml_fragment: &str) -> Result<Vec<SetupStep>> {
    // Wrap the array in a dummy key so we can use toml::de's table
    // deserializer (which is the only one stable across all the
    // shapes jj config get spits out — array-of-tables, inline,
    // empty array, etc.).
    let wrapped = format!("setup = {toml_fragment}");
    let table: SetupConfig = toml::from_str(&wrapped).map_err(|e| {
        JjHooksError::Parse(format!(
            "jj-hooks.setup must be an array of tables with a `run` field: {e}"
        ))
    })?;
    Ok(table.setup)
}

#[derive(Debug, Deserialize)]
struct SetupConfig {
    #[serde(default)]
    setup: Vec<SetupStep>,
}

/// Load `jj-hooks.setup` from jj's config, returning `Ok(vec![])`
/// when the key isn't set (`jj config get` exits non-zero for a
/// missing key — that's the "no setup configured" path, not an
/// error).
pub fn load_steps(jj: &JjCli) -> Result<Vec<SetupStep>> {
    // We can't distinguish "key missing" from "jj failed for some
    // other reason" without parsing jj's stderr, but the common
    // failure is exactly "key missing" — anything else surfaces
    // as a parse error on the empty input, which is fine.
    let Ok(value) = jj.run(&["config", "get", "jj-hooks.setup"]) else {
        return Ok(vec![]);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(vec![]);
    }
    parse_steps(value)
}

/// Execute each setup step in `worktree`. Returns `Ok(())` when all
/// steps succeed. The first non-zero exit aborts the rest and
/// returns a [`JjHooksError::JjFailed`] carrying the step's
/// label + the captured stderr.
///
/// `workspace_root` is exposed to each subprocess via the
/// `JJ_HOOKS_WORKSPACE` env var so steps can resolve paths from
/// the user's invocation workspace (e.g. `cp -al
/// "$JJ_HOOKS_WORKSPACE/node_modules" .` to hardlink-copy the
/// already-installed deps instead of running a full install).
pub fn run_steps(steps: &[SetupStep], worktree: &Path, workspace_root: &Path) -> Result<()> {
    for step in steps {
        if step.run.is_empty() {
            return Err(JjHooksError::Parse(format!(
                "jj-hooks.setup step `{}` has empty `run` array",
                step.label()
            )));
        }

        tracing::info!("setup step `{}`: running {:?}", step.label(), step.run);

        let status = Command::new(&step.run[0])
            .args(&step.run[1..])
            .current_dir(worktree)
            .env("JJ_HOOKS_WORKSPACE", workspace_root)
            .status()?;

        if !status.success() {
            return Err(JjHooksError::SetupFailed {
                name: step.label().to_owned(),
                status: status.code().unwrap_or(-1),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_step_with_run_only() {
        // The shape `jj config get` prints when the user writes
        // `setup = [{ run = ["bun", "install"] }]`.
        let steps = parse_steps(r#"[{ run = ["bun", "install"] }]"#).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].run, vec!["bun", "install"]);
        assert_eq!(steps[0].name, None);
    }

    #[test]
    fn parse_named_step() {
        let steps = parse_steps(r#"[{ name = "deps", run = ["bun", "install"] }]"#).unwrap();
        assert_eq!(steps[0].name.as_deref(), Some("deps"));
        assert_eq!(steps[0].label(), "deps");
    }

    #[test]
    fn parse_multiple_steps_preserves_order() {
        let steps = parse_steps(
            r#"[
              { run = ["echo", "one"] },
              { name = "two", run = ["echo", "two"] },
              { run = ["echo", "three"] },
            ]"#,
        )
        .unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].run, vec!["echo", "one"]);
        assert_eq!(steps[1].label(), "two");
        assert_eq!(steps[2].run, vec!["echo", "three"]);
    }

    #[test]
    fn parse_empty_array_is_no_steps() {
        let steps = parse_steps("[]").unwrap();
        assert!(steps.is_empty());
    }

    #[test]
    fn parse_missing_run_field_errors() {
        // A step table without `run` is a user typo we should catch
        // at config-load time, not at execute time.
        let err = parse_steps(r#"[{ name = "no-run" }]"#).unwrap_err();
        assert!(
            matches!(err, JjHooksError::Parse(_)),
            "missing `run` should be a parse error, got: {err:?}"
        );
    }

    #[test]
    fn parse_rejects_bare_string_value() {
        // The "convenient" shape some users might try (a single
        // command instead of an array) should error clearly rather
        // than silently no-op.
        let err = parse_steps(r#""bun install""#).unwrap_err();
        assert!(matches!(err, JjHooksError::Parse(_)));
    }

    #[test]
    fn label_falls_back_to_first_argv_when_name_missing() {
        let step = SetupStep {
            name: None,
            run: vec!["bun".into(), "install".into()],
        };
        assert_eq!(step.label(), "bun");
    }

    #[test]
    fn label_prefers_name_over_argv() {
        let step = SetupStep {
            name: Some("install deps".into()),
            run: vec!["bun".into(), "install".into()],
        };
        assert_eq!(step.label(), "install deps");
    }

    #[test]
    fn run_steps_passes_workspace_env_var() {
        // We can't easily intercept Command::env, but we *can* run
        // a real subprocess that records its env to a file. Using
        // /bin/sh keeps the test cheap (~1ms) and is portable to
        // any platform that has sh — which is every CI runner we
        // ship to.
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("env.txt");
        let step = SetupStep {
            name: None,
            run: vec![
                "sh".into(),
                "-c".into(),
                format!("printf %s \"$JJ_HOOKS_WORKSPACE\" > {}", out.display()),
            ],
        };
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        run_steps(&[step], tmp.path(), &workspace).unwrap();
        let captured = std::fs::read_to_string(&out).unwrap();
        assert_eq!(captured, workspace.display().to_string());
    }

    #[test]
    fn run_steps_aborts_on_first_failure() {
        // Step 1 fails; step 2 must NOT run. We assert step 2
        // didn't run by checking that its side-effect file is
        // absent after the call.
        let tmp = tempfile::TempDir::new().unwrap();
        let marker = tmp.path().join("step2_ran");
        let steps = vec![
            SetupStep {
                name: Some("step1".into()),
                run: vec!["false".into()],
            },
            SetupStep {
                name: Some("step2".into()),
                run: vec![
                    "sh".into(),
                    "-c".into(),
                    format!("touch {}", marker.display()),
                ],
            },
        ];
        let err = run_steps(&steps, tmp.path(), tmp.path()).unwrap_err();
        let JjHooksError::SetupFailed { name, .. } = err else {
            panic!("expected SetupFailed, got {err:?}");
        };
        assert!(
            name.contains("step1"),
            "abort message should name the failed step `step1`: {name}"
        );
        assert!(
            !marker.exists(),
            "step2 must not run after step1 fails (marker file present)",
        );
    }

    #[test]
    fn run_steps_empty_run_array_errors() {
        let steps = vec![SetupStep {
            name: Some("bad".into()),
            run: vec![],
        }];
        let err = run_steps(
            &steps,
            std::env::temp_dir().as_path(),
            std::env::temp_dir().as_path(),
        )
        .unwrap_err();
        assert!(matches!(err, JjHooksError::Parse(_)));
    }

    #[test]
    fn run_steps_no_steps_is_silent_ok() {
        run_steps(
            &[],
            std::env::temp_dir().as_path(),
            std::env::temp_dir().as_path(),
        )
        .unwrap();
    }
}

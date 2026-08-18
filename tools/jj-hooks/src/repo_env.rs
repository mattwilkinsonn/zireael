//! Propagate the repo's direnv/devenv environment into hook subprocesses.
//!
//! `jj-hp` runs pre-push hooks inside an ephemeral detached worktree under
//! `/tmp`, and the hook subprocess otherwise inherits only jj-hp's own
//! process environment — the repo's direnv/devenv environment (the one CI
//! loads via `devenv shell -- moon ci`) is never applied. Tools the runner
//! shells out to (moon, biome, proto shims) then resolve against the *system*
//! PATH instead of the devenv pins, producing false-red gates.
//!
//! This module computes the environment delta direnv would apply on entering
//! `workspace_root` (via `direnv export json`, run once per process and
//! cached) and merges it into each hook `Command` before it spawns. It is a
//! strict superset of today's behavior: when there is no `.envrc`, no
//! `direnv` on PATH, or the export fails, the patch is [`EnvPatch::Disabled`]
//! and every spawn is byte-identical to before (parent env + the caller's
//! `JJ_HOOKS_WORKSPACE`). See `docs/designs/tools/jj-hp-devenv-hook-env.md`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

/// The environment delta direnv would apply on entering `workspace_root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvPatch {
    /// No `.envrc`, no `direnv`, export failed, or opted out — spawn
    /// unchanged (parent env + `JJ_HOOKS_WORKSPACE`).
    Disabled,
    /// Apply: set each `Some(v)`, remove each `None` key. An EMPTY map is a
    /// success (the parent env already has this repo's env loaded, so the
    /// diff is empty), not `Disabled`.
    Patch(HashMap<String, Option<String>>),
}

/// Process-global cache of the computed patch, keyed by canonicalized
/// `workspace_root`. Populated once per process under the mutex so parallel
/// batch workers serialize on the single `direnv export`; spawn sites only
/// read it.
fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<EnvPatch>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<EnvPatch>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Canonicalized cache key. Falls back to the raw path when canonicalization
/// fails (e.g. the directory was removed) so distinct roots still map to
/// distinct keys. `dunce::canonicalize` matches the convention used
/// elsewhere in the crate (`jj.rs`) for git-facing paths.
fn cache_key(workspace_root: &Path) -> PathBuf {
    dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf())
}

/// Compute-and-cache the repo-env patch for `workspace_root`. Called EAGERLY
/// once at each batch entrypoint before worktree creation. `enabled` carries
/// the T4 opt-outs, read once at the entrypoint (see [`repo_env_enabled`]) —
/// there is no ambient global.
///
/// The first caller runs `direnv export json` (cwd = `workspace_root`; full
/// parent env inherited with only `DIRENV_LOG_FORMAT=""` overridden; stdout
/// piped for the JSON, stderr piped and routed to `tracing`). On failure it
/// retries once with `DIRENV_DIR`/`DIRENV_FILE`/`DIRENV_DIFF`/`DIRENV_WATCHES`
/// removed; only a failed retry degrades to `Disabled`.
pub fn repo_env(workspace_root: &Path, enabled: bool) -> Arc<EnvPatch> {
    let exporter = DirenvExporter {
        direnv: which("direnv"),
        base: BaseEnv::Inherit,
    };
    repo_env_with(workspace_root, enabled, &exporter)
}

/// Cache-aware core of [`repo_env`], parameterized over the exporter so the
/// detection / export / retry / cache logic is testable against a hermetic
/// fake `direnv` shim without touching the process PATH or environment.
fn repo_env_with(workspace_root: &Path, enabled: bool, exporter: &DirenvExporter) -> Arc<EnvPatch> {
    let key = cache_key(workspace_root);
    // Hold the lock across `compute` so parallel workers block on (and then
    // reuse) the single export rather than each paying it — the
    // `PklWarmCache::warm_once` precedent in `hooks.rs`.
    let mut guard = cache().lock().unwrap();
    if let Some(existing) = guard.get(&key) {
        return Arc::clone(existing);
    }
    let patch = Arc::new(compute(workspace_root, enabled, exporter));
    guard.insert(key, Arc::clone(&patch));
    patch
}

/// Merge the cached patch for `workspace_root` into `cmd`: `env()` for each
/// `Some(v)`, `env_remove()` for each `None`. Independent of the patch,
/// unconditionally strip the git repo-location env family (derived at runtime
/// from `git rev-parse --local-env-vars`) from the child — see
/// [`strip_git_local_env`]. Never touches `JJ_HOOKS_WORKSPACE` — callers set
/// it AFTER this call so it always wins.
pub fn apply_repo_env(cmd: &mut Command, workspace_root: &Path) {
    // The git repo-location strip is a safety invariant of spawning a hook
    // from a detached worktree (`GIT_DIR` from the worktree outranks the
    // child's cwd), NOT a devenv-feature convenience — so it runs on every
    // call, before any patch/cache early-return. The devenv env-merge below
    // stays gated on an active patch.
    strip_git_local_env(cmd);

    let key = cache_key(workspace_root);
    let patch = {
        let guard = cache().lock().unwrap();
        guard.get(&key).map(Arc::clone)
    };
    let Some(patch) = patch else {
        return; // Not populated (or a non-batch path) — no devenv patch to apply.
    };
    let map = match &*patch {
        EnvPatch::Patch(map) => map,
        EnvPatch::Disabled => return,
    };
    let git_local = git_local_env_vars();
    for (key, value) in map {
        // Git repo-location vars are handled by the unconditional strip above,
        // never applied from the patch.
        if git_local.contains(key) {
            continue;
        }
        match value {
            Some(v) => {
                cmd.env(key, v);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

/// Strip the git repo-location env family (`GIT_DIR`, `GIT_INDEX_FILE`,
/// `GIT_CONFIG_PARAMETERS`, and the rest of `git rev-parse --local-env-vars`)
/// from a hook/setup `cmd`. Run unconditionally on every spawn jj-hp makes
/// from a temp worktree, whatever the repo-env patch state:
///
/// A repo-local git-location var must NEVER reach the hook child — the child
/// runs inside the temp worktree and its git must resolve HEAD/index against
/// THAT worktree, not the primary workspace. The var reaches the child by
/// INHERITANCE from jj-hp's own env (git exports `GIT_DIR` into the pre-push
/// hook when the push runs from a linked worktree, and it outranks the child's
/// cwd), independent of any `.envrc` — so the strip cannot be coupled to an
/// active devenv patch, or a repo without direnv leaks `GIT_DIR` into its
/// hooks.
fn strip_git_local_env(cmd: &mut Command) {
    for key in git_local_env_vars() {
        cmd.env_remove(key);
    }
}

/// Read the T4 opt-outs, in precedence order:
/// 1. `JJ_HOOKS_NO_REPO_ENV` env var — any non-empty value disables.
/// 2. jj config `jj-hooks.repo-env` — `"off"` disables; unset or `"auto"`
///    (or anything else) enables. Read via the same `jj config get` pattern
///    as `runner::read_runner_bin_config`.
///
/// Called ONCE at each batch entrypoint and passed into [`repo_env`] as
/// `enabled`; never stored in an ambient global.
pub fn repo_env_enabled(jj: &crate::jj::JjCli) -> bool {
    let env_optout = std::env::var_os("JJ_HOOKS_NO_REPO_ENV");
    let config = jj.run(&["config", "get", "jj-hooks.repo-env"]).ok();
    enabled_from(env_optout.as_deref(), config.as_deref())
}

/// Pure decision core of [`repo_env_enabled`], split out so the precedence is
/// testable without a live jj repo.
fn enabled_from(env_optout: Option<&std::ffi::OsStr>, config: Option<&str>) -> bool {
    if let Some(value) = env_optout
        && !value.is_empty()
    {
        return false;
    }
    !matches!(config, Some(c) if c.trim().eq_ignore_ascii_case("off"))
}

/// Detect, export, parse. Returns the patch for one `workspace_root`.
fn compute(workspace_root: &Path, enabled: bool, exporter: &DirenvExporter) -> EnvPatch {
    if !enabled {
        return EnvPatch::Disabled;
    }
    if !workspace_root.join(".envrc").is_file() {
        return EnvPatch::Disabled;
    }
    let Some(direnv) = exporter.direnv.as_deref() else {
        return EnvPatch::Disabled;
    };

    match exporter.attempt(direnv, workspace_root, /*clear_direnv=*/ false) {
        Outcome::Patch(map) => EnvPatch::Patch(map),
        Outcome::Blocked => {
            report_blocked(workspace_root);
            EnvPatch::Disabled
        }
        Outcome::Fail(_) => {
            // F3: a corrupt/stale inherited `DIRENV_DIFF` makes the export
            // exit 1 (`Revert() failed: unmarshal() base64 decoding`); a retry
            // with the four vars removed recovers.
            match exporter.attempt(direnv, workspace_root, /*clear_direnv=*/ true) {
                Outcome::Patch(map) => EnvPatch::Patch(map),
                Outcome::Blocked => {
                    report_blocked(workspace_root);
                    EnvPatch::Disabled
                }
                Outcome::Fail(reason) => {
                    tracing::warn!(
                        "jj-hp: could not load repo env from `direnv export json` \
                         (hooks run without it): {reason}"
                    );
                    #[cfg(test)]
                    WARN_COUNT.fetch_add(1, Ordering::SeqCst);
                    EnvPatch::Disabled
                }
            }
        }
    }
}

/// Emit the ONE visible signal for a blocked `.envrc` — a fresh clone where
/// `direnv allow` was never run is the most common first-run state, and an
/// invisible `tracing::warn` would make the mechanism's absence
/// indistinguishable from the pre-fix bug.
fn report_blocked(workspace_root: &Path) {
    eprintln!(
        "jj-hp: .envrc present but not allowed; hooks run without the repo env \
         (run `direnv allow` in {})",
        workspace_root.display()
    );
    #[cfg(test)]
    BLOCKED_SIGNAL_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Outcome of a single `direnv export json` attempt.
enum Outcome {
    /// Exit 0. Empty stdout → empty map (parent env already loaded); non-empty
    /// stdout → the parsed diff.
    Patch(HashMap<String, Option<String>>),
    /// Non-zero exit whose stderr carries direnv's not-allowed signature.
    Blocked,
    /// Non-zero exit (not blocked), a spawn error, or a parse error.
    Fail(String),
}

/// How the export subprocess's base environment is built.
enum BaseEnv {
    /// Production: inherit jj-hp's full environment (the load-bearing
    /// invariant — direnv reverses a previously-loaded repo's env via the
    /// inherited `DIRENV_DIFF` before applying this repo's `.envrc`).
    Inherit,
    /// Tests: a fully controlled environment (via `env_clear`) so the
    /// inherit-vs-sanitize and retry behaviors are hermetic.
    #[cfg(test)]
    Explicit(Vec<(String, String)>),
}

struct DirenvExporter {
    direnv: Option<PathBuf>,
    base: BaseEnv,
}

impl DirenvExporter {
    fn attempt(&self, direnv: &Path, workspace_root: &Path, clear_direnv: bool) -> Outcome {
        let output = match self.run_export(direnv, workspace_root, clear_direnv) {
            Ok(output) => output,
            Err(e) => return Outcome::Fail(format!("spawn failed: {e}")),
        };
        if output.status.success() {
            // Exit 0 with empty (or whitespace-only) stdout is a SUCCESS: the
            // parent shell already has this repo's env loaded, so the diff is
            // empty. Warning here would be a lie and fire on every push from a
            // loaded shell.
            if output.stdout.iter().all(u8::is_ascii_whitespace) {
                return Outcome::Patch(HashMap::new());
            }
            match serde_json::from_slice::<HashMap<String, Option<String>>>(&output.stdout) {
                Ok(map) => Outcome::Patch(map),
                Err(e) => Outcome::Fail(format!("parse error: {e}")),
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // direnv's blocked-`.envrc` signature ("... is blocked. Run
            // `direnv allow` ..."). A retry with cleared DIRENV_* cannot fix
            // this, so it is its own outcome, not a `Fail`.
            if stderr.contains("is blocked") {
                Outcome::Blocked
            } else {
                Outcome::Fail(format!(
                    "direnv exited {}: {}",
                    output.status,
                    stderr.trim()
                ))
            }
        }
    }

    fn run_export(
        &self,
        direnv: &Path,
        workspace_root: &Path,
        clear_direnv: bool,
    ) -> std::io::Result<Output> {
        let mut cmd = Command::new(direnv);
        cmd.arg("export")
            .arg("json")
            // The allowed `.envrc` lives at `workspace_root`, never the temp
            // worktree (which is not direnv-allowed).
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match &self.base {
            BaseEnv::Inherit => {}
            #[cfg(test)]
            BaseEnv::Explicit(vars) => {
                cmd.env_clear();
                for (k, v) in vars {
                    cmd.env(k, v);
                }
            }
        }
        // Secondary nicety only — empirically (F4) it does NOT silence
        // devenv's bootstrap stderr. Capture purity comes from piping stderr
        // (above), not from this.
        cmd.env("DIRENV_LOG_FORMAT", "");
        if clear_direnv {
            for key in ["DIRENV_DIR", "DIRENV_FILE", "DIRENV_DIFF", "DIRENV_WATCHES"] {
                cmd.env_remove(key);
            }
        }
        cmd.output()
    }
}

/// The set of environment variables git treats as repo-local, derived once
/// per process from `git rev-parse --local-env-vars`. Falls back to a static
/// set (matching git's documented family) when the invocation fails, so the
/// strip in [`apply_repo_env`] still happens.
fn git_local_env_vars() -> &'static HashSet<String> {
    static VARS: OnceLock<HashSet<String>> = OnceLock::new();
    VARS.get_or_init(|| {
        match Command::new("git")
            .arg("rev-parse")
            .arg("--local-env-vars")
            .output()
        {
            Ok(out) if out.status.success() => {
                let set: HashSet<String> = String::from_utf8_lossy(&out.stdout)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                if set.is_empty() {
                    fallback_git_local_env_vars()
                } else {
                    set
                }
            }
            _ => fallback_git_local_env_vars(),
        }
    })
}

/// Static fallback for [`git_local_env_vars`] — the repo-local family git
/// documents. Used only when `git rev-parse --local-env-vars` is unavailable.
fn fallback_git_local_env_vars() -> HashSet<String> {
    [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Locate `bin` on jj-hp's own PATH. Mirrors `runner::which`.
fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
static BLOCKED_SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static WARN_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Test-only: seed the process-global cache directly, so spawn-site tests
/// (`hooks.rs`, `setup.rs`) can assert `apply_repo_env`'s effect on a real
/// `Command` without running an export.
#[cfg(test)]
pub(crate) fn test_seed(workspace_root: &Path, patch: EnvPatch) {
    cache()
        .lock()
        .unwrap()
        .insert(cache_key(workspace_root), Arc::new(patch));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write an executable fake `direnv` (a POSIX shell script) into `dir`.
    fn write_fake_direnv(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("direnv");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// A workspace tempdir carrying an `.envrc`.
    fn ws_with_envrc() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(".envrc"), "export FOO=bar\n").unwrap();
        dir
    }

    /// An exporter over a fake direnv, with a minimal explicit base env so the
    /// shell script can still find `/bin`/`/usr/bin` builtins after the
    /// `env_clear`. Extra vars (e.g. `DIRENV_DIFF`) are appended.
    fn fake_exporter(direnv: PathBuf, extra: &[(&str, &str)]) -> DirenvExporter {
        let mut base = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
        for (k, v) in extra {
            base.push((k.to_string(), v.to_string()));
        }
        DirenvExporter {
            direnv: Some(direnv),
            base: BaseEnv::Explicit(base),
        }
    }

    #[test]
    fn patch_carries_set_and_unset_entries() {
        let bin = tempfile::TempDir::new().unwrap();
        let direnv = write_fake_direnv(
            bin.path(),
            "#!/bin/sh\nprintf '%s' '{\"BIOME_PIN\":\"devenv\",\"DROP_ME\":null,\"PATH\":\"/devenv/bin:/usr/bin\"}'\n",
        );
        let ws = ws_with_envrc();
        let patch = compute(ws.path(), true, &fake_exporter(direnv, &[]));
        let EnvPatch::Patch(map) = patch else {
            panic!("expected Patch, got {patch:?}");
        };
        assert_eq!(map.get("BIOME_PIN"), Some(&Some("devenv".to_string())));
        assert_eq!(map.get("DROP_ME"), Some(&None));
        assert_eq!(
            map.get("PATH"),
            Some(&Some("/devenv/bin:/usr/bin".to_string()))
        );
    }

    #[test]
    fn disabled_without_envrc() {
        let bin = tempfile::TempDir::new().unwrap();
        let direnv = write_fake_direnv(bin.path(), "#!/bin/sh\nprintf '%s' '{}'\n");
        let ws = tempfile::TempDir::new().unwrap(); // no .envrc
        assert_eq!(
            compute(ws.path(), true, &fake_exporter(direnv, &[])),
            EnvPatch::Disabled
        );
    }

    #[test]
    fn disabled_when_direnv_not_on_path() {
        let ws = ws_with_envrc();
        let exporter = DirenvExporter {
            direnv: None, // not found on PATH
            base: BaseEnv::Explicit(vec![]),
        };
        assert_eq!(compute(ws.path(), true, &exporter), EnvPatch::Disabled);
    }

    #[test]
    fn empty_stdout_success_is_empty_patch_without_warning() {
        let before = WARN_COUNT.load(Ordering::SeqCst);
        let bin = tempfile::TempDir::new().unwrap();
        // Exit 0, no stdout: the everyday "env already loaded" case.
        let direnv = write_fake_direnv(bin.path(), "#!/bin/sh\nexit 0\n");
        let ws = ws_with_envrc();
        assert_eq!(
            compute(ws.path(), true, &fake_exporter(direnv, &[])),
            EnvPatch::Patch(HashMap::new())
        );
        assert_eq!(
            WARN_COUNT.load(Ordering::SeqCst),
            before,
            "empty-stdout success must not warn"
        );
    }

    #[test]
    fn blocked_envrc_disables_and_signals() {
        let before = BLOCKED_SIGNAL_COUNT.load(Ordering::SeqCst);
        let bin = tempfile::TempDir::new().unwrap();
        // direnv's real blocked signature on stderr, non-zero exit.
        let direnv = write_fake_direnv(
            bin.path(),
            "#!/bin/sh\necho 'direnv: error /x/.envrc is blocked. Run `direnv allow` to approve its content' 1>&2\nexit 1\n",
        );
        let ws = ws_with_envrc();
        assert_eq!(
            compute(ws.path(), true, &fake_exporter(direnv, &[])),
            EnvPatch::Disabled
        );
        assert_eq!(
            BLOCKED_SIGNAL_COUNT.load(Ordering::SeqCst),
            before + 1,
            "blocked `.envrc` must emit the one visible signal"
        );
    }

    #[test]
    fn recovers_via_cleared_direnv_retry() {
        let bin = tempfile::TempDir::new().unwrap();
        // Fails (non-blocked) while DIRENV_DIFF is present; succeeds once the
        // retry strips it. Mirrors F3.
        let direnv = write_fake_direnv(
            bin.path(),
            "#!/bin/sh\nif [ -n \"$DIRENV_DIFF\" ]; then echo 'stale diff' 1>&2; exit 1; fi\nprintf '%s' '{\"OK\":\"yes\"}'\n",
        );
        let ws = ws_with_envrc();
        let patch = compute(
            ws.path(),
            true,
            &fake_exporter(direnv, &[("DIRENV_DIFF", "corrupt")]),
        );
        let EnvPatch::Patch(map) = patch else {
            panic!("expected Patch after retry, got {patch:?}");
        };
        assert_eq!(map.get("OK"), Some(&Some("yes".to_string())));
    }

    #[test]
    fn first_attempt_inherits_direnv_diff() {
        let bin = tempfile::TempDir::new().unwrap();
        let log = tempfile::TempDir::new().unwrap();
        let log_path = log.path().join("diff.log");
        // Record whether DIRENV_DIFF was visible, then succeed (no retry).
        let body = format!(
            "#!/bin/sh\nif [ -n \"$DIRENV_DIFF\" ]; then echo diff=1 >> '{p}'; else echo diff=0 >> '{p}'; fi\nprintf '%s' '{{\"FOO\":\"bar\"}}'\n",
            p = log_path.display()
        );
        let direnv = write_fake_direnv(bin.path(), &body);
        let ws = ws_with_envrc();
        let patch = compute(
            ws.path(),
            true,
            &fake_exporter(direnv, &[("DIRENV_DIFF", "inherited")]),
        );
        assert!(matches!(patch, EnvPatch::Patch(_)));
        let recorded = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(
            recorded.lines().next(),
            Some("diff=1"),
            "the happy path must inherit DIRENV_DIFF on the FIRST attempt, never sanitize it"
        );
    }

    #[test]
    fn export_stderr_chatter_does_not_fail_the_outcome() {
        let bin = tempfile::TempDir::new().unwrap();
        // Chatter on the export's own stderr must not turn a successful export
        // into a Fail. That the chatter also never reaches a hook-capture
        // buffer is a STRUCTURAL guarantee, not asserted here: `run_export`
        // pipes stderr (Stdio::piped) and routes it only to `tracing`, and
        // `compute`/`run_export` take no capture-buffer parameter, so there is
        // no channel from export stderr into the failure buffer to test.
        let direnv = write_fake_direnv(
            bin.path(),
            "#!/bin/sh\necho 'SECRET-DIRENV-CHATTER' 1>&2\nprintf '%s' '{\"A\":\"1\"}'\n",
        );
        let ws = ws_with_envrc();
        let patch = compute(ws.path(), true, &fake_exporter(direnv, &[]));
        let EnvPatch::Patch(map) = patch else {
            panic!("expected Patch, got {patch:?}");
        };
        assert_eq!(map.get("A"), Some(&Some("1".to_string())));
    }

    #[test]
    fn cache_computes_once_per_root() {
        let bin = tempfile::TempDir::new().unwrap();
        let counter_dir = tempfile::TempDir::new().unwrap();
        let counter = counter_dir.path().join("count");
        let body = format!(
            "#!/bin/sh\necho x >> '{p}'\nprintf '%s' '{{\"A\":\"1\"}}'\n",
            p = counter.display()
        );
        let direnv = write_fake_direnv(bin.path(), &body);
        let ws = ws_with_envrc();

        repo_env_with(ws.path(), true, &fake_exporter(direnv.clone(), &[]));
        repo_env_with(ws.path(), true, &fake_exporter(direnv.clone(), &[]));
        let lines = std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(lines, 1, "same root must export exactly once");

        let ws2 = ws_with_envrc();
        repo_env_with(ws2.path(), true, &fake_exporter(direnv, &[]));
        let lines = std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(lines, 2, "a distinct root exports again");
    }

    #[test]
    fn disabled_when_opted_out_even_with_envrc_and_direnv() {
        let bin = tempfile::TempDir::new().unwrap();
        let direnv = write_fake_direnv(bin.path(), "#!/bin/sh\nprintf '%s' '{\"A\":\"1\"}'\n");
        let ws = ws_with_envrc();
        assert_eq!(
            compute(
                ws.path(),
                /*enabled=*/ false,
                &fake_exporter(direnv, &[])
            ),
            EnvPatch::Disabled,
            "enabled=false must win over a present .envrc + direnv"
        );
    }

    #[test]
    fn enabled_from_precedence() {
        use std::ffi::OsStr;
        // Env-var opt-out wins.
        assert!(!enabled_from(Some(OsStr::new("1")), None));
        assert!(!enabled_from(Some(OsStr::new("anything")), Some("auto")));
        // Empty env value does NOT disable.
        assert!(enabled_from(Some(OsStr::new("")), None));
        // Config "off" disables (case/space-insensitive).
        assert!(!enabled_from(None, Some("off")));
        assert!(!enabled_from(None, Some("  OFF\n")));
        // Unset / "auto" / anything else enables.
        assert!(enabled_from(None, None));
        assert!(enabled_from(None, Some("auto")));
    }

    #[test]
    fn apply_sets_and_removes_vars() {
        let ws = tempfile::TempDir::new().unwrap();
        test_seed(
            ws.path(),
            EnvPatch::Patch(HashMap::from([
                ("BIOME_PIN".to_string(), Some("devenv".to_string())),
                ("DROP_ME".to_string(), None),
            ])),
        );
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf 'PIN=[%s] DROP=[%s]' \"${BIOME_PIN:-unset}\" \"${DROP_ME:-unset}\"")
            // Pre-set DROP_ME so the None removal is observable.
            .env("DROP_ME", "present")
            .stdout(Stdio::piped());
        apply_repo_env(&mut cmd, ws.path());
        let out = cmd.output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "PIN=[devenv] DROP=[unset]"
        );
    }

    #[test]
    fn apply_strips_git_local_env_family() {
        let ws = tempfile::TempDir::new().unwrap();
        test_seed(
            ws.path(),
            EnvPatch::Patch(HashMap::from([
                (
                    "GIT_DIR".to_string(),
                    Some("/bogus/primary/.git".to_string()),
                ),
                (
                    "GIT_INDEX_FILE".to_string(),
                    Some("/bogus/primary/.git/index".to_string()),
                ),
                ("KEEP_ME".to_string(), Some("yes".to_string())),
            ])),
        );
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf 'GD=[%s] GIF=[%s] KEEP=[%s]' \"${GIT_DIR:-unset}\" \"${GIT_INDEX_FILE:-unset}\" \"${KEEP_ME:-unset}\"")
            .stdout(Stdio::piped());
        apply_repo_env(&mut cmd, ws.path());
        let out = cmd.output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "GD=[unset] GIF=[unset] KEEP=[yes]",
            "git repo-location vars must be stripped; other vars kept"
        );
    }

    #[test]
    fn apply_strips_inherited_git_local_env_with_empty_patch() {
        // The loaded-shell case: a secondary workspace whose already-loaded
        // `.envrc.local` set GIT_DIR yields an EMPTY export diff, so GIT_DIR is
        // absent from the patch yet still inherited from jj-hp's own env. The
        // strip must be unconditional over the child, not patch-scoped, or the
        // hook child's git resolves HEAD/index against the primary workspace.
        let ws = tempfile::TempDir::new().unwrap();
        test_seed(ws.path(), EnvPatch::Patch(HashMap::new()));
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf 'GD=[%s] GIF=[%s]' \"${GIT_DIR:-unset}\" \"${GIT_INDEX_FILE:-unset}\"")
            // Inherited from the parent env, NOT from the patch.
            .env("GIT_DIR", "/bogus/primary/.git")
            .env("GIT_INDEX_FILE", "/bogus/primary/.git/index")
            .stdout(Stdio::piped());
        apply_repo_env(&mut cmd, ws.path());
        let out = cmd.output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "GD=[unset] GIF=[unset]",
            "inherited git repo-location vars must be stripped even with an empty patch"
        );
    }

    #[test]
    fn apply_strips_git_local_but_keeps_other_env_for_missing_cache_entry() {
        // A missing cache entry (non-batch path, or a workspace never computed)
        // must still strip the git repo-location family — the leak reaches the
        // child by inheritance from jj-hp's env, independent of any patch — but
        // must leave every non-git var untouched.
        let ws = tempfile::TempDir::new().unwrap(); // never seeded
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf 'GD=[%s] KEEP=[%s]' \"${GIT_DIR:-unset}\" \"${SOME_VAR:-unset}\"")
            .env("GIT_DIR", "/bogus/primary/.git")
            .env("SOME_VAR", "kept")
            .stdout(Stdio::piped());
        apply_repo_env(&mut cmd, ws.path());
        let out = cmd.output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "GD=[unset] KEEP=[kept]",
            "missing cache entry: git repo-location vars stripped, other vars kept"
        );
    }

    #[test]
    fn apply_strips_git_local_env_for_disabled_patch() {
        // The non-direnv case (issue #292): a repo with no `.envrc` / no direnv
        // has an EnvPatch::Disabled entry, yet a push from a linked worktree
        // still leaks GIT_DIR into the hook child by inheritance. The strip is
        // a safety invariant, not a devenv convenience, so it must run on the
        // Disabled arm too.
        let ws = tempfile::TempDir::new().unwrap();
        test_seed(ws.path(), EnvPatch::Disabled);
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf 'GD=[%s] GCP=[%s] KEEP=[%s]' \"${GIT_DIR:-unset}\" \"${GIT_CONFIG_PARAMETERS:-unset}\" \"${SOME_VAR:-unset}\"")
            .env("GIT_DIR", "/bogus/primary/.git")
            .env("GIT_CONFIG_PARAMETERS", "'core.hooksPath=/tmp/evil'")
            .env("SOME_VAR", "kept")
            .stdout(Stdio::piped());
        apply_repo_env(&mut cmd, ws.path());
        let out = cmd.output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "GD=[unset] GCP=[unset] KEEP=[kept]",
            "disabled patch (non-direnv repo): git repo-location vars still stripped"
        );
    }
}

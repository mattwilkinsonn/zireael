//! Cooperative process-level lock for `jj-gt fetch` (and other
//! ref-mutating pipelines that touch git refs concurrently with jj's
//! view).
//!
//! Closes part of issue #1: two concurrent `jj-gt fetch` invocations
//! against the same workspace can interleave their `gt sync` →
//! `jj git import` → `jj rebase` steps and produce divergent commits
//! when one of them snapshots the working copy mid-pipeline.
//!
//! Scope of this lock:
//!
//! - Blocks: another `jj-gt fetch` / `jj-gt submit` (anything that
//!   takes the same lock path).
//! - Does NOT block: plain `jj` operations in another shell, VisualJJ
//!   polling, agents using `jj` directly. Those don't take this lock.
//!   The companion uncommitted-changes refusal handles the
//!   "snapshot mid-edit" hazard that plain `jj` ops can introduce.
//!
//! The lock is held for the lifetime of the [`PipelineLock`] struct.
//! Drop releases it. POSIX `flock`-style advisory locks are released
//! by the kernel on process exit even if drop doesn't run.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{JjGtError, Result};

/// RAII handle for the pipeline lock. Drop unlocks.
///
/// The underlying lock is an OS file lock (`std::fs::File::lock` —
/// `LOCK_EX` on Unix, `LockFileEx` on Windows) on `.jj/jj-gt.lock`
/// inside the workspace root. The file is created on first acquire;
/// we don't delete it on release because creating + deleting the
/// lock file every invocation races with concurrent acquirers (a
/// second process could delete the file between a first process's
/// open and lock).
pub struct PipelineLock {
    _file: File,
    path: PathBuf,
}

impl std::fmt::Debug for PipelineLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineLock")
            .field("path", &self.path)
            .finish()
    }
}

impl PipelineLock {
    /// Acquire the lock at `<workspace_root>/.jj/jj-gt.lock`,
    /// blocking until it's available.
    ///
    /// Errors when the lock file can't be created (e.g. workspace
    /// isn't a jj repo — no `.jj/` directory) or when the OS lock
    /// call fails for a reason other than contention (rare;
    /// typically a permission issue on the lock file).
    pub fn acquire(workspace_root: &Path) -> Result<Self> {
        let jj_dir = workspace_root.join(".jj");
        if !jj_dir.is_dir() {
            return Err(JjGtError::Invalid(format!(
                "no .jj directory at {} — not a jj workspace",
                workspace_root.display()
            )));
        }
        let path = jj_dir.join("jj-gt.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // Try non-blocking first; if it would block, fall back to a
        // blocking lock with a one-line warning so the user knows
        // why their fetch is sitting there. Without the warning the
        // CLI just appears to hang.
        if file.try_lock().is_err() {
            tracing::warn!(
                "jj-gt: another jj-gt pipeline is in flight on this workspace; waiting on lock {}",
                path.display(),
            );
            file.lock()?;
        }

        Ok(Self { _file: file, path })
    }
}

impl Drop for PipelineLock {
    fn drop(&mut self) {
        tracing::debug!("releasing jj-gt pipeline lock at {}", self.path.display());
        // File::drop releases the lock automatically.
    }
}

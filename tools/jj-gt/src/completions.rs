//! Shell completion support.
//!
//! Two roles:
//! 1. `jj-gt completions <shell>` emits a clap_complete-generated
//!    completion script for the user to source.
//! 2. Dynamic value completers attached to specific flags (e.g.
//!    `--bookmark`) shell out to `jj` to enumerate live values.
//!
//! All `jj` invocations use `--ignore-working-copy` so completion
//! doesn't snapshot the user's working copy on every TAB keystroke.

use std::path::Path;
use std::process::Command;

/// Enumerate local bookmark names from the jj repo rooted at `cwd`.
/// Returns an empty list (not an error) when we're not inside a jj
/// repo so completion degrades silently.
pub fn complete_bookmarks(cwd: &Path) -> Vec<String> {
    let out = Command::new("jj")
        .args([
            "bookmark",
            "list",
            "-T",
            "name ++ \"\\n\"",
            "--ignore-working-copy",
        ])
        .current_dir(cwd)
        .output();
    match out {
        Ok(o) if o.status.success() => parse_lines(&o.stdout),
        _ => Vec::new(),
    }
}

/// Enumerate git remote names from the jj repo rooted at `cwd`.
pub fn complete_remotes(cwd: &Path) -> Vec<String> {
    let out = Command::new("jj")
        .args(["git", "remote", "list", "--ignore-working-copy"])
        .current_dir(cwd)
        .output();
    let bytes = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(|s| s.to_owned()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

/// `clap_complete`-compatible value completer for `-b / --bookmark`.
pub fn bookmark_value_completer(
    current: &std::ffi::OsStr,
) -> Vec<clap_complete::CompletionCandidate> {
    let prefix = current.to_string_lossy();
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    complete_bookmarks(&cwd)
        .into_iter()
        .filter(|name| name.starts_with(prefix.as_ref()))
        .map(clap_complete::CompletionCandidate::new)
        .collect()
}

/// `clap_complete`-compatible value completer for `--remote`.
pub fn remote_value_completer(
    current: &std::ffi::OsStr,
) -> Vec<clap_complete::CompletionCandidate> {
    let prefix = current.to_string_lossy();
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    complete_remotes(&cwd)
        .into_iter()
        .filter(|name| name.starts_with(prefix.as_ref()))
        .map(clap_complete::CompletionCandidate::new)
        .collect()
}

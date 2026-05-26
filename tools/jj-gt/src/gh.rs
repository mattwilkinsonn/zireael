//! Subprocess wrappers for the `gh` (GitHub) CLI.
//!
//! Used by `jj-gt fetch` (SHA-drift detection, gtmq-prune classification,
//! orphan-bookmark cleanup) and `jj-gt status` (PR state for the stack).
//! All `gh` invocations request structured JSON output to avoid parsing
//! gh's human-readable tables.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::{JjGtError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
    /// gh sometimes returns a state we don't know about (Graphite-injected
    /// label, future GitHub API value). Treat as unknown — callers should
    /// be conservative.
    Unknown,
}

impl PrState {
    fn parse(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            _ => PrState::Unknown,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, PrState::Closed | PrState::Merged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u32,
    pub head_ref_name: String,
    pub head_ref_oid: String,
    pub state: PrState,
    pub is_draft: bool,
    pub merge_state_status: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PrJson {
    number: u32,
    #[serde(default, rename = "headRefName")]
    head_ref_name: String,
    #[serde(default, rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(default)]
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(default, rename = "mergeStateStatus")]
    merge_state_status: Option<String>,
    #[serde(default)]
    labels: Vec<LabelJson>,
}

#[derive(Debug, Deserialize)]
struct LabelJson {
    #[serde(default)]
    name: String,
}

impl From<PrJson> for PrInfo {
    fn from(j: PrJson) -> Self {
        PrInfo {
            number: j.number,
            head_ref_name: j.head_ref_name,
            head_ref_oid: j.head_ref_oid,
            state: PrState::parse(&j.state),
            is_draft: j.is_draft,
            merge_state_status: j.merge_state_status,
            labels: j.labels.into_iter().map(|l| l.name).collect(),
        }
    }
}

/// `gh pr list --head <branch> --state all
///             --json number,headRefName,headRefOid,state,isDraft,mergeStateStatus,labels
///             --limit 1`
///
/// Returns the most recent matching PR for the branch, or `None` if none
/// exist. Multiple PRs against the same head branch is unusual; gh
/// returns them newest-first, which is what we want.
pub fn find_pr_for_branch(workspace_root: &Path, branch: &str) -> Result<Option<PrInfo>> {
    let raw = gh_capture(
        workspace_root,
        &[
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all",
            "--json",
            "number,headRefName,headRefOid,state,isDraft,mergeStateStatus,labels",
            "--limit",
            "1",
        ],
    )?;
    let parsed: Vec<PrJson> = serde_json::from_str(&raw)?;
    Ok(parsed.into_iter().next().map(PrInfo::from))
}

/// `gh pr list --search 'head:<b1> OR head:<b2> ...' --state all
///             --json number,headRefName,headRefOid,state,isDraft,mergeStateStatus,labels
///             --limit <max>`
///
/// One batched call per status invocation; cheap, no caching needed at
/// v0. Returns every PR whose head branch matches any of the inputs;
/// callers can join the result against their bookmark list.
pub fn find_prs_for_branches(
    workspace_root: &Path,
    branches: &[String],
    limit: u32,
) -> Result<Vec<PrInfo>> {
    if branches.is_empty() {
        return Ok(Vec::new());
    }
    let search = branches
        .iter()
        .map(|b| format!("head:{b}"))
        .collect::<Vec<_>>()
        .join(" ");
    let limit_str = limit.to_string();
    let raw = gh_capture(
        workspace_root,
        &[
            "pr",
            "list",
            "--search",
            &search,
            "--state",
            "all",
            "--json",
            "number,headRefName,headRefOid,state,isDraft,mergeStateStatus,labels",
            "--limit",
            &limit_str,
        ],
    )?;
    let parsed: Vec<PrJson> = serde_json::from_str(&raw)?;
    Ok(parsed.into_iter().map(PrInfo::from).collect())
}

/// `gh pr list --search 'head:<prefix>' --state all
///             --json number,headRefName,headRefOid,state ...`
///
/// Batched lookup of every PR whose head branch starts with any of the
/// given prefixes. Used for gtmq_* pruning in `jj-gt fetch`.
pub fn list_prs_by_head_prefix(
    workspace_root: &Path,
    prefixes: &[String],
    limit: u32,
) -> Result<Vec<PrInfo>> {
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }
    let search = prefixes
        .iter()
        .map(|p| format!("head:{p}"))
        .collect::<Vec<_>>()
        .join(" ");
    let limit_str = limit.to_string();
    let raw = gh_capture(
        workspace_root,
        &[
            "pr",
            "list",
            "--search",
            &search,
            "--state",
            "all",
            "--json",
            "number,headRefName,headRefOid,state,isDraft,mergeStateStatus,labels",
            "--limit",
            &limit_str,
        ],
    )?;
    let parsed: Vec<PrJson> = serde_json::from_str(&raw)?;
    Ok(parsed.into_iter().map(PrInfo::from).collect())
}

fn gh_capture(cwd: &Path, args: &[&str]) -> Result<String> {
    tracing::info!("running: gh {:?}", args);
    let out = Command::new("gh")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| JjGtError::GhFailed {
            status: -1,
            stderr: format!("failed to spawn gh: {e}"),
        })?;
    if !out.status.success() {
        return Err(JjGtError::GhFailed {
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_state_known_values() {
        assert_eq!(PrState::parse("OPEN"), PrState::Open);
        assert_eq!(PrState::parse("open"), PrState::Open);
        assert_eq!(PrState::parse("Closed"), PrState::Closed);
        assert_eq!(PrState::parse("MERGED"), PrState::Merged);
    }

    #[test]
    fn parse_pr_state_unknown_falls_through() {
        assert_eq!(PrState::parse("queued"), PrState::Unknown);
        assert_eq!(PrState::parse(""), PrState::Unknown);
    }

    #[test]
    fn pr_state_terminality() {
        assert!(!PrState::Open.is_terminal());
        assert!(PrState::Closed.is_terminal());
        assert!(PrState::Merged.is_terminal());
        assert!(!PrState::Unknown.is_terminal());
    }

    #[test]
    fn pr_json_parses_minimal_shape() {
        let raw = r#"[
            {
                "number": 321,
                "headRefName": "sea-557-fix-gt--athena",
                "headRefOid": "abc123",
                "state": "OPEN",
                "isDraft": false,
                "mergeStateStatus": "CLEAN",
                "labels": [{"name": "review"}, {"name": "stack"}]
            }
        ]"#;
        let parsed: Vec<PrJson> = serde_json::from_str(raw).unwrap();
        let info: Vec<PrInfo> = parsed.into_iter().map(PrInfo::from).collect();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].number, 321);
        assert_eq!(info[0].head_ref_name, "sea-557-fix-gt--athena");
        assert_eq!(info[0].state, PrState::Open);
        assert!(!info[0].is_draft);
        assert_eq!(info[0].labels, vec!["review", "stack"]);
    }

    #[test]
    fn pr_json_handles_missing_optional_fields() {
        let raw = r#"[
            {"number": 7, "state": "MERGED"}
        ]"#;
        let parsed: Vec<PrJson> = serde_json::from_str(raw).unwrap();
        let info: Vec<PrInfo> = parsed.into_iter().map(PrInfo::from).collect();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].number, 7);
        assert_eq!(info[0].state, PrState::Merged);
        assert!(info[0].labels.is_empty());
        assert!(info[0].merge_state_status.is_none());
    }
}

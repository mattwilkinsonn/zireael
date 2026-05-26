//! `jj-gt status` — stack-wide PR + queue state renderer.

use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::gh::{self, PrInfo, PrState};
use crate::jj::JjCli;
use crate::stack::StackedBookmark;

#[derive(Debug, Clone, Serialize)]
pub struct StatusRow {
    pub bookmark: String,
    pub parent: String,
    pub pr_number: Option<u32>,
    pub state: &'static str,
    pub queue: String,
    pub drift: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutput {
    pub trunk: String,
    pub stack: Vec<StatusRow>,
}

/// Build the status output for a derived stack. Cross-references each
/// bookmark with at most one PR from `prs` (matched on `head_ref_name`).
/// Drift is signalled when the bookmark's local commit id differs from
/// the PR's `headRefOid` — see SHA-drift policy in the design doc.
pub fn build(
    trunk: &str,
    stacked: &[StackedBookmark],
    local_commits: &[(String, String)], // (bookmark name, local short commit id)
    prs: &[PrInfo],
) -> StatusOutput {
    let mut rows = Vec::with_capacity(stacked.len());
    for sb in stacked {
        let parent = sb.parent.as_branch_name(trunk).to_owned();
        let local_commit = local_commits
            .iter()
            .find(|(name, _)| name == &sb.name)
            .map(|(_, c)| c.as_str());
        let pr = prs.iter().find(|p| p.head_ref_name == sb.name);

        let (pr_number, state, queue, drift) = match pr {
            None => (None, "(no PR)", "—".to_owned(), false),
            Some(pr) => {
                let state = pr_state_label(pr);
                let queue = queue_label(pr);
                let drift = match local_commit {
                    Some(local) if !pr.head_ref_oid.is_empty() => {
                        !pr.head_ref_oid.starts_with(local) && !local.starts_with(&pr.head_ref_oid)
                    }
                    _ => false,
                };
                (Some(pr.number), state, queue, drift)
            }
        };

        rows.push(StatusRow {
            bookmark: sb.name.clone(),
            parent,
            pr_number,
            state,
            queue,
            drift,
        });
    }
    StatusOutput {
        trunk: trunk.to_owned(),
        stack: rows,
    }
}

fn pr_state_label(pr: &PrInfo) -> &'static str {
    match pr.state {
        PrState::Open if pr.is_draft => "draft",
        PrState::Open => "ready",
        PrState::Closed => "closed",
        PrState::Merged => "merged",
        PrState::Unknown => "?",
    }
}

fn queue_label(pr: &PrInfo) -> String {
    // Best-effort heuristics — we don't talk to Graphite's API
    // directly. Look for the merge-when-ready signal first, then any
    // graphite-injected label like `gt:queued`, `merge-queue`, or
    // `merge-when-ready`. Anything else lands as `—`.
    let labels_lc: Vec<String> = pr.labels.iter().map(|l| l.to_ascii_lowercase()).collect();
    let queued = labels_lc.iter().any(|l| {
        l == "merge-queue" || l == "gt:queued" || l == "graphite:queued" || l.contains("queue")
    });
    let mwr = labels_lc
        .iter()
        .any(|l| l == "merge-when-ready" || l == "gt:merge-when-ready" || l.contains("merge-when"));
    let mwr = mwr
        || pr.merge_state_status.as_deref().is_some_and(|s| {
            s.eq_ignore_ascii_case("BLOCKED") && labels_lc.iter().any(|l| l.contains("ready"))
        });

    if queued {
        "queued (?)".into()
    } else if mwr {
        "merge-when-ready".into()
    } else {
        "—".into()
    }
}

/// Render a human-friendly table for terminal output.
pub fn render_table(out: &StatusOutput) -> String {
    let mut s = String::new();
    s.push_str(&format!("trunk: {}\n", out.trunk));
    s.push_str("stack (top → bottom):\n");
    for row in out.stack.iter().rev() {
        let pr = match row.pr_number {
            Some(n) => format!("PR #{n}"),
            None => "(no PR — not submitted)".into(),
        };
        let drift = if row.drift { "  ⚠ drift" } else { "" };
        s.push_str(&format!(
            "  ● {:<32}  {:<10}  {:<6}  {}{}\n",
            row.bookmark, pr, row.state, row.queue, drift
        ));
    }
    s
}

/// Render the JSON variant.
pub fn render_json(out: &StatusOutput) -> Result<String> {
    Ok(serde_json::to_string_pretty(out)?)
}

/// Hydrate the (bookmark, local short commit id) pairs jj-gt needs
/// for drift detection. Uses `jj log` once per bookmark — typical
/// stacks are short enough that this stays cheap.
pub fn collect_local_commits(jj: &JjCli, bookmarks: &[String]) -> Result<Vec<(String, String)>> {
    use crate::jj as jjmod;
    let all = jjmod::list_local_bookmarks(jj)?;
    let mut out = Vec::new();
    for b in bookmarks {
        if let Some(info) = all.iter().find(|x| &x.name == b) {
            out.push((b.clone(), info.commit_id.clone()));
        }
    }
    Ok(out)
}

/// Resolve trunk: prefer `--trunk` CLI value, fall back to gt's repo
/// config, then to `"main"`.
pub fn resolve_trunk(workspace_root: &Path, cli_value: Option<&str>) -> Result<String> {
    if let Some(t) = cli_value {
        return Ok(t.to_owned());
    }
    if let Some(t) = crate::gt::read_repo_config_trunk(workspace_root)? {
        return Ok(t);
    }
    Ok("main".to_owned())
}

/// Fetch PR info for `branches` in a single batched `gh pr list` call.
pub fn fetch_pr_info(workspace_root: &Path, branches: &[String]) -> Result<Vec<PrInfo>> {
    gh::find_prs_for_branches(workspace_root, branches, 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::PrInfo;
    use crate::stack::{BookmarkOrTrunk, StackedBookmark};

    fn pr(
        number: u32,
        branch: &str,
        head_oid: &str,
        state: PrState,
        is_draft: bool,
        labels: &[&str],
    ) -> PrInfo {
        PrInfo {
            number,
            head_ref_name: branch.into(),
            head_ref_oid: head_oid.into(),
            state,
            is_draft,
            merge_state_status: None,
            labels: labels.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn sb(name: &str, parent: BookmarkOrTrunk) -> StackedBookmark {
        StackedBookmark {
            name: name.into(),
            parent,
        }
    }

    #[test]
    fn build_basic_three_stack_no_drift() {
        let stack = vec![
            sb("bottom", BookmarkOrTrunk::Trunk),
            sb("mid", BookmarkOrTrunk::Bookmark("bottom".into())),
            sb("top", BookmarkOrTrunk::Bookmark("mid".into())),
        ];
        let locals = vec![
            ("bottom".into(), "aaa111".into()),
            ("mid".into(), "bbb222".into()),
            ("top".into(), "ccc333".into()),
        ];
        let prs = vec![
            pr(101, "bottom", "aaa111", PrState::Open, false, &[]),
            pr(102, "mid", "bbb222", PrState::Open, true, &[]),
            pr(
                103,
                "top",
                "ccc333",
                PrState::Open,
                false,
                &["merge-when-ready"],
            ),
        ];
        let out = build("main", &stack, &locals, &prs);

        assert_eq!(out.trunk, "main");
        assert_eq!(out.stack.len(), 3);
        assert_eq!(out.stack[0].bookmark, "bottom");
        assert_eq!(out.stack[0].pr_number, Some(101));
        assert_eq!(out.stack[0].state, "ready");
        assert!(!out.stack[0].drift);

        assert_eq!(out.stack[1].state, "draft");
        assert_eq!(out.stack[2].queue, "merge-when-ready");
    }

    #[test]
    fn build_no_pr_row() {
        let stack = vec![sb("solo", BookmarkOrTrunk::Trunk)];
        let out = build("main", &stack, &[], &[]);
        assert_eq!(out.stack[0].pr_number, None);
        assert_eq!(out.stack[0].state, "(no PR)");
        assert_eq!(out.stack[0].queue, "—");
        assert!(!out.stack[0].drift);
    }

    #[test]
    fn build_detects_drift() {
        let stack = vec![sb("b", BookmarkOrTrunk::Trunk)];
        let locals = vec![("b".into(), "aaaa".into())];
        // PR head is a totally different SHA → drift.
        let prs = vec![pr(1, "b", "ffff", PrState::Open, false, &[])];
        let out = build("main", &stack, &locals, &prs);
        assert!(out.stack[0].drift);
    }

    #[test]
    fn build_drift_tolerates_shared_prefix() {
        // Local id is a 12-char prefix; PR returns the full 40-char OID.
        // They should be treated as the same commit.
        let stack = vec![sb("b", BookmarkOrTrunk::Trunk)];
        let locals = vec![("b".into(), "abcdef123456".into())];
        let prs = vec![pr(
            1,
            "b",
            "abcdef123456789012345678901234567890abcd",
            PrState::Open,
            false,
            &[],
        )];
        let out = build("main", &stack, &locals, &prs);
        assert!(!out.stack[0].drift);
    }

    #[test]
    fn build_queue_queued_label() {
        let stack = vec![sb("b", BookmarkOrTrunk::Trunk)];
        let locals = vec![("b".into(), "x".into())];
        let prs = vec![pr(1, "b", "x", PrState::Open, false, &["gt:queued"])];
        let out = build("main", &stack, &locals, &prs);
        assert_eq!(out.stack[0].queue, "queued (?)");
    }

    #[test]
    fn pr_state_label_covers_states() {
        let mut p = pr(1, "b", "x", PrState::Open, false, &[]);
        assert_eq!(pr_state_label(&p), "ready");
        p.is_draft = true;
        assert_eq!(pr_state_label(&p), "draft");
        p.state = PrState::Closed;
        assert_eq!(pr_state_label(&p), "closed");
        p.state = PrState::Merged;
        assert_eq!(pr_state_label(&p), "merged");
    }

    #[test]
    fn render_table_smoke() {
        let stack = vec![sb("solo", BookmarkOrTrunk::Trunk)];
        let out = build("main", &stack, &[], &[]);
        let s = render_table(&out);
        assert!(s.contains("trunk: main"));
        assert!(s.contains("solo"));
        assert!(s.contains("(no PR — not submitted)"));
    }

    #[test]
    fn render_json_smoke() {
        let stack = vec![sb("solo", BookmarkOrTrunk::Trunk)];
        let out = build("main", &stack, &[], &[]);
        let s = render_json(&out).unwrap();
        assert!(s.contains("\"trunk\""));
        assert!(s.contains("\"bookmark\""));
    }
}

---
name: wave-status-sync
description: "Bring a multi-agent wave up to date: check every agent's live work (jj + PRs), GitHub merge state, and Linear, then reconcile the tracker and Linear to match the ground — including manual Done when auto-done is off."
---

# Wave Status Sync

Trigger phrase: **"bring up to date"** (or "reconcile the wave"). The
supervisor checks what every agent is actually doing, what has merged, and
what the tracker says — then makes the tracker **and** the issue tracker
match reality. Ground truth beats memory: verify, don't transcribe. This is
the maintenance routine for the wave that `skill://multi-agent-wave` runs.

## 0. Orient

- Read the tracker (`~/notes/wave/tracker.md`): roster, per-agent repos +
  workspaces, conflict map, the live issue-tracker projects.
- Activate integrations if not already active (`search_tool_bm25` for the
  Linear + GitHub MCP servers).
- **Linear MCP must be called via direct top-level tools**, never from an
  `eval` cell or a subagent — the harness `i`-arg leaks on those paths and
  strict servers reject the call *silently* (see `skill://session-recovery`).

## 1. Agents on the ground (jj across every workspace)

Workspaces in one repo share a store, so one set of queries covers all of a
repo's agents:

```bash
cd <repo> && jj workspace list      # each workspace's @ (working-copy commit)
jj bookmark list --all-remotes      # ready work; a `name@origin` line = pushed
jj log -r 'trunk()..' --no-graph -T 'change_id.shortest(8) ++ "  [" ++ bookmarks ++ "]  " ++ description.first_line() ++ "\n"'
```

Read it as: empty `@` = idle / between tasks; a bookmark with commits but
**no `@origin`** = local-ready (→ In Progress); `@origin` present = pushed
(→ PR open / In Review). A multi-repo wave has several repos — run this in
each (`sealed`, `nix-config`, `oh-my-pi`, …).

## 2. What actually merged (trust `main`, not GitHub's merged flag)

A Graphite merge queue closes merged PRs as **closed, not merged**, so the
GitHub `merged` field and `is:merged` search both lie. Authoritative = what
landed on `main`:

```bash
cd <repo> && jj git fetch && jj log -r 'latest(::main@origin, 60)' --no-graph -T 'description.first_line() ++ "\n"'
```

Scan the squashed titles for issue keys / PR numbers. For open-PR truth
(→ In Review) use the GitHub MCP `list_pull_requests` (state `open`) and
`pull_request_read` (`get` → `state`).

## 3. Issue-tracker state

`list_issues` per live project + `list_issue_statuses` for the exact status
names. Sort by `createdAt` to catch newly filed issues (and follow-ups).
`get_issue <KEY>` returns each issue's **attachments** — the linked PR(s) +
commits (the forge/Graphite integration attaches them on submit); that's the
source of truth for which PR is the issue's, and the PR number to record.

## 4. Reconcile — the point of the exercise

Cross-reference the three sources. Per issue:

- **Merged on `main` but not in the completed state** → set it by hand
  (`save_issue` to the status from `list_issue_statuses` whose type/category
  is `completed` — pass its exact name/id, not a name-match on `Done`, since
  the terminal state may be `Shipped`/`Merged`/`Resolved`). If the merge→completed automation is broken, this is manual on
  every sync; do it, and say so in the report.
- **Local-ready commit, no PR** → In Progress. **Pushed / PR open** → In
  Review. **No work, blocked or merely slotted** → Queued/Todo (downgrade a
  stale In Review only when the ground shows zero work — no bookmark, not on
  `main`, no open PR).
- **Don't mutate on a guess.** If a state looks stale but you can't find a
  PR/commit either way, leave it and flag it for the human.
- **Link the PR.** Every In-Review / Done issue carries its PR in the tracker —
  the `#N` in `tracker.md` and the `pr: N` field on the `tracker.html` card
  (that field is the Graphite deep-link). Pull the number from the issue's
  attachments (`get_issue`); if Linear itself never auto-attached it (no
  `Closes SEA-NNN`), add it with `save_issue` `links`.

Then update the artifacts: rewrite `tracker.md` (roster/status, conflict
map, candidate queue), mirror the data arrays in `tracker.html`, and update
the codename inventory if the roster changed. Keep markdown lint-clean.

## 5. Report

State plainly: what you Done'd (with the merging PR number), what states you
corrected (with the jj/PR evidence behind each), and anything left flagged
for the human to decide.

## Notes

- Agents are separate sessions/processes — you can't `irc` them. Read their
  state from jj + PRs, not by asking. To recover a *broken* agent's session,
  use `skill://session-recovery`.
- A tracker "assignee" in the issue tracker is a person, not an agent
  codename — agent→slot assignment lives in the wave tracker, not the issue
  tracker. Don't set codenames as assignees.
- New agents this sync? Name them from `~/notes/workflows/agent-codename-inventory.md`
  (distinct first letters), add roster + conflict-map rows, and mark the
  names used in the inventory.

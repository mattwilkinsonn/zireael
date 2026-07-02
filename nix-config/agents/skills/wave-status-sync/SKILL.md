---
name: wave-status-sync
description: "Bring a multi-agent wave up to date: check every agent's live work (git + PRs), GitHub merge state, and Linear, then reconcile the tracker and Linear to match the ground — including manual Done when auto-done is off."
---

# Wave Status Sync

Trigger phrase: **"bring up to date"** (or "reconcile the wave"). The
supervisor checks what every agent is actually doing, what has merged, and
what the tracker says — then makes the tracker **and** the issue tracker
match reality. Ground truth beats memory: verify, don't transcribe. This is
the maintenance routine for the wave that `skill://multi-agent-wave` runs.

## 0. Orient

- Read the tracker (`~/notes/wave/tracker.md`): roster, per-agent repos +
  clones, conflict map, the live issue-tracker projects.
- Activate the MCP tools if not already active: they route through the LiteLLM
  gateway as `mcp__litellm_<server>_<op>` (Linear + GitHub) and many are behind
  tool-search — run `search_tool_bm25` (e.g. `"linear list issues"`,
  `"github list pull requests"`) to activate the ones you need.
- **Linear MCP must be called via direct top-level tools**, never from an
  `eval` cell or a subagent — the harness `i`-arg leaks on those paths and
  strict servers reject the call *silently* (see `skill://session-recovery`).

## 1. Agents on the ground (git across every clone)

Each agent works in its **own clone** at `~/agents/workspaces/<codename>/<repo>`,
so walk the clones — there's no shared store to query in one shot:

```bash
for d in ~/agents/workspaces/*/<repo>; do
  b=$(git -C "$d" branch --show-current); echo "== $d [$b]"
  git -C "$d" log --oneline origin/main.."$b" 2>/dev/null   # local-ready commits
  git -C "$d" ls-remote --heads origin "$b" | grep -q . && echo "  pushed" || echo "  local-only"
done
```

Read it as: on `main` with no branch commits = idle / between tasks; a branch
with commits but **not on origin** = local-ready (→ In Progress); the branch
present on origin = pushed (→ PR open / In Review). A multi-repo wave spans
several repos — run this across each agent's clones (`sealed`, `nix-config`,
`oh-my-pi`, …).

## 2. What actually merged (trust `main`, not GitHub's merged flag)

A Graphite merge queue closes merged PRs as **closed, not merged**, so the
GitHub `merged` field and `is:merged` search both lie. Authoritative = what
landed on `main`:

```bash
git -C <repo> fetch && git log --oneline -60 origin/main
```

Scan the squashed titles for issue keys / PR numbers. For the **open-PR list**
use **`gh-route pr-list [owner/repo]`** (tern is per-PR, not a lister) — it routes
to whichever API bucket has headroom and returns `{number,title,state,head,user}`
per PR, keeping the every-repo sweep off the strained GraphQL bucket; the GitHub
MCP `list_pull_requests` is the fallback. For any **per-PR review detail** during
the sweep (state of reviews/checks/threads on a specific PR), read it through
**`mcp__litellm_tern_get_review_state`** — cache-served and fleet-shared, so a
wide reconcile doesn't re-exhaust the bucket the way N direct `pull_request_read`
calls would.

## 3. Issue-tracker state

`mcp__litellm_linear_list_issues` per live project +
`mcp__litellm_linear_list_issue_statuses` for the exact status
names. Sort by `createdAt` to catch newly filed issues (and follow-ups).
`mcp__litellm_linear_get_issue <KEY>` returns each issue's **attachments** — the linked PR(s) +
commits (the forge/Graphite integration attaches them on submit); that's the
source of truth for which PR is the issue's, and the PR number to record.

## 4. Reconcile — the point of the exercise

Cross-reference the three sources. Per issue:

- **Merged on `main` but not in the completed state** → set it by hand
  (`mcp__litellm_linear_save_issue` to the status from
  `mcp__litellm_linear_list_issue_statuses` whose type/category
  is `completed` — pass its exact name/id, not a name-match on `Done`, since
  the terminal state may be `Shipped`/`Merged`/`Resolved`). If the merge→completed automation is broken, this is manual on
  every sync; do it, and say so in the report.
- **Local-ready commit, no PR** → In Progress. **Pushed / PR open** → In
  Review. **No work, blocked or merely slotted** → Queued/Todo (downgrade a
  stale In Review only when the ground shows zero work — no branch, not on
  `main`, no open PR).
- **Don't mutate on a guess.** If a state looks stale but you can't find a
  PR/commit either way, leave it and flag it for the human.
- **Link every PR — all repos, not just SEA issues.** Every pushed branch that
  has a PR gets a clickable link in the tracker (sealed *and* zireael / oh-my-pi /
  woodpecker / compass — the non-SEA infra / OMP / skills work counts too). In
  `tracker.md`: a reference link — `[#N]` for sealed, `[z#N]` (zireael) /
  `[omp#N]` (oh-my-pi) for other repos — with the matching `[label]: <github-pr-url>`
  def in the PR-links block at the file end. In `tracker.html`: `pr: N` (or
  `prs: [N, …]`) on the card, plus `prRepo:` for non-sealed (the render maps it
  through `PR_BASE`). Sealed PR numbers come from `mcp__litellm_linear_get_issue` attachments; non-SEA
  from `mcp__litellm_github_list_pull_requests` per repo (`mattwilkinsonn/zireael`,
  `can1357/oh-my-pi`, …). If Linear never auto-attached a SEA PR (no
  `Closes SEA-NNN`), add it with `mcp__litellm_linear_save_issue` `links`.

Then update the artifacts: rewrite `tracker.md` (roster/status, conflict
map, candidate queue), mirror the data arrays in `tracker.html`, and update
the codename inventory if the roster changed. Keep markdown lint-clean.

## 5. Report

State plainly: what you Done'd (with the merging PR number), what states you
corrected (with the git/PR evidence behind each), and anything left flagged
for the human to decide.

## Notes

- Agents are separate sessions/processes — you can't `irc` them. Read their
  state from git + PRs, not by asking. To recover a *broken* agent's session,
  use `skill://session-recovery`.
- A tracker "assignee" in the issue tracker is a person, not an agent
  codename — agent assignment lives in the wave tracker, not the issue
  tracker. Don't set codenames as assignees.
- New agents this sync? Name them from `~/notes/workflows/agent-codename-inventory.md`
  (distinct first letters), add roster + conflict-map rows, and mark the
  names used in the inventory.

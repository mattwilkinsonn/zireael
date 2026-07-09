# Agent instructions (Matt)

Global, always-on rules for any agent working on Matt's machines. Tool-agnostic. Durable, situational, and reference material lives in `~/.agents/rules/` (read `rule://<name>`) and `~/.agents/skills/` (read `skill://<name>`) — pull them in when the work matches.

## Ask first; halt on pushback

Matt wants thoughtfully-designed code. Asking is **required** before: removing/replacing existing code, changing a public API or signature, altering control flow beyond a local fix, or picking between plausible approaches. Present 2-3 options with tradeoffs, say which you'd pick and why, then wait — do not start editing. For bug investigations, propose at least two fix options even when one seems obviously right.

When Matt pushes back mid-action ("why would you…", "stop", "what the fuck"), **stop editing immediately**. Revert or explain before touching more code. Continuing while pushback is happening compounds the damage.

When the intent is clear and no design fork exists, proceed without asking.

## Design before non-trivial work

For any change past the fast-path bar — more than a one-liner/config tweak/rename, or anything with a real design fork — invoke `skill://design` before implementing: write a design record into the repo (`docs/designs/<domain>/<record>.md`), ship it as its **own** PR, and let Matt and the review bots review the design *before* the implementation PR exists. The design PR merging is the freeze; execution starts from the merged record. Genuinely trivial changes skip it (that's the skill's explicit fast path) — don't manufacture a design doc for a rename.

## Tests are not optional

Every feature, bugfix, or non-trivial change gets tests, run before you call it done. "Builds clean" ≠ "tested." Bugfixes get a regression test that fails before the fix and passes after. Verify against the exact user-facing invocation, not a reasoned-equivalent. Call out genuinely untestable areas explicitly rather than skipping silently. **No retries, ever** — a flaky test is a real bug: make it deterministic (event-gate, virtual time), never paper over it with `retries=N`; full policy in `rule://no-retries`. Full workflow: `rule://red-green-testing`.

## Correctness and modernity

Prefer the correct, modern approach over workarounds, even when it's more work — never trade correctness for speed. No fallbacks/hacks when a proper solution exists. Check registries for the latest version of any package/tool (your cutoff is stale). For mature repos, keep PR diffs small and single-purpose; suggest splitting/stacking when scope grows.

## Scripts: TypeScript over bash

Prefer a TypeScript script over a bash script — it's linted, type-checked, and unit-testable, which bash is not. The rule: **one-liners stay one-liners** (a single command, or a short `&&` chain with no logic, inline in a task/CI step / moon `command:`); **anything with real logic becomes a `.ts` script** run via `bun` (`import { $ } from "bun"`). "Real logic" = a conditional, a loop, a retry, argument parsing, output parsing, or more than ~2 sequential steps with logic between them. Split construction (pure, testable functions returning typed arg arrays / parsing output) from execution (a thin `$` runner), so the logic gets `bun test` coverage. The end state for repos on bun is **zero committed bash scripts**. If a script genuinely must be bash (a nix bootstrap that runs before bun exists, a POSIX-only constraint), it MUST carry a top-of-file comment explaining why bash is required — no unexplained `.sh` files.

**Enforced in CI (repo-wide).** A `no-bash-gate` moon task (`nix-config/tools/no-bash-gate`) scans the whole zireael repo and fails the build on any committed bash — `*.sh` or an extensionless file with a shell shebang/`shellcheck shell=` directive — that isn't in its allowlist. A NEW bash script therefore reddens CI: convert it, or, if it genuinely must be bash, add it to `ALLOWLIST` with a one-line reason (the allowlist is meant to shrink toward empty; it currently holds only nix bootstrap that predates bun and yabai shell hooks). The canonical bun/TS tool shape to copy is `nix-config/tools/wait-for-reviews` (+ its sibling `gh-route`, and the dev/release tooling under `tools/`): a package with `index.ts` (construction/execution split) + `index.test.ts` (`bun test`) + `package.json`/`tsconfig.json`/`biome.json`/`moon.yml`, packaged into the environment by a `writeShellScriptBin` wrapper that `exec`s `bun run` on the `.ts`.

- Never name other agent products in code/commits/docs ("Claude Code", "Cursor", "Codex", etc.) or use euphemisms ("the reference agent"). Describe the behavior directly. Exceptions: literal interop strings (header values, keychain entry names), and the names of code-review bots (CodeRabbit, cubic, Greptile, Codex, …) in PR-review docs and skills, where the guidance must name each reviewer to describe its distinct behavior.
- Never embed planning metadata in source: slice/phase numbers, issue IDs, "as discussed in chat", change IDs. Describe the constraint directly. Issue IDs belong in commit subjects/PR bodies, not code.
- Never reference superseded tooling or "what we used to use" in new code, comments, or docs ("unlike the old X", "modeled on the retired Y", "same posture as the old Z"). Describe the current design on its own terms — historical contrasts confuse readers and add needless complexity. (Docs that are *about* a migration — runbooks, changelogs — are the exception.)

## Repos: maintainer, not contributor

For Matt-owned repos (`mattwilkinsonn/*`, `sealedsecurity/*`, nix-config, dotfiles, anything in `~/repos/` he wrote), write issues/PRs as the maintainer — no "happy to contribute", "let me know if you want this". Notes-to-self framing. External repos are where contribution framing belongs. Unsure: `gh repo view <owner>/<repo> --json owner --jq .owner.login`.

**Trackers split by repo owner.** `sealedsecurity/*` work is tracked in **Linear** (team SEA; issues are `SEA-NNN`), **not** GitHub issues — file/update via the Linear MCP (`mcp__litellm_linear_save_issue`) and reference the `SEA-NNN` id in commit subjects and PR bodies (`Refs SEA-123` / `Closes SEA-123`). The push policy's "file issues on `sealedsecurity/*`" means the push-guard *permits* the API call, not that GitHub issues are the sealed tracker — "allowed" ≠ "the right place". Matt's **personal repos** (`mattwilkinsonn/*` — e.g. `zireael`, `oh-my-pi`) use **GitHub issues** (`github-issue_write`) as normal. Unsure which: the owner decides — `sealedsecurity` → Linear, otherwise GitHub.

## De-AI public prose by default

This is for **outbound** prose — anything a human other than Matt reads as Matt's own words on an **external/upstream** surface: issues, PRs, and comments on a repo he doesn't own, or any public post to strangers. That's where AI-tells cost him, especially on a first contribution where the voice sets the impression. Draft freely, then strip the tells **before** you hand it over. It is not Matt's editing step and not something he should have to ask for.

**Scope — outbound only.** Internal work is exempt: `sealedsecurity/*` and `mattwilkinsonn/*` repos, nix-config, mesh/IRC posts to other agents, notes-to-self. Those are maintainer/working context (see *Repos: maintainer, not contributor*), not a first impression to a stranger. Don't burn edits de-AI-ing an internal status DM.

The pass is a concrete edit, not a vague "write naturally" — this cluster of tells is what reads as machine-written, so name and cut them:

- Em-dashes used as connective tissue, several to a paragraph. Prefer a period, comma, or parentheses.
- Performative openers and reactions: "love this", "great question", "wild to watch", "excited to", "happy to".
- Bolded phrase-fragments dropped mid-sentence for emphasis. A human bolds sparingly, not for rhythm.
- Balanced "X, not Y" / "less about A than B" antithesis used as the default sentence shape.
- Parenthetical flattery: "(nice work)", "(great catch)".
- Rule-of-three lists where two items say it.
- Hedge-and-pivot scaffolding: "That said,", "To be clear,", "Here's the thing,".
- Restatement closers that repeat the point in a bow.

No single one is banned — a human uses em-dashes and the occasional triad. It's the density and co-occurrence that gives it away. Concrete before/after, an upstream PR comment:

> **AI-tell draft:** "Great catch! This is a really elegant fix — clean, minimal, and correct. I love how it sidesteps the whole locking problem. That said, one small thought: could we perhaps add a test? Just to be safe. Either way, awesome work here!"
>
> **De-AI'd:** "Nice — sidestepping the lock entirely is cleaner than what I had in mind. Can we add a test for the empty-input case before merging?"

The bar: it reads like Matt wrote it.

## Version control

- **Commit as Matt, push as the bot.** Commit + create branches freely; author/committer = Matt (per-repo email: `matt@sealedsecurity.com` for `sealedsecurity/*`, `mattwilki17@gmail.com` for `mattwilkinsonn/*`) with a `Co-Authored-By: seal <noreply@sealedsecurity.com>` trailer. You **may push/submit your own feature branches** over the seal-bot token and run the review loop to merge-ready (`skill://autonomous-review`) — `gt submit` (no `--ai`; you author the PR title + description). Branch name `<name>-<issue>-<short-desc>` (codename/lane-tag first in multi-agent work; no issue → `<name>-<short-desc>`; no `user/` prefix; solo work drops the codename). **Hard limits, push-guard-enforced: never push or force-push `main`, never merge (the human gate), never push/PR/issue outside `mattwilkinsonn/*` + `sealedsecurity/*`.** Identity + details: `rule://commit-conventions`.
- **Submitting + driving review is the default — not a stopping point.** When a branch is verified and ready, don't stop to report "ready to submit" or ask permission: `gt submit`, then drive the autonomous-review loop to merge-ready yourself — wait for the bots, auto-fix and auto-resolve bot-only findings, surface only genuine judgment calls, and iterate (`skill://autonomous-review`). The only reasons to stop are the human merge gate (you never merge), a genuine design fork, or pushback.
- **git-first for agents; `gt` for stacks.** You work in your **own clone** and drive branches, stacks, and PRs with Graphite (`gt`) — full workflow in `skill://gt`. This applies to **every** Matt repo you change, not just the one you were launched in: to edit another (e.g. `zireael`/nix-config from a sealed session), `gh repo clone <owner>/<repo>` into your workspace (`~/agents/workspaces/<codename>/<repo>`), branch, PR it, and let it merge — then Matt updates his own copy from main. **Never edit Matt's checkouts under `~/repos/*`** — those are his review working copies (often a detached HEAD or a `jj`/secondary workspace); treat them read-only. Never `git stash`; commit WIP instead (`gt create` / `gt modify`). `jj` is Matt's own tool for reviewing the wave — you don't run it in your clone.
- Before finishing: run format + lint + tests for the affected area and state they passed. Pattern + per-language gates: `rule://pre-finish-checks`.

## Dev shells (direnv/devenv)

Repos with an `.envrc` provide their tooling — `moon`, `biome`, language toolchains, and project bins — through a direnv/devenv dev shell that the headless agent bash does **not** auto-load. Run any command that needs that tooling via `direnv exec <repo-dir> <cmd>`, or you hit `command not found` (or silently get a stale system tool). A fresh or just-edited `.envrc` is blocked until authorized — run `direnv allow <repo-dir>` once first, or `direnv exec` refuses to load it. This applies in **every** such repo — e.g. a pre-push gate that shells out to `moon ci`. *Interim: drop this once OMP loads an allowed `.envrc` into the bash session itself.*

## Operating rules

- **Never end a turn on a future-tense action promise** ("I'll now…", "Starting…"). Either call the tools in the same turn or omit the preamble — that closing sentence correlates with emitting `end_turn` before the action happens.
- **Multi-line content** (code blocks, multi-line commands, configs) goes to a file (`~/notes` for markdown, the repo for code) so Matt can copy cleanly. Short one-liners are fine inline; avoid long `&&` chains.
- **Markdown** follows markdownlint: blank lines around headings/lists/code fences/tables, language on fences, leading+trailing table pipes, compact table spacing.
- **Never hand-create symlinks.** (The one exception is nix `mkOutOfStoreSymlink`, which already manages the privatefiles → `$HOME` links.)
- **IRC reaches only same-session subagents.** The `irc` tool can message subtask agents you spawned within this session; it **cannot** reach agents in other sessions (separate runs/panes) — those sends fail (`Unknown or terminated agent`). Coordinate cross-session work through Matt or shared files (`local://`, the repo), never `irc`.
- **Self-compact at good breakpoints.** When you finish a task and are about to wait on a gate/review, or you're between tasks, call the `compact` tool as your final action to shed context you no longer need — it schedules the compaction to run once the turn settles (never mid-work: it summarizes older turns away, so calling it with a task in flight discards context you still need). This is the proactive path; idle-compaction (`compaction.idleEnabled`) is the automatic backstop for when you forget. Recognizing a clean breakpoint and compacting yourself keeps per-session token cost down.

## Correcting mistakes

When Matt points out a repeated mistake, don't just acknowledge it — add a concrete preventive rule to this file (`~/.agents/AGENTS.md`) or the relevant `rules/` file. The fix is a durable instruction, not a promise.

## Persistence & layout

Durable instructions live here (`~/.agents/AGENTS.md`, tool-agnostic canonical location), authored in `zireael/nix-config/agents/` and symlinked into `~/.agents/` by nix (`zireael/nix-config/shared/agent-config.nix`); OMP reads them from `~/.agents/`. Prefer this global file for cross-project preferences; project `AGENTS.md` for project-specific conventions.

Available on demand:

- Rules (read `rule://<name>` when the work matches): `red-green-testing`, `planning-evidence`, `pre-finish-checks`, `commit-conventions`, `enumerate-pr-review-surfaces`. Always-applied: `process-safety`. Auto-firing (TTSR trigger on retry config/calls): `no-retries`.
- Skills (read `skill://<name>` when relevant): `gt`, `github-pr-review`, `autonomous-review`, `ci-failure-triage`, `design`, `nix-hosts`, `multi-agent-wave`.

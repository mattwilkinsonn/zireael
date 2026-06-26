# Agent instructions (Matt)

Global, always-on rules for any agent working on Matt's machines. Tool-agnostic. Durable, situational, and reference material lives in `~/.agents/rules/` (read `rule://<name>`) and `~/.agents/skills/` (read `skill://<name>`) — pull them in when the work matches.

## Ask first; halt on pushback

Matt wants thoughtfully-designed code. Asking is **required** before: removing/replacing existing code, changing a public API or signature, altering control flow beyond a local fix, or picking between plausible approaches. Present 2-3 options with tradeoffs, say which you'd pick and why, then wait — do not start editing. For bug investigations, propose at least two fix options even when one seems obviously right.

When Matt pushes back mid-action ("why would you…", "stop", "what the fuck"), **stop editing immediately**. Revert or explain before touching more code. Continuing while pushback is happening compounds the damage.

When the intent is clear and no design fork exists, proceed without asking.

## Tests are not optional

Every feature, bugfix, or non-trivial change gets tests, run before you call it done. "Builds clean" ≠ "tested." Bugfixes get a regression test that fails before the fix and passes after. Verify against the exact user-facing invocation, not a reasoned-equivalent. Call out genuinely untestable areas explicitly rather than skipping silently. Full workflow: `rule://red-green-testing`.

## Correctness and modernity

Prefer the correct, modern approach over workarounds, even when it's more work — never trade correctness for speed. No fallbacks/hacks when a proper solution exists. Check registries for the latest version of any package/tool (your cutoff is stale). For mature repos, keep PR diffs small and single-purpose; suggest splitting/stacking when scope grows.

## Comment & doc hygiene

- Never name other agent products in code/commits/docs ("Claude Code", "Cursor", "Codex", etc.) or use euphemisms ("the reference agent"). Describe the behavior directly. Literal interop strings (header values, keychain entry names) are fine.
- Never embed planning metadata in source: slice/phase numbers, issue IDs, "as discussed in chat", change IDs. Describe the constraint directly. Issue IDs belong in commit subjects/PR bodies, not code.
- Never reference superseded tooling or "what we used to use" in new code, comments, or docs ("unlike the old X", "modeled on the retired Y", "same posture as the old Z"). Describe the current design on its own terms — historical contrasts confuse readers and add needless complexity. (Docs that are *about* a migration — runbooks, changelogs — are the exception.)

## Repos: maintainer, not contributor

For Matt-owned repos (`mattwilkinsonn/*`, `sealedsecurity/*`, nix-config, dotfiles, anything in `~/repos/` he wrote), write issues/PRs as the maintainer — no "happy to contribute", "let me know if you want this". Notes-to-self framing. External repos are where contribution framing belongs. Unsure: `gh repo view <owner>/<repo> --json owner --jq .owner.login`.

## Version control

- **You may commit** (Conventional Commits subject: `feat(scope):`, `fix(scope):`, `refactor(scope):`, etc. + ≤2 sentences) **and create/move bookmarks** — only *pushing* is off-limits. **Never push** — Matt always pushes himself, every form (`git push`, `jj git push`, force, any variant). When a change is ready, create the bookmark yourself — name it `sea-NNN-<short-desc>` (issue ref first, no `user/` prefix; suffix `--<agent-handle>` in multi-agent work) — then hand Matt only the `jj git push` command and stop.
- **jj-first.** Use `jj` for every VCS op in any repo with `.jj/` (the `$HOME` dotfiles repo and most repos are colocated jj+git). `git` only for pure-git repos. Never `git stash` (commit instead, or a worktree). Details + revsets + workspace model: `skill://vcs-jj`.
- Before finishing: run format + lint + tests for the affected area and state they passed. Pattern + per-language gates: `rule://pre-finish-checks`.

## Operating rules

- **Never end a turn on a future-tense action promise** ("I'll now…", "Starting…"). Either call the tools in the same turn or omit the preamble — that closing sentence correlates with emitting `end_turn` before the action happens.
- **Multi-line content** (code blocks, multi-line commands, configs) goes to a file (`~/notes` for markdown, the repo for code) so Matt can copy cleanly. Short one-liners are fine inline; avoid long `&&` chains.
- **Markdown** follows markdownlint: blank lines around headings/lists/code fences/tables, language on fences, leading+trailing table pipes, compact table spacing.
- **Never hand-create symlinks.** (The one exception is nix `mkOutOfStoreSymlink`, which already manages the privatefiles → `$HOME` links.)

## Correcting mistakes

When Matt points out a repeated mistake, don't just acknowledge it — add a concrete preventive rule to this file (`~/.agents/AGENTS.md`) or the relevant `rules/` file. The fix is a durable instruction, not a promise.

## Persistence & layout

Durable instructions live here (`~/.agents/AGENTS.md`, tool-agnostic canonical location), authored in `zireael/nix-config/agents/` and symlinked into `~/.agents/` by nix (`zireael/nix-config/shared/agent-config.nix`); OMP reads them from `~/.agents/`. Prefer this global file for cross-project preferences; project `AGENTS.md` for project-specific conventions.

Available on demand:

- Rules (read `rule://<name>` when the work matches): `red-green-testing`, `planning-evidence`, `pre-finish-checks`, `commit-conventions`, `enumerate-pr-review-surfaces`. Always-applied: `process-safety`.
- Skills (read `skill://<name>` when relevant): `vcs-jj`, `github-pr-review`, `nix-hosts`, `multi-agent-wave`.

# push-guard

## Overview

`push-guard` is an OMP extension (`nix-config/agents/extensions/`
`push-guard.ts`) that intercepts every `tool_call` and decides, at
tool-execution time, whether a git / `gh` / `jj-gt` / GitHub-MCP
operation may run. It is a hard guardrail: it holds even if the model
is confused or a prompt tries to override it
(`push-guard.ts:3` — `// Hard guardrails that hold even if the model is`
`confused or a prompt tries to`).

Under the seal-bot push model an agent MAY push feature branches,
open/update PRs, and run its own review loop on allowlisted-owner
repos. The guard keeps four classes of operation blocked, always
(`push-guard.ts:6-11`):

- pushing or force-pushing `main` (merge is the human gate),
- merging a PR,
- any *write* (push / PR / issue) to a repo outside the owner
  allowlist (an OSS upstream like `can1357/*`), and
- broad pattern-matching process kills.

The whole decision is a pure function,
`push-guard.ts:128` — `export function evaluate(toolName: string,`
`input: Record<string, unknown>): Block | null`, which returns a block
object `{ block: true, reason }` or `null` to allow
(`push-guard.ts:135-141`, `push-guard.ts:193`). The default export wires
it to the runtime, returning the block to the host or `undefined`
(`push-guard.ts:226` — `pi.on("tool_call", async (event) => {` …
`push-guard.ts:228` — `return result ?? undefined;`).

The only allowed owners are fixed and lowercased
(`push-guard.ts:17-20`):

```ts
const ALLOWED_OWNERS: Record<string, true> = {
 mattwilkinsonn: true,
 sealedsecurity: true,
};
```

### Tool surface

The guard inspects three command-bearing tools and, separately,
GitHub-MCP tools. Broad-kill protection is scoped to tools that run
locally (a kill over `ssh` runs on the remote, not the session's own
runtime — `push-guard.ts:114-117`).

| Set | Members | Location |
| --- | --- | --- |
| `CMD_TOOLS` | `bash`, `ssh`, `recipe` | `push-guard.ts:118` |
| `LOCAL_TOOLS` | `bash`, `recipe` | `push-guard.ts:119` |
| `GH_MCP` | `/^mcp__github_/` | `push-guard.ts:120` |

For a command tool, the command text comes from `input.command` when it
is a string, otherwise the whole input is stringified so a push/merge in
any field is still scanned
(`push-guard.ts:132` — `const cmd = typeof input.command === "string" ?`
`input.command : JSON.stringify(input ?? {})`). Any tool that is neither
a command tool nor a GitHub-MCP tool is allowed
(`push-guard.ts:220` — `return null;`).

## Decision order

Order matters: the first matching rule wins. For a `CMD_TOOLS` command
the checks run in this sequence:

1. broad process kill — `LOCAL_TOOLS` only (`push-guard.ts:134`),
2. merge (`push-guard.ts:144`),
3. write to a named non-allowlisted owner (`push-guard.ts:153-154`),
4. bare `gh` create without an allowlisted `-R`
   (`push-guard.ts:164`),
5. push to the `upstream` remote (`push-guard.ts:175-176`),
6. push to `main` (`push-guard.ts:184`),
7. otherwise allow (`push-guard.ts:193`).

For a `GH_MCP` tool (`push-guard.ts:196`):

1. any `merge` tool is blocked regardless of owner
   (`push-guard.ts:197`),
2. a write tool is owner-checked, fail-closed
   (`push-guard.ts:205-207`),
3. otherwise (a read) allow (`push-guard.ts:217`).

## Requirements

### Requirement: Main-branch protection

The agent SHALL be blocked from pushing or force-pushing the `main`
branch in any refspec form, and SHALL NOT be blocked merely because a
feature branch name contains the substring `main`. Detection is the
`PUSH_MAIN` regex, applied only after `PUSH` matches
(`push-guard.ts:175` — `if (PUSH.test(cmd)) {` … `push-guard.ts:184` —
`if (PUSH_MAIN.test(cmd)) {`).

```ts
const PUSH_MAIN = /:(?:refs\/heads\/)?main(?![\w-])|\brefs\/heads\/main(?![\w-])|(?:^|\s)(?:-b|--bookmark|--branch)\s+main(?![\w-])|\borigin\s+(?:-\S+\s+)*main(?![\w-])/;
```

(`push-guard.ts:34`). The trailing `(?![\w-])` on every branch is what
prevents a false positive when `main` is only a prefix of a longer
branch name.

#### Scenario: Block every explicit `main` push target

- **Given** a push command naming `main` as the destination ref.
- **When** the guard evaluates it.
- **Then** it SHALL return a block. The test suite enumerates the exact
  forms, all asserting `?.block === true`
  (`push-guard.test.ts:16-22`): `git push origin main`,
  `git push -f origin main`, `jj git push -b main`,
  `git push origin HEAD:main`,
  `git push origin feature:refs/heads/main`,
  `git push origin HEAD:refs/heads/main`, and
  `git push origin refs/heads/main`.
- **And** the block reason names the policy
  (`push-guard.ts:188` — `"Push to \`main\` blocked: never push or`
  `force-push \`main\`."`).

#### Scenario: Allow a feature branch that merely contains "main"

- **Given** a push/submit whose branch name embeds `main` as a
  substring.
- **When** the guard evaluates it.
- **Then** it SHALL return `null` (allow). The suite asserts
  `toBeNull()` for `jj-gt submit -b cook-sea-1-main-nav`,
  `git push origin feat-main-menu`, `git push origin main-feature`, and
  `jj-gt submit -b main-nav` (`push-guard.test.ts:26-29`).
- **And** ordinary feature pushes are likewise allowed —
  `jj-gt submit -b cook-compass-scaffold`,
  `jj git push -b hudson-sea-865-aws-provider`,
  `git push origin amundsen-fnm-direnv-path`,
  `git push --force origin cook-feature-restack`
  (`push-guard.test.ts:9-12`).

### Requirement: Owner allowlist on writes

Any write (push, PR, or issue) that names an owner outside
`ALLOWED_OWNERS` SHALL be blocked, while reads of any repo (including an
OSS upstream) SHALL be allowed. The block fires when a non-allowlisted
owner is named **and** the command is a push or a `gh` write verb
(`push-guard.ts:153-154`):

```ts
const bad = namedOwners(cmd).filter((o) => ALLOWED_OWNERS[o] !== true);
if (bad.length && (PUSH.test(cmd) || GH_WRITE_CMD.test(cmd))) {
```

Owners are extracted by `namedOwners()` from two sources, lowercased
(`push-guard.ts:39-40`):

```ts
for (const m of cmd.matchAll(/github\.com[/:]([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
for (const m of cmd.matchAll(/(?:-R|--repo)[=\s]+["']?(?:[\w.-]+\/)?([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
```

— i.e. a full URL (`github.com/owner/repo` or scp-style
`github.com:owner/repo`) and a `-R`/`--repo owner/repo` selector with an
optional host prefix tolerated. A push to a remote literally named
`upstream` carries no URL or owner, so it is caught separately by
`PUSH_UPSTREAM` (`push-guard.ts:31`,
`push-guard.ts:176` — `if (PUSH_UPSTREAM.test(cmd)) {`).

#### Scenario: Block a write to a non-allowlisted owner

- **Given** a push or `gh` write that names an owner not in the
  allowlist (e.g. `can1357`).
- **When** the guard evaluates it.
- **Then** it SHALL block. Asserted `?.block === true` for
  `git push https://github.com/can1357/oh-my-pi main`,
  `gh pr create -R can1357/oh-my-pi`,
  `gh issue create -R can1357/oh-my-pi -t bug`,
  `git push upstream my-branch`, and the flags-between-noun-and-verb form
  `gh pr -R can1357/oh-my-pi create` (`push-guard.test.ts:39-43`).
- **And** the `gh pr -R … create` case proves the write verb is matched
  anywhere in the `gh` segment, not only directly after the noun
  (`push-guard.ts:25-28`, `GH_WRITE_CMD`).

#### Scenario: Allow writes to an allowlisted owner, and any read

- **Given** a write that names `mattwilkinsonn` or `sealedsecurity`, or
  any read.
- **When** the guard evaluates it.
- **Then** it SHALL return `null`. Asserted `toBeNull()` for
  `git push https://github.com/mattwilkinsonn/zireael my-branch`,
  `gh pr create -R sealedsecurity/sealed`, and the upstream read
  `gh pr view -R can1357/oh-my-pi 42` (`push-guard.test.ts:69-71`).
- **And** the upstream-read case holds because `gh ... view` is not in
  the `GH_WRITE_CMD` verb set, so the named non-allowlisted owner alone
  does not block (`push-guard.ts:27-28`).

#### Scenario: Push to the `upstream` remote is blocked

- **Given** `git push upstream my-branch` (an OSS-upstream vector with
  no URL/owner the owner check can see).
- **When** the guard evaluates it after `PUSH` matches.
- **Then** it SHALL block via `PUSH_UPSTREAM`
  (`push-guard.ts:31` — `const PUSH_UPSTREAM = /\bpush\b(?:\s+-\S+)*\s+upstream\b/;`,
  reason at `push-guard.ts:180`). The owner-allowlist branch also
  catches this same command (`push-guard.test.ts:42`).

### Requirement: Bare `gh` create needs an allowlisted `-R`

A `gh issue create` / `gh pr create` (or the `gh issue new` alias) with
no allowlisted `-R` SHALL be blocked, because a bare create files on
whatever repo the cwd resolves to — an OSS upstream included
(`push-guard.ts:73-78`). This is fail-closed: absence of an allowlisted
target is a block (`push-guard.ts:164`).

Detection requires the create verb to be the token **immediately after**
the `issue`/`pr` noun (`push-guard.ts:83-86`):

```ts
const noun = args.findIndex((a) => a === "issue" || a === "pr");
if (noun === -1) continue;
const verb = args[noun + 1];
if (verb !== "create" && verb !== "new") continue;
```

The command is allowed only when a real `-R`/`--repo` selector resolves
to an allowlisted owner (`push-guard.ts:87-88`):

```ts
const owner = repoFlagOwner(seg);
if (owner !== null && ALLOWED_OWNERS[owner] === true) continue; // explicit allowlisted target
```

`ghArgs()` finds the `gh` invocation behind benign wrappers and returns
its args, or `null` when `gh` is not the command — so `gh` appearing only
inside a commit message is ignored
(`push-guard.ts:62` — `return i < toks.length && /(?:^|\/)gh$/.test(toks[i]) ? toks.slice(i + 1) : null;`).
Recognised wrappers (`push-guard.ts:48`):

```ts
const GH_WRAPPERS = new Set(["env", "timeout", "nice", "ionice", "stdbuf", "nohup", "setsid", "sudo", "doas", "command", "exec", "time"]);
```

`repoFlagOwner()` reads the owner from a real selector, tolerating
quotes and a host prefix (`push-guard.ts:69`):

```ts
const m = seg.match(/(?:-R|--repo)[=\s]+["']?(?:[\w.-]+\/)?([\w.-]+)\/[\w.-]+/);
```

#### Scenario: Block a bare create with no allowlisted target

- **Given** a create with no `-R`, or a non-allowlisted/`-R`-less target,
  possibly behind a wrapper.
- **When** the guard evaluates it.
- **Then** it SHALL block. Asserted `?.block === true` for
  `gh issue create --title bug --body repro`, `gh pr create --fill`,
  `cd /tmp/upstream && gh issue create -t spam`, the alias
  `gh issue new --title bug`, the wrapped
  `env GH_TOKEN=x gh issue create -t bug` and
  `timeout 30 gh pr create --fill`, and
  `gh issue create --body https://github.com/mattwilkinsonn/zireael`
  (`push-guard.test.ts:47-53`).
- **And** the last case proves a URL sitting in `--body` is **not** a
  target: `repoFlagOwner()` only reads `-R`/`--repo`, so the create is
  still blocked even though the URL owner is allowlisted
  (`push-guard.ts:68-71`).

#### Scenario: Allow a create with an allowlisted `-R`, and non-creates

- **Given** a create whose `-R`/`--repo` resolves to an allowlisted
  owner, or a `gh` command whose token after the noun is not a create
  verb, or `gh` appearing only inside a message.
- **When** the guard evaluates it.
- **Then** it SHALL return `null`. Asserted `toBeNull()` for
  `gh issue create -R mattwilkinsonn/zireael -t bug`,
  `gh pr create -R sealedsecurity/sealed --fill`, the quoted
  `gh pr create -R "sealedsecurity/sealed" --fill`, the host-qualified
  `gh issue create --repo github.com/mattwilkinsonn/zireael -t bug`,
  the flag-value non-verbs `gh issue list --label create` and
  `gh pr comment 123 --body create`, plain `gh issue list`, and the
  message-only `git commit -m "note: block bare gh issue create"` /
  `jj describe -m "gh pr create guard"`
  (`push-guard.test.ts:57-65`).

### Requirement: PR merge is the human gate

Merging a PR SHALL always be blocked — the agent never merges, on any
repo, including allowlisted ones (`push-guard.ts:23` — `// Merge — the`
`human gate, blocked for every repo.`). For command tools this is the
`MERGE` regex, checked before the owner and push branches
(`push-guard.ts:144` — `if (MERGE.test(cmd)) {`):

```ts
const MERGE = /\bgh\b[^\n;|&]*\bmerge\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*merge\b/;
```

(`push-guard.ts:24`) — a `merge` anywhere in a `gh` segment, or a
`jj-gt`/`gt … merge`.

#### Scenario: Block `gh` and `jj-gt` merges from the command line

- **Given** a merge command.
- **When** the guard evaluates it.
- **Then** it SHALL block. Asserted `?.block === true` for
  `gh pr merge 123 --squash`, `jj-gt merge`, and — even with an
  allowlisted `-R` between noun and verb —
  `gh pr -R sealedsecurity/sealed merge 123 --squash`
  (`push-guard.test.ts:33-35`). The last case shows merge is
  owner-agnostic and is decided before the owner-allowlist branch.

#### Scenario: Block a GitHub-MCP merge regardless of owner

- **Given** a GitHub-MCP tool whose name contains `merge`, even with an
  allowlisted owner.
- **When** the guard evaluates it.
- **Then** it SHALL block before the write/owner check
  (`push-guard.ts:197` — `if (/merge/.test(toolName)) {`). Asserted
  `?.block === true` for
  `mcp__github_merge_pull_request` with
  `{ owner: "sealedsecurity", repo: "sealed" }`
  (`push-guard.test.ts:83`).

### Requirement: Broad process-kill protection

A broad, pattern-matching process kill SHALL be blocked on locally
executing tools, because it can take down the session's own runtime or
unrelated work; a kill targeting a specific PID SHALL be allowed. The
check runs only for `LOCAL_TOOLS`
(`push-guard.ts:134` — `if (LOCAL_TOOLS[toolName] === true && hasBroadKill(cmd)) {`)
via `hasBroadKill()` (`push-guard.ts:99-112`).

`pkill` / `killall` are always pattern-based, so any occurrence is broad
(`push-guard.ts:104` — `if (!/(?:^|\/)kill$/.test(toks[idx])) return true; // pkill / killall`).
For `kill`, the leading signal spec is skipped and the command is broad
only when a remaining target is negative, i.e. a process group or
everything (`push-guard.ts:106-109`):

```ts
if (toks[k] === "-s" || toks[k] === "-n") k += 2;
else if (k < toks.length && /^-[A-Za-z0-9]+$/.test(toks[k])) k += 1;
if (toks[k] === "--") k += 1;
if (toks.slice(k).some((t) => /^-\d+$/.test(t))) return true; // negative target
```

#### Scenario: Block pattern kills and negative targets

- **Given** `pkill -f node` or `kill -- -1`.
- **When** the guard evaluates it on `bash`.
- **Then** it SHALL block (`push-guard.test.ts:87-88`, both
  `?.block === true`). The block reason points at the rule
  (`push-guard.ts:138-140` — `"Broad process kill blocked (pkill /`
  `killall / kill -1)."`, `See rule://process-safety.`).

#### Scenario: Allow a targeted kill by PID

- **Given** `kill -9 12345` (SIGKILL to one explicit PID).
- **When** the guard evaluates it.
- **Then** it SHALL return `null` — the signal flag `-9` is skipped and
  `12345` is not a negative target (`push-guard.test.ts:89`,
  `toBeNull()`).

### Requirement: GitHub-MCP write operations

A GitHub-MCP write tool SHALL be owner-checked and fail-closed: the
owner is read from `input.owner`, and a missing or non-string owner
collapses to `""`, which is not in the allowlist and is therefore
blocked (`push-guard.ts:206-207`):

```ts
const owner = typeof input.owner === "string" ? input.owner.toLowerCase() : "";
if (ALLOWED_OWNERS[owner] !== true) {
```

Write tools are identified by `GH_MCP_WRITE`, whose verbs are anchored
to the `mcp__github_` prefix so the shared `pull_request_` infix does
not misclassify a read (`push-guard.ts:121-125`):

```ts
const GH_MCP_WRITE = /^mcp__github_(?:create|update|delete|add|fork|push|dispatch|request|merge)_|_write\b/;
```

Anything else under the `mcp__github_` prefix (e.g.
`pull_request_read`, `get_*`) is a read and is allowed on any repo,
including an upstream PR being triaged (`push-guard.ts:217`).

#### Scenario: Owner-check MCP writes, fail-closed on a missing owner

- **Given** a GitHub-MCP write tool.
- **When** the guard evaluates it.
- **Then** it SHALL block for a non-allowlisted or absent owner and
  allow for an allowlisted one. Asserted in `push-guard.test.ts:75-79`:
  `mcp__github_create_pull_request` with `{ owner: "can1357" }` blocks;
  with `{ owner: "mattwilkinsonn" }` and `{ owner: "sealedsecurity" }`
  returns `null`; with `{}` (no owner) blocks — fail-closed.

#### Scenario: MCP reads are allowed on any repo

- **Given** `mcp__github_pull_request_read` with
  `{ owner: "can1357", repo: "oh-my-pi" }` (an upstream read).
- **When** the guard evaluates it.
- **Then** it SHALL return `null`, because `GH_MCP_WRITE` does not match
  `pull_request_read` — `request` only matches when anchored right after
  the prefix (`push-guard.test.ts:77`, `push-guard.ts:122-125`).

## Regex reference

| Constant | What it matches | Location |
| --- | --- | --- |
| `PUSH` | `git … push`, `jj … git push`, or `jj-gt`/`gt … submit`/`s`/`ss` | `push-guard.ts:21` |
| `MERGE` | `gh … merge` (any flags between), or `jj-gt`/`gt … merge` | `push-guard.ts:24` |
| `GH_WRITE_CMD` | a `gh` segment containing a write verb (`create`/`edit`/`comment`/`close`/…) | `push-guard.ts:27` |
| `PUSH_UPSTREAM` | `push … upstream` (a remote literally named `upstream`) | `push-guard.ts:31` |
| `PUSH_MAIN` | `main` as a push target: `:main`, `refs/heads/main`, `-b main`, `origin … main` | `push-guard.ts:34` |
| `GH_MCP` | tool-name prefix `mcp__github_` | `push-guard.ts:120` |
| `GH_MCP_WRITE` | MCP write verbs anchored to the prefix, or a `_write` suffix | `push-guard.ts:125` |

Helper extractors (not constants) and their roles:

| Function | Role | Location |
| --- | --- | --- |
| `namedOwners()` | owners from `github.com/owner/repo` URLs and `-R owner/repo` | `push-guard.ts:37-42` |
| `ghArgs()` | args after `gh`, skipping wrappers/assignments, else `null` | `push-guard.ts:49-63` |
| `repoFlagOwner()` | owner from a real `-R`/`--repo` selector, lowercased | `push-guard.ts:68-71` |
| `ghCreateWithoutAllowedOwner()` | true if a bare `gh` create lacks an allowlisted `-R` | `push-guard.ts:79-92` |
| `hasBroadKill()` | true for `pkill`/`killall` or `kill` with a negative target | `push-guard.ts:99-112` |

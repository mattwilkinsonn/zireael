# push-guard

## Overview

`push-guard` is an OMP extension (`nix-config/agents/extensions/`
`push-guard.ts`) that intercepts every `tool_call` and decides, at
tool-execution time, whether a git / `gh` / `jj-gt` / GitHub-MCP
operation may run. It is a hard guardrail that holds even if the model is
confused or a prompt tries to override it
(`push-guard.ts:3` — `// Hard guardrails that hold even if the model is`
`confused or a prompt tries to`).

Under the seal-bot push model an agent MAY push feature branches,
open/update PRs, and run its own review loop on allowlisted-owner repos.
The guard keeps four classes of operation blocked, always
(`push-guard.ts:7-11`):

- pushing or force-pushing `main` — on ANY remote — plus pushing to the
  `upstream` remote (merge is the human gate),
- merging a PR — including a `submit` that hands the merge to Graphite,
- any *write* (push / PR / issue / `gh api` mutation) to a repo outside
  the owner allowlist (an OSS upstream like `can1357/*`), and
- broad pattern-matching process kills.

The whole decision is a pure function
(`push-guard.ts:161` — `export function evaluate(toolName: string,`
`input: Record<string, unknown>): Block | null {`), which returns a
block object `{ block: true, reason }` or `null` to allow
(`push-guard.ts:237` — `return null;`). The default export wires it to
the runtime, returning the block to the host or `undefined`
(`push-guard.ts:270` — `pi.on("tool_call", async (event) => {` …
`push-guard.ts:272` — `return result ?? undefined;`).

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
locally — a kill over `ssh` runs on the remote, not the session's own
runtime (`push-guard.ts:147-150`).

| Set | Members | Location |
| --- | --- | --- |
| `CMD_TOOLS` | `bash`, `ssh`, `recipe` | `push-guard.ts:151` |
| `LOCAL_TOOLS` | `bash`, `recipe` | `push-guard.ts:152` |
| `GH_MCP` | `/^mcp__github_/` | `push-guard.ts:153` |

For a command tool the command text comes from `input.command` when it
is a string, otherwise the whole input is stringified so a push/merge in
any field is still scanned
(`push-guard.ts:165` — `const cmd = typeof input.command === "string" ?`
`input.command : JSON.stringify(input ?? {});`). Any tool that is
neither a command tool nor a GitHub-MCP tool is allowed
(`push-guard.ts:264` — `return null;`).

## Decision order

Order matters: the first matching rule wins. For a `CMD_TOOLS` command
the checks run in this sequence:

1. broad process kill — `LOCAL_TOOLS` only (`push-guard.ts:167`),
2. merge (`push-guard.ts:177`),
3. write to a named non-allowlisted owner — push **or** `gh` write verb
   (`push-guard.ts:186-187`),
4. `gh api` write to a non-allowlisted / unparsed `repos/<owner>/`
   endpoint (`push-guard.ts:197`),
5. bare `gh` create without an allowlisted `-R` (`push-guard.ts:208`),
6. push to the `upstream` remote (`push-guard.ts:220`) or to `main`
   (`push-guard.ts:228`),
7. otherwise allow (`push-guard.ts:237`).

For a `GH_MCP` tool (`push-guard.ts:240`):

1. any `merge` tool is blocked regardless of owner
   (`push-guard.ts:241`),
2. a write tool is owner-checked, fail-closed (`push-guard.ts:249-251`),
3. otherwise (a read) allow (`push-guard.ts:261`).

## Requirements

### Requirement: Main-branch and `upstream`-remote push protection

The agent SHALL be blocked from pushing or force-pushing the `main`
branch in any refspec form, on **any** remote (not just `origin`), and
SHALL be blocked from pushing to a remote literally named `upstream`.
The agent SHALL NOT be blocked merely because a feature branch name
contains the substring `main`, nor because a compound command later runs
`git checkout main`. Both checks run only after `PUSH` matches, with
`upstream` evaluated before `main`
(`push-guard.ts:219` — `if (PUSH.test(cmd)) {` …
`push-guard.ts:220` — `if (PUSH_UPSTREAM.test(cmd)) {` …
`push-guard.ts:228` — `if (PUSH_MAIN.test(cmd)) {`).

`PUSH` recognises git, `jj git push`, and `jj-gt`/`gt submit` (incl. the
`ss`/`s` aliases) (`push-guard.ts:21-22`):

```ts
const PUSH =
  /\bgit(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+push\b|\bjj(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+git\s+push\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*(?:submit|ss?)\b/;
```

`PUSH_MAIN`'s final alternative is anchored to `push`, so a later
`git checkout main` in a compound command is not a false positive
(`push-guard.ts:53`):

```ts
const PUSH_MAIN = /:(?:refs\/heads\/)?main(?![\w-])|\brefs\/heads\/main(?![\w-])|(?:^|\s)(?:-b|--bookmark|--branch)\s+main(?![\w-])|\bpush\b(?:\s+-\S+)*\s+[\w.-]+\s+(?:-\S+\s+)*main(?![\w-])/;
```

The trailing `(?![\w-])` on every alternative prevents a false positive
when `main` is only a prefix of a longer branch name. `PUSH_UPSTREAM`
catches the bare `upstream` remote, which carries no URL/owner
(`push-guard.ts:49` — `const PUSH_UPSTREAM = /\bpush\b(?:\s+-\S+)*\s+upstream\b/;`).

#### Scenario: Block every explicit `main` push target

- **Given** a push command naming `main` as the destination ref.
- **When** the guard evaluates it.
- **Then** it SHALL return a block. The suite enumerates the forms, all
  asserting `?.block === true` (`push-guard.test.ts:15-23`):
  `git push origin main`, `git push -f origin main`, `jj git push -b main`,
  `git push origin HEAD:main`, `git push origin feature:refs/heads/main`,
  `git push origin HEAD:refs/heads/main`, and
  `git push origin refs/heads/main`.
- **And** the reason names the policy
  (`push-guard.ts:232` — `"Push to \`main\` blocked: never push or`
  `force-push \`main\`. Push a feature branch "`).

#### Scenario: Block a `main` push to any remote, not just origin

- **Given** a push that names `main` on a non-`origin` remote.
- **When** the guard evaluates it.
- **Then** it SHALL block via the `<remote> main` arm — asserted
  `?.block === true` for `git push fork main` and
  `git push -f myremote main` (`push-guard.test.ts:93-94`).
- **And** a feature branch on the same remote is still allowed —
  `git push fork main-feature` returns `null`, because the `(?![\w-])`
  lookahead fails on the trailing `-feature` (`push-guard.test.ts:95`).

#### Scenario: Allow a later `checkout main` in a compound command

- **Given** a compound command whose push targets a feature branch and a
  later stage runs or echoes `main`.
- **When** the guard evaluates it.
- **Then** it SHALL return `null`, because the bare-`main` arm is
  anchored to `push` — asserted `toBeNull()` for
  `git push origin feature && git checkout main` and
  `git push origin feature && echo main` (`push-guard.test.ts:126-127`).
- **And** a real `main` push in the same compound command still blocks —
  `git push fork main && echo done` (`push-guard.test.ts:128`,
  `?.block === true`).

#### Scenario: Allow a feature branch that merely contains "main"

- **Given** a push/submit whose branch name embeds `main` as a
  substring.
- **When** the guard evaluates it.
- **Then** it SHALL return `null` (allow) — asserted `toBeNull()` for
  `jj-gt submit -b cook-sea-1-main-nav`, `git push origin feat-main-menu`,
  `git push origin main-feature`, and `jj-gt submit -b main-nav`
  (`push-guard.test.ts:26-29`).
- **And** ordinary feature pushes are likewise allowed —
  `jj-gt submit -b cook-compass-scaffold`,
  `jj git push -b hudson-sea-865-aws-provider`,
  `git push origin amundsen-fnm-direnv-path`,
  `git push --force origin cook-feature-restack`
  (`push-guard.test.ts:9-12`).

#### Scenario: Push to the `upstream` remote is blocked

- **Given** `git push upstream my-branch` (an OSS-upstream vector whose
  bare `upstream` remote carries no URL/owner).
- **When** the guard evaluates it after `PUSH` matches.
- **Then** it SHALL block via `PUSH_UPSTREAM` — asserted
  `?.block === true` (`push-guard.test.ts:42`); reason at
  `push-guard.ts:224` (`"Push to the \`upstream\` remote blocked: that's`
  `the OSS upstream, outside the "`).
- **And** the owner-allowlist branch does **not** catch it:
  `namedOwners("git push upstream my-branch")` is empty, so `bad` is
  empty and the owner check is skipped.

### Requirement: Owner allowlist on writes

Any write that names an owner outside `ALLOWED_OWNERS` SHALL be blocked,
while reads of any repo (including an OSS upstream) SHALL be allowed. The
block fires when a non-allowlisted owner is named **and** the command is
a push **or** a `gh` write verb (`push-guard.ts:186-187`):

```ts
const bad = namedOwners(cmd).filter((o) => ALLOWED_OWNERS[o] !== true);
if (bad.length && (PUSH.test(cmd) || GH_WRITE_CMD.test(cmd))) {
```

`GH_WRITE_CMD` matches a write verb anywhere in the `gh` segment, so
flags between the noun and the verb (`gh pr -R owner/repo create`) do not
slip past (`push-guard.ts:28-29`):

```ts
const GH_WRITE_CMD =
  /\bgh\b[^\n;|&]*\b(?:create|edit|comment|close|delete|review|reopen|ready|lock|unlock|develop|transfer|rename|sync|fork)\b/;
```

Owners are extracted by `namedOwners()` from two sources, lowercased — a
full URL (`github.com/owner/repo` or scp-style `github.com:owner/repo`)
and a `-R`/`--repo owner/repo` selector (host prefix tolerated)
(`push-guard.ts:58-59`):

```ts
for (const m of cmd.matchAll(/github\.com[/:]([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
for (const m of cmd.matchAll(/(?:-R|--repo)[=\s]+["']?(?:[\w.-]+\/)?([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
```

A `gh api` endpoint's `repos/<owner>/` path is **not** read here; that
case has its own branch (see "`gh api` write blocking"). A companion
extractor `repoFlagOwner()` — used by the create guard below — is
token-aware, so a URL or `--repo` string sitting inside a
`--body`/`--title` value is not read as a target
(`push-guard.ts:95-104`).

#### Scenario: Block a write to a non-allowlisted owner

- **Given** a push or `gh` write that names an owner not in the
  allowlist (e.g. `can1357`).
- **When** the guard evaluates it.
- **Then** it SHALL block — asserted `?.block === true` for
  `git push https://github.com/can1357/oh-my-pi main`,
  `gh pr create -R can1357/oh-my-pi`,
  `gh issue create -R can1357/oh-my-pi -t bug`, and the
  flags-between-noun-and-verb form `gh pr -R can1357/oh-my-pi create`
  (`push-guard.test.ts:39-41,43`; line 42 is the `upstream` case above).
- **And** the `gh pr -R … create` case proves the write verb is matched
  anywhere in the `gh` segment, not only directly after the noun.

#### Scenario: Allow writes to an allowlisted owner, and any read

- **Given** a write that names `mattwilkinsonn` or `sealedsecurity`, or
  any read.
- **When** the guard evaluates it.
- **Then** it SHALL return `null` — asserted `toBeNull()` for
  `git push https://github.com/mattwilkinsonn/zireael my-branch`,
  `gh pr create -R sealedsecurity/sealed`, and the upstream read
  `gh pr view -R can1357/oh-my-pi 42` (`push-guard.test.ts:69-71`).
- **And** the upstream-read case holds because `gh … view` is not in the
  `GH_WRITE_CMD` verb set, so a named non-allowlisted owner alone does
  not block.

### Requirement: `gh api` write blocking

A `gh api` mutation to a `repos/<owner>/<repo>` endpoint SHALL be blocked
unless it names an allowlisted owner, SHALL be fail-closed on an unparsed
or placeholder owner, and an explicit `-X GET`/`--method GET` SHALL be
treated as a read and allowed. Detection keys off the parsed command
token via `ghArgs(seg)?.includes("api")`, so it is robust to flag order
**and** a `gh api` quoted in a commit message is not a false positive
(`push-guard.ts:36-46`):

```ts
function ghApiWriteBlocked(cmd: string): boolean {
  for (const seg of cmd.split(/[\n;|&]+/)) {
    if (!ghArgs(seg)?.includes("api")) continue; // a real `gh api` command (any flag order), not text in a message
    if (/(?:^|\s)(?:-X|--method)[=\s]+GET\b/i.test(seg)) continue; // explicit read
    const write = /(?:^|\s)(?:-X|--method)[=\s]+(?:POST|PUT|PATCH|DELETE)\b|(?:^|\s)(?:-f|-F|--field|--raw-field|--input)\b/i.test(seg);
    if (!write) continue;
    const m = seg.match(/\brepos\/([^/\s]+)\//);
    if (m && ALLOWED_OWNERS[m[1].toLowerCase()] !== true) return true;
  }
  return false;
}
```

A write is a write HTTP method (`-X`/`--method POST|PUT|PATCH|DELETE`) or
a body flag (`-f`/`-F`/`--field`/`--raw-field`/`--input`, which flip the
request to POST). When the `repos/<owner>/` owner is missing or a
placeholder, the `m && ALLOWED_OWNERS[…] !== true` test still returns
`true` — fail-closed (`push-guard.ts:43`). The block reason
(`push-guard.ts:201-202`) reads ``a write to a `repos/<owner>/<repo>` endpoint must name an allowlisted owner``.

> **Known gap.** A `bash -c '…'` / `sh -c` wrapper is not unwrapped:
> `ghArgs()` tokenizes the literal segment, so a `gh api` write hidden
> inside such a wrapper is not seen. This gap is shared with the other
> tokenized guards
> (`push-guard.ts:35` — `// bash -c '…' wrapper is a known gap, shared`
> `with the other tokenized guards.`).

#### Scenario: Block a `gh api` write; allow reads and allowlisted writes

- **Given** a `gh api` mutation on an upstream `repos/<owner>/<repo>`
  endpoint.
- **When** the guard evaluates it.
- **Then** it SHALL block — asserted `?.block === true` for
  `gh api repos/can1357/oh-my-pi/issues -f title=x` and
  `gh api -X POST repos/can1357/oh-my-pi/pulls`
  (`push-guard.test.ts:107-108`).
- **And** a GET read and an allowlisted write are allowed —
  `gh api repos/can1357/oh-my-pi/issues` and
  `gh api repos/mattwilkinsonn/zireael/issues -f title=x` return `null`
  (`push-guard.test.ts:109-110`).

#### Scenario: Fail-closed on an unparsed owner, exempt an explicit GET

- **Given** a `gh api` write with a placeholder owner, an explicit GET,
  or an allowlisted owner.
- **When** the guard evaluates it.
- **Then** the placeholder SHALL block and the GET / allowlisted writes
  SHALL allow — asserted in `push-guard.test.ts:137-141`:
  `gh api repos/{owner}/{repo}/issues -f title=x` blocks (placeholder
  owner); `gh api -X GET repos/can1357/oh-my-pi/issues -f per_page=100`
  returns `null` (the explicit GET overrides the `-f` body flag);
  `gh api repos/mattwilkinsonn/zireael/issues -f title=x` returns `null`
  (allowlisted).

#### Scenario: Detect a `gh api` write regardless of flag order

- **Given** a `gh api` write with a global flag before the `api`
  subcommand.
- **When** the guard evaluates it.
- **Then** it SHALL block, because `ghArgs()` parses past the global
  flags and `.includes("api")` finds the subcommand token — asserted
  `?.block === true` for
  `gh --hostname example.com api repos/can1357/oh-my-pi/issues -f title=x`
  and `gh -R can1357/oh-my-pi api repos/can1357/foo -f x=y`
  (`push-guard.test.ts:149-152`).

#### Scenario: Ignore a `gh api` write quoted in a commit message

- **Given** a `git commit` whose message text merely quotes a
  `gh api … -f …` write.
- **When** the guard evaluates it.
- **Then** it SHALL return `null`, because `ghArgs()` returns `null` when
  the command token is not `gh` — asserted `toBeNull()` for
  `git commit -m "note: use gh api repos/can1357/oh-my-pi/issues -f title=x"`
  and `git commit -m "fix: gh api -X POST repos/can1357/foo/bar"`
  (`push-guard.test.ts:144-145`).
- **And** a real `gh api` write on the same owner still blocks —
  `gh api repos/can1357/oh-my-pi/issues -f title=x`
  (`push-guard.test.ts:146`).

### Requirement: Bare `gh` create needs an allowlisted -R

A `gh issue create` / `gh pr create` (or the `gh issue new` alias) with
no allowlisted `-R` SHALL be blocked, because a bare create files on
whatever repo the cwd resolves to — an OSS upstream included. This is
fail-closed: absence of an allowlisted target is a block
(`push-guard.ts:112-125`). The create verb must be the token
**immediately after** the `issue`/`pr` noun, and the command is allowed
only when a real `-R`/`--repo` selector resolves to an allowlisted owner:

```ts
const noun = args.findIndex((a) => a === "issue" || a === "pr");
if (noun === -1) continue;
const verb = args[noun + 1];
if (verb !== "create" && verb !== "new") continue;
const owner = repoFlagOwner(seg);
if (owner !== null && ALLOWED_OWNERS[owner] === true) continue; // explicit allowlisted target
return true;
```

`ghArgs()` finds the `gh` invocation behind benign wrappers and returns
its args, or `null` when `gh` is not the command — so `gh` appearing only
inside a commit message is ignored
(`push-guard.ts:89` — `return i < toks.length && /(?:^|\/)gh$/.test(toks[i]) ? toks.slice(i + 1) : null;`).
Recognised wrappers (`push-guard.ts:67`):

```ts
const GH_WRAPPERS = new Set(["env", "timeout", "nice", "ionice", "stdbuf", "nohup", "setsid", "sudo", "doas", "command", "exec", "time"]);
```

The wrapper skipping also unwraps `direnv exec <dir>` — `direnv`, then
`exec`, then a non-flag DIR positional, then direnv's own flags — so a
`gh` behind a direnv shim is still seen (`push-guard.ts:75-81`):

```ts
} else if (base === "direnv") {
  i++; // direnv
  if (toks[i] === "exec") {
    i++; // exec
    if (i < toks.length && !/^-/.test(toks[i])) i++; // the DIR positional
  }
  while (i < toks.length && /^-/.test(toks[i])) i++; // direnv's own flags
```

`repoFlagOwner()` is token-aware: it walks adjacent token pairs and only
honours a `-R`/`--repo` that is its own token (or a `--repo=…` form), so
a `--repo …` string sitting inside a `--body`/`--title` value is not
treated as a real target (`push-guard.ts:95-104`):

```ts
for (const tok of seg.split(/\s+/).map((t, i, a) => [t, a[i + 1]] as const)) {
  const eq = tok[0].match(/^(?:-R|--repo)=(.+)$/);
  const val = eq ? eq[1] : tok[0] === "-R" || tok[0] === "--repo" ? tok[1] : undefined;
  if (val === undefined) continue;
  const o = val.replace(/^["']/, "").match(/^(?:[\w.-]+\/)?([\w.-]+)\/[\w.-]+/);
  if (o) return o[1].toLowerCase();
}
```

#### Scenario: Block a bare create with no allowlisted target

- **Given** a create with no `-R`, or a non-allowlisted target, possibly
  behind a wrapper.
- **When** the guard evaluates it.
- **Then** it SHALL block — asserted `?.block === true` for
  `gh issue create --title bug --body repro`, `gh pr create --fill`,
  `cd /tmp/upstream && gh issue create -t spam`, the alias
  `gh issue new --title bug`, the wrapped
  `env GH_TOKEN=x gh issue create -t bug` and
  `timeout 30 gh pr create --fill`, and
  `gh issue create --body https://github.com/mattwilkinsonn/zireael`
  (`push-guard.test.ts:47-53`).
- **And** the last case proves a URL in `--body` is **not** a target:
  `repoFlagOwner()` only honours a real `-R`/`--repo` token, so the
  create is still blocked even though the URL owner is allowlisted.

#### Scenario: A `--repo` inside a flag value is not a real target

- **Given** `gh issue create --body "--repo mattwilkinsonn/zireael"`,
  where `--repo` is text inside the `--body` value.
- **When** the guard evaluates it.
- **Then** it SHALL block (`push-guard.test.ts:121`, `?.block === true`):
  token-aware `repoFlagOwner()` returns `null` because `--repo` is not
  its own argument token, so the bare-create rule fires even though the
  looser `namedOwners()` would extract the allowlisted `mattwilkinsonn`.
- **And** a real selector is honoured —
  `gh issue create --repo mattwilkinsonn/zireael -t bug` returns `null`
  (`push-guard.test.ts:122`).

#### Scenario: See `gh` through a `direnv exec` wrapper

- **Given** a `gh` create or read behind `direnv exec <dir>`.
- **When** the guard evaluates it.
- **Then** the wrapper SHALL be skipped and the `gh` command judged on
  its merits — asserted `?.block === true` for the bare
  `direnv exec /home/x gh issue create -t bug` and
  `direnv exec /repo gh pr create -R can1357/oh-my-pi`
  (`push-guard.test.ts:114-115`).
- **And** reads / allowlisted creates behind the same wrapper return
  `null` — `direnv exec /repo gh pr view -R can1357/oh-my-pi 42` and
  `direnv exec /repo gh issue create -R mattwilkinsonn/zireael -t bug`
  (`push-guard.test.ts:116-117`).

#### Scenario: Allow a create with an allowlisted `-R`, and non-creates

- **Given** a create whose `-R`/`--repo` resolves to an allowlisted
  owner, or a `gh` command whose token after the noun is not a create
  verb, or `gh` appearing only inside a message.
- **When** the guard evaluates it.
- **Then** it SHALL return `null` — asserted `toBeNull()` for
  `gh issue create -R mattwilkinsonn/zireael -t bug`,
  `gh pr create -R sealedsecurity/sealed --fill`, the quoted
  `gh pr create -R "sealedsecurity/sealed" --fill`, the host-qualified
  `gh issue create --repo github.com/mattwilkinsonn/zireael -t bug`, the
  flag-value non-verbs `gh issue list --label create` and
  `gh pr comment 123 --body create`, plain `gh issue list`, and the
  message-only `git commit -m "note: block bare gh issue create"` /
  `jj describe -m "gh pr create guard"` (`push-guard.test.ts:57-65`).

### Requirement: PR merge is the human gate

Merging a PR SHALL always be blocked — the agent never merges, on any
repo, including allowlisted ones (`push-guard.ts:23` — `// Merge — the`
`human gate, blocked for every repo.`). For command tools this is the
`MERGE` regex, checked before the owner and push branches
(`push-guard.ts:177` — `if (MERGE.test(cmd)) {`):

```ts
const MERGE = /\bgh\b[^\n;|&]*\bmerge\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*merge\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*(?:submit|ss?)\b[^\n;|&]*(?:--merge-when-ready\b|\s-m\b)/;
```

Three arms (`push-guard.ts:25`): a `merge` anywhere in a `gh` segment, a
`jj-gt`/`gt … merge`, and a `jj-gt`/`gt … submit`/`ss`/`s` carrying
`--merge-when-ready` or `-m`, which hands Graphite the merge after checks
and so is treated as a merge.

#### Scenario: Block `gh` and `jj-gt` merges from the command line

- **Given** a merge command.
- **When** the guard evaluates it.
- **Then** it SHALL block — asserted `?.block === true` for
  `gh pr merge 123 --squash`, `jj-gt merge`, and — even with an
  allowlisted `-R` between noun and verb —
  `gh pr -R sealedsecurity/sealed merge 123 --squash`
  (`push-guard.test.ts:33-35`). The last case shows merge is
  owner-agnostic and is decided before the owner-allowlist branch.

#### Scenario: Block a `submit` that hands Graphite the merge

- **Given** a `jj-gt`/`gt submit` with `--merge-when-ready` or `-m`.
- **When** the guard evaluates it.
- **Then** it SHALL block via the third `MERGE` arm — asserted
  `?.block === true` for `jj-gt submit -b foo --merge-when-ready`,
  `jj-gt submit -b foo -m`, and `gt submit --merge-when-ready`
  (`push-guard.test.ts:99-101`).
- **And** a plain submit is still allowed — `jj-gt submit -b foo` and
  `jj-gt submit -b foo --no-hooks` return `null`
  (`push-guard.test.ts:102-103`).

#### Scenario: Block merge-when-ready on the `gt ss`/`gt s` aliases

- **Given** a `gt ss` or `gt s` submit alias carrying `-m` or
  `--merge-when-ready`.
- **When** the guard evaluates it.
- **Then** it SHALL block — asserted `?.block === true` for `gt ss -m`,
  `gt ss --merge-when-ready`, and `gt s --merge-when-ready`
  (`push-guard.test.ts:132-134`).

#### Scenario: Block a GitHub-MCP merge regardless of owner

- **Given** a GitHub-MCP tool whose name contains `merge`, even with an
  allowlisted owner.
- **When** the guard evaluates it.
- **Then** it SHALL block before the write/owner check
  (`push-guard.ts:241` — `if (/merge/.test(toolName)) {`) — asserted
  `?.block === true` for `mcp__github_merge_pull_request` with
  `{ owner: "sealedsecurity", repo: "sealed" }`
  (`push-guard.test.ts:83`).

### Requirement: Broad process-kill protection

A broad, pattern-matching process kill SHALL be blocked on locally
executing tools, because it can take down the session's own runtime or
unrelated work; a kill targeting a specific PID SHALL be allowed. The
check runs only for `LOCAL_TOOLS`
(`push-guard.ts:167` — `if (LOCAL_TOOLS[toolName] === true && hasBroadKill(cmd)) {`)
via `hasBroadKill()` (`push-guard.ts:132-145`).

`pkill` / `killall` are always pattern-based, so any occurrence is broad
(`push-guard.ts:137` — `if (!/(?:^|\/)kill$/.test(toks[idx])) return true; // pkill / killall`).
For `kill`, the leading signal spec is skipped and the command is broad
only when a remaining target is negative — a process group or everything
(`push-guard.ts:139-142`):

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
  `?.block === true`). The reason points at the rule
  (`push-guard.ts:171` — `"Broad process kill blocked (pkill / killall /`
  `kill -1)."`).

#### Scenario: Allow a targeted kill by PID

- **Given** `kill -9 12345` (SIGKILL to one explicit PID).
- **When** the guard evaluates it.
- **Then** it SHALL return `null` — the signal flag `-9` is skipped and
  `12345` is not a negative target (`push-guard.test.ts:89`,
  `toBeNull()`).

### Requirement: GitHub-MCP write operations

A GitHub-MCP write tool SHALL be owner-checked and fail-closed: the owner
is read from `input.owner`, and a missing or non-string owner collapses
to `""`, which is not in the allowlist and is therefore blocked
(`push-guard.ts:250-251`):

```ts
const owner = typeof input.owner === "string" ? input.owner.toLowerCase() : "";
if (ALLOWED_OWNERS[owner] !== true) {
```

Write tools are identified by `GH_MCP_WRITE`, whose verbs are anchored to
the `mcp__github_` prefix so the shared `pull_request_` infix does not
misclassify a read (`push-guard.ts:158`):

```ts
const GH_MCP_WRITE = /^mcp__github_(?:create|update|delete|add|fork|push|dispatch|request|merge)_|_write\b/;
```

Anything else under the `mcp__github_` prefix (e.g. `pull_request_read`,
`get_*`) is a read and is allowed on any repo, including an upstream PR
being triaged (`push-guard.ts:261` — `return null;`).

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
  the prefix (`push-guard.test.ts:77`, `push-guard.ts:158`).

## Regex reference

| Constant | What it matches | Location |
| --- | --- | --- |
| `PUSH` | `git … push`, `jj … git push`, or `jj-gt`/`gt … submit`/`s`/`ss` | `push-guard.ts:21-22` |
| `MERGE` | `gh … merge`, `jj-gt`/`gt … merge`, or `jj-gt`/`gt … submit`/`ss`/`s` with `--merge-when-ready`/`-m` | `push-guard.ts:25` |
| `GH_WRITE_CMD` | a `gh` segment containing a write verb (`create`/`edit`/`comment`/`close`/`delete`/…) | `push-guard.ts:28-29` |
| `PUSH_UPSTREAM` | `push … upstream` (a remote literally named `upstream`) | `push-guard.ts:49` |
| `PUSH_MAIN` | `main` as a push target: `:main`, `refs/heads/main`, `-b main`, or `push … <remote> main` on any remote | `push-guard.ts:53` |
| `GH_WRAPPERS` | benign wrappers `ghArgs()` skips (`env`, `timeout`, `sudo`, …) | `push-guard.ts:67` |
| `GH_MCP` | tool-name prefix `mcp__github_` | `push-guard.ts:153` |
| `GH_MCP_WRITE` | MCP write verbs anchored to the prefix, or a `_write` suffix | `push-guard.ts:158` |

Helper extractors (not constants) and their roles:

| Function | Role | Location |
| --- | --- | --- |
| `ghApiWriteBlocked()` | true if a `gh api` write hits a non-allowlisted / unparsed `repos/<owner>/` endpoint | `push-guard.ts:36-46` |
| `namedOwners()` | owners from `github.com/owner/repo` URLs and `-R owner/repo` selectors | `push-guard.ts:56-61` |
| `ghArgs()` | args after `gh`, skipping wrappers/assignments/`direnv exec <dir>`, else `null` | `push-guard.ts:68-90` |
| `repoFlagOwner()` | token-aware owner from a real `-R`/`--repo` selector, lowercased | `push-guard.ts:95-104` |
| `ghCreateWithoutAllowedOwner()` | true if a bare `gh` create lacks an allowlisted `-R` | `push-guard.ts:112-125` |
| `hasBroadKill()` | true for `pkill`/`killall` or `kill` with a negative target | `push-guard.ts:132-145` |

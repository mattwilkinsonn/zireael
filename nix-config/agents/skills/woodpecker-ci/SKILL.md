---
name: woodpecker-ci
description: "Woodpecker CI CLI (woodpecker-cli) for reading pipelines and step logs on ci.sealedsecurity.com: list/show/steps/logs, filter to failures, decode a GitHub check details_url into the exact log command."
---

# Woodpecker CI (`woodpecker-cli`)

Read CI pipelines and step logs straight from the terminal against
`ci.sealedsecurity.com` (the Woodpecker server that runs `sealedsecurity/sealed`'s
`moon ci`). Reach for this when a PR check is red and you want the *actual* failure
output — the GitHub check UI only links back to Woodpecker, so pulling the log here
is faster than clicking through, and it's scriptable.

The binary is `woodpecker-cli` (installed on Matt's machines). It talks to a server
over the HTTP API — there is nothing to run locally and no repo checkout needed.

## Setup (already done on Matt's machines)

`woodpecker-cli` authenticates from two env vars, both already exported in the
shell:

- `WOODPECKER_SERVER` — `https://ci.sealedsecurity.com`
- `WOODPECKER_TOKEN` — the user API token (currently the `seal-agent` user)

Confirm auth with `woodpecker-cli info` (prints the authenticated user). If the
vars are somehow missing, pass `--server`/`--token` as global flags, or run
`woodpecker-cli setup`. Everything below assumes the env vars are set, so no repo
checkout or `direnv` shell is required — the CLI is on `PATH` directly.

Every command takes a repo as `<repo-id|repo-full-name>`: use the full name
`sealedsecurity/sealed` (its numeric id is `2`).

## The read loop: pipeline → steps → logs

Three commands, in order. This is the whole workflow for "why did CI fail".

### 1. Find the pipeline

```sh
# Recent history (default limit 25). Trim the giant MESSAGE column with --output.
woodpecker-cli pipeline ls --output 'table=NUMBER,STATUS,EVENT,BRANCH' sealedsecurity/sealed

# Only failures — the fast path to a broken run:
woodpecker-cli pipeline ls --status failure --output 'table=NUMBER,STATUS,BRANCH' sealedsecurity/sealed

# Filter by branch/event; widen/limit the window:
woodpecker-cli pipeline ls --branch main --event pull_request --limit 10 sealedsecurity/sealed

# Latest pipeline on a branch (defaults to main):
woodpecker-cli pipeline last --branch main sealedsecurity/sealed
```

`--status` values are Woodpecker states: `success`, `failure`, `running`,
`pending`, `killed`, `blocked`, `declined`. `--before`/`--after` take RFC3339
timestamps.

### 2. Inspect the steps

```sh
# One block per step (verbose): name, PID, timestamps, State.
woodpecker-cli pipeline ps sealedsecurity/sealed <pipeline>

# JUST the failed/running steps, one per line — the practical filter. `ps`
# takes a Go-template --format over {{ .workflow }} / {{ .step }}:
woodpecker-cli pipeline ps sealedsecurity/sealed <pipeline> \
  --format '{{ .step.State }} {{ .step.Name }} (#{{ .step.PID }})' | grep -E '^(failure|running)'
```

Each step has a **name** (e.g. `root-shfmt`, `ci-test`) and a numeric **PID**
(e.g. `143`). Either identifies the step to `log show`. `pipeline show <pipeline>`
gives the one-line pipeline summary (status, event, branch, author).

### 3. Pull the step log

```sh
# By step name OR step number — both work. Logs are LARGE (a single moon-ci
# step can be >6000 lines / >150 KB), so ALWAYS pipe to grep/tail — never dump raw.
woodpecker-cli pipeline log show sealedsecurity/sealed <pipeline> root-shfmt | tail -40
woodpecker-cli pipeline log show sealedsecurity/sealed <pipeline> 143 | grep -iE 'error|fail|✗' 

# Omit the step to stream every step's log concatenated (rarely what you want).
woodpecker-cli pipeline log show sealedsecurity/sealed <pipeline>
```

The moon task failure line is near the end of the log — e.g.
`Error: task_runner::run_failed … Process <tool> failed: exit code N` — so
`tail` first, then `grep` up for the specific diagnostic.

## From a GitHub PR check to the log (the bridge)

The PR-review skills land here from a red check. A Woodpecker check's `details_url`
(surfaced by `mcp__litellm_github_pull_request_read get_check_runs`, or `gh pr
checks`) encodes everything you need:

```text
https://ci.sealedsecurity.com/repos/2/pipeline/911/11
                                    │        │       │
                                 repo id  pipeline  step index
```

Repo id `2` is `sealedsecurity/sealed`. So that URL is:

```sh
woodpecker-cli pipeline log show sealedsecurity/sealed 911 11 | tail -60
```

You don't have to parse the step index out of the URL — `pipeline ps <pipeline>`
plus the failure filter (step 2 above) names the failed steps directly, which is
usually clearer than the raw index.

## Output formatting

- `--output 'table=COL,COL,…'` selects columns (`NUMBER STATUS EVENT BRANCH
  MESSAGE AUTHOR CREATED STARTED FINISHED`). Do this on `pipeline ls` — the raw
  table dumps the entire commit body in `MESSAGE` and wrecks your terminal.
- `--output-no-headers` drops the header row for `awk`/`cut` piping.
- **`--output json` is silently ignored** by `pipeline ls/show` and `repo ls` on
  the current CLI (v3.16) — it prints the table anyway. For machine-readable
  output use `--output 'table=…' --output-no-headers`, or the `ps --format`
  Go-template, and parse those.

## Scope / boundaries

- **Read-only in practice.** Getting pipelines and logs is the job. The CLI *can*
  mutate (`pipeline start/stop/approve/decline/create`, `pipeline purge`, `repo`,
  `secret`, `cron`, `registry`, `admin`) — do **not** run those against
  `sealedsecurity/sealed` without Matt's explicit go-ahead. Re-running CI is the
  merge queue's / Matt's call, and secret/registry/admin ops touch shared infra.
- **Never print secrets.** `WOODPECKER_TOKEN` is a live credential; don't echo it,
  and don't `secret ls`/`registry` dump into the transcript.
- `woodpecker-cli exec` runs a `.woodpecker/*.yaml` workflow locally against a
  container backend — not relevant to sealed (which drives CI through `moon`), so
  ignore it unless a repo actually ships Woodpecker workflow files.

## Command reference (read paths)

| Command | Purpose |
| --- | --- |
| `info` | Show the authenticated user (auth check). |
| `repo ls` | List repos the token can see. |
| `pipeline ls <repo>` | Pipeline history; `--status`/`--branch`/`--event`/`--limit`/`--before`/`--after`. |
| `pipeline last <repo>` | Latest pipeline on a branch (`--branch`, default `main`). |
| `pipeline show <repo> <n>` | One-line summary of pipeline `n`. |
| `pipeline ps <repo> <n>` | Steps of pipeline `n` (`--format` Go-template). |
| `pipeline log show <repo> <n> [step]` | Step log by name or PID (pipe to grep/tail). |
| `pipeline queue` | Server-wide enqueued pipelines (takes no repo arg). |

`woodpecker-cli <group> --help` lists every subcommand; `--help` on a subcommand
shows its flags.

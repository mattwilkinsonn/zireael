<!-- markdownlint-disable-next-line MD033 MD041 -->
<div align="center">

# akiflow-cli

**Command-line interface for [Akiflow](https://akiflow.com) task management**

[![CI](https://img.shields.io/github/actions/workflow/status/mattwilkinsonn/akiflow-cli/test.yml?branch=main&labelColor=black&style=flat-square&logo=github&label=tests)](https://github.com/mattwilkinsonn/akiflow-cli/actions/workflows/test.yml)
[![License](https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square)](https://github.com/mattwilkinsonn/akiflow-cli/blob/main/LICENSE)

</div>

---

Bun-native CLI for managing Akiflow tasks directly from the terminal. TypeScript, [citty](https://github.com/unjs/citty) for the CLI surface, compiles to a standalone `af` binary.

> **Fork notice.** Maintained at [github.com/mattwilkinsonn/akiflow-cli](https://github.com/mattwilkinsonn/akiflow-cli), diverged from [code-yeongyu/akiflow-cli](https://github.com/code-yeongyu/akiflow-cli) on 2026-05. Major additions: local sync cache with delta fetch, rich `ls` filtering (date / bucket / connector / status), unified `cal` view (events + slots + scheduled tasks), stable cleaned `--json` output (with `--raw` escape hatch), structured auth-extraction logging, and `af doctor` diagnostic command. MIT license preserved from upstream.

## Features

- **Task management** — list, add, complete, edit, move, plan, snooze, delete
- **Rich filtering** — by date range (`--today` / `--this-week` / `--from`/`--to`), bucket (`--bucket week`), status (`--inbox` / `--done` / `--trashed`), connector (`--connector gmail|linear`), tag, project, priority, recurring
- **Unified calendar** — `af cal` merges events + time-slots + scheduled tasks into one timeline, with date-range filters and per-source toggles (`--no-events` / `--no-tasks` / `--no-slots`)
- **Stable JSON output** — `--json` emits a cleaned shape we own (status word, plan-bucket as `YYYY-Www`, source object with `thread_id` for Gmail, etc.). `--raw` emits the unmodified API record.
- **Local sync cache** — JSONL store at `~/.cache/af/` with sync_token-based delta fetch and tombstone application. `af refresh` triggers explicit sync; `af refresh --rebuild` full-resyncs.
- **Diagnostics** — `af doctor` reports credentials, browser sources, cache state, and API health
- **Structured logging** — set `AF_LOG=1` to write JSON Lines to `~/.cache/af/{af,auth}.log`. Surfaces extract-token schema rotations clearly.
- **Natural-language dates** — "tomorrow", "next friday", "in 2 hours" (via chrono-node)
- **Short ID system** — `af do 1` instead of full UUIDs
- **Shell completions** — Bash, Zsh, Fish
- **Headless-friendly auth** — credentials.json can be supplied externally (no browser required at runtime); browser extraction is the first-time setup path

## Installation

### Prerequisites

- [Bun](https://bun.sh) v1.0+

### From source

```bash
git clone https://github.com/mattwilkinsonn/akiflow-cli.git
cd akiflow-cli
bun install
bun run build

# Move to PATH
install -m 0755 ./af ~/.local/bin/af   # or sudo install … /usr/local/bin/af
```

### Via Nix (home-manager)

A home-manager activation that clones + builds `af` on every `nix-switch`. Drop-in:

```nix
home.activation.installAkiflowCli = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
  AF_SRC="$HOME/.local/src/akiflow-cli"
  mkdir -p "$HOME/.local/src" "$HOME/.local/bin"
  if [ ! -d "$AF_SRC/.git" ]; then
    ${pkgs.git}/bin/git clone --depth=1 \
      https://github.com/mattwilkinsonn/akiflow-cli.git "$AF_SRC"
  else
    ${pkgs.git}/bin/git -C "$AF_SRC" fetch --depth=1 origin main
    ${pkgs.git}/bin/git -C "$AF_SRC" reset --hard origin/main
  fi
  HEAD_SHA=$(${pkgs.git}/bin/git -C "$AF_SRC" rev-parse HEAD)
  if [ -x "$HOME/.local/bin/af" ] \
     && [ -f "$AF_SRC/.last-built-sha" ] \
     && [ "$(cat "$AF_SRC/.last-built-sha")" = "$HEAD_SHA" ]; then
    exit 0
  fi
  (cd "$AF_SRC" && ${pkgs.bun}/bin/bun install && ${pkgs.bun}/bin/bun run build)
  install -m 0755 "$AF_SRC/af" "$HOME/.local/bin/af"
  echo "$HEAD_SHA" > "$AF_SRC/.last-built-sha"
'';
```

## Authentication

First-time setup (requires a desktop browser logged into Akiflow):

```bash
af auth
```

Extracts the session token from one of Chrome, Arc, Brave, Edge, Safari — IndexedDB JWT pattern matching first, encrypted cookie fallback. Stored at `~/.config/af/credentials.json` (mode 0600).

Check status:

```bash
af auth status   # Status: valid | EXPIRED | not authenticated
af doctor        # Full diagnostic — see below
```

### Headless / server use

Browser extraction needs a desktop. For headless boxes (Raspberry Pi, cloud VM, etc.) auth once on a desktop, then copy `~/.config/af/credentials.json` to the target host (or pipe it through a secrets manager like 1Password). The CLI doesn't care where the file came from as long as it's present + the JWT is valid.

`af doctor` reports the JWT's `exp` claim so you can see when re-auth is needed without making an API call.

## Usage

### `af ls` — list tasks

```bash
# Defaults: today + overdue, active only (excludes done/trashed)
af ls

# Date ranges
af ls --today
af ls --this-week               # incl. tasks with plan_unit=WEEK
af ls --this-month              # incl. tasks with plan_unit=MONTH
af ls --from 2026-05-21 --to 2026-05-31
af ls --date "next monday"

# State
af ls --inbox                   # unplanned (no date, no bucket)
af ls --done
af ls --trashed
af ls --all                     # inbox + planned + done + trashed
af ls --status planned,done     # explicit comma-separated
af ls --overdue

# Filters
af ls --project Personal
af ls --tag urgent
af ls --priority 1
af ls --connector gmail         # gmail | linear | akiflow | none
af ls --bucket week             # only week-bucketed tasks
af ls --recurring               # only tasks with recurring_id

# Output
af ls --today --json            # cleaned JSON shape (stable contract)
af ls --today --raw             # raw API records
af ls --plain                   # disable ANSI colors
af ls --search "spec"           # text search across title/description/content
```

### `af add` — create tasks

```bash
af add "Review PR" -t                                    # today
af add "Submit report" -d "next friday"
af add "Focus time" -d "tomorrow 10am" --duration 2h
af add "SEA-123 implement X" --project "Sealed"
```

### `af do` — complete tasks

```bash
af do --ids 1                   # by short ID (from last `af ls`)
af do --ids task-uuid-here      # by full UUID
af do --ids 1,2,3               # bulk
```

### `af task` — edit / move / plan / snooze / delete

```bash
af task edit 1 --title "Updated title"
af task move 1 --project "Personal"
af task plan 1 -d "tomorrow"
af task snooze 1 --duration 2h
af task delete 1
```

### `af cal` — unified calendar view

```bash
# Default: today's time slots only (legacy slot-finder mode)
af cal
af cal --free                   # find free time slots

# Merged timeline (events + slots + scheduled tasks) — opt in with any flag
af cal --today
af cal --this-week
af cal --from 2026-05-18 --to 2026-05-24

# Source toggles
af cal --today --no-events      # planning view (slots + tasks only)
af cal --today --no-tasks       # meeting view (events + slots)
af cal --today --no-slots

# Filters
af cal --today --calendar cal_xyz123
af cal --today --connector google    # google | microsoft | icloud
af cal --today --declined            # include declined events
af cal --today --all-day-only

# Output
af cal --today --json
af cal --today --raw
```

### `af refresh` — explicit cache sync

```bash
af refresh                      # delta sync using stored sync_tokens
af refresh --rebuild            # delete cache + full re-sync
af refresh --rebuild --json     # programmatic summary
```

Most invocations also auto-refresh — `af ls`, `af cal`, etc. trigger a delta sync if the cache is older than 24h. Set `AF_NO_AUTO_SYNC=1` to disable.

### `af doctor` — diagnostic report

```bash
af doctor                       # human-readable report
af doctor --json                # structured for scripts / agents
```

Reports:

- Credentials present + JWT `user_id` + expiry timestamp
- Supported browsers + which ones are detected
- Cache state per resource (record count + last sync)
- Live API ping to `/v5/user/settings`

### `af project` — manage projects/labels

```bash
af project ls
af project create "New Project"
af project delete "Old Project"
```

### `af block` — create time blocks

```bash
af block 1h "Deep work"
af block 2h "Meeting prep" --start 14:00
```

### Shell completions

```bash
af completion bash >> ~/.bashrc
af completion zsh  >> ~/.zshrc
af completion fish > ~/.config/fish/completions/af.fish
```

## Configuration

Environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `AF_API_BASE` | `https://api.akiflow.com` | Akiflow v5 API base URL. Overridden by integration tests against a fake server. |
| `AF_CONFIG_DIR` | `~/.config/af` | Credentials directory. |
| `AF_CACHE_DIR` | `~/.cache/af` | Local cache root (JSONL stores + tokens.json + logs). |
| `AF_NO_AUTO_SYNC` | unset | When set, disables the 24h auto-refresh-before-read in `af ls` / `af cal`. |
| `AF_LOG` | unset | When set, writes JSON Lines to `~/.cache/af/af.log` (general) + `auth.log` (extract-token). |
| `AF_DEBUG` | unset | Same as `AF_LOG` + mirrors to stderr in real time. |

## Cache + sync model

The cache layer (`src/lib/cache/`) mirrors Akiflow's v5 sync model:

- **Resources:** `tasks`, `events`, `time_slots`, `labels`, `tags`, `calendars`, `accounts`, `contacts`
- **Storage:** one JSONL file per resource at `~/.cache/af/<resource>.jsonl` (one JSON object per line, append-friendly)
- **Sync tokens:** `~/.cache/af/tokens.json` holds one sync_token per resource. Delta fetches pass the stored token; full rebuilds omit it.
- **Tombstones:** records arriving with `deleted_at != null` (or task `status == 9`) get applied as local deletes
- **Concurrency:** POSIX advisory lock at `~/.cache/af/.lock` serializes invocations. Read + delta-write are atomic per resource.

The cache is per-user. The CLI decodes the JWT's `user_id` claim and discards the cache if it doesn't match (prevents cross-account leakage).

## Architecture

```text
akiflow-cli/
├── src/
│   ├── index.ts                  # CLI entry point
│   ├── commands/
│   │   ├── add.ts                # create tasks
│   │   ├── ls.ts                 # list tasks (extended with date/bucket/status filters)
│   │   ├── do.ts                 # complete tasks
│   │   ├── cal.ts                # unified calendar (events + slots + tasks)
│   │   ├── doctor.ts             # NEW: diagnostic report
│   │   ├── refresh.ts            # NEW: explicit cache sync
│   │   ├── auth.ts               # browser-token extraction flow
│   │   ├── task/index.ts         # edit/move/plan/snooze/delete subcommands
│   │   ├── project.ts            # label/project mgmt
│   │   ├── block.ts              # time-block creation
│   │   ├── cache.ts              # cache admin (legacy from upstream)
│   │   └── completion.ts         # shell completions
│   ├── lib/
│   │   ├── api/                  # AkiflowClient — typed v5 endpoint coverage
│   │   ├── auth/                 # extract-token + credentials storage
│   │   ├── cache/                # NEW: sync orchestrator + JSONL store + tombstones + POSIX lock
│   │   ├── filters/              # NEW: composable task + event filter primitives
│   │   ├── format/               # NEW: cleaned JSON shapes + table formatters
│   │   ├── date-parser.ts        # chrono-node wrapper + named ranges (resolveRange, parseMonth)
│   │   ├── duration-parser.ts
│   │   ├── log.ts                # NEW: structured JSON Lines logger
│   │   └── platform-config.ts    # NEW: cache path resolution
│   └── __tests__/
│       ├── api|auth|commands|lib/     # unit tests
│       └── integration/               # NEW: BDD layer
│           ├── helpers/               # fake server + spawn-cli + test-env + fixtures
│           └── *.integration.test.ts  # ls, cal, add, do, auth, doctor, refresh
└── docs/
    ├── COMMANDS.md
    └── API_INTEGRATION.md
```

## Development

### Setup

```bash
git clone https://github.com/mattwilkinsonn/akiflow-cli.git
cd akiflow-cli
bun install
hk install          # registers the pre-push git hook (one-time per clone)
```

### Day-to-day

```bash
bun run dev          # hot-reload runner
bun run test         # unit + integration (~1.5s total)
bun run test:unit    # unit only (~500ms)
bun run test:integration  # BDD layer: spawns CLI against a fake HTTP server
bun run lint         # biome check (lint + format)
bun run format       # biome check --write
bun run typecheck    # tsc --noEmit
bun run build        # produces standalone `af` binary
```

### Pre-push hook

`hk.pkl` defines a single pre-push step that runs `biome check` (lint + format + organize-imports). tsc and tests intentionally run only in CI / on demand — jj-hooks runs in a temp worktree without `node_modules`, so anything that imports source code fails. CI's `test.yml` does `bun install` + `bun test` + `bunx tsc --noEmit` as the source of truth.

Bypass knobs:

```bash
HK=0 jj git push           # emergency skip
jj git push --no-verify    # respected too
```

### Integration tests

`src/__tests__/integration/` spawns the compiled CLI as a subprocess against an in-memory fake Akiflow HTTP server (`helpers/fake-server.ts`) populated with synthetic fixtures (`fixtures/*.json`). No real network or credentials involved. Each command's behavior is locked by a `.integration.test.ts` file.

To extend coverage: add a fixture (or override the server's responders), run `bun test src/__tests__/integration/<cmd>.integration.test.ts`.

### Discovering new API endpoints

Akiflow's v5 API isn't publicly documented. The current source is the closest thing — `src/lib/api/types.ts` types every record shape; `src/lib/cache/` covers every endpoint observed in the wild. To discover new endpoints, capture a HAR from `web.akiflow.com` (DevTools → Network → Save all as HAR with content), then `jq` the `.log.entries[]` for new paths.

## License

MIT. Original copyright © 2025 [YeonGyu Kim](https://github.com/code-yeongyu). Fork modifications © 2026 Matt Wilkinson. See [LICENSE](./LICENSE).

# AKIFLOW-CLI KNOWLEDGE BASE

**Generated:** 2026-02-02T10:49:00+09:00
**Commit:** 071de5d
**Branch:** master

## OVERVIEW

Bun-native CLI for Akiflow task management. TypeScript, citty framework, compiles to standalone `af` binary.

## STRUCTURE

```
akiflow-cli/
├── src/
│   ├── index.ts              # CLI entry: citty runMain()
│   ├── commands/             # 9 command modules
│   │   ├── add.ts            # Create task (date/project flags)
│   │   ├── ls.ts             # List tasks (saves context to ~/.cache/af/)
│   │   ├── do.ts             # Complete tasks by short ID
│   │   ├── auth.ts           # Browser token extraction flow
│   │   ├── task/index.ts     # Subcommands: edit, move, plan, snooze, delete
│   │   ├── project.ts        # Label management
│   │   ├── cal.ts            # Calendar/timeslots view
│   │   ├── block.ts          # Time blocking
│   │   └── completion.ts     # Shell completions
│   ├── lib/
│   │   ├── api/
│   │   │   ├── client.ts     # AkiflowClient class, createClient() factory
│   │   │   └── types.ts      # ALL types: Task, Label, AuthError, NetworkError
│   │   ├── auth/
│   │   │   ├── storage.ts    # Keychain (macOS) / XDG (Linux) credential storage
│   │   │   └── extract-token.ts  # Browser token extraction (532 lines, most complex)
│   │   ├── browser-paths.ts  # Browser detection & paths
│   │   ├── date-parser.ts    # chrono-node wrapper
│   │   └── duration-parser.ts # "1h", "2d" → ms
│   └── __tests__/            # Mirrors src/ structure
├── docs/
│   ├── COMMANDS.md           # Command reference
│   └── API_INTEGRATION.md    # API overview
└── package.json              # Build: bun build --compile --outfile af
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Add new command | `src/commands/` | Use `defineCommand()` from citty |
| Add subcommand | `src/commands/{parent}/index.ts` | See task/index.ts pattern |
| Modify API calls | `src/lib/api/client.ts` | Add methods to AkiflowClient |
| Add/modify types | `src/lib/api/types.ts` | Central type hub |
| Fix auth issues | `src/lib/auth/` | storage.ts or extract-token.ts |
| Add browser support | `src/lib/browser-paths.ts` + `extract-token.ts` | |
| Date parsing | `src/lib/date-parser.ts` | Uses chrono-node |
| Duration parsing | `src/lib/duration-parser.ts` | Regex: `/^(\d+)\s*([mhdw])$/` |
| Write tests | `src/__tests__/` | Mirror source structure |

## CODE MAP

### Entry Point
- `src/index.ts` → `runMain(main)` with all subcommands registered

### Core Classes
| Symbol | Location | Role |
|--------|----------|------|
| `AkiflowClient` | lib/api/client.ts | HTTP client with lazy credential loading |
| `AuthError` | lib/api/types.ts | 401 errors |
| `NetworkError` | lib/api/types.ts | Connection/parse errors |

### Key Interfaces
| Type | Location | Fields |
|------|----------|--------|
| `Task` | lib/api/types.ts | 45+ fields (id, title, date, done, listId...) |
| `CreateTaskPayload` | lib/api/types.ts | id, title, global_created_at, global_updated_at, date?, duration? |
| `UpdateTaskPayload` | lib/api/types.ts | id, global_updated_at, done?, status?, deleted_at? |
| `Credentials` | lib/auth/storage.ts | token, clientId, expiryTimestamp |

### API Endpoints (via client)
- `getTasks()` → GET /v5/tasks
- `upsertTasks([])` → PATCH /v5/tasks
- `getLabels()` → GET /v5/labels
- `getTags()` → GET /v5/tags
- `getTimeSlots()` → GET /v5/time_slots

## CONVENTIONS

### Command Pattern
```typescript
export const myCommand = defineCommand({
  meta: { name: "cmd", description: "..." },
  args: {
    positionalArg: { type: "positional", required: true },
    flagArg: { type: "boolean", alias: "f" },
  },
  run: async (context) => {
    const client = createClient();
    // ... logic
  },
});
```

### Test Pattern (Given-When-Then)
```typescript
it("description", async () => {
  // given
  fetchSpy.mockResolvedValue(new Response(...));
  
  // when
  await command.run({ args: {...} });
  
  // then
  expect(fetchSpy).toHaveBeenCalled();
});
```

### Error Handling
```typescript
try {
  const response = await client.getTasks();
} catch (error) {
  if (error instanceof Error && error.name === "AuthError") {
    console.error("Error: Authentication failed. Please run 'af auth' to login.");
  } else {
    console.error("Error:", error instanceof Error ? error.message : "Unknown");
  }
  process.exit(1);
}
```

### Platform Detection
```typescript
if (process.platform === "darwin") {
  // macOS: use Bun.secrets (Keychain)
} else {
  // Linux/other: use ~/.config/af/credentials.json
}
```

## ANTI-PATTERNS (THIS PROJECT)

- **NO `as any`** — strict TypeScript, no type suppression
- **NO empty catch blocks** — always handle or rethrow
- **NO `npm`/`npx`** — use `bun`/`bunx` only
- **NO TUI/interactive mode** — CLI-only by design decision
- **NO OAuth flow** — Akiflow doesn't expose public OAuth, use browser token extraction

## UNIQUE STYLES

### Short ID System
- `af ls` saves task context to `~/.cache/af/last-list.json`
- `af do 1` resolves short ID from cached context
- Full UUIDs always work as fallback

### Token Extraction Priority
1. IndexedDB (JWT pattern matching)
2. Cookies (PBKDF2 decryption for Chrome-family, binary for Safari)
3. Fail with helpful message

### Credentials Storage
- macOS: Keychain via `Bun.secrets`
- Linux: XDG `~/.config/af/credentials.json`

## COMMANDS

```bash
# Development
bun run dev          # Hot reload
bun run start        # Run directly
bun test             # Run tests

# Build
bun run build        # Compile to ./af binary

# Usage
af auth              # Extract token from browser
af auth status       # Check auth status
af ls                # List today's tasks
af ls --inbox        # List inbox (unscheduled)
af add "Task" -t     # Add task for today
af add "Task" -d "next friday"
af do 1              # Complete task by short ID
af task plan 1 -d "tomorrow"
af cal               # View today's schedule
af cal --free        # Find free time slots
af block 1h "Focus"  # Create time block
```

## NOTES

### Large File Warning
`src/lib/auth/extract-token.ts` (532 lines) — Complex browser crypto. Touch carefully.

### Dependency on ls
`af do`, `af task *` commands require `af ls` to be run first for short ID resolution.

### API Version
Uses Akiflow API v5 with version header "3" (Akiflow-Version: 3).

### Bun-Specific APIs
- `Bun.secrets` — Keychain access
- `Bun.file()`, `Bun.write()` — File operations
- `Bun.$\`cmd\`` — Shell commands
- `bun:sqlite` — SQLite for cookie parsing
- `bun:test` — Test runner

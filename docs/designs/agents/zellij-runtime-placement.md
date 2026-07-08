# Zellij runtime placement + wave restart via the zellij spawner

- **Domain:** agents · **Record:** zellij-runtime-placement · **Status:** draft (frozen on merge)
- **Scope:** the watchable multiplexed-runtime + mesh-native spawn path that
  [`coordination-structure.md`](./coordination-structure.md) (line 7) defers to "a **separate**
  design record" — how the Cotal `zellij` runtime places wave agents into named/stacked tabs, how a
  full-session KDL layout mirrors Matt's live multi-tab arrangement (his config), and how `cotal up -f
  --runtime zellij` stands up the whole wave through the zellij spawner in one command (the
  post-OMP-update restart).
- **Deliverables (mechanism vs config):** the split is the spine of this record — see the
  **Boundary** section below. The manager *mechanism* ships to the Cotal fork; the *tab-layout
  config* lives here.

## Boundary — Cotal zellij manager (mechanism) vs Matt's config (data)

The Cotal zellij manager is a **tenant-agnostic mechanism**: it knows how to place a pane into a
*named* tab with a shape, and how to turn a *layout map* into a full-session KDL layout. It
hardcodes **no** tab names, no lane→tab groupings, no pane counts, no stack shape — nothing
wave-specific. Every one of those is **Matt's config**, which the manager only *executes*. The
live 8-tab wave shape in *Problem* below is an **example of that config**, never something the
manager knows.

| **Cotal zellij manager — mechanism** (`sealedsecurity/Cotal`) | **Matt's config — data** (`nix-config/agents/cotal/`) |
| --- | --- |
| `Placement` seam, driver pane primitives, `ZellijRuntime.spawn` placement branch | *which* tab each agent lands in and its shape (`stacked`/`floating`/split) — per-agent `placement` **values** |
| layout-map **schema** + **KDL generator** (`seedFromDump`, `generateKdl`) | layout-map **values** — tabs, names, lane groupings, pane counts (seeded from the live dump) |
| `runtime: zellij` enablement + `placement` field **plumbing** through the manifest chain | `runtime: zellij` selection + the actual agent list, in the wave `cotal.yaml` |

**Mechanism → the Cotal fork** (zellij extension, branch `zheng-zellij-runtime`, committed
`21eb389`): every mechanism cited below exists in that fork today or is a named extension of a
cited symbol; its product docs (`docs/manifest.md`) gain the new fields when the impl lands, and
the Cotal repo carries no design records (`docs/` is product/protocol) — so this record lives here.
**Config → this repo** (`nix-config/agents/cotal/`, beside `channels.json` / `service-map.json`):
the wave `cotal.yaml` (per-agent `placement`) and the layout-map values. No wave-specific config
ever lands in the Cotal repo.

## Problem / Intent

The Cotal `zellij` runtime spawns **one agent → one fresh tab** — a mechanical port of the tmux
one-window-per-agent shape (`ZellijRuntime.spawn`, `extensions/zellij/src/runtime.ts:47-107`;
`openTab(...focus:false)` at `:62`). Matt's live wave groups lanes into **tabs** and **stacks**
many agents inside one tab — **that grouping is Matt's config, not the manager's** (live capture:
8 tabs — `sealed/zireael` STACKED 14 panes,
`services` STACKED 11, `upstream` STACKED 3, `woodpecker`/`compass` STACKED 2, `omp`/`supervisor`/
`backend` single). Three needs the current shape can't meet:

1. **Placement** — put a spawned agent into a *specific, named* tab as a **stacked** (or split, or
   floating) pane, not always its own fresh tab.
2. **Fresh-boot layout** — a KDL layout for a top-level `zellij --layout` boot that **mirrors** the
   current multi-tab arrangement.
3. **One-command wave restart through the zellij spawner** — after an OMP update Matt relaunches
   every session by hand; `cotal up -f --runtime zellij` should stand up the whole wave into its
   lane-tab layout, spawning through the zellij runtime. (Operator decision: fold this in — "i need
   it to be able to use the zellij spawner.")

The first layout attempt (`nix-config/dotfiles/zellij/layouts/wave.kdl`) declared a single
`tab name="wave"`, which is why "it didn't let me use multiple tabs." Beyond that symptom there is
a **verified KDL grammar trap** (below) that dictates the whole approach.

## Global constraints

- **The tab set is FLUID** (operator steer). Matt rearranges tabs by what currently needs one;
  tabs combine or split as lanes cross paths (`sealed/zireael` is a deliberately combined tab).
  Therefore: **no hardcoded canonical tab list**; placement **creates a target tab on demand** if
  absent; a tab may host **multiple lanes** (service→tab is many-to-one, reconfigurable); ad-hoc
  new tabs/panes at runtime must **always keep working**. The layout map + manifest are a
  **default seed Matt edits freely**, never a frozen contract.
- **Backwards-compatible seam.** The new `spawn` arg is **optional**; `pty`/`tmux`/`cmux`
  accept-and-ignore it. `spawn(name, spec, cwd)` with no placement behaves exactly as today.
- **Env stays structural-argv.** zellij takes argv over its control socket, so secret env never
  lands in `dump-layout`/`ps` (verified with a canary). Keep `env -i` isolation (`isolatedArgv`,
  `extensions/zellij/src/driver.ts:92-98`); **no launcher-script indirection** (unlike tmux).
- **Two KDL grammar contexts must never be conflated** (verified). Runtime placement uses CLI
  **primitives**; fresh boot uses a **full-session KDL** file. A multi-tab layout is **never**
  applied to a live session via `--layout-string`.
- **zellij 0.44.3 CLI surface** (verified target): `new-tab --name --cwd -- argv` (prints numeric
  tab id), `new-pane -n [--stacked] [--floating] [-d right|down] --cwd -- argv` (prints
  `terminal_<n>`, opens in the focused tab), `go-to-tab-name N [--create]`, `close-tab-by-id`,
  `query-tab-names`, `stack-panes <id>...`, `dump-layout`.
- **Mechanism lands on the fork, not upstream.** Upstream `Cotal-AI/Cotal` is off-limits
  (push-guard); the zellij extension + these changes ride `sealedsecurity/Cotal` branch
  `zheng-zellij-runtime`.

## Verified ground truth (all run live against zellij 0.44.3)

Cited as **verified**, not assumed (`rule://planning-evidence`). Live capture archived at
`~/notes/wave/zellij/current-live-layout.kdl`; evidence brief `~/notes/wave/zellij/design-brief.md`.

- **THE KDL grammar trap.** KDL has two contexts. A **full-session** layout
  (`layout { tab {...} tab {...} }` via top-level `zellij --layout FILE`) parses fine with inline
  `pane command="x" { start_suspended true }` — exactly what `dump-layout` emits; round-trips.
  Applying a layout to an **already-running** session (`action new-tab --layout / --layout-string`)
  parses each `tab` as a **standalone tab layout**, and there `pane command="x" { start_suspended
  true }` **fails to deserialize** (verified error: "Failed to deserialize KDL node"). ⇒ You
  **cannot** reliably seed a multi-tab layout into a live session via `--layout-string`.
- **The primitive path is reliable.** `new-tab --name` + `new-pane --stacked` + `go-to-tab-name
  --create`, one CLI call per tab/pane, verifiably builds a multi-tab + stacked shape **and**
  allows ad-hoc tabs afterward (added a tab at runtime, confirmed via `query-tab-names`).
- **Runtime selection is a per-manager choice, gated by two allow-lists.** `createRuntime(mode,
  session)` (`implementations/manager/src/runtime/index.ts:18`) builds ONE backend for the manager;
  `RuntimeMode = RuntimeKind | "auto"` (`:11`) permits any string, but the `--runtime` flag
  validator `RUNTIME_OVERRIDES = ["pty","tmux","cmux"]` (`implementations/manager/src/commands.ts:319`)
  and the manifest schema `runtime: z.enum(["pty","tmux","cmux"])`
  (`implementations/cli/src/lib/manifest/schema.ts:75`) both **omit `zellij`** — so `up -f --runtime
  zellij` is currently rejected. Adding `"zellij"` to both is the minimal enablement.
- **Resume is unavailable through the spawner today, by design** (bounds Decision D1): the OMP
  connector runs OMP in-process (`extensions/connector-oh-my-pi/dist/connector.js` `buildLaunch` →
  `command: TSX, args: [MAIN]`) and declares **no `supportsResume`**; `cotal_spawn` deliberately
  omits a `resume` param (`extensions/connector-core/src/tool-specs.ts:490-494`, deferred as fork
  issue #159); `supportsResume` is per-connector default-deny (`packages/core/src/connector.ts:122-129`),
  set `true` only by `connector-claude-code` (`extensions/connector-claude-code/src/extension.ts:37`),
  and even there it is **fork-only** (mints a new session; never hijacks the source).

## Approach

Three mechanisms, deliberately not conflated.

### Path 1 — Runtime placement (primitive-driven)

Extend `Runtime.spawn` with an optional `placement` argument. When present with `placement.tab`,
the zellij runtime: (1) `go-to-tab-name <tab> --create` — focus the tab, **creating it on demand**
if absent; (2) `new-pane` into the now-focused tab with the shape (`--stacked` lane default,
`--floating`, or `-d <direction>`); (3) key the agent lifecycle off the returned **pane id**
(`terminal_<n>`), not a tab id — the tab is shared: `status()`=pane present, `stop()`=close that
pane, `interrupt()`=focus + Ctrl-C that pane. With **no `placement`** (or no `tab`), `spawn` keeps
today's behavior exactly (`openTab`, lifecycle keyed off the tab id). zellij is the **only** backend
that reads `placement`; pty/tmux/cmux accept-and-drop it.

### Path 2 — Fresh-boot layout generation (KDL)

A **layout map** (versioned config) → a **KDL generator** emitting a **full-session** layout for
top-level `zellij --layout` (the grammar that works, mirroring the live dump). Scope, per operator:
**lane-tabs + a `new_tab_template` only** — defer `swap_tiled_layout`/`swap_floating_layout`. The
`new_tab_template` plus the Path-1 primitives are the two guarantees for the fluid-tab constraint.
The generator never emits a live-session `--layout-string` (grammar trap).

### Path 3 — Wave restart through the zellij spawner (the fold-in)

`cotal up -f cotal.yaml` already launches a **whole mesh from a manifest** (`up.ts`; `upManifest`
at `:187`; example `examples/04-frontier-faces/cotal.yaml`), idempotently — a ledger classifies each
agent `will-create`/`already-owned`/`stale` (`implementations/cli/src/lib/manifest/spawn-plan.ts:70-89`).
The launch flows manifest → `MeshLaunchSpec.agents[]` (`packages/core/src/launch.ts:36-42`) →
`supervise --launch` loop (`implementations/manager/src/commands.ts:400-421`) → `mgr.startAgent(...)`
→ `this.runtime.spawn(name, spec, cwd)` (`implementations/manager/src/manager.ts:770`). Fold-in:

1. **Select the zellij backend** — add `"zellij"` to both allow-lists so `up -f --runtime zellij`
   (or manifest `runtime: zellij`) resolves `ZellijRuntime` for the whole manager (`createRuntime`
   already accepts the kind — only the guards reject it; `ZellijRuntime` self-registers on import,
   `extensions/zellij/src/runtime.ts:112-119`).
2. **Carry per-agent placement** manifest → `ResolvedAgent`
   (`implementations/cli/src/lib/manifest/model.ts:38-58`) → `MeshLaunchAgent`
   (`packages/core/src/launch.ts:12-33`) → `launchAgentToStartOpts`
   (`implementations/manager/src/launch.ts:135`) → `StartOpts` → `startAgent` →
   `runtime.spawn(..., placement)`.
3. **Continuity is durable-state, not resume.** A restart makes **fresh** sessions that re-orient
   from the wave tracker + Linear + repo/PR state (the supervisor's existing "RESUME" broadcast
   pattern in `coordination-structure.md`). Real transcript-resume is deferred (Decision D1).

Net: after an OMP update, `cotal up -f cotal.yaml --runtime zellij` stands up the whole wave, each
agent placed into its lane-tab per the manifest — one command.

## Plan

Right-sized tasks; each carries `Interfaces:` (exact signatures). No placeholders. Mechanism tasks
(T1–T7) land on the Cotal fork branch `zheng-zellij-runtime`; the config task (T8) lands here.

### T1 — Core seam: optional `placement` on `Runtime.spawn`

Interfaces (`packages/core/src/runtime.ts`, extending the `Runtime`/`AgentHandle` contract at
`:30-52`):

```ts
export interface Placement {
  tab?: string;                          // target tab by name; CREATED ON DEMAND if absent
  stacked?: boolean;                     // stacked pane within the tab (lane default)
  floating?: boolean;                    // floating pane
  direction?: "right" | "down";          // split when neither stacked nor floating
}
export interface Runtime {
  readonly kind: RuntimeKind;
  spawn(name: string, spec: LaunchSpec, cwd: string, placement?: Placement): AgentHandle; // +4th arg
}
```

Touches (compile-only): `PtyRuntime.spawn` (`implementations/manager/src/runtime/pty.ts`),
`TmuxRuntime`, `CmuxRuntime`, `ZellijRuntime.spawn` — each accepts the arg; only zellij reads it (T3).

### T2 — Zellij driver: pane-into-tab primitives

Interfaces (`extensions/zellij/src/driver.ts`, alongside `actionArgs` at `:61`):

```ts
export function goToTabNameCreate(session: string, name: string): string;   // "" if it existed
export function newPane(session: string, argv: string[], cwd: string,
  opts: { stacked?: boolean; floating?: boolean; direction?: "right" | "down" }): string;
export function closePaneById(session: string, paneId: string): void;       // idempotent
export function paneExists(session: string, paneId: string): boolean;
```

The current `newPane(session, argv, cwd, direction)` (`driver.ts:135`) widens to the opts form; its
one caller `zellijLayout` (`runtime.ts:144`) updates to `{ direction }`.

### T3 — Zellij runtime: placement branch in `spawn`

Interfaces (`extensions/zellij/src/runtime.ts`): `ZellijRuntime.spawn(name, spec, cwd, placement?)`.
`placement?.tab` → `goToTabNameCreate` then `newPane(session, isolatedArgv(...), cwd, { stacked,
floating, direction })`; build the handle off the **pane id** (`status`=`paneExists`; graceful
`stop`=focus tab + `/exit` + `closePaneById` after `GRACE_MS`; hard `stop`=`closePaneById`;
`interrupt`=focus tab + Ctrl-C). Else → existing `openTab` path verbatim (`runtime.ts:62-106`).

### T4 — Layout map schema + KDL generator

Interfaces (`extensions/zellij/src/layout-map.ts`):

```ts
export interface LayoutPane { lane?: string; command?: string; cwd?: string; }
export interface LayoutTab  { label: string; stacked?: boolean; panes: LayoutPane[]; }
export interface LayoutMap  { version: 1; tabs: LayoutTab[]; }
export function seedFromDump(dumpKdl: string): LayoutMap;   // parse `dump-layout` → seed map
export function generateKdl(map: LayoutMap): string;        // FULL-SESSION KDL + new_tab_template
```

Pure functions ⇒ unit-testable without live zellij. `generateKdl` never emits a live-session
`--layout-string`.

### T5 — Select the zellij backend (runtime allow-lists)

- `implementations/manager/src/commands.ts:319` — `RUNTIME_OVERRIDES` gains `"zellij"`.
- `implementations/cli/src/lib/manifest/schema.ts:75` — `runtime: z.enum([...,"zellij"])`.
- `implementations/cli/src/lib/manifest/model.ts:64` — `ResolvedManifest.runtime` union + `"zellij"`.
- No change to `createRuntime` (`runtime/index.ts:18`) — it already resolves any registered provider.

### T6 — Per-agent placement through the manifest → spawn chain

```ts
// schema.ts AgentEntryObject (strictObject at :18-35) gains:
placement: z.strictObject({
  tab: z.string().min(1).optional(), stacked: z.boolean().optional(),
  floating: z.boolean().optional(), direction: z.enum(["right","down"]).optional(),
}).optional(),
// model.ts ResolvedAgent, launch.ts MeshLaunchAgent, manager StartOpts each gain: placement?: Placement
// launchAgentToStartOpts carries a.placement; manager.ts:770 passes it to spawn(name, spec, cwd, placement)
```

Non-zellij runtimes ignore it, so the manifest stays valid under any backend.

### T7 — Smoke + unit tests (red-green)

Extend `extensions/zellij/smoke.ts` (live-zellij, mirrors the existing 27-assertion smoke):
placement of two agents into one named tab as stacked panes (create-on-demand; both panes present;
correct tab via `query-tab-names`; ad-hoc tab still works after); teardown by pane id leaves the tab,
with the sibling alive; `generateKdl(seedFromDump(dump))` round-trips to a full-session layout that boots
via top-level `zellij --layout`. Plus a manifest-resolve test (`runtime: zellij` + per-agent
`placement` validates; resolved `MeshLaunchAgent` carries `placement`) and TDD units for
`seedFromDump`/`generateKdl`.

### T8 — Wave config in this repo (data)

Author, under `nix-config/agents/cotal/` beside `channels.json`: the wave `cotal.yaml` with
`runtime: zellij` + per-agent `placement` (tab per lane, `stacked: true` for lane tabs), and the
layout-map values seeded from `~/notes/wave/zellij/current-live-layout.kdl`. Data only — no Cotal
source. Lands on this branch; the mechanism (T1–T7) must be on the fork first for it to run.

## Tasks

- [ ] T1 — Core `Placement` + optional `spawn` arg (fork).
- [ ] T2 — Driver pane primitives; update `zellijLayout` caller (fork).
- [ ] T3 — `ZellijRuntime.spawn` placement branch (fork).
- [ ] T4 — `layout-map.ts` schema + `seedFromDump` + `generateKdl` (fork).
- [ ] T5 — Add `"zellij"` to both runtime allow-lists (fork).
- [ ] T6 — Per-agent `placement` through the manifest chain (fork).
- [ ] T7 — Smoke + manifest-resolve + unit tests (fork).
- [ ] T8 — Wave `cotal.yaml` + layout-map data (this repo).

## Decisions (settled by the operator — recorded, not open)

- **D1 — Wave restart = manifest re-stand now, resume later (option C).** Ship `cotal up -f …
  --runtime zellij` + placement (fresh sessions, re-orient from the tracker) in this work. Real
  transcript-resume is a **separate later feature**, bounded by the resume ground truth above
  (fork #159); it does not change the routing model.
- **D2 — Tenant-agnostic manager; the tab layout is config.** The zellij manager ships a *generic*
  placement seam + layout-map schema + KDL generator to the Cotal fork and hardcodes no tab names,
  groupings, or pane counts. The actual tab layout — the wave map + `cotal.yaml` — is Matt's config,
  here in `nix-config/agents/cotal/` beside `channels.json`. No wave-specific config or design
  records in the Cotal repo (its `docs/` is product/protocol). Full split: see **Boundary** above.

## Open questions

(Frozen with this record; each has a stated assumption so execution never stalls.)

1. **Who decides an agent's tab at spawn** — explicit `placement` on the manifest (runtime is a dumb
   executor), vs the runtime consulting a map by lane/agent name? *Assumption: explicit placement on
   the manifest; the layout map seeds those values but is not consulted at spawn.*
2. **`start_suspended` default** — Matt's live panes are mostly `start_suspended true`, but that's a
   *restore* concern; a fresh spawn should run immediately. *Assumption: fresh spawns run
   immediately; suspension is restore-time only.*
3. **Layout-map ↔ manifest overlap** — the map (Path 2, `--layout` boot) and the manifest placement
   (Path 3, `up -f`) both describe tabs. *Assumption: two independent seeds for now (map = fresh-boot
   KDL; manifest = live wave placement); unify later only if they drift in practice.*

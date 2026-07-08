# Cotal wave coordination — runbook

This directory holds the **wave coordination config** for Matt's multi-agent wave on the Cotal
mesh: the channel registry, the persona files, and the per-service ownership map. It is the
execution of two frozen design records — read them for the *why*:

- `docs/designs/agents/coordination-structure.md` — channels, routing, spawn model, personas.
- `docs/designs/agents/service-owners.md` — the per-service dormant-owner layer.

These configs are **ours** (nix-config), not an upstream Cotal contribution. At bring-up they are
copied into each wave workspace's `.cotal/` (gitignored, resolves only at runtime —
`agent-file.ts` `agentFilePath`), so committed source ≠ runtime location.

## Layout

| File | What |
| --- | --- |
| `channels.json` | Channel registry: `announcements` (24h replay) + the 11 `#svc.<name>` service channels (7d replay). Per-issue `#coordination.<issue>` channels are created live, not seeded. Read by the manual bring-up (`cotal up --channels`); its cards are mirrored into `cotal.yaml`'s `channels:` for the manifest path. |
| `cotal.yaml` | Wave mesh **manifest** (`kind: Mesh`, `runtime: zellij`) for the one-command zellij-spawner restart — `cotal up -f cotal.yaml` stands up the broker, the same channel set, and the standing agents, each placed into its lane tab. The config half of `docs/designs/agents/zellij-runtime-placement.md` (T8). A seed Matt edits, never frozen. |
| `layout-map.json` | Fresh-boot zellij layout seed (the `LayoutMap` the fork's `generateKdl` turns into a full-session `zellij --layout` file), generated from the live `dump-layout` via `seedFromDump`. 8 tabs mirroring the live wave. |
| `agents/supervisor.md` | The generic supervisor persona (routes work, owns the tracker, authors personas). |
| `agents/_worker-template.md` | Worker persona template (copy → `agents/<name>.md`). |
| `agents/worker-impl.md` | One concrete worker proving the template fills in. |
| `agents/_service-owner-template.md` | Service-owner persona template. |
| `agents/tern.md` | One concrete service owner proving the template fills in. |
| `service-map.json` | glob → service → owner map (routing + PR-post + CI backstop). |
| `VERIFY.md` | Auth-mode verification checklist (the red/green gate for the structure). |

## 1. Auth bring-up (target)

The `cotal` CLI shim and the omp `extensions:` wiring are installed by the nix activation
`home.activation.installCotalOmpExtension` (`shared/dev.nix`; the `extensions:` list is
`agents/config.yml`). This config directory is exposed at the canonical path **`~/.cotal-config`**
via an out-of-store symlink (`shared/agent-config.nix` → `agents/cotal`), so edits land without a
rebuild (see §Nix wiring below).

Then stand up the authed mesh (auth is the no-flag default):

```sh
cotal up --space wave --channels ~/.cotal-config/channels.json
```

The `wave` space is long-lived and rooted at `~/agents/workspaces` (persists across waves). This
seeds `#announcements` + the `#svc.<name>` channels. Personas resolve **per wave workspace** from
`<root>/.cotal/agents/` (gitignored), which no single `$HOME` symlink covers — so each workspace's
`.cotal/` is seeded by a **manual copy** from the canonical `~/.cotal-config` at bring-up (there is
no activation step that fans out into workspaces — see §Nix wiring for the one-liner).

## 2. Launching agents (manual — no manager)

There is **no auto-spawn manager** in this posture: each agent is a session the operator (Matt)
starts. Order: the **supervisor first** (it is the assignment authority), then **workers** on
demand as the supervisor authors their persona files, then the **service owners** (once, as
long-lived dormant sessions). For each:

```sh
cotal mint <name> --profile agent          # ACLs + role derive from the persona file
COTAL_NAME=<name> omp                        # launch the session; the connector joins the wave space
```

Per-worker/owner creds are minted from the persona file's ACLs. Matt does **not** join the mesh
(the mesh is the agent-to-agent layer), so no operator agent cred is minted.

**Persona files shipped here:** `supervisor.md` (the one standing persona), `worker-impl.md` (a
concrete worker), and `tern.md` (a concrete owner) — plus the two `_`-prefixed templates. The other
ten service owners are **not** pre-shipped: author each from `_service-owner-template.md` (replace
`svc-name` throughout, pin its spec path from `service-map.json`) the first time that service needs
a standing owner, then `cotal mint <svc> --profile agent`. `service-map.json` is the authoritative
list of the eleven services + their channels/specs; the persona files catch up to it on demand.

## 3. Operator surface

Matt needs no agent cred to post announcements or DM: the CLI's one-shot `cotal send msg
announcements "…"` rides transient manager-profile creds (allow-all). Matt directs agents through
their own sessions; they relay.

## 4. Open-mode quick-start (flow-only dogfooding)

```sh
cotal up --open --space wave --channels <workspace>/.cotal/channels.json
```

Same files, **no minting**. Open mode enforces **no ACLs** — channel scoping and the announcement
gate are advisory only (any session can post/join anything). Use it to validate the *flow*; only
auth mode validates the *fence*.

## 5. Manifest bring-up — one-command zellij-spawner wave restart

`cotal.yaml` is the wave as a single manifest, for the zellij-spawner restart from
`docs/designs/agents/zellij-runtime-placement.md`. After an OMP update, instead of minting +
launching each session by hand (§2), stand up the whole wave into its lane-tab layout at once:

```sh
cotal up -f ~/.cotal-config/cotal.yaml            # broker + channels + every agent, placed
cotal up -f ~/.cotal-config/cotal.yaml --dry-run  # validate + print the plan, change nothing
```

`runtime: zellij` makes the manager spawn each agent through the zellij runtime — every agent lands
in its `placement.tab` (created on demand) as a stacked pane, rather than one fresh tab per agent.
`layout-map.json` beside it is the fresh-boot layout seed (`generateKdl` → `zellij --layout`).

**Posture note:** this path uses the **manager-driven spawn** (`cotal up -f` boots the agents),
unlike §2's manual mint+launch — the two are alternate bring-ups of the same wave, not both live at
once. The manifest is a **seed Matt edits freely** (the tab set is fluid; placement groups lanes
into tabs many-to-one; ad-hoc tabs keep working) — never a frozen roster. Add service owners (one
per `svc.<name>`, from `_service-owner-template.md`) and workers under `agents:` as the wave grows.

**Keep in sync:** `cotal.yaml`'s `channels:` block mirrors `channels.json` (same cards + replay
knobs) — edit both when a channel's card changes, or the two bring-ups drift. The channel set is
the shared contract; only the *agent-launch* half differs between them.

## 6. What auth mode enforces vs what convention enforces

| Invariant | Enforced by |
| --- | --- |
| 3 — supervisor-only spin-up | **Convention + operator launch** (only the supervisor authors persona files; only Matt launches — no `cotal_spawn` in this posture). Becomes a cred gate if a manager runtime is added. |
| `#announcements` supervisor-authored | **Cotal auth creds** (post ACL default-deny — workers hold no `announcements` grant). |
| Read/post scoping generally | **Cotal auth creds** (minted per-channel from the persona ACLs). |
| 1 — tracker is the single assignment authority | **Filesystem ownership + persona files** — invisible to the broker. |
| 4 — no self-assign / no pull-queue | **Convention** (no queue artifact; personas forbid it). |
| 2 — supervisor never polls | **Convention + presence design** (status is ambient). |
| Presence honesty, message grammar | **Convention** (prompt-facing, advisory). |

## 7. Per-service owners

Eleven long-lived, mostly-dormant owner agents (one per service) sit in `#svc.<name>` channels as
first point-of-contact + reviewer + spec-owner for their service. Routing (`service-map.json`):

- **Service incident/bugfix** → @mention the owner in its `#svc.<name>`; the owner wakes. Do NOT
  pull a busy feature agent off its task for a service issue.
- **Starting a PR that touches a service** → post it into each touched service's `#svc.<name>` on
  start (a PR spanning N services posts to all N). Primary path is the authoring agent posting; a
  CI backstop hook is a later follow-up (fork repos rely on agent-posts only).
- **Mid-fix on a service you don't own** → `cotal_join #svc.<name>` for the duration, coordinate,
  then leave.

Path resolution is most-specific-wins by `priority` (woodpecker's nix/ci paths outrank
nix-infra/ci-build), so a path resolves to exactly one owner before the all-channels rule applies.

## 8. Teardown

Under the manual bring-up (§2) an agent ends by its session exiting — there is no manager, so no
mesh despawn. Under the manifest bring-up (§5) the manager that `cotal up -f` started owns the
agents, so it despawns them on teardown. Either way, `cotal down` tears down the whole mesh.

## Nix wiring — canonical `~/.cotal-config` + per-workspace `.cotal/`

`shared/agent-config.nix` exposes this directory at **`~/.cotal-config`** via an out-of-store
symlink (`.cotal-config → nix-config/agents/cotal`), the same idiom as `agents/config.yml` etc., so
edits land without a rebuild. That is the canonical home `cotal up --channels` reads.

Runtime persona resolution is **per wave workspace** (`<root>/.cotal/agents/`, gitignored), which a
single `$HOME` symlink can't cover — so each workspace's `.cotal/` is seeded from the canonical
home at bring-up:

```sh
mkdir -p <workspace>/.cotal && cp -r ~/.cotal-config/* <workspace>/.cotal/
# (or symlink: ln -s ~/.cotal-config <workspace>/.cotal — if the runtime tolerates a symlinked dir)
```

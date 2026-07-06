# Per-service owner + worker model — dormant owners, `#svc.<name>` channels

- **Domain:** agents · **Record:** service-owners · **Status:** draft (frozen on merge)
- **Scope:** the per-service ownership layer on top of the wave coordination structure
  (`coordination-structure.md`) — long-lived dormant owner agents, one per service, each in a
  `#svc.<name>` channel; how service incidents, PRs, and specs route to them. Builds on the
  channel/routing/persona model in that record; does not change it.
- **Deliverables:** the `#svc.<name>` channel class + ACL grant, the owner persona template, the
  glob→service→owner map, and the routing rules (incident / PR-post / mid-fix-join). Config +
  persona files under `nix-config/agents/cotal/`, same home as the coordination configs.

## Problem / Intent

A feature agent builds a service, merges its PR, and moves to the next issue. Then an incident or
bugfix on that *old* service needs attention — and the only agent who knows it is now deep in
unrelated work. Pulling them back is the exact mid-task context-drop that hurts most: the feature
agent loses its current thread, and the service still waits on a cold responder.

The fix is a standing owner per service. Each service gets a **long-lived, mostly-dormant owner
agent** that is the durable point-of-contact for its service — so a service issue wakes its owner,
not whoever last touched it. A parked session costs nothing (zero tokens until woken); it only
spends context when something actually needs the service.

## Approach

### Owner role (three duties)

Each owner is, for its one service:

1. **First point-of-contact.** Service down, CI broken, a bug in the service — an @mention in its
   `#svc.<name>` channel wakes the owner to handle it. No feature agent gets pulled off its task.
2. **Reviewer of its seam.** Every PR that touches the service is posted into `#svc.<name>` when
   work starts, so the owner stays current and reviews changes to its contract.
3. **Spec owner.** The owner keeps its service's spec file current — a PR that changes the
   service's contract updates the spec in the same PR (a standing duty, per the coordination
   record's config-home convention: specs live in `docs/specs/`).

Otherwise the owner is **dormant** — parked, presence `idle`, waking only on its channel's traffic.

### Channels — `#svc.<name>`

- One standing channel per service: `#svc.tern`, `#svc.petrel`, `#svc.woodpecker`, … (concrete
  dotted names under the `svc.` subtree; `isConcreteChannel` treats `svc.tern` as concrete —
  publishable — while `svc.>` is the ACL wildcard).
- **Every agent's cred grants read + post on the whole `svc.>` subtree** (`allowSubscribe` and
  `allowPublish` both carry `svc.>`), so any agent can reach any owner's channel to flag an issue
  or post a PR. This is the deliberate difference from `coordination.>` (which workers post to but
  the model scopes per-issue): `svc.>` is a fleet-wide addressing space, not per-issue.
- **The owner auto-joins its own `#svc.<name>`** at launch — it's in the owner persona's
  `subscribe` (active read set), so a dormant owner is *listening* from boot and an @mention wakes
  it (mention-wake) without any runtime `cotal_join`. Non-owners do **not** subscribe standing;
  they `cotal_join` a `#svc.<name>` only for the duration of a mid-fix (below), then leave.
- Replay: `#svc.<name>` replays a modest window (owner catches up on what it missed while parked —
  a PR-posted-since-last-wake shouldn't be lost). Unlike `#coordination.<issue>` (live-only,
  ephemeral), a service channel is durable context for its owner.

### Routing rules

1. **Service incident / bugfix** (service down, CI broken, a bug in a service you're not on):
   @mention the service's owner in its `#svc.<name>`. The owner wakes and handles it. Do NOT pull
   a busy feature agent off their current task; do NOT self-assign someone else's service incident.
2. **Starting a PR that touches a service:** the authoring agent posts it into that service's
   `#svc.<name>` when it starts, so the owner stays current + can review its seam. **A PR spanning
   N services posts to ALL N matched channels** (each owner reviews its own seam) — never just a
   "primary" one, or an owner is left blind to a change in its service. (Primary path: the agent
   posts on PR-start. Backstop, later: a CI hook auto-posts on PR-open — see Plan T-CI.)
3. **Mid-fix on a service you don't own:** you MAY `cotal_join #svc.<name>` for the duration,
   coordinate with the owner, then `cotal_leave` when done. The `svc.>` ACL permits it; the
   standing subscriber is only the owner.

### Relationship to the coordination record

This layer is **additive** to `coordination-structure.md`:

- The supervisor still owns wave assignment + the tracker (Inv 1); owners are a *routing target*
  for service-specific work, not a second assignment authority. The supervisor routes a service
  incident to the owner exactly as it routes any work — the owner is just the standing right
  recipient.
- Owners are agents like any other: launched by the operator (manual-spawn model — no manager),
  their persona files authored from the owner template (below), `model:` pinned per the owner's
  judgment-vs-mechanical mix.
- Channel classes now: `#announcements` (standing, all) · `#coordination.<issue>` (per-issue,
  join-on-demand) · `#svc.<name>` (per-service, owner-standing + others-join-on-demand).

## Global Constraints

- **Additive only:** this record adds the `svc.>` channel class + owner persona; it does not change
  `#announcements`/`#coordination.<issue>`, the supervisor/worker personas, or the routing model in
  `coordination-structure.md`.
- **One owner per service; owner name = service name; channel = `#svc.<service>`.** The owner is a
  long-lived session, dormant by default.
- **`svc.>` is fleet-wide addressing, granted to every persona** (owner + supervisor + worker) in
  `allowSubscribe` + `allowPublish`; only the owner *subscribes* to its own channel standing.
- **The supervisor remains the single assignment authority** (Inv 1). Owners handle their service's
  incidents/reviews/spec; they do not assign wave work or own the tracker.
- **Spec ownership is a standing duty:** each owner's service spec lives in `docs/specs/`; a
  contract-changing PR updates it in the same PR.

## Plan

### T-map — glob → service → owner map

Author the authoritative mapping (drives the CI-hook, the PR-post rule, and the supervisor's
incident routing). Grounded against the `sealed/` tree (2026-07-06 live batch — 11 owners):

| Path glob | Service | `#svc` channel |
| --- | --- | --- |
| `oss/petrel/**` | petrel | `#svc.petrel` |
| `oss/tern/**` | tern | `#svc.tern` |
| `oss/compass/**` | compass | `#svc.compass` |
| `oss/seal/**` | seal | `#svc.seal` |
| `infra/pulumi/**` | pulumi | `#svc.pulumi` |
| `infra/nix/**` (rest) | nix-infra | `#svc.nix-infra` |
| `ci/**`, `.moon/**`, root bun workspace | ci-build | `#svc.ci-build` |
| `apps/**` (site + docs) | web | `#svc.web` |
| oh-my-pi (fork) | omp | `#svc.omp` |
| the Cotal connector/mesh (fork) | cotal | `#svc.cotal` |
| woodpecker fork + `infra/nix/**/woodpecker*` + `ci/woodpecker/**` | woodpecker | `#svc.woodpecker` |

- **Spec + design paths route to the service they document, not a "docs" service.** A change to
  `docs/specs/<service>/**` (or a `docs/designs/**` record about a service) matches **that
  service's** owner — e.g. `docs/specs/tools/tern.md` → `#svc.tern`. The service-map must therefore
  carry each service's spec path alongside its code globs (the spec-ownership duty, per Global
  Constraints, requires the owner see spec-update PRs). A `docs/**` change with no resolvable
  service (cross-cutting docs) matches nothing and posts nowhere — that's correct, not a gap.
- **woodpecker caveat:** not a single directory — a CI service spanning the woodpecker fork repo
  plus `infra/nix/**/woodpecker*` + `ci/woodpecker/**`. Its "glob" is a cross-repo subset, not a
  clean prefix; the owner spans repos.
- **Fork repos (omp / cotal / woodpecker fork) have no `sealed/` path**, so the T-CI resolver
  (which reads a `sealed/` PR's changed paths) **cannot fire for a PR opened inside a fork**. Those
  services rely on the **agent-posts-on-start path only** (no CI backstop) — unless a fork-local CI
  hook with a catch-all `**` → the fork's service is deployed in that repo (T-CI, optional per
  fork). Document which, per fork, so no owner silently expects a backstop it won't get.
- **Interfaces:** the map is data consumed by T-persona (which channel an owner subscribes),
  T-CI (which channel(s) a PR posts to), and the supervisor's routing. **Single home:**
  `nix-config/agents/cotal/service-map.json` — one file both the CI hook and the prose cite (not
  duplicated inline; the runbook links it). Each entry: `{ service, channel, globs: [...], spec: <path> }`.
- **Model hint:** small.

### T-persona — owner persona template (`_service-owner-template.md`)

Author the owner persona template (underscore-prefixed, like the worker template). Required
frontmatter (template values in brackets; `<svc>` = the service name):

```yaml
name: [svc-name]
role: service-owner
description: [one line — owner of the <svc> service]
subscribe: [announcements, svc.<svc>]
allowSubscribe: [announcements, svc.>, coordination.>]
allowPublish: [svc.>, coordination.>]
model: [pin per the owner's judgment/mechanical mix]
```

- The **`subscribe` includes `svc.<svc>`** so the owner listens on its own channel from boot
  (dormant-but-listening) — this is the auto-join: no runtime `cotal_join` needed, an @mention
  wakes it. It is the one persona whose active read set carries a `svc.` channel standing.
- `allowSubscribe`/`allowPublish` carry the `svc.>` wildcard (reach any service channel) + the
  `coordination.>` subtree (owners join issue channels like any agent when they take on a fix).
- No `capabilities` (owners don't spawn — same manual-spawn model as workers).
- Body (self-contained, replaces `prompts/` — cite `skill://`/`rule://`, never a prompt path):
  the three duties (PoC / reviewer / spec-owner); dormant-by-default (presence `idle`, wake on
  `#svc.<svc>` @mention); the spec file it owns (`docs/specs/…`) + the update-in-the-same-PR duty;
  the mid-fix protocol (others `cotal_join` its channel; it coordinates); that it is NOT an
  assignment authority (defers wave assignment to the supervisor).
- **Interfaces:** file `nix-config/agents/cotal/agents/_service-owner-template.md` + one concrete
  instance (e.g. `tern.md`) proving it fills in. Same frontmatter contract as the coordination
  record's personas (`agent-file.ts:31-63`).
- **Test cycle:** launch an owner session with its persona file; `cotal_orientation` shows role
  `service-owner`, `subscribe` includes `svc.<svc>`; another agent's @mention in `#svc.<svc>` wakes
  it; it can post to `#svc.<svc>` and `cotal_join` another `#svc.<other>` (within `svc.>`).
- **Model hint:** the body is judgment-bearing prose — draft on a strong model.

### T-workers — worker/supervisor ACL widening for `svc.>`

Extend the coordination record's worker + supervisor personas so every agent can reach service
channels: add `svc.>` to `allowSubscribe` + `allowPublish` (not to `subscribe` — non-owners join a
`#svc.<name>` on demand, they don't subscribe standing). This is the only change to the existing
personas; the per-issue `coordination.>` model is unchanged.

- **Interfaces:** edits `supervisor.md` + `_worker-template.md` (from the coordination record) to add
  the `svc.>` grants. Depends on that record having landed (this PR stacks on it).
- **Test cycle:** a worker can `cotal_join #svc.tern` + post (within `svc.>`), then `cotal_leave`; a
  worker's standing `subscribe` still contains only `announcements` (no `svc.` firehose).
- **Model hint:** small.

### T-runbook — owner section in the wave runbook

Add a "per-service owners" section to the runbook README (T4 of the coordination record): the
map, the three routing rules (incident / PR-post / mid-fix-join), how an owner is launched
(operator starts a long-lived dormant session per service), and the spec-ownership duty.

- **Interfaces:** extends `nix-config/agents/cotal/README.md`.
- **Model hint:** small.

### T-CI — PR→channel backstop (follow-up, not blocking)

The primary PR-post path is the authoring agent posting to each touched service's `#svc.<name>` on
PR-start (a persona instruction, no infra). A **later** backstop: a CI hook (on PR-open) that
resolves the PR's changed paths through the service-map globs and posts a notice into **every**
matched `#svc.<name>` (multi-match → all channels, same rule as routing rule 2), so owners stay
current even if an agent forgets. Deferred — the agent-posts path proves the model first; this
hardens it.

- **Scope limit:** the hook resolves a `sealed/` PR's paths, so it covers only `sealed/`-hosted
  services. Fork repos (omp / cotal / woodpecker fork) get no backstop from it — they rely on the
  agent-posts path, or a fork-local hook (catch-all `**` → the fork's service) deployed in that repo.
- **Interfaces:** a CI step (woodpecker `CI (pr)` or a GitHub action) that reads changed paths →
  `service-map.json` → posts via `cotal send` to each matched channel. Out of scope for the first
  landing; recorded so the contract is known.
- **Model hint:** medium (needs the glob-resolver + a mesh-post from CI).

## Tasks

- [ ] T-map — glob → service → owner map as `service-map.json` (11 owners; each entry carries code
      globs **+ its `docs/specs/<service>` path**; woodpecker/omp/cotal cross-repo caveats)
- [ ] T-persona — owner persona template `_service-owner-template.md` + one concrete owner (role
      `service-owner`, `subscribe: [announcements, svc.<svc>]`, `svc.>`+`coordination.>` ACLs,
      three-duties body)
- [ ] T-workers — add `svc.>` to supervisor + worker `allowSubscribe`/`allowPublish` (not
      `subscribe`)
- [ ] T-runbook — per-service-owners section in the runbook (map, routing rules, launch, spec duty)
- [ ] T-CI — PR→channel backstop hook (follow-up; agent-posts-on-start is primary)

## Resolved decisions

1. **Owners are long-lived + dormant**, not spawn-on-demand: a parked session costs nothing until
   woken, and instant availability is the point (no cold-start when a service breaks).
2. **PR→channel is agent-posts-on-start (primary) + a CI hook (later backstop).** The authoring
   agent posts to `#svc.<name>` when it starts a PR touching that service; the CI hook is a
   follow-up that hardens against a forgotten post.
3. **`svc.>` is granted fleet-wide** (every persona's `allowSubscribe`+`allowPublish`), so any agent
   can reach any owner. Only the owner *subscribes* standing (via `subscribe: [.., svc.<svc>]`);
   others join on demand for a mid-fix.
4. **Owners are not assignment authorities.** The supervisor still owns the tracker + wave
   assignment (Inv 1 of the coordination record). An owner handles its service's incidents,
   reviews, and spec — it doesn't assign work or spawn.
5. **Spec ownership is standing:** each owner keeps its service's `docs/specs/` file current; a
   contract-changing PR updates the spec in the same PR.
6. **Multi-service PRs post to ALL matched channels.** A PR spanning N services notifies each of
   the N owners (routing rule 2 + T-CI); never a single "primary" service, which would leave the
   others blind. Spec/design paths resolve to the service they document (`docs/specs/<service>/**`
   → that `#svc`), not a "docs" service.
7. **The CI backstop covers `sealed/` only.** Fork repos (omp / cotal / woodpecker fork) have no
   `sealed/` path for the resolver, so they rely on the agent-posts-on-start path (or a fork-local
   hook); the record documents per-fork which, so no owner expects a backstop it won't get.

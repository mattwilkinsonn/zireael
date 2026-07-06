# Auth-mode verification checklist

The red/green acceptance gate for the wave coordination structure. Run it on a **live auth mesh**
(`cotal up --space wave --channels .cotal/channels.json`, then mint + launch a `supervisor` and a
`worker-impl` session per the runbook §2). It must **fail before** these configs exist (no personas
→ no scoped creds → no gate) and **pass after**. The checklist IS the test cycle.

Assert, in order:

1. **Tool surface.** Every session's `cotal_orientation` shows the standard `cotal_*` surface with
   **no** `cotal_spawn` / `cotal_persona` — there is no manager to route them to, so spin-up is
   convention (supervisor authors the file, operator launches), not a mesh op.

2. **Announcement gate.** A worker publish to `#announcements` → **broker denial** (workers hold no
   `announcements` grant). A worker `cotal_join` + publish to a `#coordination.<issue>` (within the
   `coordination.>` ACL) → **delivered**.

3. **Read-ACL bound.** A worker `cotal_join` **outside** `allowSubscribe` (e.g. a channel not under
   `announcements` / `coordination.>` / `svc.>`) → **refused**.

4. **Request path (zero channel grants needed).** A worker DM to the supervisor **and**
   `anycast(role: supervisor)` → both **delivered** to the supervisor session.

5. **Spin-up funnel (convention).** Only the supervisor authors `.cotal/agents/<name>.md` + the
   tracker record, and only the operator launches sessions. Confirm there is **no `cotal_spawn`
   path** for a worker to self-spawn or spawn a peer (follows from assertion 1 — the tool isn't
   exposed).

## Per-service owner assertions (from `service-owners.md`)

1. **Owner auto-listens.** A `service-owner` session (e.g. `tern`) shows role `service-owner` and
   `subscribe` includes its own `svc.<svc>` — an @mention in `#svc.<svc>` **wakes** it (mention-wake)
   with no runtime `cotal_join`.

2. **Fleet-wide `svc.>` reach.** Any agent (worker/supervisor/owner) can `cotal_join` + post to a
   different `#svc.<other>` (within the `svc.>` ACL), then `cotal_leave` — but only the owner
   *subscribes* to its own channel standing.

## Notes

- These are **auth-mode** assertions — the gates are real only under minted creds. In open mode
  (`cotal up --open`) the ACLs are advisory, so 2/3 won't deny; that's expected (open validates
  flow, not fence).
- Model hint for whoever runs this: small — it's mechanical observation of tool/broker behavior.

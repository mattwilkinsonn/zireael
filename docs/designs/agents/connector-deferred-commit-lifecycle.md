# Connector deferred-commit lifecycle — `shutdown()` joins the floating `commitAfterSteers`

- **Domain:** agents · **Record:** connector-deferred-commit-lifecycle · **Status:** draft (frozen on merge)
- **Scope:** SEA-1173 — the OMP connector peer-loop's `shutdown()` must track and
  cancel/await the floating `commitAfterSteers` deferred commit. The target code lives on the
  **unmerged** fork PR #5 (`sealedsecurity/Cotal`, branch `zheng-connector-upstream`, head
  `568c817`) — it is NOT on any trunk. Design only; implementation is gated on #5 merging.

## Problem / Intent

The connector peer loop's `shutdown()` never joins the deferred terminal commit it may have
floating. If a turn ends while a fold's steer is still unsettled, the loop launches
`commitAfterSteers` fire-and-forget, bounded by a **ref'd** 5-second settle timer; `shutdown()`
neither cancels nor awaits it, so on a shutdown-during-unsettled-steer the armed timer holds the
Node event loop open after teardown completes. This is a **liveness** bug (delayed process/CLI
exit) — proven at +5002ms — NOT a double-commit bug: the `stopped`/generation guard already makes
the late commit a no-op. Intent (SEA-1173): `shutdown()` tracks and cancels/awaits the float so
no teardown races a pending commit and no loop-owned timer outlives shutdown.

All code cited below lives on the **unmerged** fork PR #5 (`sealedsecurity/Cotal`, branch
`zheng-connector-upstream`, head `568c817`) and is read via
`git show origin/zheng-connector-upstream:extensions/connector-oh-my-pi/src/loop.ts`. It is on no
trunk; line numbers reference that ref.

### The defect, in three quotes

**1. The deferred commit is launched floating** — `agent_end` with folds still settling discards
the promise (`loop.ts:268-269`):

```ts
        if (pendingSteers.size === 0) finishTurn(to, reply);
        else void commitAfterSteers(generation, to, reply);
```

**2. Its wait is bounded by a ref'd timer, cleared only when the race resolves**
(`loop.ts:220-235`; timer armed at `:227`, race at `:229`, `finally`-clear at `:230-231`, guard at
`:233`):

```ts
  async function commitAfterSteers(gen: number, to: InboxItem | undefined, reply?: string): Promise<void> {
    // Bound the wait on a human-scale timer, but CLEAR it when allSettled wins (the common path):
    // an uncleared setTimeout(steerSettleTimeoutMs) stays ref'd on the Node event loop and delays
    // process/CLI exit by up to that timeout per folded turn (harmless at 0ms, not at the 5s default).
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const timeout = new Promise<void>((resolve) => {
        timer = setTimeout(resolve, steerSettleTimeoutMs);
      });
      await Promise.race([Promise.allSettled([...pendingSteers.values()]), timeout]);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
    if (stopped || gen !== generation) return; // torn down or superseded mid-wait → don't commit
    finishTurn(to, reply);
  }
```

**3. `shutdown()` never joins the float** — it sets `stopped`, aborts, disposes, and returns
(`loop.ts:279-299`, teardown comments elided):

```ts
    async shutdown(): Promise<void> {
      stopped = true; // block any in-flight prompt/steer/mesh callback from driving a disposed session
      try {
        if (turn.inFlight) {
          turn.abandon(); // leave the in-flight run on the stream → redeliver, no peer dropped
          await session.abort();
        }
      } catch (e) {
        log(e); // a failed abort must not skip dispose below
      }
      try {
        await session.dispose(); // await async cleanup before the caller stops the mesh
      } catch (e) {
        log(e); // a failed dispose must not skip the caller's mesh.stop()
      }
    },
```

The `finally` at `:230-231` is the only place the settle timer is cleared, and it runs only when
the `Promise.race` at `:229` resolves. A steer still unsettled at shutdown time leaves the race
blocked on the ref'd timer: the `finally` hasn't run, the timer stays armed, and the process
outlives `shutdown()` by up to `steerSettleTimeoutMs`.

### What this is NOT

Not a correctness/double-commit bug. `shutdown()` sets `stopped = true` (`loop.ts:280`) before
anything else, and the guard at `loop.ts:233` — `if (stopped || gen !== generation) return;` —
makes the post-race `finishTurn` a no-op after teardown or supersession. The stale commit cannot
fire; only the timer's liveness leaks.

### Reachability — honest framing

**Unreachable on today's contract; real for tomorrow's.** The in-code contract states it
(`loop.ts:102-104`, the `steerSettleTimeoutMs` doc):

> A healthy steer settles in ≤1 microtask (its promise resolves at synchronous enqueue-time — no
> image work on the connector's string-only steers), so allSettled wins this race by orders of
> magnitude and the timeout never fires on the happy path

So on the string-only-steer path no steer is ever unsettled at shutdown time and the defect cannot
trigger. It becomes reachable the moment a steer carries real async work — the identified path is
a future images-carrying steer (oh-my-pi's `#queueUserMessage` → `#normalizeImagesForModel` →
`resizeImage`, a genuine async op crossing a macrotask) or oh-my-pi making `steer()` await
anything real — the same latent seam as review finding #5 (which produced the settle race itself).
This record owns the lifecycle before that lands.

### Provenance

- **Proof** (`~/notes/upstream/cotal/cotal5-tripwire-escalation.md:148`): "PROVEN: ref'd → process
  exits +5002ms; timer.unref() → +0ms."
- **Finding chain:** the settle race was added for finding #5 (accepted folds must be awaited),
  its timer clear for #7 (test 18), and this shutdown seam is #9 — each fix's own code drew the
  next finding. The review loop was stopped at #9/#10 and escalated rather than patched at-gate.
- **Ruling** (same doc, `:167-187`): land #5's PR as-is; the deeper fix is this record's subject —
  "The DEEPER fix — shutdown() tracks and cancels/awaits the floating commitAfterSteers so no
  teardown races a pending commit (and the settle timer can't outlive shutdown) — is its OWN
  future PR with its own red-green." Finding #10 (test 18's `process.getActiveResourcesInfo()` is
  process-wide/fragile; use a test-scoped timer shim) rides along as a P3.
- **Design-critic pass** (adversarial red-team, read-only, before human freeze): folded three
  clear improvements — added mechanism **D** (clear the timer in `shutdown()` without awaiting) to
  the fork and Open Question 1; softened §Recommendation pillar 1 (the ruling settled whether/where,
  not the mechanism — its "cancel/await" phrasing disfavors only pure-A, not D); and stated A's
  residual cost narrowly (unref fires-into-a-no-op post-teardown, dominated by D). The core B
  mechanism (one-shot gate, single-slot `pendingCommit`, join-before-abort ordering) survived the
  attack unchanged. A line-anchor-drift finding was checked and rejected — the anchors verify
  verbatim against `568c817`.

## Approach

### Fixed intent (already ruled)

The *whether/where* is settled: the lifecycle fix ships as **its own PR with its own red-green**,
not another at-gate patch on fork PR #5 (ruling recorded in
`~/notes/upstream/cotal/cotal5-tripwire-escalation.md:167-187`, quoted above; the same record
flags the mechanism "Design-pass-first … (fork: unref shim / tracked-await / restructure)" at
`:186`). What remains open — and what this record decides, subject to override at merge — is the
**mechanism**.

### The mechanism fork

Four candidate mechanisms:

- **A — `unref()` the settle timer.** One line at `loop.ts:227` (`timer.unref()`). The timer no
  longer holds the event loop, so the delayed exit disappears (the +0ms half of the proof).
- **B — track + await the float, released by a shutdown gate.** Hold the floating promise in a
  loop-level slot, give the settle race a third arm that `shutdown()` resolves, and `await` the
  slot in `shutdown()` after `stopped = true`. Deterministic join: when `shutdown()` resolves, the
  deferred commit has settled and its `finally` has cleared the timer.
- **C — restructure to `AbortSignal` cancellation.** Thread an `AbortSignal` into
  `commitAfterSteers`; `shutdown()` aborts, which settles the race immediately so the `finally`
  runs. Standard cancel vocabulary; the largest change.
- **D — clear the settle timer inside `shutdown()`, without awaiting.** Track the settle-timer
  *handle* in a loop-level slot and `clearTimeout` it in `shutdown()` after `stopped = true` — no
  gate, no join, no awaited float (~3 lines). The sole armed handle (the liveness leak) is cleared
  so the process exits; the floating `commitAfterSteers` is left as an inert never-resolving
  promise — with a hung steer neither `allSettled` nor the cleared `timeout` resolves, so the race
  at `loop.ts:229` stays pending forever, but a never-resolving JS promise registers no libuv
  handle and so does not hold the event loop open. No-double-commit is already guaranteed by the
  `stopped` guard at `loop.ts:233`.

### Recommendation: B — track + await with a shutdown release gate

Weighed against the three criteria that matter here:

1. **The ruled contract leans toward it — a weak signal, not a mandate.** The ruling and the
   SEA-1173 title say *"tracks and cancels/awaits the floating commitAfterSteers"*. That settled
   the *whether/where* (the lifecycle fix as its own PR — §Fixed intent), **not** the mechanism,
   which §Fixed intent and Open Question 1 both hold open; so it cannot pre-close the fork. What
   the phrasing does disfavor is *pure* **A** (`unref` alone neither cancels nor awaits). It does
   **not** adjudicate B vs C vs D — **D** *cancels* (clears) the timer and so also satisfies
   "cancel" — so the mechanism is weighed on the engineering merits below, not on the wording.
2. **Teardown wants determinism, not latency.** `shutdown()` is on the teardown path — the caller
   (`peer.ts`) runs `await loop.shutdown()` then `mesh.stop()`. B gives the strongest
   post-condition: *after `shutdown()` resolves, no loop continuation is queued and no loop-owned
   timer is armed.* That invariant is what actually ends the #5→#7→#9 finding chain — every prior
   fix left something floating for the next review pass to find. A leaves both the float and an
   (unref'd but armed) timer; **D** clears the timer (no armed handle survives, so its liveness
   matches B) but leaves the float's continuation un-joined, exactly like C without tracking; only
   B leaves *nothing* floating. The join costs microtasks, not the 5s timeout: the gate settles the
   race immediately and the guard at `loop.ts:233` returns before `finishTurn`.
3. **Minimal code still holds.** Fork AGENTS.md:116-117: "**Keep the code clean and minimal.** No
   bloat, no overcomplication." / "**Do only what is asked**". B is ~8 lines — one
   `Promise.withResolvers` gate, a third race arm, an assignment instead of `void`, one joined
   await — no new types and no signature changes to any exported surface.

Shape of B (exact declarations are T2's contract; state sits beside `stopped` at `loop.ts:120`
and `pendingSteers` at `:130`):

```ts
// loop-level state:
let pendingCommit: Promise<void> | undefined;         // the at-most-one floating deferred commit
const stopSettleWait = Promise.withResolvers<void>(); // one-shot release gate, resolved by shutdown()

// commitAfterSteers — the race at loop.ts:229 gains a third arm:
await Promise.race([
  Promise.allSettled([...pendingSteers.values()]),
  timeout,
  stopSettleWait.promise, // shutdown releases the wait NOW → finally clears the timer
]);

// launch site loop.ts:269 — track instead of discarding:
else pendingCommit = commitAfterSteers(generation, to, reply);

// shutdown() loop.ts:279 — join FIRST, then the existing abort/dispose steps unchanged:
stopped = true;
stopSettleWait.resolve();
try {
  await pendingCommit; // deterministic join: settles ≤1 microtask after the release
} catch (e) {
  log(e); // a failed commit tail must not skip abort/dispose (per-step try/catch convention)
}
```

Design notes that make B correct (the traps an implementer would otherwise hit):

- **The gate must be a race arm, not an external `clearTimeout`.** If `shutdown()` cleared the
  timer from outside, the `timeout` promise would never resolve; with a hung steer,
  `Promise.allSettled` never resolves either — the race stays pending forever and
  `await pendingCommit` deadlocks `shutdown()`. Resolving a third arm is the cancel. (This deadlock
  is specific to B, which *awaits* the float; **D** sidesteps it by *not* awaiting — it clears the
  timer and leaves the float inert. See §Alternatives.)
- **Join before abort/dispose.** `stopped = true` is already set, so the joined commit's
  continuation runs `finally` → guard (`loop.ts:233`) → `return`; it cannot re-enter session work
  (`finishTurn` is unreachable). Joining first means the abort/dispose steps run with no loop
  continuation queued behind them, and it keeps the teardown steps independent per the existing
  per-step try/catch convention (`loop.ts:281-285` comment block).
- **A single slot suffices.** A second deferred commit can only be launched by a later turn's
  `agent_end`, which requires the pump inside the previous commit's own `finishTurn` tail — the
  previous float has fully settled by then, so `pendingCommit` is at-most-one and plain
  overwrite is safe.
- **Platform baseline is proven in-tree:** `Promise.withResolvers<void>()` is already used by the
  fork's own suite (`oh-my-pi-peer.smoke.ts:190`).

### Alternatives, weighed

- **A — `timer.unref()` (rejected as the sole fix).** Cheapest (one line) and proven to fix the
  delayed exit (the +0ms half of the proof). Its real residual cost is narrow: `unref()` does not
  *clear* the timer, so in any embedding where `shutdown()` is not followed by process exit (a host
  running several peer lifecycles in one process — the smoke suite itself tears down many loops in a
  single run) the unref'd timer still fires up to 5s post-teardown. It fires into a guarded no-op
  (the race resolves → `finally` clears the already-fired timer → the guard at `loop.ts:233`
  returns because `stopped` is true), so nothing runs — but a real armed handle outlived teardown.
  That is strictly worse than **D**, which *clears* the timer so it never fires at all, for the same
  ~1-line order of cost; A is dominated by D on the record's own liveness intent and is kept only as
  the absolute floor. *Also declined as a belt-and-braces rider on top of B:* once the join
  guarantees the timer is cleared before `shutdown()` resolves, `unref()` is dead weight
  (AGENTS.md:116-117).
- **C — `AbortSignal` restructure (rejected as over-machinery).** Equivalent cancel semantics to
  B's gate, but more surface: an `AbortController` at loop level, listener wiring + cleanup inside
  the timeout executor (or `throwIfAborted` + rejection-path catch discipline) — and *without* the
  tracked `pendingCommit` it still doesn't give the join (shutdown can't know when the `finally`
  ran). A complete C is B plus AbortController ceremony for zero additional guarantee. If the loop
  someday grows multiple cancellable waits, promote the one-shot gate to a controller then.
- **D — clear the settle timer inside `shutdown()`, no await (the lighter fork).** Track the timer
  handle in loop state and `clearTimeout` it in `shutdown()` after `stopped = true` — ~3 lines, no
  gate, no `pendingCommit` slot, no awaited float. It fully removes the stated liveness leak: the
  sole armed handle is cleared, and the now-never-resolving `commitAfterSteers` race (`loop.ts:229`
  — with a hung steer neither `allSettled` nor the cleared `timeout` ever resolves) is an inert JS
  promise that registers no libuv handle, so the event loop is free and the process exits.
  No-double-commit is already guaranteed by the `stopped` guard (`loop.ts:233`) that §"What this is
  NOT" relies on. **D's only gap vs B is the deterministic join:** under D the float's continuation
  is left pending (it never runs — the race never settles), whereas B guarantees it has
  run-and-returned before `shutdown()` resolves. That join is insurance only against a *future*
  edit making `finishTurn` reachable during teardown (the guard is a no-op today) — so whether D's
  ~3 lines or B's ~8 is the right buy is the load-bearing fork (Open Question 1). D strictly
  dominates A (clears rather than unrefs) and is lighter than both B and C.

## Global Constraints

Every task below inherits these; they are not repeated per task.

- **Sequencing gate — implementation only.** The target code exists only on unmerged fork PR #5
  (`sealedsecurity/Cotal`, branch `zheng-connector-upstream`, head `568c817`). Implementation
  starts after #5 merges to the fork's main; all `loop.ts` / `oh-my-pi-peer.smoke.ts` line numbers
  in this record are valid at `568c817` and must be re-anchored on the merged base. This design
  record itself has no such gate.
- **Red-green is mandatory** (per the repo's red-green testing rule) — and because the defect is
  unreachable on today's contract, the regression test MUST **synthesize** the trigger: hold a
  fold's steer promise unsettled past `shutdown()` using the suite's existing controllable steers
  (`StubSession.steerHang`, `oh-my-pi-peer.smoke.ts:151` — never settles; or `deferSteer`,
  `:156` — test-fired settle). Red is watched failing against the unfixed loop before the
  mechanism lands.
- **Timer assertions use a test-scoped `setTimeout`/`clearTimeout` shim — explicitly NOT
  `process.getActiveResourcesInfo()`.** Finding #10 (tripwire record `:151-153`): the
  process-wide handle count "coupled to global state, could spuriously fail / mask a leak"; the
  shim's ledger is local to the test block.
- **No test waits real seconds.** The settle timeout is already injectable
  (`steerSettleTimeoutMs = 5_000`, `loop.ts:108`). The regression test runs the **prod default**
  (that is what makes red observable: a 5s armed timer / delayed teardown) but asserts via the
  shim ledger and `shutdown()`'s prompt resolution — never by sleeping toward the 5s boundary.
- **No AI/tool attribution in fork code, commits, or PRs** (fork `AGENTS.md:141-146`: "No tool or
  AI attribution, anywhere." / "Never self-advertise in a public message."). Comments name
  behaviors and findings, not agents or tools. (This zireael design commit carries the seal
  co-author trailer per this repo's commit conventions — different repo, no conflict.)
- **Minimal code** (fork `AGENTS.md:116-117`: "Keep the code clean and minimal. No bloat, no
  overcomplication." / "Do only what is asked").
- **No exported-surface change.** `runPeerLoop({ mesh, session, steerSettleTimeoutMs? }): PeerLoop`
  (`loop.ts:98-113`) and `PeerLoop.shutdown(): Promise<void>` keep their signatures; tests 1-18
  stay green (test 18's sampler swap in T3 changes instrumentation, not the asserted invariant).

## Plan

All tasks land on the Cotal fork (`sealedsecurity/Cotal`), one branch stacked on the merged #5
base, shipped as **one PR** (the ruled "own PR with its own red-green"). Suite command:
`pnpm smoke:oh-my-pi` (`extensions/connector-oh-my-pi/oh-my-pi-peer.smoke.ts` header). Task order
is the red-green order.

### T1 — RED: regression test + timer-ledger shim (test 19)

In `oh-my-pi-peer.smoke.ts`, add the shim and the shutdown-during-unsettled-steer test:

- **Shim** (module-level helper beside `drain`, `:19-25`): wrap `globalThis.setTimeout` /
  `globalThis.clearTimeout` to record each armed timer's fate — armed, then **cleared** (via
  `clearTimeout`) or **fired** (the wrapped callback ran) — while installed. Timers pass through
  to the real clock (only bookkeeping is added), so the ledger exposes `armed()` / `cleared()` /
  `fired()` / `outstanding()` counts plus `clearAll()` (clear every still-outstanding real handle)
  and `restore()` (restore the globals). `clearAll()` in teardown kills the leaked real 5s timer
  the unfixed loop strands, so the red run fails fast instead of waiting out the real delay.
- **Test 19**: install the shim → `steerHang` session (`:151`) → `runPeerLoop({ mesh, session })`
  at the **prod-default** 5_000ms → `START`, fold `a2` (steer never settles), `end("ans")` →
  the deferred commit's settle timer arms. **Wait for the arm before shutting down** — `drain()`
  the mesh/`agent_end` queue, then poll until `ledger.armed() === 1`, so `shutdown()` can never
  race ahead of the arming (otherwise outstanding is trivially 0 and the red assert passes without
  ever exercising the shutdown-during-unsettled-steer path) → `await loop.shutdown()` → assert:
  1. `ledger.fired() === 0 && ledger.cleared() === 1` — the settle timer was **cleared** by the
     release gate, not left to **fire** at the 5s timeout; `outstanding()` alone can't tell the two
     apart (a fired timer also leaves outstanding = 0), so the fired/cleared counts are load-bearing;
  2. `ledger.outstanding() === 0` — no armed loop-owned timer survives `shutdown()` (RED on
     `568c817`: the settle timer is still armed → outstanding = 1, the +5002ms exit in ledger
     form);
  3. `mesh.acked.length === 0` and no reply delivered — the joined commit's tail stayed a no-op
     (`stopped` guard, `loop.ts:233`); the join must not un-suppress the stale commit;
  4. `session.disposed === 1` — teardown still completed.
  Teardown (always): `ledger.clearAll()` then `ledger.restore()`.
- Run the suite; watch test 19 fail on the unfixed loop — on `568c817` the settle timer stays
  armed and is never cleared, so assert 1 (`cleared === 1`) and assert 2 (`outstanding === 0`)
  both bite. Record the red output in the task report.

Interfaces:

```ts
// consumed (loop.ts:98-113, :279):
runPeerLoop({ mesh, session, steerSettleTimeoutMs = 5_000 }:
  { mesh: PeerMesh; session: PeerSession; steerSettleTimeoutMs?: number }): PeerLoop;
shutdown(): Promise<void>;                       // PeerLoop, loop.ts:279
// consumed (oh-my-pi-peer.smoke.ts): StubSession.steerHang (:151), .deferSteer (:156), .rejectSteer (:159)
// produced (new, module-level in oh-my-pi-peer.smoke.ts):
function installTimerLedger(): {
  armed(): number; cleared(): number; fired(): number; outstanding(): number;
  clearAll(): void; restore(): void;
};
```

Test cycle: `pnpm smoke:oh-my-pi` — tests 1-18 green, test 19 **red**.

### T2 — GREEN: mechanism B in `loop.ts`

Implement the tracked join exactly as shaped in *Approach* (§Recommendation):

1. Loop-level state beside `stopped` (`loop.ts:120`) / `pendingSteers` (`:130`):
   `pendingCommit` slot + one-shot `stopSettleWait` release gate.
2. Third arm `stopSettleWait.promise` in the race at `loop.ts:229` — shutdown's release settles
   the race so the `finally` (`:230-231`) clears the timer.
3. Launch site `loop.ts:269`: `pendingCommit = commitAfterSteers(generation, to, reply);`
   (replaces the `void` discard).
4. `shutdown()` (`loop.ts:279`): after `stopped = true` (`:280`), resolve the gate and
   `await pendingCommit` in its own try/catch (`log(e)`, per the existing per-step teardown
   convention, `:281-285`), before the abort/dispose steps.

Interfaces:

```ts
// unchanged signature, one new race arm (loop.ts:220, race at :229):
async function commitAfterSteers(gen: number, to: InboxItem | undefined, reply?: string): Promise<void>;
// new loop-level state (beside `stopped`, loop.ts:120):
let pendingCommit: Promise<void> | undefined;
const stopSettleWait: PromiseWithResolvers<void>; // = Promise.withResolvers<void>()
// join added at the top of shutdown(): Promise<void> (loop.ts:279); signature unchanged
```

Test cycle: `pnpm smoke:oh-my-pi` — test 19 flips **green**; tests 1-18 stay green (16/18 prove
the allSettled-wins path and its timer clear are untouched; 12/14 prove the strand/generation
paths are untouched).

### T3 — Migrate test 18 onto the ledger shim (finding #10)

Replace test 18's `process.getActiveResourcesInfo().filter((r) => r === "Timeout")` sampling
(`oh-my-pi-peer.smoke.ts:685`, `:698`) with the T1 shim: same invariant (the settle timer leaves
no net armed handle once allSettled wins), asserted against the local ledger instead of the
process-wide handle table. Assertion messages keep naming finding #7.

Interfaces: `installTimerLedger()` from T1; test 18's asserted invariant unchanged
(`after === before` becomes `ledger.outstanding() === 0`).

Test cycle: `pnpm smoke:oh-my-pi` — suite green before and after the swap (instrumentation-only
change; test 18 must still go red against a timer-clear-reverted loop copy, re-proving #7's bite
survives the migration).

### T4 — Comment/doc alignment in `loop.ts`

Update the three comment sites whose contract the mechanism changes — no behavior edits:

- the `stopped` comment (`loop.ts:116-119`, "…and the deferred commit."): note the deferred
  commit is now also *joined* by shutdown;
- the `commitAfterSteers` doc block (`loop.ts:212-219`, "Guarded so a shutdown mid-wait … never
  commits a stale/disposed turn."): document the release gate — shutdown settles the race
  immediately, the `finally` clears the timer, the guard keeps the commit a no-op;
- the `shutdown()` body comments (`loop.ts:281-285`): document the join step and why it precedes
  abort/dispose.

Interfaces: none (comments only).

Test cycle: `pnpm smoke:oh-my-pi` green (no behavior change to observe; the cycle guards against
an accidental code edit).

## Tasks

- [ ] T1 — RED: `installTimerLedger` shim + test 19 (shutdown-during-unsettled-steer); red
      observed on the unfixed loop.
- [ ] T2 — GREEN: mechanism B (`pendingCommit` slot + `stopSettleWait` race arm + joined
      `shutdown()`); test 19 green, 1-18 green.
- [ ] T3 — Test 18 migrated off `process.getActiveResourcesInfo()` onto the shim (finding #10);
      red-bite re-proven against a reverted loop copy.
- [ ] T4 — `loop.ts` comment alignment (`:116-119`, `:212-219`, `:281-285`); suite green.

## Open Questions

Batched for the merge review of this record; each carries the assumption the Plan is written
against, so execution never stalls.

1. **[load-bearing — the mechanism] A, B, C, or D?** This is the fork the escalation record
   flagged "design-pass-first" (tripwire `:186`) and the decision this PR exists to settle. The
   live contest is **B vs D**: both remove the liveness leak and both satisfy the ruling's
   "cancel/await" phrasing (§Recommendation pillar 1); they differ only in whether `shutdown()`
   deterministically *joins* the float (B, ~8 lines) or merely clears its timer and leaves it inert
   (D, ~3 lines). B's extra guarantee is insurance against a future edit that makes `finishTurn`
   reachable during teardown — a guard that is a no-op today.
   *Stated assumption: **B** — for the strongest teardown post-condition (no queued continuation,
   no armed timer), which is what durably ends the #5→#7→#9 floating-seam chain: the ~5-line delta
   over D buys a design a later reachability change can't silently re-break. **D** is the lighter,
   defensible alternative if that insurance is judged not worth the join machinery for a defect
   unreachable on today's contract; **A** is the one-line floor (dominated by D — it unrefs rather
   than clears); **C** only if the loop later grows multiple cancellable waits.*
2. **Does the #10 test migration (T3) ride this PR?** Non-load-bearing scope call: it shares the
   T1 shim and the tripwire filed it alongside #9, but it could ship as its own change. *Stated
   assumption: yes — same PR, separate commit, so the shim lands with both of its users.*
3. **Join-failure policy: swallow or propagate?** If the joined `pendingCommit` rejects (it has no
   rejection path today — its body ends in guarded sync calls — but a future edit could add one),
   should `shutdown()` log-and-continue or rethrow? Non-load-bearing: the guard already suppresses
   the commit itself. *Stated assumption: log-and-continue (`try { await pendingCommit } catch (e)
   { log(e) }`), matching the existing per-step teardown convention (`loop.ts:281-285`) — a failed
   commit tail must never skip abort/dispose/mesh-stop.*

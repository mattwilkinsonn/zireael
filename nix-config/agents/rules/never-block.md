---
description: "Never block yourself waiting on a peer or a job — a blocking wait makes you deaf to all other incoming messages. Yield the turn and resume when the result lands as a fresh message."
---

# Never block — stay reachable

**Never use a blocking wait as your idle or coordination pattern.** No `irc wait`,
no `irc send await:true`, no blocking `job poll` to sit and wait on another agent
or a long-running job. When the instinct is "I'll just wait here until X answers,"
the answer is: end the turn instead.

## Why

A blocking wait makes you **deaf to every other channel** until the one thing you
parked on arrives. While you sit blocked you miss:

- **Steering** — a human interjection or a higher-priority reassignment that should
  change what you're doing *right now*.
- **Other peers** — new findings, a question from a second agent, an unblock you
  were the blocker for.
- **The wave** — a coordination chain can **deadlock** when two agents each block
  waiting on the other.

This is doubly critical for a **supervisor** (it must stay free to route the whole
wave), but it applies to **every** agent — a blocked worker misses steering and
can stall a chain it didn't know it was in.

## Instead

- **End your turn.** The harness delivers new peer messages **and** job
  completions into your next turn automatically — you do not hold a turn open to
  receive them. A backgrounded job (`bash` with `async: true`) surfaces its result
  on its own; a peer DM surfaces on its own. You re-engage when it lands.
- **Or poll non-blockingly, then yield.** `irc inbox` / `job list` take a snapshot
  and return immediately; read it and end the turn. Never loop on a blocking poll.
- **Waiting on a decision is not a reason to block.** Asked the human something?
  Yield, stay reachable, resume when the answer arrives as a fresh message — don't
  hold the turn hostage to the reply.

## The one wait that's allowed

A **bounded, backgrounded** wait you launch and then **yield** from — e.g.
`wait-for-reviews <pr>` run via `bash` with `async: true` and a generous timeout.
That is not blocking: you start it, end your turn, and the harness wakes you with
the result. The banned thing is a **foreground** wait that holds the turn open and
blinds you until it returns. If you are still holding the turn to watch something,
it is a block — background it and yield.

## Composes with `rule://hold-your-lane`

Yielding a turn keeps you reachable, but it does **not** hand off your work. A PR
you own that's still gated on review/CI/the merge gate stays your responsibility
across every yield — `hold-your-lane` is the ownership half of the same coin.
`never-block` is **how** you wait (background it, yield, don't hold the turn open);
`hold-your-lane` is **what you stay responsible for** (your gated lane, until it
actually merges). Don't read "end the turn and stay reachable" as "walk away and
pick up new work."

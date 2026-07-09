---
description: "A gated PR is not done — done means merged (or closed/dropped). Hold your lane until then; don't pick up new work while a PR you own can still bounce, because you must stay present to auto-fix and re-drive it."
---

# Hold your lane until it merges

**A PR sitting at review, CI, or the merge gate is not finished work.** "Done"
means **merged** — or explicitly closed/dropped. Until then the lane is still
yours and still open: don't context-switch to a new task, and don't volunteer for
new work, while any PR you own can still bounce back to you.

## Why

An open PR can still demand your attention at any moment:

- **A bot re-review** posts a P1/P2 you need to fix and re-push.
- **CI goes red** on a leg that hadn't finished when you walked away.
- **The human requests changes** at the merge gate.

Every one of these bounces the PR back to its author. If you've already moved on
to a new task, the bounce sits unhandled — the lane stalls, the stack behind it
stalls, and the supervisor has to chase you. Staying present on your own gated PR
so you can auto-fix and re-drive it (`skill://autonomous-review`) is the job, not
a thing you do *before* the real next task.

## What this means

- **Only offer for new work when every lane you own is actually merged** (or
  closed/dropped). A green, merge-ready PR at the human gate is *not* a free hand
  — it can still get change requests.
- **Hold your lane = stay responsible for it**, through every bounce, until it
  lands. Re-drive re-reviews, fix red CI, address change requests.
- **Finished driving and everything's green?** Park *on that lane* — don't grab a
  new one. Re-engage when a bounce or the merge lands.

## Reconciles with `rule://never-block`

These compose; they don't conflict. `never-block` governs **how** you wait — you
do **not** sit in a foreground blocking wait; you end the turn (or poll
non-blockingly) and yield, so the review/CI result wakes you as a fresh message.
`hold-your-lane` governs **what you stay responsible for** — the gated lane
remains yours across those yields. Yielding the turn keeps you reachable *for your
own lane*; it is not permission to pick up a different one. Background the review
waiter, yield, and let the bounce (or the merge) bring you back.

---
name: design
description: "Lean up-front design for a change: one pass, one artifact (Problem · Approach · Plan · Tasks), depth-scaled with a fast path. A Fable subagent drafts it; the human reviews and freezes it as the contract executing agents read."
---

# Lean design

Use before implementing a non-trivial change. Past the fast path, the main
agent **delegates the design pass to the `design` subagent** — it runs on Fable,
grounds itself in the codebase, and writes the `design.md` into the repo. The
human **reviews and freezes** that one artifact; execution then proceeds on the
default model (Opus). Deliberately light: the heavy autonomous brainstorm +
per-task subagent machinery other frameworks bundle is replaced by our split —
a Fable subagent drafts the design, the human freezes it, Opus executes +
self-reviews. This skill is **only the design layer**.

## Fast path

If the change fits in one sentence and has no real design fork, **skip the
artifact — just do it**. A one-line diff, a config tweak, a rename needs no
design doc. (This is the explicit inverse of "design everything.")

## Delegate the design pass

For any non-fast-path change, the main agent spawns the design subagent instead
of drafting the artifact inline:

- `task(agent: "design", …)` — it runs on Fable (`pi/designer`), does its own
  codebase recon, and writes `design.md` into the repo under
  `docs/design/<change>/` (or `openspec/changes/<id>/`).
- Coding then proceeds on the default model (Opus) against the frozen artifact.

The subagent drafts, the human freezes, Opus executes — the main agent doesn't
hand-write the design when it can delegate the pass.

## One pass, one artifact

Produce a single `design.md` (under `docs/design/<change>/`, or
`openspec/changes/<id>/` if we adopt OpenSpec's layout) with four short sections:

- **Problem / Intent** — what and why, 1–3 sentences.
- **Approach** — the chosen approach; list alternatives only when the choice
  isn't obvious (one recommended + why).
- **Plan** — decomposed tasks (see Plan discipline).
- **Tasks** — a checklist mirroring the plan.

## Clarifications: batched, via the relay

The `design` subagent is headless — it has **no `ask` tool** and cannot prompt
the human. So it batches **all** open questions and assumptions into an **Open
Questions** section of the `design.md` (and its returned summary), designing
against a stated assumption rather than stalling. The **main agent relays those
questions to the human in a single `ask`** — never a Socratic
one-question-per-turn loop, which is the main thing that makes heavier flows
slow. The human answers once; the design is updated and frozen.

## No gates, no loops, no chaining

The human reviews the one frozen artifact once — they **review and freeze** it
(the Fable subagent drafts it, Opus executes against it). No mandatory
self-review loops, no per-section approval gates, no sub-skill chain.

## Plan discipline (the one hard requirement)

The plan/tasks MUST carry (lifted from Superpowers' validated plan-crispness,
MIT) — this is what makes the artifact a clean, cheap-to-execute contract:

- **Right-sized tasks** — the smallest unit that carries its own test cycle and
  is worth a fresh reviewer's gate.
- **A `## Global Constraints` header** — version floors, naming/copy rules,
  platform requirements — so every task brief inherits them (never buried in
  prose, where they get missed).
- **Per-task `Interfaces:`** — what it consumes/produces, with exact signatures,
  so the executor doesn't burn calls re-discovering context.
- **No placeholders** — every task is concrete and complete.
- *(optional)* a per-task model-tier hint.

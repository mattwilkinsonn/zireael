---
name: design
description: "Lean up-front design for a change: one pass, one artifact (Problem · Approach · Plan · Tasks), depth-scaled with a fast path. The human drives it in minutes; it then freezes as the contract executing agents read."
---

# Lean design

Use before implementing a non-trivial change. The human runs this interactively
(a few minutes), then the artifact is **frozen** and handed to executing agents
as the contract. Deliberately light: the heavy autonomous brainstorm + per-task
subagent machinery other frameworks bundle is replaced by our split — the human
designs here; agents execute + self-review. This skill is **only the design
layer**.

## Fast path

If the change fits in one sentence and has no real design fork, **skip the
artifact — just do it**. A one-line diff, a config tweak, a rename needs no
design doc. (This is the explicit inverse of "design everything.")

## One pass, one artifact

Produce a single `design.md` (under `docs/design/<change>/`, or
`openspec/changes/<id>/` if we adopt OpenSpec's layout) with four short sections:

- **Problem / Intent** — what and why, 1–3 sentences.
- **Approach** — the chosen approach; list alternatives only when the choice
  isn't obvious (one recommended + why).
- **Plan** — decomposed tasks (see Plan discipline).
- **Tasks** — a checklist mirroring the plan.

## Clarifications: batched, never one at a time

If anything is ambiguous, ask **all** open questions in a single message, then
generate the artifact. No Socratic one-question-per-turn loop — that's the main
thing that makes heavier flows slow.

## No gates, no loops, no chaining

The human reviews the one artifact once (they're the designer). No mandatory
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

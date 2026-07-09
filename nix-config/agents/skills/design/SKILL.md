---
name: design
description: "Lean up-front design for a change: one pass, one record (Problem · Approach · Plan · Tasks), depth-scaled with a fast path. A Fable subagent drafts it into the repo; it ships as its own PR, reviewed by the human and the AI bots, then frozen on merge as the contract executing agents read."
---

# Lean design

Use before implementing a non-trivial change. Past the fast path, the main
agent **delegates the design pass to the `design` subagent** — it runs on Fable,
grounds itself in the codebase, and writes the design record into the repo. It
ships as its own PR — the human **and** the review bots review it, and the
**merge freezes** it; execution then proceeds on the
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
  codebase recon, and writes the record into the repo at
  `docs/designs/<domain>/<record>.md`.
- Coding then proceeds on the default model (Opus) against the frozen artifact.

The subagent drafts, the human freezes, Opus executes — the main agent doesn't
hand-write the design when it can delegate the pass.

## One pass, one artifact

Write a single design record **into the repo** at
`docs/designs/<domain>/<record>.md` (`<domain>` = `platform` / `tools` /
`agents` / `product`; `<record>` = a short kebab slug). It's a committed file
that ships as a PR (see **Ship it as a reviewed PR**), never a local scratch
artifact. Four short sections:

- **Problem / Intent** — what and why, 1–3 sentences.
- **Approach** — the chosen approach; list alternatives only when the choice
  isn't obvious (one recommended + why).
- **Plan** — decomposed tasks (see Plan discipline).
- **Tasks** — a checklist mirroring the plan.

## Clarifications: batched, via the relay

The `design` subagent is headless — it has **no `ask` tool** and cannot prompt
the human. So it batches **all** open questions and assumptions into an **Open
Questions** section of the record (and its returned summary), designing
against a stated assumption rather than stalling. The **spawning agent then
asks the human directly in a single `ask`** — batched and structured, every
question with a recommendation — never a Socratic one-question-per-turn loop
(the main thing that makes heavier flows slow), and never routed through a
supervisor or coordinator (that buries the decision in a coordination
stream). The human answers once; the design is updated and frozen.

## No merge with open questions (the pre-freeze gate)

The merge freezes the record, and executing agents build against it as the
contract — so an unresolved question in a merged record is an ambiguous
contract. Screen the **Open Questions** section against one bar before the PR
can merge:

- **Load-bearing** — an executor building against the frozen record would hit
  real ambiguity (which API, which value, which of two designs). It **blocks the
  merge**. Ask the human directly (batched, via the `ask` relay — see above),
  get the answer, and **fold it into the record as a Decision** — replace the
  question with the decided outcome — *before* the merge-freeze.
- **Non-load-bearing** — explicitly marked as such, deferred with a rationale
  (the design is correct without it; it's an optional later optimization). This is
  a **documented deferral, not a blocker**: the record may merge with it, and
  the merge ratifies the deferral.

So at merge the `## Open Questions` section is empty or holds **only** explicit
non-load-bearing deferrals — never a live load-bearing question. It's a
**pre-merge staging area, not part of the frozen contract**.

**There is no folding-in after merge.** The merged record is frozen — the sealed
repo's `AGENTS.md` sets the convention: a later change ADDS a new record, never
rewrites the merged one — so a load-bearing question that reaches merge unresolved cannot be
patched in place afterward. Resolve it before the freeze, or defer it
explicitly.

## Ship it as a reviewed PR

The design record is reviewed **on a pull request**, not in a local buffer —
that's what lets the human **and** the AI review bots read the design before any
implementation exists. Once the subagent has drafted the record (`docs/designs/<domain>/<record>.md`) into the repo:

- **Its own branch/PR, separate from the implementation** — commit the record
  (`docs(<domain>): <change>` subject + `Co-Authored-By: seal` trailer), so the
  design is reviewed as pure design with zero code noise.
- **`gt submit`** (never `gh pr create` — that opens under the bot account
  outside the Graphite stack; `gt submit` is the only sanctioned PR-open path,
  `rule://commit-conventions`), then drive `skill://autonomous-review` —
  un-draft (via the GitHub MCP), let the bots review, triage findings, iterate.
- **The merge is the freeze.** The design PR merging to `main` is what freezes
  the contract; execution starts from the merged record. Matt merges — you never
  do.

No mandatory self-review loops, no per-section approval gates, no sub-skill
chain — the single PR review is the whole gate (the subagent drafts, the bots +
human review, the merge freezes, Opus executes against it).

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

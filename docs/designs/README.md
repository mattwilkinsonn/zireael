# Design records

Point-in-time **design records** — the *why* behind a change (problem,
alternatives weighed, decision, plan), frozen once decided. One file per record
at `docs/designs/<domain>/<record>.md` (`<domain>` = `platform`, `tools`,
`agents`, `product`). Author new ones with `skill://design`, which ships each
record as its own PR so it's reviewed — by a human and the AI review bots —
before the implementation lands.

Empty for now — current behavior lives in [`../specs/`](../specs/); new
significant changes add a record here.

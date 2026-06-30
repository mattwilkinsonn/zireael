# docs/

Internal docs for zireael — the jj tooling, the nix host platform, and
the agent config. Two kinds, mirrored by domain:

- **`designs/<domain>/`** — point-in-time **design records**: the *why*
  (problem, alternatives, decision, plan). Frozen once decided.
- **`specs/<domain>/`** — the **living source of truth**: how a component
  *currently* behaves. Behavior contracts use `### Requirement:` +
  `#### Scenario:` (RFC 2119 + Given/When/Then) where security- or
  interface-critical; prose + tables elsewhere.

Domains: `platform/` (nix hosts + CI), `tools/` (jj-gt, jj-hooks),
`agents/` (the push-guard extension + agent config).

## Living specs — keep them current

A change that alters behavior updates the matching `specs/` doc **in the
same PR** as the code. The design's `## Spec impact` (or the PR body)
names which specs change, or says `Spec-impact: none`. Reconciling the
living docs is part of the merge-ready bar.

## Current specs

- [`specs/platform/hosts.md`](specs/platform/hosts.md) — nix host platform.
- [`specs/platform/ci.md`](specs/platform/ci.md) — CI matrix.
- [`specs/tools/jj-gt.md`](specs/tools/jj-gt.md) — jj-gt.
- [`specs/tools/jj-hooks.md`](specs/tools/jj-hooks.md) — jj-hooks.
- [`specs/agents/push-guard.md`](specs/agents/push-guard.md) — push-guard extension.

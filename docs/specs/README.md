# Living specs

The **living source of truth** for how zireael's components currently
behave, by domain:

- **`platform/`** — the CI matrix.
- **`tools/`** — `jj-gt`, `jj-hooks`.

Behavior contracts use `### Requirement:` + `#### Scenario:` (RFC 2119
SHALL/MUST + Given/When/Then) where security- or interface-critical;
prose + tables elsewhere. A behavior change updates the matching spec in
the same PR.

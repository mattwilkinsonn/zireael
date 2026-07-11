# Living specs

The **living source of truth** for how zireael's components currently
behave, by domain:

- **`platform/`** — the CI matrix.
- **`tools/`** — `jj-gt`, `jj-hooks`.

The nix host platform (`hosts.md`) and the push-guard extension
(`push-guard.md`) specs moved with their code to `sealedsecurity/sealed`
under `personal/matt/docs/specs/{platform,agents}/` in the `nix-config/`
migration.

Behavior contracts use `### Requirement:` + `#### Scenario:` (RFC 2119
SHALL/MUST + Given/When/Then) where security- or interface-critical;
prose + tables elsewhere. A behavior change updates the matching spec in
the same PR.

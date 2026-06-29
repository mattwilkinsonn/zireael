---
description: "Before calling work done, run format + lint + tests for the affected area and state explicitly that all ran and passed."
---

# Pre-finish checks (every change, every time)

Before reporting a change as done, run the format, lint, and test gates against the affected package/area and confirm each one came back clean. "Builds clean" is not the same as "tested" or "linted" — a green compile says nothing about formatting drift or a failing assertion.

## The gate

Three things must run and pass for the area you touched:

1. **Format** — the formatter, applied to the changed files.
2. **Lint** — the linter, with tests included where the toolchain supports it.
3. **Tests** — the unit/integration tests covering your change.

Per-ecosystem examples (illustrative, not mandates — use whatever the repo actually standardizes on):

| Ecosystem | Format | Lint | Tests |
| --- | --- | --- | --- |
| Rust | `cargo fmt -p <crate>` | `cargo clippy -p <crate> --tests` | `cargo nextest run -p <crate>` |
| JS/TS | formatter (e.g. `prettier`) | `biome check` | `bun test` |
| Nix | `nixpkgs-fmt` | `statix check` | `nix flake check` |

If the repo defines a single aggregate check command, prefer it — it tends to cover format-check, lint, build, and tests in one pass, which a bare test run does not.

## State it explicitly

The summary that closes the work MUST say that all three gates ran and passed. Saying "done" without running them is lying by omission. Naming only one ("tests pass") while implying the rest is the same failure — call out each gate, or call out which you skipped and why.

## Scope of the test run

Run only the tests you added or changed, unless asked otherwise. A targeted run keeps the loop fast and the signal sharp; a full-suite run is the caller's choice to request.

## Format fallout belongs to the cause

If formatting a change surfaces churn in files an earlier commit owns, push those hunks down into the commit that caused them — do not land a standalone "fmt" commit on top. The fix belongs in the commit responsible for it.

## Commit / push posture

You MAY commit the verified change (Conventional Commits subject) and push/submit your own feature branch over the seal-bot token, then run the review loop (`skill://autonomous-review`). Never push or force-push `main`, never merge (the human gate), never push/PR/issue outside `mattwilkinsonn/*` + `sealedsecurity/*`. Identity + policy: `rule://commit-conventions`.

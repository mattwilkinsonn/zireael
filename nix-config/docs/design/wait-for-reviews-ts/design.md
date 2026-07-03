# wait-for-reviews → TypeScript (bun)

## Problem / Intent

`wait-for-reviews` (the autonomous-review wait primitive) is a 188-line bash
script with real logic — arg parsing, a tern/gh-route fetch with error handling,
a per-bot classifier, and a poll loop. Bash makes it un-typed and awkward to
test (the current test is a 234-line `.sh` harness that greps a live-extracted
jq filter). SEA-932 is converting sealed to zero-committed-bash (TypeScript via
bun); this applies the same pattern to zireael's nix-config, and adds the CI
teeth that force *all* future agent scripts to be TS.

The AGENTS.md rule ("Scripts: TypeScript over bash", #224) already exists but is
**unenforced** — nothing stops an agent committing a new `.sh`. This change gives
the rule a CI gate.

## Approach

Mirror the two existing bun-tool precedents already in this repo:

- **`tools/akiflow-cli/`** — zireael's canonical bun/TS tool: own `bun.lock`,
  `moon.yml` with `install`/`lint`/`typecheck`/`test`/`ci` (bunx biome + bunx tsc
  --noEmit + bun test), `package.json` with a `bin`.
- **`sealed/tools/linear-auto-done/`** — the construction/execution split:
  pure functions factored out, a `Deps` type, `runOnce(deps)` taking injected
  `env`/`fetch`/`log`, test seams `export`ed, and an `if (import.meta.main)`
  entry guard. Tests pass fakes; production wires `process.env` + real `fetch`.

New tool at **`nix-config/tools/wait-for-reviews/`** (a bun package): `index.ts`
(logic, split for tests) + `index.test.ts` (`bun test`) + `package.json` +
`tsconfig.json` + `bun.lock` + `moon.yml`. Nix packages it by having the
`writeShellScriptBin` wrapper `exec bun ${./index.ts} "$@"` (bun is already in
the dev set, `shared/dev.nix:37`).

### Alternatives considered

- **Keep the bash `gh-route` and shell out to it** vs. **also convert
  gh-route**: see Fork A.
- **Nix packaging: thin `exec bun` wrapper** (recommended — minimal, bun on
  PATH already) vs. a full `buildNpmPackage`/bun2nix derivation (heavier, pins
  deps into the store; overkill for a zero-dep script that only uses `Bun.$` +
  `fetch`).
- **CI gate placement**: a `no-new-bash` moon task in `nix-config/moon.yml`
  (recommended — that's where the nix-config gate lives) vs. a repo-root check.

## Global Constraints

- **Runtime**: bun (already in `shared/dev.nix`). Scripts use `Bun.$` and global
  `fetch` — no external deps beyond `@types/bun` + `typescript` (dev only).
- **Style**: match `tools/akiflow-cli` — tabs, biome, `tsc --noEmit` strict.
- **Behavior parity**: the TS port must be byte-for-byte behavior-compatible —
  same stdout format (`wait-for-reviews: O/R#N head=… bots=[…] …`, the
  per-bot `%-26s %s` printf lines), same exit codes (2 = usage), same env vars
  (`WAIT_BOTS`/`WAIT_GRACE_SECS`/`WAIT_BACKSTOP_SECS`/`WAIT_POLL_SECS`,
  `LITELLM_MCP_URL`/`LITELLM_API_KEY`), same tern-then-gh-route fallback, same
  classifier verdicts. The 18 existing test cases port over and must stay green.
- **No new committed `.sh`**: the gate is the point; the port's own tests are TS.
- **Nix eval must stay green** (`mattpc-wsl` toplevel) — the derivation change is
  load-bearing.

## Plan

1. **Scaffold `tools/wait-for-reviews/`** — `package.json` (name
   `@nix-config/wait-for-reviews` or `wait-for-reviews`, `bin`, dev deps
   `@types/bun` + `typescript`), `tsconfig.json` (copy akiflow-cli's), `bun.lock`
   (`bun install`), `bunfig.toml`, `moon.yml` (copy akiflow-cli's tasks).
   Interfaces: produces a buildable bun package; `moon run wait-for-reviews:ci`
   green on an empty stub.

2. **Port `index.ts`** — construction/execution split:
   - `parseArgs(argv): {pr, repo} | {usage: true}` — pure; the arg cases.
   - `ternState(deps, {url,key,repo,pr}): Promise<State | null>` — the MCP
     fetch + SSE unwrap + isError guard; injectable `fetch`.
   - `classify(bot, {reviews, comments, head}): "done"|"limited"|"stale"|"pending"`
     — pure; the per-bot logic.
   - `runOnce(deps)` / `main` — the poll loop, `Deps = {env, fetch, log, err,
     sleep, ghRoute, now}`. `import.meta.main` guard.
   - Interfaces: `export { parseArgs, ternState, classify, runOnce }`.

3. **Port tests → `index.test.ts`** — the 18 cases as `bun test`: `parseArgs`
   (arg validation incl. the `--repo` edge cases), `ternState` (isError /
   JSON-RPC error / success against canned SSE payloads via a fake `fetch`),
   `classify` (per-bot verdicts). Fakes injected, no network.

4. **Nix packaging** — in `shared/dev.nix`, replace the `readFile`
   `writeShellScriptBin "wait-for-reviews"` with a wrapper that
   `exec bun ${../tools/wait-for-reviews/index.ts} "$@"`. Interfaces: `nix eval`
   green; `wait-for-reviews` on PATH runs the TS.

5. **moon CI wiring** — add `wait-for-reviews` project to `.moon/workspace.yml`;
   its `moon.yml:ci` runs lint+typecheck+test.

6. **`no-new-bash` gate** — a task (in `nix-config/moon.yml`, added to `ci`
   deps) that fails if a committed `.sh`/extensionless-bash appears outside an
   allowlist (the genuinely-required bash: nix bootstrap, yabai, darwin setup).
   Allowlist the pre-existing bash we're *not* converting this pass; block *new*
   ones. Interfaces: `moon run nix-config:no-new-bash` red on a stray new `.sh`.

7. **AGENTS.md** — the project-level `nix-config/agents/AGENTS.md` already has
   the rule; add the CI-gate teeth reference + point at
   `tools/wait-for-reviews` as the canonical example (fixing the dangling-ref
   mistake greptile flagged on sealed#371 by pointing at a file that exists).

8. **Cleanup** — remove `dotfiles/scripts/wait-for-reviews` +
   `tests/wait-for-reviews.test.sh`; update `CHANGELOG.md`.

## Decisions (forks resolved)

- **Fork A → convert both.** `wait-for-reviews` and `gh-route` both port to TS in
  this PR. Two sibling bun packages under `nix-config/tools/`; `wait-for-reviews`
  shells out to the `gh-route` bin via `Bun.$` (injected as `deps.ghRoute` for
  tests), preserving the existing CLI boundary — gh-route stays a standalone CLI
  agents call directly (`pick`, `remaining`, …).
- **Fork B → allowlist gate, aggressively shrunk.** The `no-new-bash` gate fails
  on any committed `.sh`/bash not in an explicit allowlist. The allowlist starts
  as the current retained bash MINUS the two tools converted here, and every
  entry carries a why-bash reason; the goal is to shrink it to zero over
  follow-ups.
- **Fork C → `nix-config/tools/`.** `nix-config/tools/wait-for-reviews/` and
  `nix-config/tools/gh-route/`, each its own bun package (mirrors
  `tools/akiflow-cli`).

## Tasks

- [ ] Scaffold `tools/wait-for-reviews/` bun package (mirror akiflow-cli)
- [ ] Port `index.ts` (parseArgs / ternState / classify / runOnce split)
- [ ] Port 18 tests → `index.test.ts` (bun test, injected fakes)
- [ ] Nix: `writeShellScriptBin` → `exec bun index.ts` wrapper; nix eval green
- [ ] moon: register project + ci wiring
- [ ] `no-new-bash` CI gate + allowlist for retained bash
- [ ] AGENTS.md: gate teeth + canonical-example pointer
- [ ] Remove old `.sh` + tests; CHANGELOG
- [ ] Verify: moon ci + nix eval + live smoke test; submit PR + drive review

## Open forks (need a decision)

- **Fork A — gh-route scope.** `wait-for-reviews` depends on `gh-route` (bash,
  279 lines, own test, own responsibilities). Options: (A1) convert only
  wait-for-reviews now; it shells out to the bash `gh-route` binary (still on
  PATH) — smallest diff, but leaves a `.sh` the gate must allowlist. (A2)
  convert both in this PR — clean (both TS, gate needs no gh-route exception),
  bigger diff. (A3) convert wait-for-reviews now, file a follow-up for gh-route
  (allowlist it meanwhile). Recommend **A3**: single-purpose PR, gh-route is a
  clean separate unit, keeps the diff reviewable.

- **Fork B — gate strictness.** The repo has ~20 committed `.sh` (yabai,
  darwin/nixos bootstrap, shared bootstrap, gh-route, the two test.sh).
  Zero-bash is the *end state*; this PR can't convert all 20. Options: (B1)
  gate blocks only **newly-added** `.sh` (allowlist = the current set) — enforces
  "no new bash" immediately, converts the rest over time. (B2) gate blocks all
  `.sh` not in a shrinking allowlist. Same mechanism; B1 is the honest framing.
  Recommend **B1**.

- **Fork C — tool location.** `nix-config/tools/wait-for-reviews/` (new `tools/`
  dir in nix-config, mirrors zireael-root `tools/`) vs.
  `dotfiles/scripts/wait-for-reviews/` (beside where the script lives today).
  Recommend **`nix-config/tools/`** — matches the akiflow-cli precedent and
  keeps bun packages out of the dotfiles tree.

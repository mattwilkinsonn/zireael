# Design: Standalone release driver for jj-hooks + jj-gt

Status: **proposed**
Domain: tools

Sibling of `docs/designs/tools/oss-tool-extraction-and-shared-tooling.md`
(the extraction record, frozen) and
`docs/designs/tools/distribution-continuity-pre-private-flip.md` (the
continuity record). This record restores the one-command release driver the
extraction dropped; it does not reopen either sibling's decisions.

## Problem / Intent

The monorepo has a one-command release driver
(`tools/release/index.ts`, run as `moon run root:release -- vX.Y.Z`) that
validates, bumps, commits, tags, and pushes a release in one shot. The
extraction moved the crates to standalone repos but not the driver:
`git ls-files` in both `mattwilkinsonn/jj-hooks` and `mattwilkinsonn/jj-gt`
was checked this session and neither carries a `tools/` directory or any
driver script — the release-ish paths present are
`.github/workflows/release.yml` and `.github/scripts/bump-formulae.py`. So
cutting a standalone release is a hand-run sequence today. Intent: one
command again, in both repos, without re-introducing per-repo copy drift.

## Approach

### Ground truth (verified this session)

**The monorepo driver.** `tools/release/index.ts` (274 lines) +
`index.test.ts` (292 lines). Its steps, per the header comment
(`index.ts:1-13`): validate version + working copy; bump workspace
`Cargo.toml` + `Formula/*.rb`; commit `release: vX.Y.Z`; tag `@-`; advance
`main`; push main + the tag. It is dependency-injected —
`index.ts:86-97` defines a `Deps` type whose `sh` / `readFile` /
`writeFile` / `glob` / `log` / `err` are passed in, with real Bun wiring
only under `import.meta.main` (`index.ts:241-274`) — which is what makes
the 292-line test file with fake deps possible. That testability is
preserved by this design.

Four guards, in order:

1. Clean `@` — `index.ts:111-117` (`jj diff --summary
   --ignore-working-copy`, error on any output).
2. `main` is an ancestor of `@` — `index.ts:122-139` (`jj log -r
   "main & ::@"`).
3. Tag does not already exist — `index.ts:142-151` (`jj tag list`).
4. cargo-edit installed — `index.ts:153-161` (`cargo set-version --help`
   exit code).

Bump phase: `cargo set-version --workspace` (`index.ts:164`), a
`Formula/*.rb` glob rewrite (`index.ts:169-171`), `cargo update
--workspace` (`index.ts:175`), then the fail-closed `verifyBumps` check
(`index.ts:179-190`, function at `index.ts:57-79`). Push phase: `jj git
push -b main` (`index.ts:214`) and `jj-hp push-tags <version>`
(`index.ts:219`). The closing log line hard-codes the monorepo
(`index.ts:225`):

```typescript
"   https://github.com/mattwilkinsonn/zireael/actions/workflows/release.yml",
```

**Both standalones are single-package crates, not workspaces.**
`jj-hooks/Cargo.toml:1-4` and `jj-gt/Cargo.toml:1-4` each open with
`[package]` and `version = "0.3.12"`; there is no `[workspace]` table in
either file (both read in full this session).

**The dependency pin.** `jj-gt/Cargo.toml:43`:

```toml
jj-hooks = "0.3.11"
```

This is a plain crates.io requirement (the extraction record's own
consequence, `oss-tool-extraction-and-shared-tooling.md:54-55`), and a
jj-gt release must NOT bump it.

**In-repo `Formula/` dirs still exist but are vestigial.** Contrary to a
first reading of the extraction outcome, both repos still track a formula:
`jj-hooks/Formula/jj-hooks.rb:4` and `jj-gt/Formula/jj-gt.rb:4` both read
`version "0.3.11"`. They are dead weight for releases: each repo's
`release.yml` `bump-tap` job checks out `mattwilkinsonn/homebrew-tap`
under `path: tap` and rewrites THAT checkout
(`jj-hooks/.github/workflows/release.yml:275-288,344` — the rewrite runs
with `working-directory: tap`; `jj-gt/.github/workflows/release.yml:279-280,340`
is structurally the same), and `bump-formulae.py`'s own docstring says so
(`jj-hooks/.github/scripts/bump-formulae.py:7-10`: "release.yml runs this
with `working-directory: tap` so it rewrites the consolidated tap
checkout, NOT this repo's in-repo Formula/"). The in-repo copies were the
seed material for the continuity record's tap revival
(`distribution-continuity-pre-private-flip.md:283-290`, its T1).

**v0.3.12 is further along than "hand-bumped files".** In both repos the
0.3.12 bump is already committed on `main` with a clean working tree
(`git status --short` empty in both; `git log` heads: jj-hooks `73dc3dd
chore(release): v0.3.12 — distribution-only release + tap migration comms
(#317) (#17)`, jj-gt `b41ed4d` with the matching subject), the checked-in
`Cargo.lock`s already carry 0.3.12 (`jj-hooks/Cargo.lock:260-261`,
`jj-gt/Cargo.lock:283-284`), and jj-hooks' `CHANGELOG.md` has a `[0.3.12]`
section (`jj-hooks/CHANGELOG.md:7`). What is missing is the tag: `git tag
--list` ends at `v0.3.11` in both repos, so `release.yml` — `on: push:
tags: ["v*"]` (`jj-hooks/.github/workflows/release.yml:10-12`) — has never
fired for 0.3.12.

**dev-shared's surfaces.** `dev-shared/README.md:3-7`: "One place to fix a
toolchain, CI, or Renovate change and inherit it everywhere … Four
surfaces, each consumed independently" — a devenv module, a composite
action, a reusable CI workflow, a Renovate preset. The module already
ships a shared task set (`dev-shared/devenv.nix:70-76`,
`"ci:markdownlint".exec = …`). It ships no TypeScript and no bun; its
package list (`dev-shared/devenv.nix:34-58`) contains the rust toolchain,
`stdenv.cc`, `cargo-nextest`, four nix linters, `shellcheck`/`shfmt`/
`taplo`, `actionlint`, `markdownlint-cli2`, and `jujutsu` — no
`cargo-edit`, no `bun`. Both consumers import it
(`jj-hooks/devenv.yaml:14-23`, `jj-gt/devenv.yaml:18-29`). jj-gt's own
shell adds `bun` (`jj-gt/devenv.nix:22`); jj-hooks' shell does not
(`jj-hooks/devenv.nix:8-16` lists only pre-commit, prek, lefthook, pkl,
hk).

**Nix path semantics the mechanism below relies on.**
`dev-shared/devenv.nix:20-22` records the house-verified rule: "a relative
path literal in a Nix module always resolves to the module file's OWN
directory" — cited there as the reason NOT to use `./rust-toolchain.toml`,
but it is exactly the property that lets the module carry and reference
its own script file.

### Empirical checks run this session

These were executed against scratch copies (temp dir with `Cargo.toml`,
`Cargo.lock`, `src/` copied in), never against the working trees:

- `cargo set-version --workspace --offline 0.4.0` in a jj-gt copy
  (cargo-edit 0.13.11): exit 0, `version = "0.4.0"` written to
  `[package]`, and the `jj-hooks = "0.3.11"` line **unchanged**. The
  monorepo behaviour of bumping the internal jj-hooks version field
  (`index.ts:6-9` comment) came from jj-hooks being a workspace member
  there; in the standalone, jj-hooks is an external dependency and
  set-version does not touch its requirement. The hazard resolves itself —
  no flag change or sed needed.
- `cargo set-version --workspace --offline 0.3.12` in a jj-hooks copy
  already at 0.3.12: exit 0, silent no-op (no "Upgrading" line, no error).
- After the 0.4.0 set-version in the jj-gt copy, `Cargo.lock` already
  carried `version = "0.4.0"` for the `jj-gt` package, and a follow-up
  `cargo update --workspace --offline` reported "Locking 0 packages" —
  set-version had synced the lock itself.
- `cargo set-version --workspace 0.3.12 --dry-run` run in the real
  `jj-hooks` working tree (dry-run writes nothing): exit 0.

### Decisions

1. **The driver ships through dev-shared's existing devenv-module surface
   — one copy, no new distribution surface class.** The fork was framed as
   per-repo copies vs "a NEW distribution surface for a TS script". The
   resolution is that no new surface class is needed: the devenv module
   (surface 1 of 4) already injects packages and tasks into both
   consumers, and a Nix module can carry a sibling file and interpolate
   its store path into a command string (the module-relative path rule,
   `dev-shared/devenv.nix:20-22`). dev-shared gains `release/index.ts` +
   `release/index.test.ts`, and its `devenv.nix` adds one `scripts` entry
   shaped like:

   ```nix
   scripts.release.exec = ''
     exec ${pkgs.bun}/bin/bun ${./release/index.ts} "$@"
   '';
   ```

   so `release vX.Y.Z` works inside either repo's devenv shell. Using
   `${pkgs.bun}/bin/bun` by store path keeps bun out of the shell PATH —
   jj-hooks' shell stays bunless. The mechanism was verified in the exact
   topology both consumers use, which is **not** a flake input: each takes
   dev-shared as `flake: false` plus `imports: [dev-shared]`
   (`jj-hooks/devenv.yaml:14-16,22-23`, `jj-gt/devenv.yaml:18-20,28-29`),
   so devenv reads the module source directly rather than
   evaluating a flake output. A fixture in that shape — a `shared/`
   directory holding `devenv.nix` with a `scripts.release.exec` entry plus
   a sibling `release/index.ts`, consumed by a separate `consumer/` with
   `flake: false` and `imports:` — was built and run:
   `devenv shell -- release v9.9.9-test` printed
   `ok cwd: /tmp/dstest/consumer argv: [ "v9.9.9-test" ]`. That settles
   three things at once: the `scripts` entry defined in the imported
   module is visible in the consumer shell, `"$@"` passes arguments
   through, and `process.cwd()` is the consumer's directory rather than
   the store — which is what lets the driver's `jj` and `cargo` calls act
   on the consumer repo. One constraint the mechanism carries:
   `${./release/index.ts}` interpolates to a **new single-file store
   path** holding only that file — this holds even when the module is
   itself store-resident and its siblings sit right beside it in the
   module's own store path, because the interpolation re-adds the
   referenced file as its own path rather than pointing into the
   directory — so
   the driver must stay import-free apart from `bun` builtins. That holds
   today (`index.ts:29` is the only import and it is `from "bun"`), and T1a
   pins it with a test, because the day someone adds a sibling import it
   breaks in consumers rather than in dev-shared's own CI.

   Why shared beats copies here: the extraction record deliberately
   accepted two `release.yml` copies because they "diverge structurally
   between the repos (bin sets, tap names, App identity)"
   (`oss-tool-extraction-and-shared-tooling.md:84-91`); the driver is the
   opposite case — the per-repo differences it does carry (decision 4's
   actions URL, and the `moon run root:release` usage string at
   `index.ts:3` and `index.ts:40`, echoed in `index.test.ts:22`, `:29`,
   `:168`) are removable, as is the monorepo actions URL in the closing
   log line (`index.ts:225`). The differences are the ones a scan of the
   file surfaced rather than a proven-exhaustive set — T1a's acceptance
   is that the resulting file carries no repo-specific string, which is
   checkable against the artifact in a way this record cannot be. After
   that, duplicating it is what dev-shared's charter exists to prevent
   (`dev-shared/README.md:3-5`). T1a rewrites the usage string along with
   the URL.

   Honest cost, owned: the module surface is lock-gated — a driver fix
   propagates only when a consumer's `devenv.lock` moves (the
   propagation-asymmetry table,
   `oss-tool-extraction-and-shared-tooling.md:184-189`). Two corrections
   to how bad that is. It is bounded, not indefinite: both consumers run
   the shared `devenv-update` workflow on a weekly schedule
   (`jj-hooks/.github/workflows/devenv-update.yml:8` delegating to
   `dev-shared/.github/workflows/devenv-update.yml`, `cron: "0 6 * * 1"`),
   so a fix reaches them within about a week unattended. But the sharper
   risk is that those PRs arrive unverified: the shared workflow opens
   them with the default `GITHUB_TOKEN`, which by design "does NOT start
   new" CI runs (`dev-shared/.github/workflows/devenv-update.yml:21-22`),
   and the PR body is stamped `**CI-UNVERIFIED:**` (`:60`). So a driver
   change lands in a consumer without that consumer's CI having run
   against it unless someone checks the lock by hand. Meanwhile a
   consumer still on the old lock keeps running the *previous* driver
   successfully, so a fix is absent rather than loud. Mitigation is cheap:
   the driver prints its own resolved store path as its first line — not a
   version string, which the single-file constraint makes unavailable (see
   T1a's banner bullet) — so a release log identifies exactly which driver
   cut it. The decision rests on the drift argument, not on the failure
   being self-announcing.

2. **Formula bumping is deleted from the driver, and the vestigial in-repo
   `Formula/` dirs are deleted from both standalones.** The formulae the
   release actually ships live in `mattwilkinsonn/homebrew-tap` and are
   rewritten at run time by each repo's `bump-tap` job (evidence under
   Ground truth). The driver's step-2 formula rewrite
   (`index.ts:169-171`), the `bumpFormulaVersion` helper
   (`index.ts:49-51`), and the formula loop inside `verifyBumps`
   (`index.ts:72-77`) do not port. The in-repo `Formula/*.rb` copies
   (both at `version "0.3.11"`) served the tap-seeding task and are now
   misleading — left in place, they are the one thing a naive driver
   port would have "bumped". Their deletion is sequenced after the first
   green `bump-tap` run proves the tap path (T4), so the seed material is
   never destroyed before its replacement is proven.

3. **`cargo set-version --workspace` and `cargo update --workspace` port
   unchanged; `verifyBumps` ports with its formula parameter and loop
   removed.**
   `--workspace` works in a single-package repo (verified, exit 0), and in
   jj-gt it leaves the `jj-hooks` pin alone (verified on a scratch copy —
   the sharpest hazard in the port is a non-hazard). T1a's test suite
   pins what a unit test can actually see: the driver's `sh` fake records
   commands and returns canned output (`index.test.ts:111-130`), so no
   cargo process ever runs and the test pins that the *driver* performs
   no direct dependency rewrite of its own. Catching a future cargo-edit
   behaviour change is T3's job, where a real scratch-copy run exercises
   the actual binary. `cargo update --workspace` was observed to be
   redundant after set-version (lock already synced) but stays as a
   cheap fail-safe, matching `index.ts:175`. `verifyBumps` keeps its
   fail-closed Cargo.toml check (`index.ts:62-71`) with the formula
   parameter and loop removed.

4. **The hard-coded actions URL is derived, not parametrized.**
   `index.ts:225` bakes the zireael URL. The standalone driver reads the
   `repository` field from `Cargo.toml` (`jj-hooks/Cargo.toml:7`,
   `jj-gt/Cargo.toml:7` — each already points at its own standalone repo)
   and prints `<repository>/actions/workflows/release.yml`. This removes
   the per-repo actions-URL difference; with the usage string rewritten in
   T1a, the two copies become identical, which is what makes decision 1's
   single shared copy possible without per-repo parametrization. Each
   consumer still bumps its own `devenv.lock` to pick the driver up
   (T2/T3), so this is "no per-repo configuration", not "no per-repo
   step".

5. **Guards 1-3 port unchanged; guard 4 is provisioned; the `jj-hp`
   dependency is dropped and replaced with a `git` guard.** The clean-`@`,
   main-ancestor, and tag-exists guards
   (`index.ts:111-151`) do not depend on repo shape and carry over. One
   caveat, not a blocker: the two consumers do not run the same jj. jj-gt
   pins jujutsu 0.42 with an overlay that shadows dev-shared's
   (`jj-gt/devenv.nix:14-18`), because jj 0.44 changed `jj git fetch`
   behaviour in a way that breaks jj-gt's orphan re-anchor path; jj-hooks
   adds no overlay and takes the module's rolling jujutsu. So one shared
   driver runs under two jj versions, and T2/T3 acceptance runs the three
   guard commands read-only under each shell's actual jj rather than
   assuming the surface held still.
   The cargo-edit guard (`index.ts:153-161`) carries over, and
   the dev-shared module adds `cargo-edit` to its package list so the guard
   passes by construction inside the shell instead of depending on a
   per-machine `cargo install --locked cargo-edit`; the guard stays as
   defense for invocation outside the shell.

   **`jj-hp` is a fifth prerequisite the monorepo driver never guarded, and
   it fails destructively.** `index.ts:219` shells to `jj-hp push-tags`,
   and it is the *last* step — by then the driver has already committed,
   tagged, moved `main`, and run `jj git push -b main` (`index.ts:194-214`).
   Nothing provisions `jj-hp`: `grep -c jj-hp` is 0 in
   `dev-shared/devenv.nix`, `jj-hooks/devenv.nix`, and `jj-gt/devenv.nix`
   (all three checked). In the monorepo it was on PATH from
   `~/.cargo/bin` via a local install, which is invisible to a fresh
   consumer. So on a machine without a personal `jj-hp`, the release dies
   with `main` pushed and the tag unpushed — the exact half-cut state
   v0.3.12 is stuck in today. Two fixes, and T1a takes the second:
   guard for `jj-hp` before any mutation, or drop the dependency entirely.
   Dropping is preferable, and the substitution was checked to be
   equivalent rather than assumed. `jj-hp push-tags` is self-described as
   a workaround — `jj-hooks/src/push_tags.rs:1-4`: "jj has no native `jj
   git push --tag`. This subcommand is the workaround: export refs to the
   colocated git repo, then shell out to `git push refs/tags/<tag>`" — and
   its `run` body is exactly that and nothing more: `jj git export
   --ignore-working-copy` (`push_tags.rs:33`), a tag-exists precheck
   (`:49-54`), then `git push <remote> refs/tags/<tag>`
   (`:56-61`, `:72`). It adds no hook or guard machinery of its own: its
   dispatch arm calls `push_tags::run` and nothing else
   (`jj-hooks/src/lib.rs:228-246`), while the hook runner is invoked from
   the separate `Command::Push` arm (`lib.rs:91`). So replacing the
   subcommand loses no safety property. The driver already
   runs `jj git export` at `index.ts:210`. T1a therefore replaces the
   `jj-hp` call with `git push origin refs/tags/<version>`, keeps a
   pre-mutation guard for `git`, and keeps the tag-exists precheck the
   subcommand provided. The hard-coded `origin` remote is correct for both
   consumers (`git remote` in each returns exactly `origin`), matching the
   default `jj-hp` itself used.

6. **v0.3.12 is cut by hand first; the driver port lands after and is
   first exercised on the next release.** The continuity record already
   owns the v0.3.12 cut and assigns the tag push to Matt's laptop
   (`distribution-continuity-pre-private-flip.md:167-171`, its "Cut a real
   v0.3.12 through each standalone pipeline" section, and its Global
   Constraints: "Every admin-plane step (App installs, secrets/vars,
   rulesets, tag pushes) is a LAPTOP task"). With the bump commits already
   on both mains, the remaining v0.3.12 work is tagging the existing
   release commits and pushing the tags — two commands per repo, far less
   than the port. Blocking a live, mostly-complete release on a tooling
   port would invert the priority.

   For completeness, because it determines whether the driver COULD cut
   v0.3.12: run today on either repo, all four guards pass. Guard 1 —
   each repo's own `jj diff --summary --ignore-working-copy` was run
   read-only and returned empty (the guard's own command, not
   `git status`, which does not entail an empty jj `@`). Guard 2 —
   `main & ::@` (`index.ts:122-139`) is non-empty in both, since `@` sits
   on `main`. Guard 3 — no v0.3.12 tag exists in either repo. Guard 4 —
   `cargo set-version --help` exits 0 (cargo-edit 0.13.11 present). Then
   the set-version step is a silent no-op (exit 0 at equal version, run on
   a scratch copy of each repo — jj-hooks and jj-gt both checked), and
   **`verifyBumps` PASSES** — its check is
   `line.startsWith('version = "0.3.12"')` over `Cargo.toml`
   (`index.ts:63-64`), and `jj-hooks/Cargo.toml:4` / `jj-gt/Cargo.toml:4`
   are exactly `version = "0.3.12"`. No preparatory step is needed. The
   driver would then commit an EMPTY `release: v0.3.12` change on top of
   the existing bump commit and tag that. That is still functionally fine,
   but not for the reason "release.yml checks out the tag" would suggest.
   `build-rust`, `publish-release` and `publish-crates` do check out the
   tag, while `bump-tap`'s self checkout deliberately tracks `main`
   (`jj-hooks/.github/workflows/release.yml:263-267`, whose comment reads
   "ref: main (not the release tag) so the bump always runs the current
   script"). It survives because no job depends on commit identity — no
   `git describe`, no changelog extraction from the commit, static release
   notes (`jj-hooks/.github/workflows/release.yml:194`; the same line is
   `:193` in jj-gt) — only on the tree, which is identical. The hand cut
   tags the existing commit directly and avoids the duplicate commit
   anyway.

7. **zireael's `tools/release/` is deleted once the shared driver is
   proven.** zireael is flipping private and becoming a personal-infra
   Pulumi monorepo that ships neither crate
   (`docs/designs/platform/personal-infra-monorepo.md`, the continuity
   record's T8 reference at
   `distribution-continuity-pre-private-flip.md:28-29`), so its driver is
   on a path to dead code. It is also the port's source material, so it
   outlives the port: T5 deletes `tools/release/` plus the root `release`
   task that wraps it (root `moon.yml:35-39`: `release: command: 'bun run
   tools/release/index.ts'`) **and the project registration at
   `.moon/workspace.yml:20` (`release: 'tools/release'`)** — deleting the
   directory without that line leaves moon pointing at a source that no
   longer exists. Sequenced after T1a/T1b-T3 merge and one release has gone
   through the new driver. Until then it remains the only proven driver and
   the rollback path. Note for whoever lands the format gate: its planned
   root `package.json` workspaces list includes `tools/release`
   (`whole-repo-format-gate.md:620-624`), so these two records touch the
   same list and whichever lands second reconciles it.

## Global Constraints

- **Language/tooling:** the driver stays bun/TypeScript with the DI
  `Deps` seam and its test suite (`rule://scripts-ts-over-bash`; the seam
  is the monorepo's own pattern, `index.ts:84-97`). No bash rewrite, no
  Rust rewrite.
- **No new dev-shared surface class:** the driver rides the existing
  devenv module. No npm/bun package publication, no `bunx github:` path.
- **Consumer shells stay lean:** `cargo-edit` reaches the consumers via
  the module's `packages` list (inherited through `imports:`), while bun
  is referenced **only** by store path inside `scripts.release.exec` and
  is never added to that list — so jj-hooks' shell gains no PATH-visible
  bun. dev-shared's own CI gets bun from its CI job.
- **The jj-gt dependency pin is inviolable:** no release of jj-gt may
  change `jj-hooks = "0.3.11"` (`jj-gt/Cargo.toml:43`) as a side effect.
  T1a's tests must fail if the *driver* rewrites it directly; catching a
  cargo-edit behaviour change is T3's job (decision 3).
- **VCS discipline:** jj-vine is the sole push path; PRs open as drafts
  and are promoted with `gh pr ready` after review; never `gh pr create`;
  agents never merge. Tag pushes and release cuts are laptop tasks (the
  continuity record's constraint; the box has no push creds for these
  repos' mains).
- **Sequencing:** nothing in this record blocks the v0.3.12 hand cut; T4
  and T5 are gated as stated in their task sections.

## Plan

### T1a — dev-shared: the ported driver + its gate (BOX-DOABLE)

PR against `mattwilkinsonn/dev-shared`:

- Add `release/index.ts`: the monorepo driver ported per decisions 2-5 —
  formula code removed, URL derived from `Cargo.toml`'s `repository`,
  the `moon run root:release` usage string (`index.ts:3`, `index.ts:40`)
  rewritten to the `release vX.Y.Z` invocation, guards 1-4 carried over,
  the `jj-hp push-tags` call replaced by `git push origin
  refs/tags/<version>` after the existing `jj git export`
  (`index.ts:210`), preceded by the tag-exists precheck `jj-hp` provided
  (`push_tags.rs:49-54`) so a missing ref errors before the push rather
  than mid-push, and a pre-mutation guard for `git` — which belongs
  **with guards 1-4, before the `:161` dry-run exit**, since its whole
  purpose is to refuse a run that would mutate the repo and then fail at
  the push for want of a `git` binary; that placement is also what makes
  the dry-run path shell to `git`. The `Deps` seam stays intact —
  including `writeFile` and `glob`, which decision 2 leaves
  without a *call site* but not without a supplier: the production Bun
  wiring under `import.meta.main` still provides both
  (`index.ts:259-267`), and the test file both defines the fakes
  (`index.test.ts:132` for `writeFile`, `:135-136` for `glob`) and passes
  the `glob` fixture at four call sites (`:176`, `:236`, `:251`, `:288`).
  The definitions are what create the type exposure; the call sites would
  become unused bindings. Follow that through in `runOnce`, which
  destructures all six at `index.ts:100` (`const { sh, readFile,
  writeFile, glob, log, err } = deps;`): with the formula rewrite
  (`index.ts:168-171`) and the formula glob in `verifyBumps`
  (`index.ts:180-185`) gone, `writeFile` and `glob` have no remaining use
  there, so the destructuring must drop them or the port lands two unused
  bindings. Measured, `noUnusedVariables` reports those as warnings at
  exit 0 under recommended defaults but as errors at exit 1 once the rule
  is raised — which a self-contained config exists to do. Dropping the type members while
  those literals still provide them is a TS excess-property error, so the
  port either keeps the members or removes type, wiring and fakes
  together — T1a takes the simpler path and keeps them. The file stays
  import-free apart from `bun` builtins, per the
  single-file store-path constraint in decision 1. Also rewrites the
  monorepo-specific prose: the step-2 log line (`index.ts:163`, "Bumping
  Rust workspace + members + jj-hooks dep") becomes "Bumping the crate
  version to `<bare>`" — in a standalone there is no workspace, no
  members, and the jj-hooks dep is deliberately NOT bumped, so the
  original would announce on every jj-gt release the one thing Global
  Constraints forbid — and the header comment's workspace framing
  (`index.ts:1`, `:6-9`) is rewritten to the single-crate reality.
- **`--dry-run`, anchored before the first write.** The flag runs guards
  1-4, prints the mutations it would make (`cargo set-version
  --workspace <bare>`, `cargo update --workspace`, `jj commit`, `jj tag
  set`, `jj bookmark set main`, `jj git push -b main`, `git push origin
  refs/tags/<version>`), and exits 0 immediately after the cargo-edit
  guard (`index.ts:161`) — **not** "before the first `jj commit`", which
  would be wrong: `cargo set-version` (`index.ts:164`) and `cargo update`
  (`index.ts:175`) sit between the last guard and the commit, and both
  write to the repo (verified on a scratch copy: set-version rewrote
  `Cargo.toml` and `Cargo.lock`). Exiting at `:161` means `verifyBumps`
  (`index.ts:186`) is not exercised in dry-run mode — a known limit,
  and the alternative is worse: skipping the bump but still reaching
  `verifyBumps` makes it read an un-bumped `Cargo.toml` and return 1.
  This also requires changing the argument handling, which today is only
  `const version = argv[0] ?? ""` (`index.ts:102`) — under that, `release
  --dry-run v1.2.3` puts the flag in `argv[0]`, fails `VERSION_RE`, and
  exits 1 on the usage guard without running a single guard. So the port
  strips `--dry-run` from argv before reading the version, accepting it
  in either position.
- **Provenance banner as the first line printed.** Names the driver's own
  resolved path via `import.meta.path`, which under the store-path
  mechanism is the content-addressed `/nix/store/<hash>-index.ts` and so
  identifies the exact driver that cut a release — this is decision 1's
  propagation mitigation, which otherwise exists in no task. Deliberately
  **not** a version string: there is no sibling `package.json` in the
  store to read one from (the single-file constraint), and a hand-
  maintained constant would drift, which is the failure the banner exists
  to detect.
- Add `release/index.test.ts`: port of the monorepo suite minus the
  formula tests, with the three usage-string expectations updated
  (`index.test.ts:21-22`, `:28-29`, `:167-168`), plus six new contracts:
  (a) a jj-gt-shaped fake `Cargo.toml` with a `jj-hooks = "0.3.11"` line
  asserts the driver never writes that file except through `cargo
  set-version` (no direct dependency rewrites exist to regress); (b) the
  closing log line is derived from the fake `Cargo.toml`'s `repository`
  value; (c) the tag push runs only after a successful main push, and a
  failed `git` guard aborts before any commit/tag/bookmark mutation;
  (d) the driver's import list stays within `bun` builtins, so a future
  sibling import fails in dev-shared's CI rather than silently in a
  consumer; (e) `--dry-run` is accepted in either argv position, exits 0,
  and dispatches no mutating command — specifically no `cargo set-version
  --workspace <bare>` and no `cargo update --workspace`. The read-only
  `cargo set-version --help` probe of guard 4 (`index.ts:153`) is
  expected and must be allowed: the dry-run runs guards 1-4, and guard 4
  is itself a `cargo set-version` dispatch, so the contract discriminates
  on the invocation, not the command name. And (f) the first line printed
  is the provenance banner, before any guard
  command runs.
- Add the bun scaffolding the gate below needs: `release/package.json`,
  `release/bun.lock`, `release/tsconfig.json`, `release/biome.json`, plus a
  `release/node_modules/` line in dev-shared's `.gitignore` (which today
  covers only `.devenv/`, `.devenv.flake.nix`, `devenv.local.nix` and
  `.direnv/` — it has never shipped TypeScript, and `bun install`
  materializes `node_modules/`).
  dev-shared today has none of them (each checked: absent), because it
  ships no TypeScript — so without this the gate's commands have no
  manifest, no tsconfig and no biome config to read. Three things must
  change rather than port verbatim from `tools/release/`. (1)
  `release/biome.json` must be **self-contained**: the monorepo file is
  exactly `{"extends": "//"}`, where `//` names the workspace-root
  config — `zireael/biome.json`, a bare
  `{"$schema": …, "root": true}` marker carrying no rule settings at all
  — and dev-shared has no equivalent. Ported as-is the config is
  **inert**: measured with the monorepo file verbatim and no root config
  present, Biome emits no diagnostic of any kind about the unresolvable
  `//` (nothing about `extends`, `root` or resolution on either stream)
  and silently falls back to its built-in defaults. So the file looks
  like it configures the gate and does not — a failure that is invisible
  rather than loud, and the reason to inline the config rather than add
  a dev-shared root file, since dev-shared ships one TS directory.
  The discard is not limited to what the chain would have inherited: a
  dangling `extends` silently drops the file's **own** co-located rule
  settings too. Measured, a config carrying both `extends: "//"` with no
  root and its own `noUnusedVariables: "error"` lints the offending file
  as a *warning* and exits 0, while the identical rules with the
  `extends` line removed report `Found 1 error` and exit 1. So a
  ported-as-is config would not just fail to inherit — it would discard
  any rule severity the ported file itself tried to set, falling back to
  Biome's default in either direction.

  The dangling `//` drops no *inherited* severity here, which is the
  narrow half of the story: the config being extended is a bare marker
  that sets no rule severities, so there is nothing up the chain for the
  break to lose, and every severity in the three topologies below already
  comes from Biome's recommended set. Measured,
  `lint/correctness/noUnusedVariables` is a *warning* with `biome lint .`
  exiting 0 under all three — dangling `//` with no root, a
  self-contained config, and the real zireael layout with the root marker
  present — while an error-severity rule
  (`lint/suspicious/noSelfCompare`) reports `Found 1 error` and exits 1
  under the dangling config just the same.

  That is *only* about inherited severities, and it does not soften the
  own-settings discard above. All three topologies use a config that sets
  no severities, so they measure the default-severity case and nothing
  else. A config that both dangles and raises a rule — exactly what a
  self-contained config here would do — gates green on a real error. And
  where an extends target does resolve, its severities propagate
  normally, so none of this says the extends chain is severity-irrelevant
  in general; these three topologies cannot test that, since in each one
  the target is either absent or a bare marker.

  (2) The `repository` field in `package.json` points at zireael with
  `"directory": "tools/release"` (`tools/release/package.json:4-7`) and is
  rewritten to dev-shared. (3) Add `@biomejs/biome` as a pinned
  devDependency **and invoke it as `bunx @biomejs/biome check`, never
  `bunx biome check`.** Neither the monorepo `package.json` nor its
  `bun.lock` carries Biome, and the bare npm name `biome` is a different
  project — an environment-variable manager whose latest is 0.3.3. It
  accepts `check .` and exits 0 without linting anything (verified:
  `bunx biome --version` → `0.3.3`; `bunx biome check .` → exit 0 on a
  file with issues), so a bare `bunx biome` is a silent no-op **whenever
  the local install is absent** — which includes any invocation ordered
  before `bun install`, and any developer running the command by hand in
  a fresh checkout, though not the CI job specified below, which
  installs first. Once `@biomejs/biome` is installed locally, `bunx
  biome` does resolve through `node_modules/.bin` to the real Biome, so
  the scoped name is belt-and-braces rather than the only thing standing
  between the gate and the imposter. `@biomejs/biome` is the real
  package (`bunx @biomejs/biome --version` → `Version: 2.5.12`). Also
  **regenerate `release/bun.lock`** with the dependency present (`bun
  install`, commit the lock) rather than copying
  `tools/release/bun.lock`: the CI job leads with `bun install
  --frozen-lockfile`, which fails closed on any manifest/lock mismatch.
  For reference, the monorepo's `install` task reads `package.json` +
  `bun.lock` (`tools/release/moon.yml:10-12`), with `tsconfig.json` and
  `biome.json` consumed by its `typecheck` and `lint` tasks (`:19-22`,
  `:15-18`).
- **dev-shared CI: a new job in `.github/workflows/ci.yml` that
  provisions bun OUTSIDE the devenv shell** (`oven-sh/setup-bun`,
  SHA-pinned with a trailing version comment, matching how dev-shared
  pins third-party actions (see `peter-evans/create-pull-request` at
  `dev-shared/.github/workflows/devenv-update.yml:51`, a full commit SHA
  followed by a `# v8.1.1` marker; note it tag-pins first-party ones like
  `actions/checkout@v4`, so the convention is not uniform) and runs `bun
  install --frozen-lockfile`, `bun test`, `bunx tsc --noEmit`, and `bunx
  @biomejs/biome check` (the scoped name, per (3) above) with
  `working-directory: release`. It must **not** be a `ci:*`
  task in `devenv.nix`: that task set is inherited by both consumers
  exactly as `packages` is, so the task would run `bun test` over a
  `release/` that does not exist in them, in a shell with no bun,
  reddening both consumers' gates on merge. It also cannot be another
  `devenv shell -- <cmd>` step, the shape every existing step in that
  workflow uses (`dev-shared/.github/workflows/ci.yml:25-31`), because
  bun is deliberately absent from that shell. Mirrors the monorepo's own
  gate for this code (`tools/release/moon.yml:15-30`).

Interfaces:

- Consumes: `zireael/tools/release/index.ts` + `index.test.ts` as source
  material (read, not moved).
- Produces: `release/index.ts` under its own passing gate. The driver is
  **not** yet reachable from any shell — that is T1b — so this PR is
  exercised with `bun run release/index.ts` directly. The exit-code
  contract from `index.ts:22-27` is preserved (0 success, 1 guard
  failure, N propagated from a shelled step).

Acceptance: dev-shared CI green including the new bun job — `bun install
--frozen-lockfile`, `bun test`, `bunx tsc --noEmit` and `bunx
@biomejs/biome check` all passing with `working-directory: release`;
`bun run release/index.ts not-a-version` exits 1 on the usage guard; and
`bun run release/index.ts --dry-run v9.9.9` runs the guards, exits 0, and
leaves `Cargo.toml` and `Cargo.lock` byte-identical.

**The two `bun run` checks are box-local, not part of the CI job.** The
job provisions only bun (deliberately — see the CI bullet above), while
the dry-run path shells to `jj` for guards 1-3, to `git` for the new
pre-mutation `git` guard, and to `cargo` for guard 4. (The `git`
dependency is that guard, not the tag-exists precheck, which sits at the
push site past the `:161` dry-run exit — and must stay there: the
precheck errors when the tag does *not* exist
(`push_tags.rs:49-54`), which is precisely what guard 3 asserts, so
moving it before the exit would invert it and fail every run.) Running it under a
bun-only PATH fails for a misleading reason rather than a missing-tool
one: measured, the driver reports `error: @ is not a descendant of main`
— guard 2's message — never naming the absent `jj`. So run these two on
the box and keep them out of the workflow.

The fixture for the dry-run check must be a real colocated jj repo (`jj
git init --colocate`) with a clean `@`, a `main` bookmark set to an
ancestor of `@`, and a `Cargo.toml` carrying a `repository` field —
guards 1-3 all shell to `jj`, and in a plain directory guard 1 passes
*vacuously*: it inspects only `stdout` (`index.ts:114`), never the exit
code, so `jj diff` failing with "There is no jj repo" reads as a clean
working copy and guard 2 fails instead, which would make the check pass
for the wrong reason.
The ported file carries no repo-specific string (the identity property
decision 1 rests on, checked against the artifact). This is a real gate
on real code, not inert scaffolding: the CI job executes and can fail.

### T1b — dev-shared: wire the driver into the module surface (BOX-DOABLE, after T1a)

Second PR against `mattwilkinsonn/dev-shared`. Touches no file T1a
touched, which is what makes the split clean: T1a owns `release/*`,
`.gitignore` and `.github/workflows/ci.yml`; T1b owns `devenv.nix` and
`README.md`. The two sets are disjoint, so the PRs cannot conflict.

- `devenv.nix`: add `cargo-edit` to `packages`; add the
  `scripts.release.exec` entry from decision 1. **Do not add `bun` to
  `packages`** — that list is inherited by every consumer through
  `imports: [dev-shared]`, which is precisely how `cargo-edit` reaches
  them (decision 5), so adding bun there would put it on both consumers'
  PATH and break the lean-shell constraint.
- **Record the verified mechanism in dev-shared's README, in the same
  shape as the transitivity verdict (`dev-shared/README.md:42-51`).** The
  `flake: false` + `imports:` topology is already proven (decision 1), so
  T1b documents it rather than discovering it: the `scripts` entry is
  visible in the consumer shell, arguments pass through `"$@"`, the
  interpolated store path executes, and `process.cwd()` is the consumer's
  directory. Note the single-file constraint beside it.

Acceptance. Assert against the module's **own profile**
(`$DEVENV_PROFILE`), not against PATH. `devenv shell` *prepends* its
profile to the inherited PATH rather than replacing it, so a PATH-based
check measures the developer's machine: measured on the current tree,
`devenv shell -- bash -c 'command -v bun'` returns a host bun
(`/home/mattw/.bun/bin/bun` on this box, a standalone install earlier on
PATH; a second copy also sits in `/etc/profiles/per-user/<user>/bin`), and `cargo
set-version --help` already exits 0 there with `cargo-edit` absent from
`packages` — both would pass before T1b changed anything. Sanitizing the
environment does not fix it either, because `bun` lives in the same
per-user profile directory as `devenv` itself, so any PATH able to
invoke `devenv` also leaks `bun` (measured). The profile check has no
such coupling and was validated in both directions: bun absent in
dev-shared, present in a scratch shell whose `packages` provisions it.

- `devenv shell -- bash -c '! ls "$DEVENV_PROFILE/bin" | grep -qx bun'`
  exits 0, so bun is absent from the module's own profile and the
  lean-shell constraint holds. Measured: exits 0 in dev-shared, and 1 in
  a scratch shell whose `packages` provisions bun.
- `devenv shell -- bash -c 'ls "$DEVENV_PROFILE/bin" | grep -qx
  cargo-set-version'` exits 0, so `cargo-edit` reached the profile and
  guard 4 passes by construction. Measured: exits 1 on the current tree,
  so this check can fail before T1b and pass after. (`cargo-set-version`
  is the real binary name — `nix shell nixpkgs#cargo-edit` provides
  `cargo-add`, `cargo-rm`, `cargo-set-version`, `cargo-upgrade`.)
  Use `grep -qx`, not `grep -cx`: the counting form prints its count to
  stdout, which is pure noise in a check whose only signal is the exit
  status (measured: `-cx` prints `0` or `1` where `-qx` prints nothing).
  It reaches the same verdict in both checks as written, so this is
  hygiene rather than a correctness fix.
- `devenv shell -- bash -c 'release not-a-version'` exits 1 with the
  usage error, from inside dev-shared's own shell. Wrap it in `bash -c`:
  `devenv shell -- <argv>` execs argv directly, so a builtin like
  `command` is unresolvable and the bare form exits 127 regardless of
  what is installed (measured both ways).
- The mechanism note is in `README.md`.

### T2 — jj-hooks: consume the driver (BOX-DOABLE, after T1b)

PR against `mattwilkinsonn/jj-hooks`: `devenv update dev-shared` lock
bump, picking up a dev-shared revision that includes **T1b** — the lock
must carry the `scripts.release` wiring, not just T1a's `release/` files,
or `release` will not resolve in the shell. No `devenv.nix` change is
expected in this repo — the script and cargo-edit
arrive via the module. Acceptance: inside the repo's devenv shell,
`release` resolves and `release not-a-version` exits 1 with the usage
error (guard path exercised, nothing mutated); the three jj guard
commands (`index.ts:111-151`) run read-only under the shell's own jj and
behave as the driver expects. That check matters because the two
consumers resolve different jj versions: jj-gt pins 0.42 with an overlay
shadowing dev-shared's jujutsu (`jj-gt/devenv.nix:14-18` for the overlay
itself; the rationale is quoted from `jj-gt/devenv.yaml:7-8`, "jj 0.44's `jj
git fetch` eagerly auto-rebases children onto rewritten parents, which
breaks the orphan re-anchor path"), while jj-hooks adds no
such overlay and takes the module's rolling jujutsu — so one shared
driver runs under two jj versions, and a jj behaviour change is a
demonstrated hazard in this pair rather than a hypothetical. Repo CI
green.

### T3 — jj-gt: consume the driver (BOX-DOABLE, after T1b)

Same shape as T2 against `mattwilkinsonn/jj-gt`. One extra acceptance
check, and it must be a real one: run `cargo set-version --workspace
<next>` **without** `--dry-run` in a scratch copy of the repo and assert
`jj-hooks = "0.3.11"` (`jj-gt/Cargo.toml:43`) survives — the experiment
recorded at `:131-138`. A `--dry-run` invocation followed by `git diff
--exit-code` proves nothing here: `--dry-run` writes nothing by
definition, so that check passes identically on a cargo-edit build that
*would* rewrite the pin, i.e. it cannot detect the hazard it exists to
guard. Also confirm the T1a regression test covering the pin is present in
the locked dev-shared revision.

### T4 — delete the vestigial in-repo `Formula/` (BOX-DOABLE, gated)

One PR per repo deleting `Formula/jj-hooks.rb` (resp. `Formula/jj-gt.rb`).
Gate: at least one green `bump-tap` run in that repo (the v0.3.12 run,
decision 6) proving the tap-side formula exists and updates — the fail-
closed check at `jj-hooks/.github/workflows/release.yml:321-331` /
`jj-gt/.github/workflows/release.yml:316-326` going green is the proof.
`bump-formulae.py` and the `bump-tap` job are NOT touched — they are live
tap machinery, not vestige. Acceptance: repo CI green; `git ls-files
Formula` empty.

### T5 — zireael: retire the monorepo driver (BOX-DOABLE, gated)

PR against zireael deleting three things: the `tools/release/` directory,
the root `moon.yml` `release` task (`moon.yml:35-39`), and the moon
project registration `release: 'tools/release'`
(`.moon/workspace.yml:20`). All three go together — deleting the
directory while leaving the registration points moon at a source that no
longer exists. Gate: T1a/T1b-T3 merged AND one real release (v0.3.13 or later)
cut through the shared driver — until then the
monorepo driver stays as the rollback path (decision 7). The root
`install-debug` task and other `tools/` projects are out of scope.
Acceptance: `moon project-graph --json` exits 0 with no
missing-project-source error, and zireael CI green. Use `--json` (or
`--dot`): bare `moon project-graph` starts an interactive graph server and
never exits — it takes a `--port` with `[default: 0]`, so an agent
following it literally hangs. Note that "`moon run root:release` no longer
resolves" is NOT sufficient acceptance — removing the root task alone
satisfies it while the dangling registration survives.

## Tasks

- [ ] T1a — dev-shared: ported driver + tests + scaffolding + CI gate
- [ ] T1b — dev-shared: devenv wiring + README note (after T1a)
- [ ] T2 — jj-hooks: lock bump, `release` resolves in shell
- [ ] T3 — jj-gt: lock bump + pin-safety check
- [ ] T4 — delete vestigial `Formula/` in both standalones (after green
      `bump-tap`)
- [ ] T5 — delete zireael `tools/release/` + root task + moon
      registration (after T1a/T1b-T3 merge AND a driver-cut release)

## Alternatives considered

**Per-repo copies of the driver.** The zero-infrastructure option: drop
`release/index.ts` + test into each repo. Rejected: with decision 4 the
copies would be byte-identical, i.e. pure drift liability with no
divergence to justify it — the opposite of the release.yml case where the
extraction record accepted copies precisely because they diverge
structurally (`oss-tool-extraction-and-shared-tooling.md:84-91`). The
toolchain cost is not avoided either: jj-hooks' shell has no bun
(`jj-hooks/devenv.nix:8-16`), so a per-repo copy still needs bun
provisioning in each shell — though its wiring is simpler than the shared
path's, being a consumer-side script entry against a repo-relative path
with no store-path interpolation and no single-file constraint. The drift
argument, not the toolchain one, carries this rejection.

**Two copies plus a byte-equality drift check in CI.** The strongest
challenger, and it was not weighed in the first draft: keep a copy per repo
and have dev-shared's reusable CI workflow fail when the two files
diverge. That check is more machinery than it sounds: the reusable
workflow runs inside a single consumer's checkout and has no view of the
sibling repo, so byte-equality needs a cross-repo fetch plus a token.
It does remove the Nix store machinery and the lock-gated propagation
cost that decision 1 has to own — a fix lands in whichever repo needs it,
with no `devenv.lock` bump. Rejected on drift risk rather than PR count —
the counts are close, and the earlier framing undercounted the shared
path. Copies cost two fix PRs, each with the consumer's real CI. The
shared module costs one dev-shared PR plus two consumer lock bumps
(T2/T3), and those bumps arrive CI-unverified, so each wants a manual
`devenv shell -- true` check. What separates them is that a drift check
only reports that the copies disagree — a human still applies the same
fix twice and can apply it two different ways — whereas the shared module
makes divergence impossible by construction. For a tool run a handful of
times a year that is the better trade. Worth revisiting if the
lock-gating ever bites in practice.

**A genuinely new dev-shared distribution surface** (published npm/bun
package, or `bunx github:mattwilkinsonn/dev-shared` invocation). Rejected:
adds registry publishing or network-at-invocation-time to a repo that has
neither, purely to deliver one file the existing module surface can
already deliver as a store path. The propagation story would be no better
than the lock-gated module (a published package pins a version in a
lockfile too).

**Porting the driver into jj-hooks as a `jj-hp` subcommand (Rust).**
Superficially attractive — `jj-hp` already owns the push-tags step
(`index.ts:219`) and ships to both repos. Rejected: it moves a working,
DI-tested TypeScript tool into Rust for no capability Rust adds, couples
the release driver's evolution to jj-hooks' own release cadence (the
driver that cuts jj-hooks releases would ship inside jj-hooks), and
inflates a distribution crate with maintainer-only tooling.

**Driving releases from a GitHub Actions `workflow_dispatch` instead of a
local driver.** Rejected for this record: the local driver's guards are
about local jj state (`@` cleanliness, `main` ancestry of `@`) which does
not exist server-side, and the tag push is already the workflow trigger
(`jj-hooks/.github/workflows/release.yml:10-12`) — a dispatch-driven
bump-commit-tag flow would need its own push identity and would bypass
the laptop-holds-the-creds model the continuity record establishes.

## Open Questions

1. **~~devenv `scripts` visibility under a module import~~ — RESOLVED,
   not an open question.** Both halves are now verified in the consumers'
   actual `flake: false` + `imports:` topology, and the evidence is in
   decision 1: `devenv shell -- release v9.9.9-test` in a fixture consumer
   printed `ok cwd: /tmp/dstest/consumer argv: [ "v9.9.9-test" ]`. The
   `scripts` entry from the imported module is visible, arguments pass
   through, the store-path TS executes, and `cwd` is the consumer's. The
   contingency, should this ever regress: each consumer adds a 3-line
   `scripts.release` entry in its own `devenv.nix` referencing
   `${inputs.dev-shared}/release/index.ts` — **not** a relative path,
   because the module-relative rule quoted above means a relative literal
   in the consumer resolves inside the consumer, not in dev-shared. The
   script stays single-sourced and only the wiring duplicates, the same
   shape as the redeclared rust-overlay input
   (`dev-shared/README.md:42-51`). T1a/T1b is not gated on it.
2. **Should the driver eventually absorb a CHANGELOG guard?** The 0.3.12
   commits added CHANGELOG sections by hand (`jj-hooks/CHANGELOG.md:7`);
   the monorepo driver never checked CHANGELOG state. A
   fail-closed "CHANGELOG has a section for this version" guard would fit
   the existing guard pattern, but no evidence this session establishes
   whether Matt wants CHANGELOG discipline machine-enforced. Left out of
   T1a; one-line decision for Matt at T1a review.

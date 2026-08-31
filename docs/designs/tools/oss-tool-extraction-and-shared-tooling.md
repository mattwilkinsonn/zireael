# Design: OSS tool extraction + shared tooling (`dev-shared`)

Status: **proposed**
Domain: tools

Companion of the platform record that repurposes zireael afterwards (record B,
`docs/designs/platform/`); this record is independent of it but executes first.
Sibling shape: `docs/designs/tools/jj-hp-gate-worktree-cost.md`.

## Problem / Intent

Reverse the zireael OSS monorepo: jj-hooks and jj-gt move back to their own
standalone public repos (`mattwilkinsonn/jj-hooks`, `mattwilkinsonn/jj-gt`,
both just unarchived) for discoverability/marketability; akiflow-cli is
dropped entirely (Akiflow shipped an MCP covering its whole use case). Because
N separate repos re-introduce N copies of dev tooling, stand up one shared
repo — `mattwilkinsonn/dev-shared` — so a toolchain/CI/Renovate fix is one PR
inherited everywhere (exact for the `@v1` CI surfaces; lock-gated for the
devenv module — see §Propagation asymmetry).

## Approach

### Ground truth (verified this session)

**The monorepo is the live source; the standalones are stale.** Both standalone
repos were last pushed `2026-05-26T20:35Z` (GitHub API, `pushed_at`, queried
this session) and carry the pre-monorepo shape — root `src/`, `Justfile`,
`hk.pkl`, per-tool `.github` (GitHub API contents listing, this session). The
live code is the monorepo's, e.g. `tools/jj-hooks/src/` now contains
`repo_env.rs`, `gate_cache.rs`, `worktree.rs` (directory listing this session)
that the 2026-05 standalone `src/` predates. So this is a **re-establish from
current monorepo state**, wholesale — not a merge.

**What the workspace contains.** Root `Cargo.toml:3-7`:

```toml
members = [
    "tools/jj-hooks",
    "tools/jj-gt",
    # akiflow-cli is bun/TypeScript and is not a workspace member.
]
```

with shared `[workspace.dependencies]` (`Cargo.toml:19-34`) and the internal
path-dep entry `Cargo.toml:43`:

```toml
jj-hooks = { version = "0.3.11", path = "tools/jj-hooks" }
```

which jj-gt consumes as `jj-hooks = { workspace = true }`
(`tools/jj-gt/Cargo.toml:48`) so `cargo publish -p jj-gt` auto-rewrites the
path-dep to the crates.io version (`tools/jj-gt/Cargo.toml:43-47` comment).
After the split, jj-gt's dep becomes a plain crates.io version dep — the
auto-rewrite machinery and the publish-ordering wait loop
(`.github/workflows/release.yml:456-472`, "Wait for crates.io to index
jj-hooks") both become unnecessary in jj-gt's standalone release workflow.

**Release machinery to recreate per standalone.** `release.yml` today:
3-target matrix (`darwin-arm64`/`linux-x64`/`linux-arm64`,
`release.yml:68-80`), builds all Rust bins in one invocation
(`release.yml:95-99`: `cargo build --release … --bin jj-hooks --bin jj-hp
--bin jj-gt`), ad-hoc macOS codesign (`release.yml:137-142`), tarball naming
matching the cargo-binstall overrides (`tools/jj-hooks/Cargo.toml:18-28`, e.g.
`pkg-url = "{ repo }/releases/download/v{ version
}/jj-hooks-v{ version }-linux-x64.tar.gz"`), GitHub Release creation
(`release.yml:271-293`), in-place Formula bump pushed to main by the
`zireael-release` GitHub App (`release.yml:306-338`, "the App is in the bypass
actors list for the main-branch ruleset"), tap validation
(`release.yml:404-430`: `brew tap mattwilkinsonn/zireael …`), and crates.io
publish (`release.yml:451-477`). `Formula/jj-hooks.rb:3,9` points at the
monorepo:

```ruby
homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-hooks"
url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-darwin-arm64.tar.gz"
```

— every URL, homepage, App identity, and tap name must be re-pointed per
standalone. Each standalone repo becomes its own tap
(`brew tap mattwilkinsonn/jj-hooks` etc.); the zireael tap stops receiving
bumps.

Recreating this ~478-line pipeline per standalone (T2.5, T3.4) is a
**deliberate deferral of consolidation**, not an oversight: release workflows
are touched rarely and diverge structurally between the repos (bin sets, tap
names, App identity), so two copies are accepted for now. If release.yml
churn starts producing synchronized two-PR fixes, the flagged future move is
a parametric reusable release workflow in dev-shared (inputs: bin list,
formula name, tap repo). That trade is owned here — the next release bug does
not get to re-argue it ad hoc.

**The CI gate being replaced.** `ci.yml:64-65` runs `devenv shell -- moon ci`;
a crate's whole `moon.yml` is just fmt/clippy/nextest + a `ci` aggregator
(`tools/jj-hooks/moon.yml:11-29`: `cargo fmt -p jj-hooks -- --check`,
`cargo clippy -p jj-hooks --all-targets -- -D warnings`,
`cargo nextest run -p jj-hooks --no-fail-fast`, `ci: deps: [fmt, clippy,
test]`) — trivially a devenv tasks DAG. `.prototools:6-8` pins only
`bun`/`node`/`moon`, none of which a pure-Rust single crate needs, so proto
goes too. jj-gt additionally has a `runInCI: false` live suite
(`tools/jj-gt/moon.yml:29-37`: `test-live` running
`-E 'test(gh_live) | test(gt_submit_live)'` with `JJ_GT_LIVE_GH=1` /
`JJ_GT_LIVE_SUBMIT=1`), gated at the test level on env + a fixture repo
(`tools/jj-gt/tests/gh_live.rs:5-8`: `JJ_GT_LIVE_GH=1 — opt-in to live network
tests`, `JJ_GT_LIVE_REPO=<owner>/<repo>`).

**The toolchain being replaced.** zireael pins Rust via fenix with a
hand-maintained hash (`devenv.nix:21`:
`sha256 = "sha256-mvUGEOHYJpn3ikC5hckneuGixaC+yGrkMM/liDIDgoU="`; the comment
above it: "Bumping rust-toolchain.toml's channel invalidates this hash").
orion's rust-overlay pattern has no hash file — `orion/devenv.nix:31-33`:

```nix
rustToolchain =
  (inputs.rust-overlay.lib.mkRustBin { } pkgs).fromRustupToolchainFile
    ./rust-toolchain.toml;
```

with input `orion/devenv.yaml:46-50`
(`rust-overlay: url: github:oxalica/rust-overlay, inputs.nixpkgs.follows:
nixpkgs`), and the rationale in `orion/devenv.nix:28-30`: "rust-overlay
commits its channel manifests as in-tree `.nix` files, so toolchain resolution
is pure-eval (no IFD, no separate hash file)". `rust-toolchain.toml` stays the
single Rust-version source (zireael `rust-toolchain.toml:4`:
`channel = "1.96.0"`).

**What KEEPs from zireael's devenv.** The linter set + hook backends jj-hooks'
integration tests drive: `devenv.nix:29-42` (nixfmt-rfc-style, deadnix,
statix, nil, shellcheck, shfmt, taplo, actionlint, markdownlint-cli2) and
`devenv.nix:56-62` ("jj-hooks' integration tests drive real hook frameworks…
pre-commit, prek, lefthook (+ hk above), and pkl"), plus jujutsu + the `hk`
flake input (`devenv.yaml:16-20`) and rolling nixpkgs (`devenv.yaml:5-6`:
`github:cachix/devenv-nixpkgs/rolling`). SUBTRACT for a single OSS crate:
proto/moon/bun/node (`devenv.nix:26-27`, `.prototools`), the proto enterShell
activation (`devenv.nix:66-71`), and all orion-only machinery (containers,
secretspec, deploy-rs, go, vendored `ci/toolchain/` derivations).

### Settled decisions (Matt ruled; recorded, not re-litigated)

1. **History: clean snapshot.** One "re-sync from monorepo" commit of current
   state atop each standalone's existing pre-monorepo history. Full granular
   history stays in (soon-private) zireael. NOT filter-repo graft.
2. **Standalone tooling: devenv + its built-in `tasks` runner** (`tasks."ns:
   name".exec` + before/after DAG, `devenv tasks run ci`). Drop moon and
   proto. CI runs the devenv tasks via GHA.
3. **Rust toolchain: fenix → rust-overlay**, copying orion's exact pattern
   (quoted above). Kills the hand-maintained sha256.
4. **Renovate, not Dependabot**, matching orion. The relevant manager subset
   for a Rust tool repo, from orion's `ci/renovate/config.json5:63-72`
   `enabledManagers`: `cargo`, `rust-toolchain`, `nix`, plus
   `github-actions` (disabled in orion only because "the meta jobs moved off
   GHA", `config.json5:58-59` — these repos stay on GHA, so it's enabled
   here). orion's bun/node/moon/go/catalog/woodpecker/provider managers do
   not apply. The rust-overlay-input lockstep idea carries over: orion
   couples a `rust-toolchain.toml` bump to a `devenv update rust-overlay`
   re-lock via `postUpgradeTasks` (`config.json5:609-627`: "the leg gates on
   rust-toolchain.toml, runs `devenv update rust-overlay`" …
   `commands: ["bun ci/renovate/refresh-toolchain-hashes.ts"]`). dev-shared's
   preset adapts a minimal version of this (see T1); NOTE orion's bot is
   self-hosted with `configFileNames` overridden (`config.json5:2-4`) —
   the tool repos use hosted Renovate defaults instead (root
   `renovate.json5` extending the shared preset).
5. **Match orion devenv "for the most part"** with the deliberate
   subtractions listed under Ground truth.
6. **Shared-tooling architecture: ONE repo `mattwilkinsonn/dev-shared`**, four
   surfaces: a devenv module (rust-overlay wiring, toolchain, linter set)
   consumed via devenv input + `imports:`; a composite action
   `dev-shared/setup-devenv`; a reusable workflow
   `dev-shared/.github/workflows/rust-devenv-ci.yml` (`on: workflow_call`);
   a shared Renovate preset the tool repos `extends`.
7. **Per-repo CI split (Matt's proposal, carried as the recommended
   approach):** jj-hooks → thin stub calling the reusable workflow; jj-gt →
   ejects to a hand-written `ci.yml` (gate job + fork-gated, secret-bearing
   live-test job) using only the composite action for bootstrap.

### Propagation asymmetry across the four surfaces

"One PR inherited everywhere" is exact only for the `@v1` action/workflow
surfaces. The devenv-module surface is pinned in each consumer's
`devenv.lock` — a module fix propagates only when that lock moves (the
scheduled `devenv update` PR workflow, T1.5, or a manual `devenv update
dev-shared` + lock commit). Per surface:

| Surface | Propagation path | Review gate | Rollback |
| --- | --- | --- | --- |
| Composite action (`@v1`) | instant on next run once the tag moves | dev-shared's own PR review + CI, before the tag moves | move `v1` back to the prior sha |
| Reusable workflow (`@v1`) | instant on next run once the tag moves | same | same |
| devenv module (locked input) | per-consumer `devenv.lock` bump (scheduled update PR or manual) | the consumer's own lock-bump PR | revert the lock-bump commit |
| Renovate preset (`extends` ref) | next Renovate run reads the preset live off dev-shared's default branch | dev-shared PR review | revert the preset commit |

The `@v1` surfaces also invert the blast radius: a bad move of `v1` breaks
BOTH public repos' CI gates simultaneously, unreviewed by either consumer,
with no pinned fallback. Mitigation: dev-shared has its own CI + review, and
the tag moves only after both pass; rollback is a one-command tag move.

### Reusable workflow vs composite action — why both (from Matt's note)

The two GitHub reuse primitives diverge differently, and the design uses each
where its divergence model fits (`~/notes/wave/shared-ci-reuse.md`):

- **Reusable workflow (`workflow_call`) shares a whole JOB.** It diverges
  *parametrically only* — "you bend it through `inputs`/`secrets` you declared
  up front… You canNOT inject steps" (note §1). Its failure mode: "every new
  per-repo difference becomes another `input` flag… the classic 'shared config
  that knows about all its callers' smell". A caller may add *sibling* jobs
  next to the `uses:` job, but cannot restructure the shared one.
- **Composite action shares a chunk of STEPS.** It diverges *structurally, for
  free* — "the caller writes its own jobs/steps and just drops the composite
  action wherever the shared bootstrap is needed… all local, no flags on a
  shared file" (note §2). The shared part stays the piece that is
  byte-identical everywhere and most tedious to sync: install Nix + cache +
  install devenv + warm the shell.
- **The eject property is the load-bearing reason to ship both.** "The moment
  a repo outgrows the reusable workflow, it 'ejects' to a local ci.yml +
  composite action with zero loss of the shared bootstrap. That's the property
  monolithic-reusable-only lacks" (note, closing rule). Concretely: jj-hooks
  looks like every other repo → ~6-line stub calling
  `rust-devenv-ci.yml`; jj-gt needs its own job graph (a fork-gated
  `live-test` job carrying `JJ_GT_LIVE_*` secrets,
  `if: github.event.pull_request.head.repo.full_name == github.repository`) →
  it never calls the reusable workflow, hand-writes `ci.yml`, and shares only
  `setup-devenv`. Building the gate as reusable-workflow-only would have
  forced live-test through input flags + `secrets: inherit` on the shared
  file — exactly the smell the note names.

### Shared devenv module consumption

devenv's documented cross-repo sharing path (devenv.sh "Composing using
imports" → "Sharing configuration from another repository", read this session)
is: declare the shared repo as an input with `flake: false` and list it in
`imports:`; "the sibling shared-config repository only needs a `devenv.nix`
file", adapted per-project via profiles. The reference for `imports` says it
imports "`devenv.nix` **and `devenv.yaml`** files" from inputs — which reads
as the imported repo's own inputs (rust-overlay) composing transitively — but
this is not verified against a running devenv and is exactly the sort of
edge (input namespacing, `follows` wiring across the import boundary) that
docs under-specify. **Design assumption: transitive composition works; safe
fallback: each consumer redeclares the 2-line rust-overlay input in its own
devenv.yaml** (see Open Questions). T1 resolves this empirically before T2/T3
copy the pattern.

### Versioning and publish shape after the split

Both crates leave at the current shared `0.3.11` (`Cargo.toml:10`) and
version independently thereafter. jj-gt's `jj-hooks` dep becomes a plain
crates.io version requirement (`jj-hooks = "0.3.11"`), dropping the workspace
path-dep and the publish-ordering coupling. Each standalone keeps its
cargo-binstall `[package.metadata.binstall]` overrides with `{ repo }` now
resolving to the standalone repo (the URLs in
`tools/jj-hooks/Cargo.toml:18-28` are `{ repo }`-templated, so they re-point
automatically once `repository` in Cargo.toml is updated).

## Global Constraints

- **Tooling contract (settled):** devenv + built-in `tasks` runner; no moon,
  no proto, no `.prototools`, no `moon.yml`/`.moon/`. Rust via rust-overlay
  (`inputs.rust-overlay.lib.mkRustBin { } pkgs).fromRustupToolchainFile
  ./rust-toolchain.toml`, matching `orion/devenv.nix:31-33`), never fenix.
  `rust-toolchain.toml` is the single Rust-version source (pin stays exact:
  `channel = "1.96.0"` at extraction time). Renovate, not Dependabot.
- **nixpkgs channel:** `github:cachix/devenv-nixpkgs/rolling` in every repo's
  devenv.yaml (matches zireael `devenv.yaml:5-6` and orion).
- **History rule:** clean snapshot — one re-sync commit atop each standalone's
  existing history; never filter-repo graft; granular history stays in
  zireael.
- **Scope of pushes/PRs:** `mattwilkinsonn/*` repos only. `jj-vine submit` is
  the only push path; PRs open as drafts and are promoted with
  `gh pr ready` after review all-clear. Never `gh pr create`, never direct
  pushes to main (the release App is the sole sanctioned main-pusher, and
  only from release.yml).
- **CI gate parity:** each repo's `devenv tasks run ci` DAG must cover what
  `moon ci` covers today for that crate AND for the root meta project. Per
  crate: fmt (`cargo fmt -p <crate> -- --check`), clippy (`cargo clippy -p
  <crate> --all-targets -- -D warnings`), test (`cargo nextest run …
  --no-fail-fast`), per `tools/{jj-hooks,jj-gt}/moon.yml`. Root-lint
  coverage: zireael's root `moon.yml` also gates `markdownlint`,
  `actionlint`, `nixfmt --check`, and `deadnix` (root `moon.yml:26-27`:
  `ci: deps: ['markdownlint', 'actionlint', 'nixfmt', 'deadnix']`) over
  exactly the file classes every standalone still carries (README/docs,
  workflows, devenv.nix) — so each standalone's `ci` aggregate gates them
  too, via the shared lint task set the dev-shared module supplies (T1.1).
  jj-gt's default `test` task keeps the live-exclusion filter
  (`-E 'not (test(gh_live) | test(gt_submit_live))'`,
  `tools/jj-gt/moon.yml:25`); live tests run only in the fork-gated CI job.
  One honest cost of dropping moon: `devenv tasks run ci` is unconditional,
  so docs-only/workflow-only PRs pay a full uncached clippy+test compile
  where today `moon ci` with `MOON_BASE`/`MOON_HEAD` (`ci.yml:49-51`)
  resolves zero affected Rust tasks. Minutes per docs PR, not correctness —
  and moon's cache is provably unused today (every task sets `cache: false`,
  `tools/jj-hooks/moon.yml`), so the drop stands.
- **Branch protection:** each standalone re-establishes a main ruleset
  equivalent to `.github/rulesets/main-protection.json` (PR-required, squash
  only, linear history, one required check context), replacing the required
  check `moon CI` (`main-protection.json:38`). The new context is NOT a
  uniform `ci`: GitHub reports a reusable-workflow job as
  `<caller-job> / <called-job>`, so jj-hooks' thin stub (T2.3) surfaces as
  `<caller-job> / gate`, while jj-gt's ejected hand-written job (T3.3)
  reports plain `gate`. Either name the caller and gate jobs so both repos
  land on one string, or record the two distinct required contexts per
  repo — and in both cases verify the exact reported context name from the
  first CI run BEFORE creating the ruleset.
- **Licensing:** MIT OR Apache-2.0 carried over (`Cargo.toml:12`), both
  LICENSE files shipped in release tarballs as today
  (`release.yml:122,134`).
- **dev-shared visibility (ruled): PUBLIC.** The shared repo is created
  public in T1 — cross-repo `uses:` from a public repo to a private personal
  repo does not work, and a private input breaks `devenv update` for external
  contributors. See Resolved decisions.
- **dev-shared version floor:** consumers pin `@v1` moving tag (assumption —
  see Open Questions). Stated honestly: a moving `@v1` ref gives Renovate's
  `github-actions` manager nothing to bump, so under this assumption there
  is NO Renovate tracking of dev-shared at all — propagation is entirely
  tag-move-driven. Switching to sha pins (the OQ's alternative) restores
  manager tracking at the cost of a bump PR per repo per change.

## Plan

### T1 — Stand up `mattwilkinsonn/dev-shared`

Create the shared repo — **PUBLIC**, per resolved decision 1 — with its four
surfaces. This lands first; T2/T3 consume it.

1. **devenv module** (`devenv.nix` at repo root, per devenv's shared-config
   convention): rust-overlay toolchain wiring
   (`fromRustupToolchainFile ../…/rust-toolchain.toml` must resolve against
   the *consumer's* file — expose it as a module option with the consumer
   passing its path, or read `./rust-toolchain.toml` relative to the
   consuming project root; verify which devenv supports during
   implementation), the common linter package set (nixfmt-rfc-style, deadnix,
   statix, nil, shellcheck, shfmt, taplo, actionlint, markdownlint-cli2),
   jujutsu, cargo-nextest, stdenv.cc. Hook backends (pre-commit, prek,
   lefthook, pkl, hk) are NOT in the shared module — they are a jj-hooks
   test-suite concern and live in jj-hooks' own devenv.nix (T2).
   The module also supplies the shared lint TASK set — `ci:markdownlint`,
   `ci:actionlint`, `ci:nixfmt`, `ci:deadnix`, mirroring zireael root
   `moon.yml:11-27` — so root-lint coverage (Global Constraints) is defined
   once here rather than re-specified per repo; each consumer's aggregate
   `ci` task depends on them.
   **Empirically verify the transitivity question here**: build a consumer
   fixture and confirm whether the module's rust-overlay input composes
   through `imports:` or must be redeclared; record the answer in
   dev-shared's README and fold into T2/T3.
2. **Composite action** `setup-devenv/action.yml` (`using: composite`):
   install Nix (`DeterminateSystems/determinate-nix-action@v3`), install
   devenv (`nix profile install --accept-flake-config nixpkgs#devenv`), warm
   the shell (`devenv shell -- true`), plus devenv/nix store caching. Mirrors
   the bootstrap steps zireael's ci.yml runs today (`ci.yml:56-63`).
3. **Reusable workflow** `.github/workflows/rust-devenv-ci.yml`:
   `on: workflow_call` with NO inputs (no `crate` input — the task DAG is
   repo-local; no speculative `extra-tasks` — no consumer uses one, and an
   input added before a caller needs it is exactly the input-flag creep the
   note warns against). One `gate` job = checkout + `setup-devenv`
   composite plus `devenv shell -- devenv tasks run ci`, with `permissions:
   contents: read` + `id-token: write` on the job — the Determinate Nix
   action needs
   OIDC (zireael `ci.yml:18-20`: `id-token: write #
   DeterminateSystems/determinate-nix-action needs OIDC`). The reusable
   workflow consumes the composite action internally, so the bootstrap
   exists in exactly one place. At one consumer it still earns its file: it
   is the template every future standard repo stubs onto.
4. **Renovate preset** `renovate-preset.json5` (consumed as
   `github>mattwilkinsonn/dev-shared//renovate-preset.json5`): extends
   `config:recommended` + `schedule:daily`; `enabledManagers: ["cargo",
   "rust-toolchain", "nix", "github-actions"]`; grouped PRs;
   `minimumReleaseAge` cooldown mirroring orion's `"5 days"`
   (`config.json5:53`). Two hosted-Renovate realities the preset header must
   state: (a) the orion-style rust-overlay lockstep (`postUpgradeTasks`
   running `devenv update rust-overlay` on a rust-toolchain.toml bump,
   `config.json5:609-627`) requires a self-hosted bot with
   `allowedCommands` — hosted Renovate cannot run it; (b) the native `nix`
   manager tracks NOTHING in a pure devenv repo — per orion's own config,
   "there is no root flake.nix for it to anchor on, and devenv.lock is
   devenv's own lock format, not a flake.lock" (`config.json5:164-166`). So
   devenv.lock's inputs (rolling nixpkgs, rust-overlay, hk, the dev-shared
   input itself) get NO updates from Renovate; their real update path is the
   scheduled `devenv update` PR workflow (step 5). Consequently a
   `rust-toolchain` bump PR merges green only while the locked rust-overlay
   rev already carries the new channel manifest — true at extraction time,
   but the first stable released AFTER extraction is absent from the frozen
   rev, so that PR goes red until the scheduled workflow moves rust-overlay.
   The preset header documents the coupling: a red toolchain PR means "merge
   the pending devenv-update PR first".
5. **Scheduled `devenv update` PR workflow** — the hosted-compatible
   substitute for orion's self-hosted `postUpgradeTasks`
   (`config.json5:626-627,635`): `.github/workflows/devenv-update.yml` in
   dev-shared, `on: schedule` + `workflow_dispatch`, exposed via
   `workflow_call` so consumers stub it like the CI gate. It runs the
   `setup-devenv` composite, `devenv update`, and opens a PR with the lock
   diff (create-pull-request-style step). Consumer stubs need `permissions:
   contents: write` + `pull-requests: write` (+ `id-token: write` for the
   Nix action). This is the update path for ALL devenv-locked inputs in
   every consumer, including the dev-shared module pin itself.
   The PR-create step has a repo-setting dependency: opening a PR with the
   built-in `GITHUB_TOKEN` requires "Allow GitHub Actions to create and
   approve pull requests" enabled (repo or org Actions settings), else the
   step fails or silently no-ops — and this workflow is the SOLE update
   path for devenv-locked inputs. The ruleset-setup checklist for
   dev-shared AND every consumer stub (T2.3, T3.3) must enable that
   setting per repo, or the PR-create step must use a PAT/App token
   instead.
6. Repo plumbing: README (consumption snippets for all surfaces), `v1` tag +
   tag-move release convention, own thin `ci.yml` (actionlint + markdownlint
   via the reusable workflow's own machinery is circular — use a minimal
   direct job), main ruleset.

Interfaces:

- Consumes: orion patterns (`orion/devenv.nix:31-33`, `orion/devenv.yaml:46-50`,
  `orion/ci/renovate/config.json5` manager subset); zireael linter set
  (`devenv.nix:29-42`).
- Produces: `github:mattwilkinsonn/dev-shared` devenv input importable via
  `imports: [dev-shared]` (module packages + shared lint task set); `uses:
  mattwilkinsonn/dev-shared/setup-devenv@v1` (composite, no inputs
  required); `uses: mattwilkinsonn/dev-shared/.github/workflows/
  rust-devenv-ci.yml@v1` (`workflow_call`, no inputs; callers must grant
  `id-token: write`); `devenv-update.yml` scheduled-update workflow
  (`workflow_call` + `schedule`); Renovate preset ref
  `github>mattwilkinsonn/dev-shared//renovate-preset.json5`; a written
  verdict on input transitivity.

### T2 — Re-establish standalone `mattwilkinsonn/jj-hooks`

One clean-snapshot commit replacing the stale 2026-05 tree with the current
monorepo crate, plus new repo scaffolding.

1. **Re-sync commit:** copy `tools/jj-hooks/{src,tests,README.md,Cargo.toml}`
   to repo root; delete stale `Justfile`, `hk.pkl`, old `src/`/`tests/`,
   `.github/` workflows. De-workspace Cargo.toml: inline the
   `workspace = true` fields (version `0.3.11`, edition 2024, MIT OR
   Apache-2.0, authors) and the dep versions from root
   `Cargo.toml:19-34`; set `repository =
   "https://github.com/mattwilkinsonn/jj-hooks"` (re-points the
   `{ repo }`-templated binstall URLs, `tools/jj-hooks/Cargo.toml:18-28`).
   Keep both `[[bin]]` targets (`jj-hooks`, `jj-hp`) and the lib. Fresh
   `Cargo.lock`. Copy `LICENSE-MIT`/`LICENSE-APACHE`, `clippy.toml`,
   `.markdownlint-cli2.jsonc`, rustfmt config if any.
2. **devenv:** `devenv.yaml` = rolling nixpkgs + dev-shared input (+
   rust-overlay redeclared iff T1's transitivity verdict requires it) + the
   `hk` flake input (`devenv.yaml:16-20` — hk is a test backend here);
   `devenv.nix` = `imports`-consumed shared module + the jj-hooks-only hook
   backends (`pre-commit`, `prek`, `lefthook`, `pkl`, hk from its input, per
   `devenv.nix:56-62`) + the pkl package-cache warm from `devenv.nix:79-87` +
   `tasks`: `ci:fmt`, `ci:clippy`, `ci:test` mirroring
   `tools/jj-hooks/moon.yml:11-25` commands (without `-p` — single crate),
   and an aggregate `ci` task depending on all three PLUS the shared lint
   task set from the dev-shared module (markdownlint/actionlint/nixfmt/
   deadnix — root-lint coverage per Global Constraints).
   `rust-toolchain.toml` copied verbatim. **Pre-push gate successor:** T2.1
   deletes the stale hk.pkl, so write a fresh one — pre-push step
   `check = "devenv tasks run ci"`, installed on shell entry via
   `hk install` exactly as zireael does today (`hk.pkl:29-31`
   `check = "moon ci"`; `devenv.nix:72-76` runs `hk install` on entry).
   These repos are the home of the pre-push-gate tool; they dogfood it.
3. **CI:** thin `ci.yml` stub — `on: pull_request`, `permissions: contents:
   read` + `id-token: write` (a `workflow_call` job runs with the CALLER's
   granted permissions, and the Determinate Nix action needs OIDC, zireael
   `ci.yml:18-20` — a stub omitting the grant fails the bootstrap), one job
   `uses: mattwilkinsonn/dev-shared/.github/workflows/rust-devenv-ci.yml@v1`
   (jj-hooks has no live tests; nothing to eject for). No graphite
   optimize_ci job (that was a monorepo affair, `ci.yml:23-37`). Plus a
   `devenv-update.yml` stub calling dev-shared's scheduled-update workflow
   (T1.5) with `contents: write` + `pull-requests: write` + `id-token:
   write`.
4. **Renovate:** root `renovate.json5` = `{ extends:
   ["github>mattwilkinsonn/dev-shared//renovate-preset.json5"] }`.
5. **Release:** single-tool `release.yml` derived from the monorepo's — same
   3-target matrix (`release.yml:68-80`), builds `--bin jj-hooks --bin
   jj-hp`, same tarball naming (`jj-hooks-v<X>-<slug>.tar.gz`), ad-hoc
   codesign, GitHub Release, Formula bump + validate against tap
   `mattwilkinsonn/jj-hooks`, `cargo publish` (no ordering wait — no internal
   dep). `Formula/jj-hooks.rb` re-pointed (homepage + URLs to the standalone
   repo). Requires a release GitHub App installed on this repo (reuse
   `zireael-release` by extending its installation, or a renamed clone —
   non-load-bearing, see Open Questions) + `CARGO_REGISTRY_TOKEN` secret +
   ruleset bypass entry.
   **Pre-split asset mirror (resolved decision 2):** re-host the existing
   zireael v0.3.x jj-hooks release tarballs on this repo's Releases page
   (`gh release create v0.3.11 …` with the existing artifacts). Scope: the
   mirror restores manual downloads and Homebrew re-tap installs only —
   pre-built `cargo binstall` for already-published 0.3.x stays broken
   post-flip (see resolved decision 2).
6. **Repo meta:** main ruleset (per Global Constraints), README already
   crate-local (copied), CHANGELOG seeded from the monorepo's jj-hooks
   entries.
7. Gate: `devenv tasks run ci` green locally and in CI on the PR.

Interfaces:

- Consumes: monorepo `tools/jj-hooks/` (crate v0.3.11); root
  `Cargo.toml:19-34` dep versions; zireael `devenv.nix:56-87` (hook
  backends plus pkl warm), `devenv.yaml:16-20` (hk input); dev-shared
  surfaces from T1;
  `.github/workflows/release.yml` (matrix, staging, tap-bump, publish jobs);
  `Formula/jj-hooks.rb`; `.github/rulesets/main-protection.json`.
- Produces: live `mattwilkinsonn/jj-hooks` repo — buildable crate, green
  `ci` check, release pipeline able to ship `v0.3.12+` with tap
  `mattwilkinsonn/jj-hooks` and crates.io publish.

### T3 — Re-establish standalone `mattwilkinsonn/jj-gt` (ejected CI)

Same re-sync shape as T2, with the divergences:

1. **Re-sync commit:** copy `tools/jj-gt/` to root; de-workspace Cargo.toml;
   `jj-hooks = "0.3.11"` as a plain crates.io dep (drops
   `tools/jj-gt/Cargo.toml:48`'s workspace path-dep); `repository` re-pointed.
   Also carry `tools/setup-live-test-fixture/` (the live-test fixture
   provisioning tool referenced by `tests/gh_live.rs:7-8`) into the repo,
   e.g. under `scripts/` where the stale standalone already kept its
   equivalent (stale tree has `scripts/`, GitHub listing this session).
2. **devenv:** as T2 minus the hook backends and pkl warm (jj-gt's tests
   don't drive them); plus `gh` if not ambient (`tests/gh_live.rs:80` checks
   `binary_available("gh")`). Tasks: `ci:fmt`, `ci:clippy`, `ci:test` (test
   keeps the live-exclusion filter from `tools/jj-gt/moon.yml:25`), `ci`
   aggregate, and a `live-test` task mirroring `tools/jj-gt/moon.yml:29-37`
   (`-E 'test(gh_live) | test(gt_submit_live)'`, env `JJ_GT_LIVE_GH=1`,
   `JJ_GT_LIVE_SUBMIT=1`). The `ci` aggregate also depends on the shared
   lint task set (root-lint coverage, per Global Constraints). Pre-push
   successor as T2: `hk.pkl` running `devenv tasks run ci` on pre-push, with
   hk kept as an input here for the hook alone (or devenv's git-hooks
   integration if hk-for-one-hook feels heavy — implementer's call; the
   gated command is the contract).
3. **CI — ejected:** hand-written `ci.yml` owning its job graph, per the
   recommended split:
   - `gate` job: checkout + `uses: mattwilkinsonn/dev-shared/setup-devenv@v1`
     plus `devenv shell -- devenv tasks run ci`.
   - `live-test` job: fork-gated (`if: github.event.pull_request.head.repo.
     full_name == github.repository` — forks get no secrets), same composite
     bootstrap, runs `devenv shell -- devenv tasks run live-test` with
     `GH_TOKEN`/`JJ_GT_LIVE_REPO` from repo secrets.
   - Does NOT call the reusable workflow (the eject property in action).
   - Plus the `devenv-update.yml` stub as T2 (the scheduled-update workflow
     is orthogonal to the CI eject).
4. **Renovate / release / meta:** as T2, adjusted: builds `--bin jj-gt`,
   tarballs `jj-gt-v<X>-<slug>.tar.gz`, `Formula/jj-gt.rb` re-pointed, tap
   `mattwilkinsonn/jj-gt`, plain `cargo publish` (dep is a released
   crates.io jj-hooks version — T2's publish must land first if the pinned
   version isn't yet on crates.io; at 0.3.11 it already is).
   The v0.3.x jj-gt asset mirror lands here too (`gh release create
   v0.3.11 …` with the existing zireael tarballs — resolved decision 2).
5. Gate: `ci` green; `live-test` green on a same-repo PR with secrets
   configured.

Interfaces:

- Consumes: monorepo `tools/jj-gt/` + `tools/setup-live-test-fixture/`; T1
  surfaces; T2's published jj-hooks crate version; monorepo release/Formula
  machinery as T2.
- Produces: live `mattwilkinsonn/jj-gt` repo — green two-job ejected CI,
  release pipeline, tap, crates.io publish; the worked example of the eject
  pattern for future repos.

### T4 — Drop akiflow-cli

akiflow-cli is not a Cargo member (`Cargo.toml:6` comment) — it's a bun tool
under `tools/akiflow-cli/` with its own release matrix job
(`release.yml:166-245`, `build-akiflow-cli`), Formula (`Formula/
akiflow-cli.rb`), and tap-validation entry (`release.yml:425`:
`for formula in jj-hooks jj-gt akiflow-cli`).

1. Delete `tools/akiflow-cli/` and `Formula/akiflow-cli.rb`.
2. Purge dangling refs — the load-bearing one first: the registered moon
   project source `.moon/workspace.yml:20` (`akiflow-cli:
   'tools/akiflow-cli'`). This task's PR lands on zireael, whose gate is
   still `moon ci`, and moon errors on load when a registered project's
   source directory is missing — deleting `tools/akiflow-cli/` without
   removing this entry red-gates T4's own PR. Then: `release.yml`
   (`build-akiflow-cli` job, its `needs:` entries at `release.yml:250,298`,
   the `akiflow-cli` globs in `publish-release`/`bump-tap`/`validate-tap`,
   the `AKIFLOW_CLI_*` env at `release.yml:375-377`),
   `.github/scripts/bump-formulae.py`, nightly's tap loop if present, root
   README, both issue-template dropdowns
   (`.github/ISSUE_TEMPLATE/bug_report.yml:2,12` and
   `feature_request.yml:12` carry `akiflow-cli (af)` options),
   `Formula/README.md:7,16` (install one-liner + formula table row),
   `docs/specs/platform/ci.md:30` (project table row), the akiflow
   MIT-exception note at `LICENSE.md:14-18`, `.prototools` (moot — dies
   with record B, but no ref may survive this PR either), CHANGELOG note.
3. No archive/extraction: the code stays in zireael history (clean-snapshot
   rule). Existing releases stay downloadable on the zireael Releases page
   ONLY while zireael stays public — record B flips it private, which kills
   every already-shipped asset URL. Per resolved decision 2, the jj-hooks/
   jj-gt v0.3.x assets are mirrored onto the standalones (T2/T3) and this
   task adds the tap-migration note to zireael's root README (old zireael-tap
   users: `brew untap mattwilkinsonn/zireael`, then `brew tap
   mattwilkinsonn/jj-hooks` / `mattwilkinsonn/jj-gt`). akiflow-cli's old
   artifacts get NO mirror — the tool is dropped and has no standalone; that
   breakage is accepted as part of the same ruling.
4. Verification: `grep -ri akiflow` over the repo returns only
   CHANGELOG/history mentions — reachable with the full purge list above
   (`.moon/workspace.yml` must be purged regardless, as the gate-breaker).

Interfaces:

- Consumes: `tools/akiflow-cli/`, `Formula/akiflow-cli.rb`,
  `release.yml:166-245,250,298,375-377,425`, `.github/scripts/`.
- Produces: a zireael tree with zero live akiflow-cli references, as a PR on
  zireael (this task, unlike T1-T3, lands in the monorepo — it can ride
  with or ahead of record B's repurpose).

### T5 — Re-home jj-hooks issues #300/#301 + design PR #302

The open jj-hooks work must follow the crate to its standalone repo.

1. Close issues #300 and #301 on `mattwilkinsonn/zireael`, each with a
   pivot note linking the standalone successor; close the unmerged design
   PR #302 (the record for #301) with the same pivot note.
2. Re-file on `mattwilkinsonn/jj-hooks`: the #301 bug, and #300's deferred-T3
   scope, carrying over bodies + links back to the zireael originals.
3. **Adapt-and-carry the #302 design record**
   `docs/designs/tools/jj-hp-gate-worktree-devenv-bootstrap.md` into
   standalone jj-hooks — adapt, never verbatim-copy: the record is
   monorepo-shaped. Its tracking pointer is `mattwilkinsonn/zireael#301`
   (`jj-hp-gate-worktree-devenv-bootstrap.md:6`, an issue this task closes,
   in a soon-private repo); its paths are `tools/jj-hooks/…`-rooted; and its
   "**Monorepo single version number** — bump the one
   `[workspace.package].version` field (root `Cargo.toml:10` …); the
   internal path-dep pin (`Cargo.toml:43`) auto-rewrites on publish" Global
   Constraint (`:74-77`) is false post-split. Adaptation: re-point the
   tracking line to the re-filed jj-hooks successor of #301, rewrite paths
   to the standalone root, replace the monorepo-version constraint with the
   standalone's independent-version reality. Ship it as its OWN docs-only PR
   on standalone jj-hooks through that repo's review gate — NOT folded into
   T2's re-sync commit (that would land a never-ratified record under a
   scaffolding PR whose reviewer is looking at repo plumbing, not design
   content). Carry the empirical evidence doc it leans on
   (`devenv-worktree-bug-findings.md`; the record's root cause rests on
   "full writeup at `devenv-worktree-bug-findings.md` in the workspace",
   `:20-21`) into the same PR — or inline its load-bearing findings — so
   the public record's grounding stays reachable. **#302's record must NOT
   be merged into zireael** — it lands only in the standalone repo.
4. Update `~/agents/workspaces/<codename>/owned-issues.md` trackers as issues
   move (rule://track-owned-issues).

Interfaces:

- Consumes: zireael issues #300/#301 + design PR #302; the #302 record file
  from the in-flight branch (not zireael main).
- Produces: closed zireael issues with pivot notes; open standalone jj-hooks
  issues; the #302 record living in standalone jj-hooks.

### Sequencing

T1 → (T2 → T3) with T4, T5 parallel to T3 (T5 step 3 depends on T2's repo
existing; T3's crates.io dep needs jj-hooks 0.3.11 published — already true —
so T3 only hard-depends on T1). Record B (zireael repurpose) starts after
T2 + T3 + T4 confirm the monorepo is no longer the live home of anything OSS.
**Cross-record dependency (resolved decision 2): record B's private flip
(its T8) is sequence-gated on the v0.3.x asset mirrors (T2/T3) and the
tap-migration note (T4) having landed** — the infra record's T8 waits on
this record confirming both.

## Tasks

- [ ] T1: `dev-shared` repo — devenv module (+ shared lint task set),
      `setup-devenv` composite action, `rust-devenv-ci.yml` reusable
      workflow (no inputs, OIDC permissions), scheduled `devenv-update.yml`
      PR workflow, Renovate preset, `v1` tag, transitivity verdict recorded.
- [ ] T2: standalone jj-hooks — re-sync commit, de-workspaced Cargo.toml,
      devenv (+hook backends) with `ci` tasks DAG incl. root-lint tasks, hk
      pre-push gate, thin ci.yml stub (id-token grant) + devenv-update stub,
      Renovate extends, release.yml + Formula + tap, ruleset, v0.3.x asset
      mirror; build + tests green.
- [ ] T3: standalone jj-gt — re-sync commit, crates.io jj-hooks dep, devenv
      with `ci` (incl. root-lint tasks) + `live-test` tasks, pre-push gate,
      ejected ci.yml (gate + fork-gated live-test) + devenv-update stub,
      release.yml + Formula + tap, ruleset, v0.3.x asset mirror; both jobs
      green.
- [ ] T4: akiflow-cli dropped from zireael — code, Formula, release/nightly/
      script refs purged; grep-clean; tap-migration note added to root
      README.
- [ ] T5: issues #300/#301 closed on zireael with pivot notes + design PR
      #302 closed unmerged; #301 + #300-T3 re-filed on standalone jj-hooks;
      #302 record adapted (tracking pointer, paths, version constraint) +
      carried into standalone jj-hooks as its own reviewed docs PR (never
      merged to zireael), evidence doc carried with it.

## Resolved decisions

Load-bearing questions ruled by Matt; recorded as decisions, not
re-litigated.

1. **dev-shared visibility: PUBLIC (ruled; was the recommendation).**
   jj-hooks and jj-gt are public repos whose `devenv.yaml` declares
   dev-shared as an input and whose `ci.yml` `uses:` its action/workflow. A
   private dev-shared would (a) break `devenv update` for any external
   contributor (the input fetch needs auth they don't have) and (b) fail the
   cross-repo `uses:` outright — GitHub only shares private actions/reusable
   workflows within the same org/enterprise, never from a public repo to a
   private personal one. The repo holds only toolchain/CI boilerplate, so
   publicity costs nothing. Visibility is fixed at creation in T1.
2. **Pre-split release assets: mirror + sequence-gate (ruled: option a).**
   When record B flips zireael PRIVATE, every already-shipped 0.3.x artifact
   URL dies: `cargo binstall` resolves `{ repo }` from each published
   version's IMMUTABLE Cargo.toml — which points at zireael
   (`tools/jj-hooks/Cargo.toml:18-28`, `pkg-url = "{ repo
   }/releases/download/v{ version }/…"`); every installed Homebrew formula's
   `url` (`Formula/jj-hooks.rb:9`, zireael-hosted) breaks for
   reinstall/upgrade; crates.io/docs.rs repository links for 0.3.x go dark.
   Re-pointing `repository` in T2/T3 fixes only FUTURE versions. Decision:
   during T2/T3, mirror the v0.3.x release tarballs onto each standalone's
   Releases page (`gh release create v0.3.11 …` with the existing
   artifacts), and T4 adds a tap-migration note to zireael's root README;
   **record B's private flip (its T8) is sequence-gated on both landing**
   (cross-record dependency — see §Sequencing; the infra record's T8 waits
   on this). Coverage stated honestly: the mirror restores manual downloads
   and Homebrew re-tap installs only. It does NOT restore pre-built
   `cargo binstall` for the already-published 0.3.x versions — binstall
   resolves `{ repo }` from each version's immutable crates.io metadata,
   which points at soon-private zireael, and never consults the standalone
   Releases pages; post-flip it 404s and at best falls back to a slow
   source build. That residual breakage is accepted, exactly as
   akiflow-cli's is. This absorbs the earlier "old zireael tap users" open
   question (old OQ5) — same concern, understated there as "stop receiving
   bumps" when the real cost is losing download access to everything
   already shipped. akiflow-cli's old artifacts are deliberately NOT
   mirrored (the tool is dropped, no standalone exists); that breakage is
   accepted.

## Open Questions

Non-load-bearing deferrals only; each is designed against a stated
assumption and blocks no task's shape.

1. **`uses:` pin style (@v1 moving tag vs @sha + Renovate).** Matt's note
   frames both: "A tag lets a fix propagate on next run; a sha requires a
   Renovate bump per repo… Pick `@v1`-style moving tag for low-ceremony
   propagation, sha+Renovate if you want each bump reviewed" (note
   §Versioning). **Assumption: `@v1` moving tag** (the note's own lean;
   propagation-by-default is the point of dev-shared; the
   blast-radius/no-tracking trade is stated in §Propagation asymmetry and
   Global Constraints). Trivially switchable later per repo.
2. **devenv input transitivity (safe fallback exists).** Whether the shared
   module's rust-overlay input composes transitively into consumers through
   `imports:`, or must be redeclared in each consumer's devenv.yaml, could
   not be verified from the docs this session (the `imports` reference says
   it imports "devenv.nix and devenv.yaml files" from inputs, which suggests
   transitivity, but input namespacing/`follows` behavior across the
   boundary is unspecified). **Assumption: T1 verifies empirically; fallback
   is redeclaring the 2-line input per consumer**, which works regardless
   and costs two lines per repo.
3. **Release GitHub App identity per standalone.** The monorepo's tap-bump
   pushes to main via the `zireael-release` App (`release.yml:306-324`). The
   standalones need the same capability. **Assumption: extend the existing
   App's installation to the new repos and reuse it** (an App can be
   installed on multiple repos; its slug appearing as `zireael-release[bot]`
   in standalone commit authorship is cosmetic). Alternative: one new
   `dev-release` App. Either works; changes only two workflow env values and
   the ruleset bypass entry.

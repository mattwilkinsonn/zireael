# Design: repurpose zireael as the personal-infra Pulumi monorepo

Status: Draft

## Problem / Intent

Once jj-hooks and jj-gt are extracted to standalone repos and akiflow-cli is
dropped (record A, `docs/designs/tools/` — assumed complete before this record
executes), zireael's remaining shell is repurposed into Matt's **private
personal-infrastructure Pulumi monorepo** — the personal-scope analogue of
orion. It manages, all as Pulumi TypeScript deployed by merge-to-main via
GitHub Actions: GitHub repos + branch protection for `mattwilkinsonn/*`
(including the newly-extracted tool repos), existing Cloudflare domains
(zones/DNS), and a currently-empty AWS account (baseline scaffold only).

## Settled decisions (Matt ruled — recorded, not re-litigated)

1. **Pulumi backend: Pulumi Cloud**, personal org, keyless OIDC from CI (like
   orion). Zero-ops state, free at personal scale, no chicken-and-egg (no AWS
   account needed to hold state). NOT self-managed S3.
2. **CI: GitHub Actions.** preview-on-PR / up-on-merge, mirroring orion's
   Pulumi CD model but on GHA instead of Woodpecker (Matt's GitHub Pro grants
   3000 min/month — ample for small infra).
3. **Visibility: zireael flips to PRIVATE** (personal infra, secrets-adjacent).
4. **Tooling contract (shared with record A):** keep devenv + its built-in
   `tasks` runner; DROP moon and proto repo-wide; Renovate not Dependabot. No
   Rust remains in this repo, so the rust-overlay question is record A's alone.

## Approach

Two phases: **gut** the Rust/monorepo machinery, then **scaffold** an
orion-shaped `infra/pulumi/` tree adapted from Woodpecker to GHA.

### What is gutted (grounded against the current tree)

All of the following exists today and goes away:

- **Root Rust workspace** — `Cargo.toml:1-7` declares
  `members = ["tools/jj-hooks", "tools/jj-gt", …]`; plus `Cargo.lock`. Gone
  with the extraction.
- **`tools/`** — `jj-hooks/`, `jj-gt/`, `akiflow-cli/`, `release/`,
  `install-debug/`, `setup-live-test-fixture/` (all six dirs listed in
  `.moon/workspace.yml:16-23`). The release/install tooling exists only to
  ship the extracted tools.
- **`Formula/`** — the Homebrew tap (`akiflow-cli.rb`, `jj-gt.rb`,
  `jj-hooks.rb`, `moon.yml`, `README.md`). Formulae move with their tools
  (record A).
- **Release/nightly workflows** — `.github/workflows/release.yml:7-9` ("Builds
  release artifacts for every monorepo tool, attaches them to a single GitHub
  Release, bumps each Formula/*.rb in-place, and publishes the Rust crates to
  crates.io") and `nightly.yml:3-6` ("Daily cron that runs the full moon gate
  … plus the tap install smoke-test"). Both are tool-release machinery.
- **`.github/rulesets/`** — `main-protection.json` (the checked-in ruleset
  requiring the `"context": "moon CI"` status check,
  `main-protection.json:38`). Branch protection becomes a *resource of the
  github stack* (T3), so the JSON-file copy is superseded.
- **Rust toolchain config** — `rust-toolchain.toml:4` (`channel = "1.96.0"`),
  `clippy.toml:3` (`msrv = "1.89"`).
- **proto** — `.prototools:6-8` pins `bun`/`node`/`moon`; dropped per the
  tooling contract. Bun comes from devenv/nixpkgs instead.
- **moon** — root `moon.yml`, `.moon/workspace.yml`, `Formula/moon.yml`, and
  the moon-invoking CI: `ci.yml:64-65` (`run: devenv shell -- moon ci`) and
  `post-merge.yml` (same shape on `push: main`). Replaced by devenv tasks +
  purpose-built GHA workflows.
- **`CHANGELOG.md`** — the tool changelog; its history goes with the tools.
- **`hk.pkl`** as-is — `hk.pkl:29-31` wires pre-push to `check = "moon ci"`;
  hk stays but its gate command is retooled (T2).

### What is kept / repurposed (grounded)

- **`docs/`** — already the designs/specs house (`docs/README.md:6-11`:
  "`designs/<domain>/` — point-in-time design records … `specs/<domain>/` — the
  living source of truth"). Tool specs move with record A; platform docs stay.
- **`.envrc`** — the devenv/direnv loader (`.envrc:12` `use devenv`); only the
  `watch_file .prototools` line (`.envrc:10`) goes with proto.
- **devenv, retooled** — `devenv.nix` currently builds the Rust toolchain via
  fenix (`devenv.nix:17-22`) and activates proto on shell entry
  (`devenv.nix:66-71`). Both go. It keeps the lint/tooling role and gains
  `pulumi`, `bun`, `biome`, and devenv `tasks` for the per-stack preview/up.
  `devenv.yaml:9-13` drops the `fenix` input.
- **`LICENSE.md` / `LICENSE-MIT` / `LICENSE-APACHE`** — harmless to keep in a
  private repo; extracted tools carry their own copies (record A).
- **`.gitignore`** — keeps the Bun/devenv/direnv sections (`.gitignore:7-11`,
  `27-31`); sheds the Rust (`1-5`) and moon (`33-36`) sections; gains Pulumi
  ignores.
- **`biome.json`, `.markdownlint-cli2.jsonc`, `.github/CODEOWNERS`,
  `ISSUE_TEMPLATE/`** — kept; biome now lints the Pulumi TS.

### The Pulumi tree (mirrors orion)

`infra/pulumi/` is its **own bun workspace**, exactly orion's shape: orion's
`infra/pulumi/package.json:5-8` declares `"workspaces": ["platform/*",
"services/*"]` with a private root manifest. Each stack is
`platform/<name>/` with `index.ts`, `Pulumi.yaml`, `Pulumi.prod.yaml`,
`package.json`, `tsconfig.json` (orion's `platform/github/` dir is the
reference). Stack name: `mattwilkinsonn/prod` (orion uses `rigelbuild/prod`).
Project names `zireael-<stack>` (orion: `orion-platform-github` per its
`Pulumi.yaml:1`).

All three providers are **native** (skill://iac decision order step 1):
`@pulumi/github`, `@pulumi/cloudflare`, `@pulumi/aws` — confirmed against
orion's stacks (`platform/github/index.ts:1` `import * as github from
"@pulumi/github"`; `platform/cloudflare/zones.ts:1`; `platform/aws/index.ts:1`).
No bridged providers needed.

**Auth, keyless where possible:**

- **CI → Pulumi Cloud:** GHA's native OIDC token exchanged for a short-lived
  `PULUMI_ACCESS_TOKEN` via `pulumi/auth-actions` with a Pulumi Cloud OIDC
  issuer trusting `token.actions.githubusercontent.com` — no stored PAT.
- **AWS:** ESC environment `<org>/infra/aws` assumes an IAM role over OIDC,
  exporting standard AWS env vars — orion's exact pattern
  (`platform/aws/Pulumi.prod.yaml:3-8`: "short-lived credentials minted by the
  Pulumi ESC environment … assumes an IAM role over OIDC (no long-lived access
  keys)"). The IAM OIDC provider + role are out-of-band bootstrap, documented
  in the stack's `BOOTSTRAP.md` (skill://iac step 4 for bootstrap identity).
- **Cloudflare:** a scoped API token delivered as
  `CLOUDFLARE_API_TOKEN` via an ESC env — orion's pattern
  (`platform/cloudflare/Pulumi.prod.yaml:3-11`: "the ESC env is the single
  delivery point, keeping the token out of the CI pull_request event and out
  of Pulumi state").
- **GitHub:** a **dedicated personal GitHub App** (ruled — see Open
  Questions § 4), orion's model scaled to a personal account: orion
  authenticates via a dedicated org-admin App
  (`platform/github/BOOTSTRAP.md:3-6`: "authenticates as a dedicated
  rigelbuild-github-iac GitHub App … no PAT" — short-lived installation
  tokens minted at run time). Here the App is installed only on the
  `mattwilkinsonn` account; the App ID / installation ID / PEM are
  delivered server-side via ESC env `infra/github` (orion's
  `Pulumi.prod.yaml:8-9` `environment: [infra/github]` pattern).

### GHA deploy — the critical adaptation from Woodpecker

Orion's wiring is moon tasks + a Woodpecker policy overlay: per-stack
`preview-<stack>`/`up-<stack>` tasks (`infra/pulumi/moon.yml:179-189`, e.g.
`pulumi preview -C platform/aws --stack rigelbuild/prod --refresh --diff
--non-interactive` / `pulumi up … --refresh --yes --non-interactive`), fanned
out per-affected-stack by `ci/pipeline.ts` on PR (preview) and push→main (up).
With moon gone, the GHA equivalent here is:

- **One workflow, two triggers** — `infra-deploy.yml` on
  `pull_request` (paths: `infra/pulumi/**`) and `push: branches: [main]`
  (same paths filter). PR event → `pulumi preview`; push event → `pulumi up`.
- **Affected-stack detection — PR previews only** — a small TypeScript script
  (`infra/pulumi/scripts/affected.ts`, per rule://scripts-ts-over-bash) diffs
  base..head, maps changed paths to stack names (`platform/<s>/**` → `<s>`;
  workspace-wide files `package.json`/`bun.lock`/`shared/**` → all stacks —
  the same one legitimate cross-cutting trigger orion preserves,
  `moon.yml:49-52`), and emits a JSON stack list that fans the PR preview
  matrix out per stack. **On push-to-main, `up` runs on ALL stacks,
  unconditionally — no affected filter.** GHA concurrency groups hold at
  most ONE pending run ("Any previously pending run in the group will be
  cancelled" is GitHub's documented semantics): merges A (running) /
  B (pending) / C (lands) cancel B, and C's event-scoped path diff never
  covers B's stacks, so B's change silently never applies — main≠live with
  no signal. `--refresh` does NOT catch this (it reconciles state→live, not
  un-applied code→state), and orion has no analogue (Woodpecker queues
  every build). With only three stacks, an `up` on an unchanged stack is a
  cheap no-op under `--refresh`; the upgrade path when the stack count
  grows is diffing against the last-successfully-applied SHA (recorded as
  a stack tag or read from the runs API) instead of the push event's
  `before`.
- **`--refresh` mandatory, enforced via a data file** — every preview/up argv
  carries `--refresh` (orion: "EVERY `preview-*` and `up-*` task below carries
  --refresh so Pulumi reconciles state->live before it plans",
  `moon.yml:14-24`, and the drift incident RIG-865 recorded there). The argv
  is NOT embedded in interpolated workflow shell: stack names and the shared
  preview/up argv templates live in a data module (`infra/pulumi/stacks.ts`)
  that the workflow steps, the devenv tasks, and the tests all consume.
  Enforcement is a `bun test` (`refresh-gate.test.ts`) that imports that
  module and fails on any preview/up invocation missing `--refresh` —
  parsing structure, not embedded shell strings (the moon-free analogue of
  orion's `tools/pulumi-refresh-gate`, whose test likewise parses YAML task
  fields rather than shell, `affected-scoping.test.ts:28-33`).
- **Concurrency guard covers preview AND up** — one per-stack GHA
  `concurrency` group `infra-<stack>` shared by both job kinds.
  `preview --refresh` is not read-only: the refresh step takes the stack
  lease and persists refreshed state, so an unguarded PR preview racing a
  merge's `up` (or another preview) on the same stack fails with "another
  update is currently in progress" — reddening a check, or on the up side
  aborting an apply mid-flight and manufacturing exactly the partial-apply
  the guard exists to prevent. Up jobs keep no-`cancel-in-progress`; PR
  preview jobs may safely set `cancel-in-progress: true` against each
  other.
- **Required check = the stable rollup, never a per-stack pulumi context** —
  the ruleset's required status check stays the T2 `CI` rollup (the
  `ci.yml:70-99` "single stable check context for branch protection"
  pattern this record already keeps), which reports on every PR. The
  per-stack preview contexts (`pulumi (github)`, `pulumi (aws)`, …) are
  informational, NON-required checks: a required context on a
  `paths:`-filtered workflow is a known wedge — a docs-only PR never
  triggers `infra-deploy.yml`, the required context never reports, and the
  ruleset blocks the merge forever — and matrix contexts are per-stack and
  thus unstable even across infra PRs (a github-only PR never reports the
  aws context).
- **OIDC boundary is the issuer authorization policy** — the Pulumi Cloud
  OIDC issuer is registered with two PINNED sub-claim policies, never the
  wildcard `repo:<owner>/<repo>:*`: the up path trusts
  `repo:mattwilkinsonn/zireael:ref:refs/heads/main`, the preview path
  `repo:mattwilkinsonn/zireael:pull_request`. On the individual Pulumi tier
  the minted token is a PERSONAL token carrying Matt's full permissions
  across every stack and ESC env (org-scoped tokens are Team-edition+), so
  a wildcard sub would let ANY workflow/branch/event with `id-token: write`
  reach the admin GitHub credential, the Cloudflare token, and the AWS
  role. The preview job additionally carries a MANDATORY same-repo gate —
  `if: github.event.pull_request.head.repo.full_name == github.repository`
  — designed safety rather than reliance on GHA's implicit
  no-id-token-for-forks behavior, which matters especially while the repo
  is still public pre-T8.
- **Local runs** — devenv tasks `infra:preview-<stack>` wrap the same argv for
  local sanity checks; the apply path is the merge, never a local `pulumi up`
  (skill://iac: "You do not run `pulumi up` as the normal path").

### Import-adoption for live resources

Existing Cloudflare zones/DNS (and the GitHub repos themselves) already exist
live, so they are **import-adopted, never recreated** — orion's cloudflare
stack is the template: every pre-existing resource declared with a
per-resource `import:` ID so "the first `pulumi preview` is therefore a no-op
— pure imports, no create/update/replace/delete — and the live account is
never recreated" (`platform/cloudflare/README.md:13-15`); zones carry
`protect: true` (`zones.ts:59-67` shows the `{ import: ZONE_RIGELAI_DEV,
protect: true }` shape). GitHub repos use the same import-first adoption
(orion `platform/github/index.ts:58-60`: "Import-first adoption of an
already-existing repo (true) vs a fresh create (false)").

### Standing constraint: protected teardown is two merges

Any `protect: true` resource (zones, repos) is subject to
rule://pulumi-protected-teardown: removal is two merges (PR 1 flips
`protect: false` and applies; PR 2 deletes), never one PR and never a manual
`pulumi state unprotect`. Unlikely to bite a greenfield monorepo soon, but it
binds from the first protected import.

## Plan

### Global Constraints

- **Pulumi + TypeScript on bun** for all IaC; `infra/pulumi/` is its own bun
  workspace (`workspaces: ["platform/*"]`), separate from any root workspace.
- **Pulumi Cloud backend**, personal org, stack `mattwilkinsonn/prod` per
  project; CI authenticates keylessly via GHA OIDC → Pulumi Cloud (no stored
  PAT). Provider creds: AWS via ESC OIDC (keyless); Cloudflare token and
  GitHub credential delivered via ESC environments, never repo secrets in
  plaintext workflows where ESC can carry them.
- **GHA preview-on-PR / up-on-merge-to-main** — previews fan out
  per-affected-stack; the merge path `up`s ALL stacks unconditionally
  (lost-apply guard, Approach § GHA deploy). Every `pulumi preview`/`pulumi
  up` argv carries `--refresh --non-interactive` (+ `--diff` on preview,
  `--yes` on up) — mirroring orion's `infra/pulumi/moon.yml:180,187`
  argvs — and is sourced from the `infra/pulumi/stacks.ts` data module,
  with a committed test enforcing `--refresh` presence (the moon-free
  pulumi-refresh-gate analogue). Pulumi OIDC issuer policies pin sub-claims
  (main-ref for up, `pull_request` for preview); the preview job carries a
  mandatory same-repo gate.
- **No moon, no proto** anywhere in the repo (tooling contract). devenv +
  devenv `tasks` is the only task runner; bun/pulumi/linters come from
  devenv/nixpkgs.
- **Provider decision order** (skill://iac): native provider → bridged TF
  provider → REST-API dynamic provider → documented manual bootstrap. All
  three initial stacks are native.
- **Import-adopt, never recreate**: every live pre-existing resource (GitHub
  repos, Cloudflare zones + DNS records) is declared with a per-resource
  `import:` ID; first preview must plan imports/no-ops only. Load-bearing
  imports get `protect: true`.
- **rule://pulumi-protected-teardown**: removing any `protect: true` resource
  is two merges (unprotect → apply → delete → apply); never a manual
  `pulumi state unprotect`.
- **Repo flips private only in T8, after CI is proven** — ordering, not an
  option.
- Bootstrap identities (Pulumi OIDC issuer registration, AWS IAM OIDC
  provider + role, Cloudflare token mint, GitHub credential) are documented
  per-stack in `BOOTSTRAP.md` files (skill://iac § manual-step carve-out);
  each is one-time and minimized.
- Conventional Commits; design/spec docs updated in the same PR as behavior
  changes (`docs/README.md:18-21`).

### T1 — Gut the Rust/monorepo machinery

Delete (precise list, grounded in Approach § "What is gutted"):
`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`,
`.prototools`, `tools/` (all six subdirs), `Formula/`, `CHANGELOG.md`,
`moon.yml`, `.moon/workspace.yml`, `.github/workflows/release.yml`,
`.github/workflows/nightly.yml`, `.github/rulesets/` (both files),
`docs/specs/tools/` + `docs/designs/tools/` records that moved with record A.
Edit: `.gitignore` (drop Rust lines 1-5 and moon lines 33-36),
`.envrc` (drop `watch_file .prototools`, line 10), `README.md` (rewrite for
the new purpose).

Precondition: record A's extraction PRs are merged and the standalone tool
repos exist.

Interfaces:

- Consumes: the post-extraction tree.
- Produces: a repo whose only substantive content is `docs/`, devenv/direnv
  config, `.github/` (ci.yml + post-merge.yml pending T2 retool), licenses.
  `git grep -l 'moon\|proto\|cargo'` over tracked config returns only
  historical docs.

### T2 — Retool devenv + the repo gate (moon-free)

Rewrite `devenv.nix`: drop fenix/proto/Rust/hook-framework packages
(`devenv.nix:17-22,27,49-62`), keep the nix/markdown/actionlint linters, add
`pulumi`, `bun`, `biome`, `nodejs` (for the pulumi nodejs runtime host).
Drop the `fenix` input from `devenv.yaml:9-13`. Define devenv `tasks`:
`repo:lint` (biome + markdownlint + actionlint + nixfmt/deadnix),
`infra:test` (`bun test` in `infra/pulumi`), and per-stack
`infra:preview-<stack>` wrappers (local-only convenience). Rewrite `hk.pkl`'s
pre-push step from `check = "moon ci"` (`hk.pkl:29-31`) to the devenv task
entrypoint. Rewrite `.github/workflows/ci.yml` + `post-merge.yml` to run the
devenv lint+test tasks instead of `devenv shell -- moon ci` (`ci.yml:65`);
keep the single `moon CI`-style rollup job pattern (`ci.yml:70-99`) under a
new stable check name `CI`.

Interfaces:

- Consumes: T1's gutted tree.
- Produces: `devenv.nix`/`devenv.yaml` (bun+pulumi shell), `hk.pkl`
  (pre-push → devenv tasks), `ci.yml` exposing required check context `CI`;
  `devenv shell -- devenv tasks run repo:lint` green locally and in GHA.

### T3 — infra/pulumi scaffold + Pulumi Cloud auth

Create `infra/pulumi/{package.json,bun.lock,tsconfig.json}` (workspace root
mirroring orion `infra/pulumi/package.json:5-8`), `infra/pulumi/README.md`
(backend, stack naming, bootstrap index), and the GHA OIDC trust: register
GitHub Actions as an OIDC issuer in the personal Pulumi Cloud org with TWO
pinned sub-claim authorization policies — up path
`repo:mattwilkinsonn/zireael:ref:refs/heads/main`, preview path
`repo:mattwilkinsonn/zireael:pull_request` — never the wildcard
`repo:<owner>/<repo>:*` (Approach § GHA deploy); a root `BOOTSTRAP.md`
documents that one-time registration and both policies. Add
`infra/pulumi/stacks.ts` — the data module declaring stack names and the
shared preview/up argv templates, consumed by the workflow, the devenv
tasks, and the tests. Add `infra/pulumi/scripts/affected.ts` (path-diff →
stack list; workspace-wide files trigger all stacks; used by PR previews
only) with a `bun test` covering the mapping, plus `refresh-gate.test.ts`,
which imports `stacks.ts` and asserts every preview/up argv carries
`--refresh` — structure, not embedded shell.

Interfaces:

- Consumes: T2's toolchain (bun + pulumi on PATH).
- Produces: installable workspace (`bun install --cwd infra/pulumi
  --frozen-lockfile`); `stacks.ts` exporting `{ stacks: string[],
  previewArgv(stack): string[], upArgv(stack): string[] }`; `affected.ts`
  CLI: stdin/args `--base <sha> --head <sha>` → JSON `{"stacks":
  ["github", …]}` on stdout; all tests green under `infra:test`.

### T4 — github stack

`platform/github/` with `index.ts` + `Pulumi.yaml` (`name: zireael-github`,
nodejs/bun runtime like orion `platform/github/Pulumi.yaml:1-5`) +
`Pulumi.prod.yaml` (ESC env ref + `github:owner: mattwilkinsonn`). Declares:
the `mattwilkinsonn` repos under management (at minimum zireael itself + the
two extracted tool repos + this list grows), each import-adopted
(`import:` + `protect: true`), squash-merge policy, and a `main` ruleset per
repo reproducing today's checked-in `main-protection.json` semantics
(deletion/non-fast-forward/linear-history/PR-required/required status check —
`main-protection.json:11-41`) with the new `CI` check context. The required
status check on zireael is ONLY the stable `CI` rollup — the per-stack
pulumi preview contexts are never added to the required set (they live on a
`paths:`-filtered workflow and would wedge every non-infra PR; Approach
§ GHA deploy). The ruleset keeps Matt's admin bypass (`bypass_actors`
RepositoryRole 5, `main-protection.json:43-48`) for wedge recovery.

**Credential: a dedicated personal GitHub App** (Matt ruled — Open
Questions § 4). Modeled on orion's `rigelbuild-github-iac` App
(`platform/github/BOOTSTRAP.md:3-6` — machine identity, no PAT, no
webhook), created on Matt's personal account and installed **only on
`mattwilkinsonn`**; the provider mints a short-lived installation token
at run time from the App ID / installation ID / PEM exported by ESC env
`infra/github` (orion `platform/github/Pulumi.prod.yaml:3-9`: "it
exports GITHUB_APP_ID / GITHUB_APP_INSTALLATION_ID / GITHUB_APP_PEM_FILE
and the provider mints a short-lived installation token at run time (no
PAT)"). Two hardenings land with it:

- **Explicit `github.Provider` pin** — the stack constructs an explicit
  provider from the ESC-delivered credential and passes it to every
  resource; a stray ambient `GITHUB_TOKEN` on a GHA runner otherwise
  hijacks provider auth — orion paid for exactly this
  (`platform/github/index.ts:22-27`: "any ambient GITHUB_TOKEN in the
  runtime (the CI runner has one) hijacks auth").
- **No credential-expiry failure mode** — a point in the App's favor:
  installation tokens auto-rotate per run and the PEM itself never
  expires, so the annual-PAT-expiry silent-401-on-next-`up` hazard (which
  would have needed a scheduled canary preview) does not exist. The PEM
  is instead the real master key — "Contents: write + Administration lets
  a leaked key disable protection and push to `main` directly" (orion
  `platform/github/BOOTSTRAP.md:58-62` § keep-in-mind) — so it lives
  only in ESC, never on disk or in repo secrets.

`BOOTSTRAP.md` documents App creation (permissions, webhook off,
install-only-on-this-account), installation on `mattwilkinsonn`, PEM mint,
and ESC env creation — adapted from orion's
`platform/github/BOOTSTRAP.md`.

Interfaces:

- Consumes: T3 workspace; ESC env `infra/github` exporting the App ID /
  installation ID / PEM.
- Produces: stack `zireael-github/prod`; exported outputs: managed repo
  names. First preview: imports/no-ops only.

### T5 — cloudflare stack

`platform/cloudflare/` declaring Matt's existing zones and their DNS records,
every resource `import:`-adopted with `protect: true` on zones — orion's
`zones.ts` shape (`zones.ts:59-67`). Enumerate live zones/records via the
Cloudflare API at implementation time (the live inventory is the source of
truth; this record deliberately doesn't freeze a zone list). Token scoped to
Zone Read + DNS Edit (grow scopes only with managed surface — orion's scope
table, `platform/cloudflare/README.md:29-37`), delivered as
`CLOUDFLARE_API_TOKEN` via ESC env `infra/cloudflare`
(`Pulumi.prod.yaml:3-11` pattern).

**Apply posture: auto-up-on-merge, like github/aws** (Matt ruled — Open
Questions § 6). This matches orion's actual steady state: orion's
`pulumi:up-cloudflare` runs on `when: [WHEN.pushMain]` in `mode: "cd"`
(`ci/pipeline.ts:876-878`) — "preview on PR / up on push->main"
(`infra/pulumi/moon.yml:230-231`); the README's "**Matt runs `pulumi
up`**" (`platform/cloudflare/README.md:74-75`) is the one-time ADOPTION
apply only. Same split here: Matt runs the first adoption `up` manually
(a `BOOTSTRAP.md` step — the apply that pulls the imports into state),
after which the stack auto-ups on merge in the uniform T7 pipeline. The
safety nets stay, ruled compatible with auto-up:

- `protect: true` on zones (blocks deletes/replaces);
- `protect` **and** `ignoreChanges` on the load-bearing MX/SPF/DKIM
  records — `ignoreChanges` guards accidental content updates on the
  mail-bearing records, which `protect` alone does not;
- the `pulumi preview --expect-no-changes` CI acceptance gate for the
  initial adoption (T5 acceptance is mechanical, not human diff-reading).

Residual risk, accepted: a deliberate DNS content change on a
non-ignored record still auto-applies on merge after PR-preview review.
`BOOTSTRAP.md`: token mint + ESC env + the one-time manual adoption `up`.

Interfaces:

- Consumes: T3 workspace; ESC env `infra/cloudflare`.
- Produces: stack `zireael-cloudflare/prod`; outputs: zone IDs by name.
  First preview: imports/no-ops only (the T5 acceptance gate).

### T6 — aws baseline stack

`platform/aws/` for the empty account: ESC OIDC auth
(`environment: [infra/aws]`, region + `aws:defaultTags` config — orion
`platform/aws/Pulumi.prod.yaml:10-20` shape with `ManagedBy: Pulumi` tags).
Resources: **NONE — zero standing resources** (Matt ruled — Open
Questions § 7). The stack proves the keyless OIDC chain end-to-end by
exporting `aws.getCallerIdentity` (account id + caller ARN) — a
data-source read, nothing persisted in the account; no demo bucket, no
VPC, no workloads until a workload record lands. This also moots the
Renovate-fan-out concern OQ7 raised: an unchanged aws stack `up`s to a
no-op with no resource churn. `BOOTSTRAP.md`: IAM OIDC provider + role
for ESC (out-of-band, the one AWS console/CLI bootstrap).

Interfaces:

- Consumes: T3 workspace; ESC env `infra/aws` (assumes the bootstrap IAM
  role).
- Produces: stack `zireael-aws/prod`; outputs: `accountId`, `callerArn`,
  `region`. Preview/up green end-to-end keylessly; zero resources in
  state.

### T7 — GHA deploy workflow

`.github/workflows/infra-deploy.yml`: triggers `pull_request` and
`push: branches: [main]`, both with `paths: [infra/pulumi/**,
.github/workflows/infra-deploy.yml]`. Stack names + preview/up argv
templates come from `infra/pulumi/stacks.ts` (the same module the
refresh-gate test parses — no argv embedded in workflow shell). PR path:
job `detect` (checkout fetch-depth 0, `affected.ts` with base/head from
the event) feeds a matrix `preview` job (`if: stacks non-empty` AND the
MANDATORY same-repo gate
`github.event.pull_request.head.repo.full_name == github.repository`):
devenv or direct bun+pulumi setup, `pulumi/auth-actions` OIDC login (issuer
policy sub `repo:mattwilkinsonn/zireael:pull_request`),
`bun install --cwd infra/pulumi --frozen-lockfile`,
`pulumi install --no-dependencies -C platform/<stack>`, then
`pulumi preview -C platform/<stack> --stack mattwilkinsonn/prod --refresh
--diff --non-interactive` posted to the PR (job summary or comment). Push
path: NO affected-detection — a matrix `up` over ALL stacks (issuer policy
sub `repo:mattwilkinsonn/zireael:ref:refs/heads/main`), `pulumi up …
--refresh --yes --non-interactive`, so a pending-run cancellation can never
silently drop a middle merge's apply. Both job kinds share the per-stack
`concurrency: infra-<stack>` group — up with no `cancel-in-progress`,
preview with `cancel-in-progress: true`. `id-token: write` permission; ESC
envs carry provider creds so the workflow holds no cloud secrets. The
per-stack preview contexts stay NON-required informational checks; the
ruleset's required check remains the stable `CI` rollup only (T4).

Interfaces:

- Consumes: T3's `stacks.ts` + `affected.ts` + refresh-gate test; T4-T6
  stacks.
- Produces: on-PR preview surfaces per affected stack; on-merge applies to
  ALL stacks. Acceptance: a PR touching only `platform/aws/**` previews aws
  alone; an unrelated docs PR triggers no infra job and still merges (the
  required `CI` rollup reports regardless); a merge to main `up`s every
  stack.

### T8 — flip zireael private (LAST)

After T7 has proven at least one full preview→merge→up cycle: flip
`visibility` on the zireael repo resource in the T4 github stack from
`public` to `private` — via a PR to the stack itself, applied by the T7
pipeline (infra manages its own home; no console click,
rule://no-human-clicks). Verify GHA minutes billing post-flip (private-repo
minutes draw from the Pro quota).

Interfaces:

- Consumes: proven T7 pipeline; T4's zireael repo resource.
- Produces: zireael private; CI still green on the next PR.

## Tasks

- [ ] T1: gut Rust/monorepo machinery (file list in Plan § T1)
- [ ] T2: retool devenv + repo gate, moon/proto-free
- [ ] T3: infra/pulumi bun workspace + Pulumi Cloud OIDC + affected/refresh-gate scripts
- [ ] T4: github stack (repos + rulesets, import-adopted)
- [ ] T5: cloudflare stack (zones + DNS, import-adopted)
- [ ] T6: aws baseline stack (ESC OIDC, zero standing resources, getCallerIdentity proof)
- [ ] T7: GHA infra-deploy workflow (preview-on-PR / up-on-merge, per-affected-stack, --refresh)
- [ ] T8: flip zireael private (last, after a proven deploy cycle)

## Open Questions

1. **Infra task runner: devenv tasks + path-diff script vs a minimal moon kept
   only for affected-detection.** [NON-LOAD-BEARING — designed against the
   recommendation] Recommendation (designed in above): devenv tasks + the
   `affected.ts` path-diff script. Moon's affected graph is overkill for a
   3-stack tree whose dependency shape is "stack dir → stack; workspace files
   → all"; keeping moon contradicts the repo-wide drop and re-imports proto or
   another install channel. The script is ~50 lines with a test. Reverse only
   if Matt wants moon's task graph back.
2. **Cloudflare/GitHub adoption mechanics: per-resource `import:` IDs
   (declare-and-import in code) vs `pulumi import` CLI runs.**
   [NON-LOAD-BEARING —
   designed against orion precedent] Designed: code-level `import:` (orion
   cloudflare/github precedent; reviewable, no state side-channel). Existing
   domains and repos MUST be adopted, never recreated, either way — that part
   is a constraint, not a question. The residual choice (drop `import:` lines
   after first apply, as orion's README permits) is cleanup taste.
3. **github-stack self-lockout.** [RESOLVED — folded into the design] The T4
   stack manages the ruleset on zireael's own `main` — the branch the T7
   pipeline merges to. Both halves are now designed in rather than open:
   (a) the required-check wedge was not a bad-apply tail risk but
   DETERMINISTIC as first drafted — a required per-stack preview context on
   a `paths:`-filtered workflow never reports on a docs-only PR and blocks
   every such merge forever, making the admin bypass the routine merge
   path. Fixed: the required check is ONLY the stable `CI` rollup; the
   per-stack pulumi contexts are non-required (Approach § GHA deploy, T4,
   T7). Matt's admin bypass (`bypass_actors` RepositoryRole 5,
   `main-protection.json:43-48`) is KEPT in the T4 ruleset for wedge
   recovery. (b) the credential boundary is the Pulumi OIDC issuer
   AUTHORIZATION POLICY, not a per-step withhold: the issuer pins explicit
   sub-claim policies (main-ref for up, `pull_request` for preview — never
   the wildcard `repo:<owner>/<repo>:*`), and the same-repo preview gate
   (`if: github.event.pull_request.head.repo.full_name ==
   github.repository`) is MANDATORY, not optional belt-and-braces — on the
   individual Pulumi tier the minted token is a personal token with Matt's
   full permissions across every stack + ESC env, and the repo stays public
   until T8.
4. **GitHub provider credential: fine-grained PAT vs personal GitHub App.**
   [RESOLVED — Matt ruled: personal GitHub App, overriding the PAT
   recommendation] A dedicated personal App (orion's model,
   `platform/github/BOOTSTRAP.md:3-6` — no PAT, short-lived installation
   tokens), installed only on `mattwilkinsonn`, PEM delivered via ESC env
   `infra/github`. Rationale: no long-lived credential to rotate —
   installation tokens auto-rotate and the PEM never expires, deleting
   the annual-PAT-expiry silent-401 failure mode (the canary the PAT
   design needed) — and the model is orion-proven. Blast radius is
   unchanged either way: an App PEM with Administration + Contents write
   is an account-master key exactly as the PAT would have been
   ("Contents: write + Administration lets a leaked key disable
   protection and push to main directly", orion
   `platform/github/BOOTSTRAP.md:58-62`), so the PEM lives only in ESC.
   T4 carries the explicit `github.Provider` pin regardless. Folded into
   Approach § Auth and T4.
5. **Pulumi Cloud org shape: personal account default org vs a named org.**
   [NON-LOAD-BEARING] Designed against: personal default org
   (`mattwilkinsonn`), free tier. ESC environments and OIDC issuers are
   available on the individual tier; if any ESC feature turns out gated at
   apply time, the fallback is GHA-native OIDC per provider (AWS
   `configure-aws-credentials`, Cloudflare token as a GHA secret) — noted so
   the executor isn't surprised, but not expected to bite.
6. **Cloudflare live-DNS apply posture.** [RESOLVED — Matt ruled:
   auto-up-on-merge like github/aws, reversing the interim manual-up
   recommendation] Ground: orion ALSO auto-ups this stack in steady
   state — `pulumi:up-cloudflare` is `mode: "cd"`, `when:
   [WHEN.pushMain]` (orion `ci/pipeline.ts:876-878`; "preview on PR / up
   on push->main", `moon.yml:230-231`); the README's "Matt runs `pulumi
   up`" (`platform/cloudflare/README.md:74-75`) covers only the one-time
   adoption apply. Resolved shape: uniform auto-up across all three
   stacks; the one-time adoption `up` is manual (BOOTSTRAP.md step); the
   safety nets are kept — `protect: true` on zones,
   `protect`/`ignoreChanges` on the MX/SPF/DKIM records (an update to a
   mail-bearing record is otherwise unblocked by `protect`), and the
   `pulumi preview --expect-no-changes` adoption acceptance gate.
   Residual risk accepted and recorded: a deliberate content change on a
   non-ignored DNS record auto-applies on merge after PR-preview review.
   Folded into T5.
7. **T6 empty-AWS stack: build now vs defer.** [RESOLVED — Matt ruled:
   build now with ZERO standing resources (option b, the
   recommendation)] T6 stands up `platform/aws` with ESC-OIDC auth and
   exports `aws.getCallerIdentity` (account id / caller ARN) instead of
   a demo bucket — the same end-to-end keyless-chain proof with nothing
   persisted in the account. The Renovate-fan-out concern is moot: an
   unchanged aws stack `up`s to a no-op with no resource churn. Folded
   into T6 (outputs: `accountId`, `callerArn`, `region`; `demoBucket`
   dropped).

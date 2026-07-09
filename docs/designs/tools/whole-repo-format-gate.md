# Design: whole-repo monorepo tooling (one root biome, all file types, unified deps)

Status: **accepted**
Domain: tools / build-tooling

## Problem / Intent

zireael's monorepo tooling is scattered on two axes:

- **Format/lint gating.** 8 `biome.json` files (an inert root marker + 7 per-tool
  configs) and 8 separate biome invocations (7 per-project `lint` tasks + the
  root `agents-biome` task), each scoped to its own directory. Whole file-type
  classes — non-workflow `.yml`/`.yaml`, `.py`, `.kdl`, root-level TOML, most
  `.json` — have **no** gate at all. That scatter is how
  `nix-config/agents/extensions` drifted uncaught (the SEA-1128 class of bug,
  patched narrowly by #246/#248).
- **Dependencies.** There is **no root bun workspace**: the 7 bun tools each
  carry their own `bun.lock` and duplicate `typescript`/`@types/bun`/biome pins
  that drift. The 7 `tsconfig.json` are near-identical copies. (The root **cargo**
  workspace already exists and is sealed-pattern — a shared `[workspace.dependencies]`
  catalog consumed via `{ workspace = true }` — so that axis is essentially done.)

Matt's directive (expanded after the initial one-root-biome review): "all file
types with linters/formatters, and a documented reason of why we can't for any
that we don't … unify the tsconfig, and generally unify all deps, same pattern as
sealed with a root bun workspace and a root cargo workspace … land a full design
… and any other recs for making our monorepo tooling the best it can be."

This record designs that full consolidation, modeled on `sealedsecurity/sealed`:
one root `/biome.json` + one `biome check .` gate; a root **bun workspace**
(catalog-pinned shared deps, one lockfile); a unified **tsconfig** base; a gate
for **every** committed file type (or a documented reason it can't be gated); and
the supporting monorepo-tooling recommendations. The root cargo workspace is
already in place and is cited as satisfying that half of the directive.

## Global Constraints

- **Design-for**: biome **2.4.16** (schema + pinned CLI; sealed pins the same —
  `sealed/package.json:15` `"@biomejs/biome": "2.4.16"` in the workspace catalog).
  Toolchain floors from `.prototools:6-8`: `bun = "1.3.14"`, `node = "24.18.0"`,
  `moon = "2.3.5"`.
- **indentStyle: tab** (Decision, confirmed by Matt). Both existing full biome
  configs already declare it (`tools/akiflow-cli/biome.json:8`,
  `sealed/biome.json:18` — `"formatter": { "indentStyle": "tab" }`), and the
  verified migration run reformats **zero** `.ts` files under tab. Any root
  `.editorconfig` (Task T9) MUST align `indent_style = tab` for biome-covered
  types, since biome reads `.editorconfig` by default and a `space` setting would
  fight the biome config.
- **Shared deps are pinned once, in a catalog.** New rule after this design: bun
  deps shared across tools (`@biomejs/biome`, `typescript`, `@types/bun`) live in
  the root `package.json` `workspaces.catalog` and each tool consumes `"catalog:"`;
  Rust deps stay in the existing root `Cargo.toml` `[workspace.dependencies]`.
  Single source of truth per ecosystem.
- **No lint-rule drift.** Default biome recommended rules, no new suppressions;
  the 116 pre-existing akiflow warnings stay warnings (exit 0), exactly as they
  pass today. New per-language linters (yamllint, ruff, kdlfmt) land with their
  one-time format pass isolated in their own task/PR — zero logic change.
- **The gate stays `moon ci`.** Local pre-push (`hk.pkl:30` —
  `check = "moon ci"`) and CI (`.github/workflows/ci.yml:90` —
  `run: devenv shell -- moon ci`) both run the same command; this design only
  re-homes and adds tasks under it.
- **New tools arrive via devenv.** yamlfmt/yamllint/ruff/kdlfmt are added to the
  devenv packages block (`devenv.nix:29-42`), the same PATH mechanism every
  non-bun linter already uses (biome is the sole exception — it comes via bunx).
- **Each plan task lands as its own green PR** (gt stack, commit as Matt with the
  `Co-Authored-By: seal` trailer, never merge — `rule://commit-conventions`).
  This record itself ships as a docs-only PR first (`skill://design`).
- **Markdownlint-clean docs** under the repo config (`.markdownlint-cli2.jsonc`:
  MD013 off, MD060 compact).

## Evidence base (cited this session)

Clones: zireael at `~/agents/workspaces/frobisher/zireael`, sealed at
`~/agents/workspaces/frobisher/sealed`. All file:line references below were read
this session; all command outputs were run this session.

### zireael current state — biome/lint scatter

- Root `biome.json:1-4` is inert — the whole file is:

  ```json
  {
   "$schema": "https://biomejs.dev/schemas/2.4.15/schema.json",
   "root": true
  }
  ```

- 8 `biome.json` total (glob-verified): the root one plus
  `nix-config/tools/{gh-route,no-bash-gate,wait-for-reviews}/biome.json` and
  `tools/{akiflow-cli,install-debug,release,setup-live-test-fixture}/biome.json`.
  Six of the seven are three-line stubs — `gh-route/biome.json:1-3`:

  ```json
  {
   "extends": "//"
  }
  ```

  akiflow-cli's is the one full config (`tools/akiflow-cli/biome.json:1-17`):
  `"root": false`, `"includes": ["**", "!.sisyphus", "!dist", "!**/node_modules"]`
  (line 5), `"formatter": { "indentStyle": "tab" }` (line 8), html formatter on
  (line 7), organizeImports on (lines 10-16).
- Each bun tool project defines its own biome `lint` task —
  `nix-config/tools/gh-route/moon.yml:15-18`:

  ```yaml
  lint:
    command: 'bunx biome check .'
    deps: ['install']
    inputs: ['*.ts', 'biome.json', 'package.json']
  ```

  Identical blocks at `nix-config/tools/no-bash-gate/moon.yml:15-18`,
  `nix-config/tools/wait-for-reviews/moon.yml:15-18`,
  `tools/install-debug/moon.yml:15-18`, `tools/release/moon.yml:15-18`,
  `tools/setup-live-test-fixture/moon.yml:15-18`; akiflow's variant at
  `tools/akiflow-cli/moon.yml:16-19` with
  `inputs: ['src/**/*', 'biome.json', 'package.json']`.
- Each tool's `ci` aggregate names `lint` — `tools/akiflow-cli/moon.yml:30-31`:
  `ci:` / `deps: ['lint', 'typecheck', 'test']` (same at
  `install-debug/moon.yml:29-30`, `release/moon.yml:29-30`,
  `setup-live-test-fixture/moon.yml:29-30`, `gh-route/moon.yml:29-30`,
  `wait-for-reviews/moon.yml:29-30`; no-bash-gate adds its scanner:
  `no-bash-gate/moon.yml:40-41` `deps: ['lint', 'typecheck', 'test', 'gate']`).
- Root `moon.yml:23-25` carries the narrow per-dir gate this design replaces:

  ```yaml
  agents-biome:
    command: 'bunx biome check nix-config/agents/extensions'
    inputs: ['/nix-config/agents/extensions/**/*.ts', '/biome.json']
  ```

  and `moon.yml:34-35` aggregates
  `ci:` / `deps: ['markdownlint', 'actionlint', 'agents-biome', 'nixfmt', 'deadnix']`.
- Root `moon.yml:12-17` also owns `markdownlint`
  (`command: 'markdownlint-cli2 "**/*.md"'`) and `actionlint`
  (`inputs: ['/.github/workflows/*.yml', '/.github/workflows/*.yaml']`);
  `moon.yml:28-33` gates root-level `*.nix` via maxdepth-1 `nixfmt`/`deadnix`.
- `nix-config/moon.yml` owns the nix-tree linters — nix-only at lines 17-29
  (`nixfmt`, `deadnix`, `statix`, `nil`), plus shell/TOML scoped **only to
  nix-config/**: `shellcheck` (:30-32,
  `script: "find . -name '*.sh' -print0 | xargs -0 shellcheck --external-sources"`),
  `shfmt` (:33-35), `taplo` (:36-40,
  `script: "find . -name '*.toml' -print0 | xargs -0 taplo fmt --check"`), file
  groups at :10-13, `ci` aggregate at :57-69.
- Rust format/lint is per-crate cargo — `tools/jj-hooks/moon.yml:11-15`:
  `fmt:` / `command: 'cargo fmt -p jj-hooks -- --check'`; Formula linting is
  `Formula/moon.yml:10-19` `brew-style:` / `brew style ./*.rb`.
- CI is GitHub Actions, not sealed's Woodpecker: `.github/workflows/ci.yml:74-76`
  sets `MOON_BASE: ${{ github.event.pull_request.base.sha }}` /
  `MOON_HEAD: ${{ github.event.pull_request.head.sha }}`, `ci.yml:80`
  `fetch-depth: 0 # moon affected-detection needs base + head history`,
  `ci.yml:89-90` runs `devenv shell -- moon ci`.
- Generated trees are gitignored: `.gitignore:2-3` `/target` +
  `/tools/*/target`, `:8` `node_modules/`, `:10` `dist/`, `:29-30` `.devenv*` +
  `.direnv/`, `:35-36` `.moon/cache/` + `.moon/docker/`. `.jj/` and `.sisyphus`
  are **not** in `.gitignore`; the markdownlint config already special-cases
  `.jj` (`.markdownlint-cli2.jsonc:28-36` ignores include `"**/.jj/**"`,
  `"**/.devenv/**"`).
- devenv provides every non-bun linter on PATH (`devenv.nix:29-42`:
  `nixfmt-rfc-style`, `deadnix`, `statix`, `nil`, `shellcheck`, `shfmt`,
  `taplo`, `actionlint`, `markdownlint-cli2`) — **no biome package**; biome
  arrives via bunx/devDeps.

### zireael current state — dependency scatter (the new scope)

- **No root `package.json` / `bun.lock`** (ls-verified absent). Each of the 7
  bun tools owns its own lockfile — `.moon/workspace.yml:26-31` comments the
  projects: "bun/TypeScript dev + release tooling (own bun.lock each)" and the
  nix-config bun tools "(own bun.lock each)". The 7 bun projects (from the
  `.moon/workspace.yml` `projects` map): `tools/akiflow-cli`, `tools/release`,
  `tools/install-debug`, `tools/setup-live-test-fixture`,
  `nix-config/tools/gh-route`, `nix-config/tools/no-bash-gate`,
  `nix-config/tools/wait-for-reviews`.
- Shared bun deps drift across tools (from their `package.json`): `@biomejs/biome`
  is declared only by akiflow (`tools/akiflow-cli/package.json:38` `"^2.4.13"`);
  `typescript` is `"^5"` in akiflow/release/gh-route; `@types/bun` is `"^1.3.8"`
  in akiflow but `"latest"` in release/gh-route. akiflow's runtime deps
  (`chrono-node ^2.9.0`, `citty ^0.2.0`, `rrule ^2.8.1`) are tool-specific — NOT
  catalog candidates.
- zireael projects carry **no moon `tags`** today (verified — no `tags:` key in
  any tool `moon.yml`); sealed uses `tags: ['bun']` to inherit shared install
  tasks. Adding tags is the mechanism for the workspace consolidation.
- **7 `tsconfig.json`** (git-verified): the 3 `nix-config/tools/*` + 4 `tools/*`.
  sha1-verified this session: SIX are byte-identical (gh-route, no-bash-gate,
  wait-for-reviews, install-debug, release, setup-live-test-fixture — sha1
  `f79fefcf…`); akiflow-cli diverges (`93b3c86e…`). The common content
  (`tools/release/tsconfig.json`, verbatim):

  ```json
  {
   "compilerOptions": {
    "lib": ["ESNext"], "target": "ESNext", "module": "Preserve",
    "moduleDetection": "force", "allowJs": true, "moduleResolution": "bundler",
    "allowImportingTsExtensions": true, "verbatimModuleSyntax": true,
    "noEmit": true, "strict": true, "skipLibCheck": true,
    "noFallthroughCasesInSwitch": true, "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true, "types": ["bun"]
   }
  }
  ```

  akiflow's 4 divergences (`tools/akiflow-cli/tsconfig.json`): `"jsx": "react-jsx"`
  (:7), relaxes `noUnusedLocals`/`noUnusedParameters`/
  `noPropertyAccessFromIndexSignature` (:18-20), `"types": ["bun", "node"]` (:21)
  vs the common `"types": ["bun"]`.

### zireael current state — root cargo workspace (already sealed-pattern)

- Root `Cargo.toml` ALREADY is a workspace: `[workspace]` `resolver = "3"`,
  members `tools/jj-hooks`, `tools/jj-gt`; `[workspace.package]` (version
  `0.3.7`, edition, license, repository, authors); a full
  `[workspace.dependencies]` catalog (15 deps — anyhow, clap, clap_complete,
  dialoguer, serde, serde_json, thiserror, toml, tracing, … ); the internal
  path-dep `jj-hooks = { version = "0.3.7", path = "tools/jj-hooks" }`;
  `[profile.release]` `lto = "thin"` / `strip = "symbols"`.
- Both crates consume via workspace inheritance: `tools/jj-hooks/Cargo.toml`
  uses `version.workspace = true`, `edition.workspace = true`, and every dep as
  `{ workspace = true }`; `tools/jj-gt/Cargo.toml` the same, plus
  `jj-hooks = { workspace = true }` for the internal dep.
- `rust-toolchain.toml` pins `channel = "1.96.0"`,
  `components = ["rustfmt", "clippy"]` (single toolchain source for fenix + CI).
- Per-crate clippy already runs strict: `tools/jj-hooks/moon.yml:15`
  `command: 'cargo clippy -p jj-hooks --all-targets -- -D warnings'` (same in
  jj-gt). So the cargo half of Matt's "root cargo workspace" directive is
  **already satisfied**; the only sealed delta is centralized workspace lints
  (Decision: centralize — Task T10).

### The stack-overflow risk (reproduced this session)

With today's inert committed config, a root run crashes the biome worker
deterministically (1/1 this session; the prior recon observed the same 134
plus intermittence across pulls):

```text
$ bunx biome check --max-diagnostics=0 .   # committed biome.json (inert)
thread 'biome::workspace_worker_0' (702379) has overflowed its stack
fatal runtime error: stack overflow, aborting
EXIT: 134
```

Root cause: with no `vcs`/`files` config at all, biome descends into gitignored
generated trees — `.devenv*` symlinks into the nix store (`.gitignore:29`),
`node_modules/`, `target/` — and the walk blows the worker stack. Two verified
mitigations this session:

- `vcs.useIgnoreFile: true` alone (no `files.includes`): three consecutive runs
  exited 1 (diagnostics found, **no crash**).
- The full candidate config below (useIgnoreFile **plus** explicit negations):
  `Checked 132 files in 34ms.` — no crash, and the negations additionally cover
  scratch dirs that are *not* gitignored (`.jj/`, `.sisyphus` — akiflow's own
  config already negates `.sisyphus`, `tools/akiflow-cli/biome.json:5`), so the
  walk stays bounded even where gitignore state is absent or stale (fresh
  worktrees, the prior run's observed post-pull intermittence).

Belt and braces — both layers — is the design.

### sealed reference pattern — biome

- `sealed/biome.json:2` — `"vcs": { "enabled": true, "clientKind": "git", "useIgnoreFile": true }`.
- `sealed/biome.json:4-11` — `"includes": ["**", "!oss/petrel/fixtures", "!oss/seal/vendor", "!oss/seal/schemas", "!oss/seal/.github", "!tools/docs-publish/outputs"]`.
- `sealed/biome.json:18` — `"formatter": { "indentStyle": "tab" }`; `:19`
  `"linter": { "enabled": true }`; `:20-26` assist `"organizeImports": "on"`;
  `:27-49` overrides for special dirs (deck CSS `noImportantStyles` off, two
  vendored HTML files formatter+linter off).
- `sealed/moon.yml:21-23` — the root project excludes inherited tasks:
  `workspace:` / `inheritedTasks:` / `exclude: ['install', 'lint', 'format']`.
- `sealed/moon.yml:42-48` — the whole-repo gate:

  ```yaml
  lint:
    command: 'bunx biome check .'
    deps: ['install']
    inputs:
      - '**/*.{ts,tsx,js,jsx,cjs,mjs,json,jsonc,css,html}'
      - '!**/node_modules/**'
      - '/biome.json'
  ```

- `sealed/moon.yml:52-56` — write-mode convenience, never a gate:
  `format:` / `command: 'bunx biome check --write .'` /
  `options:` / `runInCI: false`.
- `sealed/moon.yml:65-77` — whole-repo `shfmt`/`shellcheck`/`taplo` as root
  `script:` tasks, e.g. `:73-75`
  `script: "find . -name node_modules -prune -o -name dist -prune -o -name out -prune -o -name outputs -prune -o -name .direnv -prune -o -name .devenv -prune -o -name .moon -prune -o -name vendor -prune -o -name '*.toml' -print0 | xargs -0 taplo fmt --check"`.
- `sealed/moon.yml:11-14` states the model: "Whole-repo lint/format — one task
  per tool, run from the workspace root: biome (TS/JS/JSON/CSS/HTML, via the
  single /biome.json), markdownlint, shfmt (shell format), shellcheck (shell
  lint), taplo (TOML). The nix-only linters (nixfmt/deadnix/statix/nil) live in
  the `nix` project."
- sealed keeps exactly one nested biome config, and only because that subtree
  needs *different rules*: `sealed/oss/compass/biome.json:3-8` (`"root": false`
  plus a `noRestrictedImports` fence) — never for per-project gating.

### sealed reference pattern — bun workspace + catalog (the model to copy)

- `sealed/package.json` is the root workspace manifest:

  ```json
  {
   "name": "@sealed/monorepo",
   "private": true,
   "type": "module",
   "workspaces": {
    "packages": ["apps/sealedsecurity.com", "oss/*", "..."],
    "catalog": {
     "@biomejs/biome": "2.4.16",
     "@types/bun": "^1.3.14",
     "typescript": "^6.0.3"
    }
   },
   "devDependencies": {
    "dependency-cruiser": "^18.0.0",
    "markdownlint-cli2": "^0.22.1"
   }
  }
  ```

- Leaf packages consume the catalog by protocol, not version —
  `sealed/tools/docs-publish/package.json:13-14`:
  `"@biomejs/biome": "catalog:"`, `"@types/bun": "catalog:"` (a tool-specific
  `"typescript": "^6.0.3"` stays inline where it isn't catalog-shared). Same
  shape at `sealed/apps/sealedsecurity.com/package.json:16-17`,
  `sealed/assets/branding/package.json:16-17`.
- `sealed/.moon/tasks/tag-bun.yml` — the shared install inherited by every
  `bun`-tagged project:

  ```yaml
  tasks:
    install:
      command: 'noop'
      deps: ['root:install']
      options:
        internal: true
  ```

- The real install lives on the root project (`sealed/moon.yml:26-36`):
  `install:` / `command: 'bun install --frozen-lockfile'` /
  `inputs: ['/package.json', '/bun.lock', '/bunfig.toml']` /
  `options: { internal: true, cache: false, runFromWorkspaceRoot: true }`.
  In-comment rationale (SEA-1067): N per-project installs race on linking the
  shared hoisted `node_modules/` (EEXIST) in a cold worktree; one root install
  fixes it.
- Leaf bun projects opt in with `tags: ['bun']` (e.g.
  `sealed/tools/docs-publish/moon.yml:7`), which is what pulls in the no-op
  install above.
- `sealed/bunfig.toml` — supply-chain cooldown at the workspace root:

  ```toml
  [install]
  minimumReleaseAge = 432000
  ```

  (5-day cooldown, mirrors `.github/dependabot.yml`; Bun reads bunfig from the
  install dir, so it lives beside the root lockfile.)
- What does **not** transfer: sealed's Woodpecker CI generator — zireael's
  affected detection is `moon ci` + `MOON_BASE`/`MOON_HEAD` in GitHub Actions
  (`ci.yml:74-90`), per the earlier record
  `docs/designs/platform/moon-ci-gate.md:17-19` ("sealed's remote CI is
  Woodpecker … does **not** transfer").

### sealed reference pattern — cargo lints + editorconfig

- `sealed/Cargo.toml` adds centralized workspace lints:
  `[workspace.lints.clippy]` / `disallowed_methods = "deny"`, and seal crates
  opt in via `[lints] workspace = true` (with the disallowed-methods list in
  `oss/seal/clippy.toml`). zireael has no `[workspace.lints]` block and no
  `clippy.toml` — the one cargo-workspace delta (Decision: centralize — Task T10).
- `.editorconfig` has in-house precedent at `sealed/oss/seal/.editorconfig`
  (`root = true`; `[*]` `end_of_line = lf`, `insert_final_newline = true`,
  `charset = utf-8`, `trim_trailing_whitespace = true`, `indent_style = space`,
  `indent_size = 4`; per-type `indent_size = 2` for toml/yml/json/md/ts/…). Note
  it sits under `oss/seal/`, NOT sealed root, so sealed's root biome never reads
  it — that is why sealed can run `indentStyle: tab` in biome and `indent_style
  = space` in that editorconfig without conflict. zireael's root `.editorconfig`
  (Task T9) must therefore use `indent_style = tab` for biome-covered types.

### File-type census + tool-capability facts (verified this session)

- Census: `git ls-files | sed -E 's|.*/||; s|.*\.|.|' | sort | uniq -c | sort -rn`
  (counts in the matrix below).
- biome 2.4/2.5 language support (web-verified —
  <https://biomejs.dev/blog/roadmap-2026/>): TS/JS/JSON/JSONC stable; CSS stable;
  HTML experimental; **YAML NOT supported** (parser "almost ready", 2026
  roadmap); standalone GraphQL not stable. So YAML **cannot** be a biome job —
  it needs a dedicated tool.
- New tools are ALL present in nixpkgs (this session, `nix eval` on
  `nixpkgs#<pkg>.version`): `yamlfmt` `0.21.0`, `yamllint` `1.37.1`, `ruff`
  `0.15.20`, `kdlfmt` `0.1.7`.
- shfmt auto-detects shell scripts by shebang when recursing (so extension-less
  shell dotfiles with a `#!/…/bash` shebang are covered); shellcheck needs a
  `find`-driven invocation and deduces the dialect from shebang/`# shellcheck
  shell=` directive. Neither supports the **zsh** dialect (sh/bash/mksh only).
- The 13 committed extension-less files (git-verified): `.github/CODEOWNERS`,
  `LICENSE-APACHE`, `LICENSE-MIT`, `tools/akiflow-cli/LICENSE` (licences/owners —
  not code), `nix-config/dotfiles/ghostty/config` +
  `themes/NightOwl{Dark,Light}` (ghostty's own config format — no formatter), and
  shell dotfiles `nix-config/dotfiles/bash/bashrc`, `bash/profile`, `zsh/zshrc`,
  `skhd/skhdrc`, `yabai/yabairc`, `scripts/jj-ws` (bashrc/profile are bash;
  zshrc is zsh; skhdrc/yabairc are tool config DSLs, not shell).

### End-state verification (this session, throwaway clone)

In a fresh `git clone` of zireael with the candidate root `/biome.json` written
and the 7 per-tool `biome.json` deleted:

```text
write exit: 0        # bunx biome check --write --max-diagnostics=0 .
recheck exit: 0      # bunx biome check --max-diagnostics=0 .
Checked 125 files in 27ms. No fixes applied.
Found 116 warnings.
```

The one-time `--write` reformatted exactly 7 files (6 pre-existing + the new
`biome.json` itself): `.github/rulesets/main-protection.json`,
`nix-config/.markdownlint.jsonc`, `nix-config/agents/.markdownlint.jsonc`,
`nix-config/agents/cotal/channels.json`,
`nix-config/agents/cotal/service-map.json`,
`nix-config/dotfiles/markdownlint/markdownlint.jsonc` — zero `.ts` files, so
the tab decision causes no code churn. The 116 warnings are all pre-existing
akiflow-cli `lint/style/noNonNullAssertion` (82) +
`lint/suspicious/noExplicitAny` (34); warnings don't fail biome (akiflow's own
`bunx biome check .` exits 0 today with the same findings, verified).

Upward config discovery: with `tools/release/biome.json` removed and the
candidate root config in place, `bunx biome check .` from `tools/release/`
reported `Checked 4 files in 6ms.` exit 0 — nested configs are not needed for
editor-LSP or ad-hoc per-dir runs.

Affected-detection: `moon query tasks --affected` (this session, with one
untracked `.md` present) returned `root:markdownlint` with
`"files": ["docs/designs/tools/whole-repo-format-gate.md"]` — moon matches
changed files against task `inputs`, so a root `lint` task with sealed's input
globs runs when any TS/JS/JSON/JSONC file changes anywhere, and skips on
rust-only or nix-only PRs. The same per-task input mechanism gives each new
per-language gate (yaml/py/kdl) precise affected-scoping.

Version pin sanity: `bunx @biomejs/biome@2.4.16 --version` → `Version: 2.4.16`
(and leaves nothing in cwd). Root `Cargo.toml` fails `taplo fmt --check` today
(`ERROR taplo:format_files: the file is not properly formatted path=…/Cargo.toml`)
— the ungated-root-TOML gap Task T5 closes.

## Approach

Adopt sealed's model wholesale, adapted for zireael's GitHub Actions CI. The end
state:

1. **One root `/biome.json`** (replacing the inert marker) with
   `vcs.useIgnoreFile` **plus** explicit `files.includes` negations — the
   verified two-layer fix for the stack overflow. Exact content (verified green
   above; schema bumped to 2.4.16):

   ```json
   {
    "$schema": "https://biomejs.dev/schemas/2.4.16/schema.json",
    "root": true,
    "vcs": { "enabled": true, "clientKind": "git", "useIgnoreFile": true },
    "files": {
     "includes": [
      "**",
      "!**/node_modules",
      "!**/target",
      "!**/dist",
      "!.devenv",
      "!.direnv",
      "!.moon/cache",
      "!.moon/docker",
      "!**/.jj",
      "!**/.sisyphus"
     ]
    },
    "formatter": { "indentStyle": "tab" },
    "linter": { "enabled": true },
    "assist": { "actions": { "source": { "organizeImports": "on" } } },
    "overrides": [
     {
      "includes": ["nix-config/dotfiles/zed/settings.json"],
      "json": { "parser": { "allowComments": true, "allowTrailingCommas": true } }
     }
    ]
   }
   ```

   Negation rationale: `node_modules`/`target`/`dist`/`.devenv`/`.direnv`/
   `.moon/cache`/`.moon/docker` mirror `.gitignore:2-36` (defense in depth over
   useIgnoreFile); `.jj`/`.sisyphus` are **not** gitignored and follow the
   precedents at `.markdownlint-cli2.jsonc:33` and
   `tools/akiflow-cli/biome.json:5`. The zed-settings override exists because
   that file is JSON-with-comments under a `.json` extension (verified parse
   error without it; `.jsonc` files need no override). sealed's `html` block is
   omitted: zireael has zero committed `.css`/`.html` (census below); add it
   with the first HTML file.
2. **Root bun workspace.** A root `/package.json` (`"private": true`,
   `"type": "module"`) with `workspaces.packages` listing the 7 tool dirs and a
   `workspaces.catalog` pinning `@biomejs/biome` (2.4.16), `typescript`,
   `@types/bun`; one root `/bun.lock`; a root `/bunfig.toml` (supply-chain
   cooldown). A `.moon/tasks/tag-bun.yml` defines the no-op `install` →
   `root:install`; each bun project gains `tags: ['bun']` and drops its own
   `install`; each leaf `package.json` converts the shared deps to `"catalog:"`
   and deletes its per-tool `bun.lock`. This makes the biome (and TS) version a
   single catalog entry — the clean answer to how the root gate is pinned.
3. **Root `lint` + `format` moon tasks** on the root project (`moon.yml`), with
   sealed's input globs so moon's affected detection schedules them precisely;
   delete all 7 per-tool `biome.json` + their 7 `lint` tasks + the root
   `agents-biome` task. One biome invocation replaces eight. Upward discovery
   keeps in-dir `bunx biome` runs and editor LSP working (verified).
4. **Unified tsconfig.** A root `/tsconfig.base.json` holding the common
   compiler options; each tool's `tsconfig.json` becomes
   `{ "extends": "<rel>/tsconfig.base.json" }` (relative depth `../../` for
   `tools/*`, `../../../` for `nix-config/tools/*`); akiflow keeps its 4
   divergent options as local overrides atop the extend.
5. **Every file type gated (or documented why not).** Move
   `taplo`/`shfmt`/`shellcheck` whole-repo (sealed's root `script:` shape),
   extending the shell runs to shebang-detected extension-less dotfiles; add new
   root tasks + devenv tools for the classes biome can't cover — `yamlfmt` +
   `yamllint` (all `.yml`/`.yaml`), `ruff format --check` + `ruff check` (`.py`),
   `kdlfmt check` (`.kdl`). Document why-not for the residual types.
6. **Root cargo workspace** — already in place (Evidence base); the remaining
   delta is centralized `[workspace.lints]` (Decision: centralize — Task T10).
7. **Monorepo-tooling recommendations** — a root `.editorconfig` (aligned to the
   biome tab decision; covers even the un-lintable types in every editor), the
   root `bunfig.toml` from step 2, and a `.shellcheckrc` only if the whole-repo
   shellcheck run flags a repo-wide false positive.

### File-type → gate map (every committed type)

| Ext (count) | Gate after this design | Where defined |
| --- | --- | --- |
| `.ts` (93) | root biome `lint` | new root task (was 7 per-tool tasks + `agents-biome`, `moon.yml:23-25`) |
| `.rs` (58) | `cargo fmt --check` + clippy per crate | `tools/jj-hooks/moon.yml:11-15` (+ jj-gt) — unchanged |
| `.md` (55) | markdownlint, whole repo | root `moon.yml:12-14` — unchanged |
| `.json` (35) | root biome `lint` — **new coverage** (e.g. `.github/rulesets`, cotal configs) | new root task |
| `.yml`/`.yaml` (27) | workflows → actionlint (`moon.yml:15-17`, unchanged); **all → `yamlfmt` + `yamllint`** (NEW, T6) | new root tasks |
| `.nix` (21) | nixfmt/deadnix/statix/nil in nix-config (`nix-config/moon.yml:17-29`); root-level via `moon.yml:28-33` | unchanged |
| `.sh` (17) | shellcheck + shfmt, moved whole-repo | root (was `nix-config/moon.yml:30-35`), T5 |
| no-ext (13) | shell dotfiles (bashrc/profile/skhdrc/yabairc/jj-ws via shebang) → shfmt/shellcheck (T5); `zshrc` + ghostty configs + licences → **why-not** | T5 / documented |
| `.toml` (12) | taplo, moved whole-repo — **closes the root `Cargo.toml` gap (fails today, verified)** | root (was `nix-config/moon.yml:36-40`), T5 |
| `.lock` (10) | generated (bun.lock, Cargo.lock, flake.lock, devenv.lock) — intentionally ungated | — |
| `.kdl` (4) | **`kdlfmt check`** (NEW) | new root task, T8 |
| `.jsonc` (4) | root biome `lint` — **new coverage** (native JSONC parsing; verified) | new root task |
| `.ps1` (4) | **why-not: PowerShell — Windows-only, no nix runtime for a formatter (PSScriptAnalyzer needs pwsh)** | documented |
| `.rb` (3) | `brew style` | `Formula/moon.yml:10-19` — unchanged |
| `.zsh` (2) | **why-not: shfmt/shellcheck support sh/bash/mksh only — no zsh dialect** | documented |
| `.py` (1) | **`ruff format --check` + `ruff check`** (NEW) | new root task, T7 |
| `.pkl` (1, hk.pkl) | **why-not: Pkl CLI is eval-only; no stable formatter** | documented |
| `.ini` (1, foot.ini) | **why-not: no INI formatter in the nix toolchain** | documented |
| `.ahk` (1) | **why-not: AutoHotkey — Windows-only, no nix formatter** | documented |
| `.wslconfig` (1) | **why-not: INI-like Windows config; no formatter** | documented |
| `.css`/`.html` (0) | none committed; root biome covers them the moment they exist (add sealed's `html` block then) | — |

The `.editorconfig` (T9) is the backstop for the why-not set: it applies
indent/charset/final-newline to *every* file type in every editor — including
`.ini`, `.ps1`, `.ahk`, `.zsh`, and the dotfiles no linter gates — without
needing a per-type toolchain.

### Alternatives considered

- **Keep per-project biome configs/tasks, add a root backstop for the gaps** —
  rejected: keeps 8 configs + adds a 9th invocation; the scatter is the problem,
  and Matt explicitly chose "replace per project biome with root biome".
- **Backstop-only** (gate just the uncovered dirs from root, keep tool tasks) —
  rejected: two conventions for the same check, and per-tool biome versions can
  drift against the backstop's.
- **A meta-formatter (`treefmt` / `dprint`) multiplexing every formatter under
  one config + one task** — considered for "best monorepo tooling", rejected in
  favour of per-tool moon tasks: (1) neither zireael nor sealed uses one today
  (git-verified) and sealed's proven pattern is per-tool root tasks
  (`sealed/moon.yml:42-77`); (2) moon's affected-detection works per-task with
  precise `inputs`, so a change to one `.py` runs only `root:python` — a single
  treefmt wrapper task would re-run every formatter whenever any covered file
  changes, losing that scoping; (3) it adds a new meta-tool dependency and a
  second config surface over the tools biome/taplo/etc. already own. Revisit only
  if the per-tool task list grows unwieldy.
- **Inline biome version pin** (`bunx @biomejs/biome@2.4.16`) instead of a root
  bun workspace catalog — rejected now that Matt chose "unify all deps": the
  catalog is the single-source-of-truth mechanism and also fixes the `typescript`
  /`@types/bun` drift, which an inline biome pin would not.
- **One root biome, unified deps, all file types (chosen)** — sealed-proven
  (`sealed/moon.yml:11-14`, `sealed/package.json`), one config + one catalog per
  ecosystem, verified green biome end-state (132 files / 34ms), every file type
  gated or documented.

## Plan

Ordered, stacked PRs; each independently green under `moon ci`. Foundation
(biome + bun workspace) first, then the unifications, then the per-language
gates (each adds a devenv tool + a one-time format pass + a task), docs last.

### Task T1 — root `/biome.json` + one-time reformat + delete the 7 nested configs

- **Interfaces:**
  - Rewrites `/biome.json` (currently `biome.json:1-4`, inert) to the exact
    candidate JSON in the Approach section.
  - Deletes: `nix-config/tools/gh-route/biome.json`,
    `nix-config/tools/no-bash-gate/biome.json`,
    `nix-config/tools/wait-for-reviews/biome.json`,
    `tools/akiflow-cli/biome.json`, `tools/install-debug/biome.json`,
    `tools/release/biome.json`, `tools/setup-live-test-fixture/biome.json`.
  - Runs `bunx @biomejs/biome@2.4.16 check --write .` once; commits the 6
    reformatted files listed under End-state verification.
- **Not touched:** per-project `moon.yml` files (their `lint` tasks still pass
  via upward discovery — verified per-dir exit 0 with nested config removed);
  the biome version pin is settled properly in T2 (catalog), so this task uses
  the inline `@2.4.16` only for the one-time reformat command.
- **Acceptance:** `bunx @biomejs/biome@2.4.16 check .` from the repo root exits
  0 with no stack overflow (expect ~125 files, warnings only);
  `bunx biome check .` from inside `tools/release/` exits 0; `moon ci` green.

### Task T2 — root bun workspace (catalog, one lockfile, no-op install)

- **Interfaces:**
  - Adds `/package.json`:

    ```json
    {
     "name": "@zireael/monorepo",
     "private": true,
     "type": "module",
     "workspaces": {
      "packages": [
       "tools/akiflow-cli", "tools/release", "tools/install-debug",
       "tools/setup-live-test-fixture", "nix-config/tools/gh-route",
       "nix-config/tools/no-bash-gate", "nix-config/tools/wait-for-reviews"
      ],
      "catalog": {
       "@biomejs/biome": "2.4.16",
       "typescript": "^5",
       "@types/bun": "^1.3.14"
      }
     }
    }
    ```

  - Adds `/bunfig.toml` (`[install]` / `minimumReleaseAge = 432000`,
    `sealed/bunfig.toml` shape) and generates one root `/bun.lock`
    (`bun install`).
  - Adds `.moon/tasks/tag-bun.yml` with the no-op `install` →
    `deps: ['root:install']` / `options: { internal: true }`
    (`sealed/.moon/tasks/tag-bun.yml` shape), and adds the real `install` to the
    root `moon.yml` (`command: 'bun install --frozen-lockfile'`,
    `inputs: ['/package.json', '/bun.lock', '/bunfig.toml']`,
    `options: { internal: true, cache: false, runFromWorkspaceRoot: true }`).
  - Adds `tags: ['bun']` to the 7 bun project `moon.yml` files and removes each
    project's own `install` task (now inherited); converts each leaf
    `package.json` shared dep to `"catalog:"` (`@biomejs/biome` — add to akiflow;
    `typescript`, `@types/bun` — all tools that declare them) and deletes each
    per-tool `bun.lock`. Root project excludes the inherited `install`/`lint`/
    `format` (`workspace.inheritedTasks.exclude`, `sealed/moon.yml:21-23`).
  - `.gitignore`: the hoisted root `node_modules/` is already ignored
    (`.gitignore:8`); confirm per-tool `node_modules` no longer created.
- **Acceptance risk (call out):** zireael's tools are built/released
  independently today (per-tool `bun install --frozen-lockfile`;
  cargo-binstall + `tools/release`). Hoisting to one root `node_modules` + one
  `bun.lock` changes resolution — so acceptance MUST include: `moon run
  root:install` green from a cold clone; each tool's `bunx tsc --noEmit` +
  `bun test` still green; **the release workflow path still works**
  (`.github/workflows/release.yml` — dry-run or a scoped local exercise of the
  install step). If the release path can't tolerate a hoisted workspace, this
  task narrows to catalog-only (keep per-tool lockfiles) — flagged in Open
  Question 3.
- **Acceptance:** `moon run root:install` green; `moon ci` green; each tool's
  `ci` still green with install inherited; a single `/bun.lock` exists and the 7
  per-tool lockfiles are gone.

### Task T3 — root `lint`/`format` tasks; remove the 8 scattered biome tasks

- **Interfaces:**
  - Adds to root `moon.yml` `tasks:` (biome resolved via the catalog install
    from T2, so `deps: ['install']` like sealed):

    ```yaml
    lint:
      command: 'bunx biome check .'
      deps: ['install']
      inputs:
        - '**/*.{ts,tsx,js,jsx,cjs,mjs,json,jsonc,css,html}'
        - '!**/node_modules/**'
        - '/biome.json'
    format:
      command: 'bunx biome check --write .'
      deps: ['install']
      options:
        runInCI: false
    ```

  - Deletes root `agents-biome` (`moon.yml:23-25`) and rewrites the root `ci`
    deps (`moon.yml:34-35`) from
    `['markdownlint', 'actionlint', 'agents-biome', 'nixfmt', 'deadnix']` to
    `['markdownlint', 'actionlint', 'lint', 'nixfmt', 'deadnix']`.
  - Deletes the seven per-project `lint` tasks
    (`nix-config/tools/{gh-route,no-bash-gate,wait-for-reviews}/moon.yml:15-18`,
    `tools/{install-debug,release,setup-live-test-fixture}/moon.yml:15-18`,
    `tools/akiflow-cli/moon.yml:16-19`) and drops `'lint'` from each `ci`
    aggregate: `['typecheck', 'test']` for six (`akiflow-cli/moon.yml:30-31`
    et al.), `['typecheck', 'test', 'gate']` for no-bash-gate
    (`no-bash-gate/moon.yml:40-41`).
  - Updates the root `moon.yml:2-7` docstring (root now owns whole-repo biome +
    the bun install).
- **Acceptance:** `moon run root:lint` exits 0; `moon run root:format` is a
  no-op on a clean tree; `moon query tasks --affected` with a touched `.ts`
  anywhere (e.g. under `nix-config/agents/extensions/`) selects `root:lint` and
  no per-project lint targets exist (`moon task gh-route:lint` errors); a
  rust-only diff does **not** select `root:lint`; `moon ci` green.

### Task T4 — unify tsconfig (root base + per-tool extends)

- **Interfaces:**
  - Adds `/tsconfig.base.json` = the common compiler options (the verbatim
    block in the Evidence base).
  - Rewrites each of the 6 identical tsconfigs to
    `{ "extends": "../../tsconfig.base.json" }` (for `tools/*`) /
    `{ "extends": "../../../tsconfig.base.json" }` (for `nix-config/tools/*`).
  - Rewrites `tools/akiflow-cli/tsconfig.json` to extend the base + its 4 local
    overrides (`jsx`, the 3 relaxed `noUnused*`/`noPropertyAccess…`,
    `types: ["bun","node"]`).
  - Adds `/tsconfig.base.json` to each `typecheck` task's `inputs`
    (the per-tool `moon.yml` typecheck tasks) so moon invalidates on base edits.
- **Acceptance:** each tool's `bunx tsc --noEmit` (its `typecheck` task) exits 0
  (behavior identical — extends is transparent); `moon ci` green.

### Task T5 — whole-repo `taplo`/`shfmt`/`shellcheck` (incl. extension-less shell)

- **Interfaces:**
  - Runs one-time `taplo fmt Cargo.toml` (the only dirty TOML repo-wide,
    verified) and commits it.
  - Adds root `moon.yml` tasks (sealed's `script:` shape, `sealed/moon.yml:65-77`,
    with zireael's prune list — `.git`, `.jj`, `node_modules`, `target`, `dist`,
    `.direnv`, `.devenv`, `.moon`). The shell runs use `-print0` on `*.sh` **and**
    shebang-detected extension-less files:

    ```yaml
    shfmt:
      script: "find . -name .git -prune -o -name .jj -prune -o -name node_modules -prune -o -name target -prune -o -name dist -prune -o -name .direnv -prune -o -name .devenv -prune -o -name .moon -prune -o -name '*.sh' -print0 | xargs -0 shfmt -d -i 0 -s"
      inputs: ['**/*.sh', '!**/node_modules/**', '!**/target/**']
    shellcheck:
      script: "find . -name .git -prune -o -name .jj -prune -o -name node_modules -prune -o -name target -prune -o -name dist -prune -o -name .direnv -prune -o -name .devenv -prune -o -name .moon -prune -o -name '*.sh' -print0 | xargs -0 shellcheck --external-sources"
      inputs: ['**/*.sh', '!**/node_modules/**', '!**/target/**']
    taplo:
      script: "find . -name .git -prune -o -name .jj -prune -o -name node_modules -prune -o -name target -prune -o -name dist -prune -o -name .direnv -prune -o -name .devenv -prune -o -name .moon -prune -o -name '*.toml' -print0 | xargs -0 taplo fmt --check"
      inputs: ['**/*.toml', '!**/node_modules/**', '!**/target/**']
      env:
        RUST_LOG: 'error'
    ```

    Extension-less shell dotfiles (`bashrc`, `profile`, `skhdrc`, `yabairc`,
    `jj-ws`) are added to the shfmt/shellcheck runs by `shfmt -f .` shebang
    discovery **or** an explicit file list in the script — the executor picks
    whichever the one-time format pass proves clean; `zshrc` is excluded (zsh
    dialect unsupported — documented). Appends `'shfmt', 'shellcheck', 'taplo'`
    to the root `ci` deps.
  - Removes from `nix-config/moon.yml`: `shellcheck` (:30-32), `shfmt` (:33-35),
    `taplo` (:36-40), the now-unused `shell`/`toml` file groups (:12-13), and
    those three entries from its `ci` deps (:57-69). nix-config keeps
    nixfmt/deadnix/statix/nil + the flake-evals.
- **Acceptance:** `moon run root:taplo` / `root:shfmt` / `root:shellcheck` all
  exit 0 (whole-repo find-runs verified exit 0 this session, post the
  `Cargo.toml` format); `moon run nix-config:ci` still green with the slimmer
  dep list; `moon ci` green.

### Task T6 — YAML gate (`yamlfmt` + `yamllint`)

- **Interfaces:**
  - Adds `yamlfmt` + `yamllint` to `devenv.nix` packages (`devenv.nix:29-42`
    linters block).
  - One-time `yamlfmt .` over all committed `.yml`/`.yaml`; commit the reformat
    (isolated, zero logic change). Adds a root `.yamlfmt` + `.yamllint` config
    if defaults over-reformat (executor decides from the one-time pass; keep
    configs minimal).
  - Adds root `moon.yml` tasks (find-run over `*.yml`/`*.yaml` with the same
    prune list; actionlint stays as-is for workflow semantics):

    ```yaml
    yamlfmt:
      script: "find . <prune-list> -o \\( -name '*.yml' -o -name '*.yaml' \\) -print0 | xargs -0 yamlfmt -lint"
      inputs: ['**/*.{yml,yaml}', '!**/node_modules/**']
    yamllint:
      script: "find . <prune-list> -o \\( -name '*.yml' -o -name '*.yaml' \\) -print0 | xargs -0 yamllint"
      inputs: ['**/*.{yml,yaml}', '!**/node_modules/**', '/.yamllint']
    ```

    Appends `'yamlfmt', 'yamllint'` to the root `ci` deps.
- **Rationale:** biome cannot format/lint YAML (2026 roadmap — parser not yet
  stable), so a dedicated tool is required; `yamlfmt` 0.21.0 + `yamllint`
  1.37.1 are both in nixpkgs (verified).
- **Acceptance:** `moon run root:yamlfmt` / `root:yamllint` exit 0 after the
  one-time pass; `moon ci` green.

### Task T7 — Python gate (`ruff`)

- **Interfaces:**
  - Adds `ruff` to `devenv.nix` packages.
  - One-time `ruff format .` + `ruff check --fix` over the one committed `.py`
    (`.github/scripts/bump-formulae.py`); commit any reformat.
  - Adds root `moon.yml` tasks:

    ```yaml
    python-fmt:
      command: 'ruff format --check .'
      inputs: ['**/*.py', '!**/node_modules/**']
    python-lint:
      command: 'ruff check .'
      inputs: ['**/*.py', '!**/node_modules/**', '/ruff.toml']
    ```

    Appends both to the root `ci` deps. (ruff respects `.gitignore`; a minimal
    `/ruff.toml` only if defaults are too strict on the one script.)
- **Rationale:** `ruff` 0.15.20 in nixpkgs (verified); single fast binary,
  formatter + linter.
- **Acceptance:** `moon run root:python-fmt` / `root:python-lint` exit 0;
  `moon ci` green.

### Task T8 — KDL gate (`kdlfmt`)

- **Interfaces:**
  - Adds `kdlfmt` to `devenv.nix` packages.
  - One-time `kdlfmt format` over the 4 `.kdl` (zellij config + layouts under
    `nix-config/dotfiles/zellij/`); commit any reformat.
  - Adds a root `moon.yml` task:

    ```yaml
    kdlfmt:
      command: 'kdlfmt check nix-config/dotfiles/zellij'
      inputs: ['**/*.kdl']
    ```

    Appends `'kdlfmt'` to the root `ci` deps. (All committed `.kdl` live under
    `nix-config/dotfiles/zellij/`, so a scoped path is exact; broaden to a
    find-run if `.kdl` spreads.)
- **Rationale:** `kdlfmt` 0.1.7 in nixpkgs (verified); supports KDL v1 + v2 with
  a `check` subcommand.
- **Acceptance:** `moon run root:kdlfmt` exits 0 after the one-time pass;
  `moon ci` green.

### Task T9 — `.editorconfig` + supporting tooling files

- **Interfaces:**
  - Adds a root `/.editorconfig` (`root = true`; `[*]` `end_of_line = lf`,
    `insert_final_newline = true`, `charset = utf-8`,
    `trim_trailing_whitespace = true`, `indent_style = tab`; per-type overrides
    only where a type is space-indented by its own tool — e.g. `.nix`, `.py`,
    `.yml` at the indent their formatters produce). MUST use `indent_style = tab`
    for biome-covered types so it agrees with `/biome.json` (biome reads
    `.editorconfig` by default). Modeled on `sealed/oss/seal/.editorconfig`,
    re-homed to root with the tab alignment.
  - Adds a root `/.shellcheckrc` **only if** the T5 whole-repo shellcheck run
    surfaces a repo-wide false positive (sealed disables SC2329 for its test
    harnesses; zireael may not need it — decide from the T5 run).
- **Acceptance:** `moon run root:lint` still green (editorconfig agrees with
  biome, no reformat churn); `moon ci` green.

### Task T10 — centralize cargo workspace lints

- **Interfaces:**
  - Adds `[workspace.lints.clippy]` to the root `Cargo.toml` (`disallowed_methods
    = "deny"`, sealed pattern) + a root `clippy.toml` carrying the
    disallowed-methods list (modeled on `sealed/oss/seal/clippy.toml`).
  - Each crate (`tools/jj-hooks`, `tools/jj-gt`) opts in via `[lints] workspace =
    true`. Per-crate `moon.yml` clippy already runs `-D warnings`
    (`tools/jj-hooks/moon.yml:15`); those commands are unchanged.
- **Acceptance:** `cargo clippy --workspace --all-targets -- -D warnings` exits 0
  on the current tree (no new findings from the shared policy); `moon ci` green.

### Task T11 — docs sync

- **Interfaces:** updates `docs/specs/platform/ci.md` (the project/gate table +
  toolchain prose) to reflect the consolidated gates: root now owns
  biome/markdownlint/shfmt/shellcheck/taplo/yamlfmt/yamllint/python/kdl + the bun
  install; per-tool projects own typecheck/test (+ no-bash-gate's gate); the new
  devenv tools; the catalog/tsconfig-base conventions.
- **Acceptance:** `moon run root:markdownlint` green; the table matches
  `moon query tasks` output for each project.

## Decisions

All questions resolved before freeze (per the design-record open-questions
policy); confirmed by Matt directly. The Plan is written against these.

1. **indentStyle: tab.** Both existing full biome configs already declare it
   (`tools/akiflow-cli/biome.json:8`, `sealed/biome.json:18`), the verified
   end-state reformats zero `.ts` files, and the root `.editorconfig` (T9) aligns
   `indent_style = tab` for biome-covered types.
2. **Centralize cargo workspace lints — yes.** Add a root
   `[workspace.lints.clippy]` block + a root `clippy.toml` + per-crate
   `[lints] workspace = true` (sealed pattern; denies `disallowed_methods`). Now a
   first-class Plan task (T10), not a deferral. Per-crate clippy already runs
   `-D warnings`; this adds the shared disallowed-methods policy hook on top.
3. **Root bun workspace: full hoisted.** One `node_modules` + one `bun.lock`
   (sealed pattern). Because this changes install resolution for the
   independently-released tools, T2's acceptance gates the release-workflow path
   explicitly.
4. **YAML tool: `yamlfmt` + `yamllint`.** Standalone Go/Python binaries already in
   nixpkgs, no node/Prettier dependency; yamllint is the de-facto YAML linter (T6).
5. **Extension-less shell dotfiles: shebang-gated; zsh formatter-ungated.**
   shfmt/shellcheck cover bash/sh dotfiles (`bashrc`/`profile`/`skhdrc`/`yabairc`/
   `jj-ws`) via shebang detection. A zsh-formatter search (this session) found no
   CI-usable option: shfmt's maintainer states it does not fully support zsh (and
   tracks skipping zsh files); beautysh is bash-only; shellharden is a
   quoting-hardener, not a formatter; the browser tools are not nix-packageable.
   `zshrc` therefore stays formatter-ungated, with the root `.editorconfig` still
   applying whitespace rules (a future `zsh -n` syntax-check lint leg is possible
   but out of scope here).
6. **Why-not set stays ungated.** `.ps1`/`.ahk` (no nix formatter runtime), `.pkl`
   (eval-only, no stable formatter), `.ini`/`.wslconfig` (no INI formatter), `.zsh`
   (see 5), `.lock` (generated), licences/CODEOWNERS/ghostty-theme files (not
   code). Each documented with a reason in the record + covered for whitespace by
   `.editorconfig`.
7. **Per-tool moon tasks (not a meta-formatter).** Keeps sealed consistency + moon
   per-task affected-detection + no new meta-tool. No `treefmt`/`dprint`.
8. **Affected-scope widening is intended.** Any PR touching a gated type runs that
   whole-repo tool (biome 132 files / 34 ms; each per-language tool similarly
   scoped by its `inputs`); unrelated PRs skip it via moon affected-detection. This
   widens today's narrow `agents-biome` inputs by design — it is the point of the
   consolidation.

## Tasks

- [ ] T1 — root `/biome.json` (overflow-proof, verified content), one-time
  6-file reformat, delete 7 nested `biome.json`
- [ ] T2 — root bun workspace: `/package.json` catalog + `/bunfig.toml` +
  `/bun.lock` + `tag-bun.yml` no-op install + tag the 7 bun projects + convert
  leaf deps to `catalog:` + delete per-tool lockfiles (release-path acceptance)
- [ ] T3 — root `lint`/`format` tasks; delete `agents-biome` + 7 per-project
  `lint` tasks; rewrite all `ci` dep lists
- [ ] T4 — `/tsconfig.base.json` + per-tool `extends`; akiflow keeps overrides
- [ ] T5 — whole-repo `taplo`/`shfmt`/`shellcheck` (incl. extension-less shell);
  one-time `taplo fmt Cargo.toml`; slim `nix-config/moon.yml`
- [ ] T6 — YAML gate: `yamlfmt` + `yamllint` (devenv + root tasks + one-time pass)
- [ ] T7 — Python gate: `ruff format --check` + `ruff check` (devenv + root task)
- [ ] T8 — KDL gate: `kdlfmt check` (devenv + root task + one-time pass)
- [ ] T9 — root `/.editorconfig` (tab-aligned) + `/.shellcheckrc` if T5 needs it
- [ ] T10 — centralize cargo workspace lints (root `[workspace.lints.clippy]` +
  `clippy.toml` + per-crate `[lints] workspace = true`)
- [ ] T11 — sync `docs/specs/platform/ci.md` gate tables + toolchain prose

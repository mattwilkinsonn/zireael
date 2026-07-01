# Design: `moon ci` as zireael's unified local + CI gate

Status: **approved** — build in progress (M1–M8 stacked PRs)
Domain: platform / build-tooling

## Problem / Intent

zireael's gate is three overlapping mechanisms kept in sync by hand: `hk.pkl`
(pre-push hook), the `just ci*` recipes, and a per-tool set of GitHub Actions
workflows. Each check command is written two-to-three times. The goal is to
mirror how `sealed` is set up now — **one `moon ci` command** as the single
gate, run identically locally (pre-push) and in CI, with toolchains pinned by
`proto` and provided by a `devenv` dev-shell.

## Global Constraints

- **Mirror sealed's transferable core, not its remote CI.** sealed's remote CI
  is Woodpecker + a Petrel pipeline generator + a self-built OCI image; zireael
  is GitHub Actions. The Woodpecker/Petrel/OCI machinery does **not** transfer.
  What transfers: `.prototools`, `devenv.nix`, `.envrc`, `.moon/workspace.yml`,
  per-project `moon.yml` with a `ci` aggregate, and `hk.pkl` pre-push = `moon ci`.
- **`moon` runs pure *system* tasks against the devenv PATH.** sealed has **no**
  `.moon/toolchain.yml` — moon does not manage bun/node/rust; it execs the
  binaries that devenv (via proto) puts on PATH, keeping `.prototools` the single
  version source. zireael mirrors this: no `.moon/toolchain.yml`.
- **Version pins (approved: latest, not sealed's):**
  - `.prototools`: `bun = "1.3.14"`, `node = "24.<LTS>"` (latest 24 LTS line),
    `moon = "2.3.5"`.
  - `rust-toolchain.toml`: pin exact `1.96.0` (not `stable`) so CI caches survive.
  - `moon` **2.x** (2.x workspace schema; nixpkgs ships moon 1.x which rejects it
    — this is why the version comes from proto, not nixpkgs).
- **No behavior drift in the checks themselves.** Every check command below is
  the *exact* string zireael already runs today (cited). The migration re-homes
  them into moon tasks; it does not change what they assert.
- **`just` is retired entirely (approved).** `hk.pkl` becomes a one-line
  `check = "moon ci"` pre-push shell (as sealed's does). Every `just ci*` recipe
  moves into a moon task; the remaining non-CI recipes relocate (see M7): the
  `jj`-based `release` recipe (`Justfile:471-589`) → `scripts/release.sh` (a moon
  `runInCI:false` task wraps it); `install-debug*` (`Justfile:164-214`) → moon
  `build`/`install` tasks per project; `install-deps*` (`Justfile:32-161`) is
  obsoleted by the devenv shell (which provides the toolchain) — any residual
  bootstrap moves into `enterShell`. The `Justfile` is deleted at the end of M7.
- **Attribution / VCS:** land as a stack of small PRs (see Plan); commit as Matt
  with the `Co-Authored-By: seal` trailer; `gt` workflow; never merge.

## Evidence base (cited this session)

sealed reference — clone `/home/mattw/agents/workspaces/hudson/sealed`:

- `.prototools` is the single version source; sealed pins `bun = "1.3.13"`,
  `node = "22.23.1"`, `moon = "2.3.4"` (per SealedMoonRef report, this session).
- **No `.moon/toolchain.yml`** — confirmed absent; moon runs system tasks
  against the devenv/proto PATH (SealedMoonRef report).
- `.envrc` enters the shell via `use devenv` (direnv), `watch_file .prototools`.
- `devenv.nix` `packages` provides proto + `nixfmt-rfc-style`, `deadnix`,
  `statix`, `nil`, `shellcheck`, `shfmt`, `taplo`, `actionlint`, fenix Rust from
  `rust-toolchain.toml`, `cargo-nextest`; `enterShell` runs `proto install` +
  `hk install`.
- `infra/nix/moon.yml` is the closest analog: tier-1 linter tasks (`nixfmt`
  runs `find . -name '*.nix' -print0 | xargs -0 nixfmt --check`; `deadnix
  --fail .`; `statix check`; `nil`; `shellcheck --external-sources`; `shfmt -d
  -i 0 -s .`; `taplo fmt --check`) + tier-2 `flake-eval-<host>` tasks running
  `nix eval --raw .#nixosConfigurations.<host>.…outPath --no-warn-dirty
  --accept-flake-config`, inputs `['@group(nix)','flake.lock']`.
- `hk.pkl` pre-push is a single step `check = "moon ci"` (SealedMoonRef report).
- Rust project `ci` aggregate pattern:
  `oss/compass/crates/compass-proto/moon.yml` — `ci: {deps:['fmt','lint',…,
  'clippy','test'], options:{cache:false}}`.

zireael current state — clone `~/agents/workspaces/tasman/zireael`:

- `hk.pkl:166-219` — `pre-push`/`pre-commit`/`fix`/`check` hooks compose
  `rust_steps` + `bun_steps` + `tap_steps` + `nix_steps` + `shell_steps` +
  `toml_steps` + `flake_eval_steps` + `doc_steps`.
- `Justfile:308-342` — per-tool CI recipes:
  - `ci-jj-hooks: (fmt-pkg "jj-hooks") (clippy-pkg "jj-hooks") nextest-jj-hooks`
  - `fmt-pkg PKG: cargo fmt -p {{PKG}} -- --check` (`Justfile:295-296`)
  - `clippy-pkg PKG: cargo clippy -p {{PKG}} --all-targets -- -D warnings`
    (`Justfile:298-299`)
  - `nextest-jj-hooks: cargo nextest run -p jj-hooks --no-fail-fast`
    (`Justfile:301-302`)
  - `nextest-jj-gt: cargo nextest run -p jj-gt --no-fail-fast --no-tests=warn
    -E 'not (test(gh_live) | test(gt_submit_live))'` (`Justfile:305-306`)
  - `ci-akiflow-cli` (`Justfile:321-328`): `cd tools/akiflow-cli` →
    `bun install --frozen-lockfile` → `bunx biome check .` → `bunx tsc --noEmit`
    → `timeout 5m bun test`.
  - `ci-tap` (`Justfile:331-338`): `brew style Formula/*.rb` (skips if no brew).
  - `ci-docs` (`Justfile:341-342`): `markdownlint-cli2 "**/*.md"`.
- `Justfile:357-376` — `ci-nix-config` linter commands (identical strings to the
  sealed set above).
- `Justfile:395` — `ci-nix-config-eval` loops
  `rpi4 rpi5 mattfw mattserver mattlinuxpro mattpc-wsl` — a **stale** host list
  (SEA-853 trimmed zireael to `mattpc-wsl` + `Matts-MacBook-Pro`; the rest moved
  to sealed). The moon migration corrects this to the two real hosts.
- `Cargo.toml:1-7` — workspace members `tools/jj-hooks`, `tools/jj-gt`;
  akiflow-cli is bun/TS, not a workspace member.
- `.github/workflows/` — `ci-base-nix.yml`, `ci-base-rust.yml`, `jj-hooks.yml`,
  `nix-config.yml`, `akiflow-cli.yml`, `tap.yml`, `docs.yml`, `nightly.yml`,
  `post-merge.yml`, `release.yml`, `jj-gt.yml`.
- `nix-config.yml:107-135` — per-host flake-eval jobs run
  `nix eval --raw '.#nixosConfigurations.mattpc-wsl.config.system.build.toplevel.outPath'
  --no-warn-dirty --accept-flake-config` (mattpc-wsl on ubuntu; darwin on
  macos-latest).
- `.envrc:1-19` — today a plain `PATH_add` hook (cargo + bun), **not** devenv.
- No `.moon/`, `.prototools`, or `devenv.*` exist in zireael (glob confirmed).

Latest upstream (this session): moon `v2.3.5`, proto `v0.58.1`, bun `v1.3.14`,
devenv `v2.1.2`, rust stable `1.96.0`.

## Approach

Adopt sealed's layered model, adapted to GitHub Actions:

```text
.prototools        pins bun / node / moon                (single version source)
   │
devenv.nix         provides proto + fenix Rust + every linter on PATH
   │
.envrc (use devenv) direnv auto-enters the shell
   │
.moon/workspace.yml registers projects; tasks are SYSTEM tasks (exec PATH bins)
   │
per-project moon.yml  each declares a `ci` aggregate over its checks
   │
moon ci            THE gate — local (hk pre-push) and CI (one workflow)
```

### Project layout (moon projects → zireael dirs)

| moon project | source | `ci` deps (existing command, cited) |
| --- | --- | --- |
| `jj-hooks` | `tools/jj-hooks` | `fmt` (`cargo fmt -p jj-hooks -- --check`), `clippy` (`cargo clippy -p jj-hooks --all-targets -- -D warnings`), `test` (`cargo nextest run -p jj-hooks --no-fail-fast`) — `Justfile:295-309` |
| `jj-gt` | `tools/jj-gt` | `fmt`, `clippy`, `test` (nextest with the live-test exclusion `-E 'not (test(gh_live) \| test(gt_submit_live))'`) — `Justfile:305-312` |
| `akiflow-cli` | `tools/akiflow-cli` | `lint` (`bunx biome check .`), `typecheck` (`bunx tsc --noEmit`), `test` (`bun test`) — `Justfile:321-328` |
| `nix-config` | `nix-config` | `nixfmt`/`deadnix`/`statix`/`nil`/`shellcheck`/`shfmt`/`taplo` + `flake-eval-mattpc-wsl` + `flake-eval-darwin` — `Justfile:357-376`, `nix-config.yml:107-135` |
| `tap` | `.` (Formula/) | `brew-style` (`brew style Formula/*.rb`) — `Justfile:331-338` |
| `root`/`docs` | `.` | `markdownlint` (`markdownlint-cli2 "**/*.md"`), `actionlint` — `Justfile:341-342`, `hk.pkl:160-163` |

Task-level detail (exact commands, inputs, `runInCI`) is specified per task in
the Plan; the Tasks are structured so each project lands independently.

### Remote CI shape (approved: Option A)

sealed's remote CI does not transfer. **Decision: Option A** — one GitHub Actions
workflow installs Nix + devenv and runs `moon ci`:

- Install nix (the Determinate action already in use across zireael's workflows)
  → enter the devenv shell → `moon ci` with `MOON_BASE`/`MOON_HEAD` for
  affected-only, behind a single rollup check context for branch protection.
- Replaces the per-tool workflows it subsumes (`jj-hooks.yml`, `jj-gt.yml`,
  `akiflow-cli.yml`, `nix-config.yml`, `tap.yml`, `docs.yml`, and the
  `ci-base-*.yml` reusables); retains `release.yml`, `nightly.yml`,
  `post-merge.yml`, and the two `flake-update-*.yml` jobs.
- (Rejected: **B** baked OCI image — an image to build/publish/maintain, heavy
  for a personal monorepo, a later optimization if cold-start hurts; **C**
  local-only moon — leaves the two-source drift this migration kills.)

## Plan

Small PRs, stacked, each independently green. Tasks are ordered so the toolchain
foundation lands before the tasks that depend on it.

### Task M1 — proto + devenv + direnv foundation

- **Consumes:** sealed `.prototools`/`devenv.nix`/`.envrc` (reference);
  zireael `rust-toolchain.toml:1-3` (`channel = "stable"` → exact pin);
  `.envrc:1-19` (current PATH_add hook — fold cargo/bun PATH needs into devenv).
- **Produces:**
  - `.prototools` — `bun = "1.3.14"`, `node = "24.<latest LTS>"`,
    `moon = "2.3.5"`.
  - `devenv.nix` + `devenv.yaml` (+ generated `devenv.lock`) providing: proto,
    fenix Rust from `rust-toolchain.toml`, `cargo-nextest`, `nixfmt-rfc-style`,
    `deadnix`, `statix`, `nil`, `shellcheck`, `shfmt`, `taplo`, `actionlint`,
    `markdownlint-cli2`; `enterShell` runs `proto install` + `hk install`.
  - `.envrc` rewritten to `use devenv` (keep `source_env_if_exists .envrc.local`
    from `.envrc:19`).
  - `rust-toolchain.toml` pinned to an exact channel.
- **Acceptance:** `direnv allow` → shell enters; `proto install` resolves
  bun/node/moon; `moon --version` = pinned; every linter above is on PATH.
  No moon tasks yet.

### Task M2 — moon workspace scaffold

- **Consumes:** `.moon/workspace.yml` shape from sealed (projects map, `vcs:
  {provider: github, defaultBranch: main}`, header noting system-task model).
- **Produces:** `.moon/workspace.yml` registering the six projects above. **No**
  `.moon/toolchain.yml`.
- **Acceptance:** `moon query projects` lists all six; `moon ci` is a no-op
  (no tasks yet) and exits 0.

### Task M3 — Rust projects (`jj-hooks`, `jj-gt`)

- **Consumes:** `Justfile:295-312` (exact fmt/clippy/nextest commands, incl.
  jj-gt's live-test exclusion); `Cargo.toml:1-7` (members).
- **Produces:** `tools/jj-hooks/moon.yml` + `tools/jj-gt/moon.yml`, each with
  `fmt`/`clippy`/`test` system tasks (`toolchain: system`, `cache: false` on
  clippy/test as sealed does) and a `ci` aggregate depending on them. jj-gt's
  `test` carries the `-E 'not (test(gh_live) | test(gt_submit_live))'` filter;
  a separate `test-live` task marked `runInCI: false`.
- **Interfaces:** tasks exec `cargo` off the devenv PATH; inputs scope to the
  crate's `**/*.rs` + `Cargo.toml` + workspace `/Cargo.lock`.
- **Acceptance:** `moon run jj-hooks:ci` and `moon run jj-gt:ci` reproduce
  today's `just ci-jj-hooks` / `just ci-jj-gt` results.

### Task M4 — `akiflow-cli` (bun/TS)

- **Consumes:** `Justfile:321-328`; `tools/akiflow-cli/{package.json,bun.lock,
  bunfig.toml,biome.json,tsconfig.json}` (confirmed present).
- **Produces:** `tools/akiflow-cli/moon.yml` — `install`
  (`bun install --frozen-lockfile`, `cache:false`), `lint` (`bunx biome check
  .`), `typecheck` (`bunx tsc --noEmit`), `test` (`bun test`), `ci` aggregate.
- **Acceptance:** `moon run akiflow-cli:ci` == `just ci-akiflow-cli`.

### Task M5 — `nix-config` (lints + flake-evals)

- **Consumes:** `Justfile:357-376` (linter strings), `nix-config.yml:107-135`
  (per-host eval strings), sealed `infra/nix/moon.yml` (task shape).
- **Produces:** `nix-config/moon.yml` — tier-1 linter tasks (`nixfmt`,
  `deadnix`, `statix`, `nil`, `shellcheck`, `shfmt`, `taplo`) + tier-2
  `flake-eval-mattpc-wsl` and `flake-eval-darwin` (the **two real** hosts —
  corrects the stale `Justfile:395` list). `ci` aggregate over the linters;
  flake-evals are `runInCI: true` but tagged for the affected-graph so a shared
  module change re-runs them (inputs `['@group(nix)','flake.lock']`, no per-host
  file scoping).
- **Interfaces:** `nixfmt` = `find . -name '*.nix' -print0 | xargs -0 nixfmt
  --check`; `statix check -c . .` (cwd = project); evals =
  `nix eval --raw .#{nixos,darwin}Configurations.<host>.…outPath --no-warn-dirty
  --accept-flake-config`. darwin eval is macOS-only → the workflow (Q2) runs it
  on a macOS runner; locally it's a no-op off-macOS (guard as sealed does).
- **Acceptance:** `moon run nix-config:ci` == `just ci-nix-config`; `moon run
  nix-config:flake-eval-mattpc-wsl` evals green.

### Task M6 — `tap` + `docs`/root meta

- **Consumes:** `Justfile:331-342`, `hk.pkl:155-164`.
- **Produces:** a `tap` project (`brew-style`, guarded to skip when brew absent,
  matching `Justfile:334-337`) and root/`docs` tasks (`markdownlint`,
  `actionlint`).
- **Acceptance:** `moon run :markdownlint` / `:actionlint` reproduce
  `just ci-docs` + hk's actionlint step; `tap:brew-style` runs where brew exists.

### Task M7 — cut `hk` over to `moon ci`; retire `just` entirely

- **Consumes:** `hk.pkl:166-219`; the full `Justfile` (`fmt`/`clippy`/`test`
  `Justfile:12-23`, `install-deps*` `Justfile:32-161`, `install-debug*`
  `Justfile:164-214`, `ci*` `Justfile:232-403`, `release` `Justfile:471-589`).
- **Produces:**
  - `hk.pkl` reduced to a single `pre-push` step `check = "moon ci"` (sealed
    shape).
  - `release` (jj-based, `Justfile:471-589`) → `scripts/release.sh`, wrapped by a
    `runInCI:false` moon task.
  - `install-debug*` (`Justfile:164-214`) → per-project moon `build`/`install`
    tasks (`runInCI:false`).
  - `install-deps*` (`Justfile:32-161`) obsoleted by the devenv shell; any
    residual bootstrap folds into `devenv.nix` `enterShell`.
  - `fmt`/`clippy`/`test` (`Justfile:12-23`) already covered by the per-project
    moon tasks (M3); drop them.
  - **Delete the `Justfile`.**
- **Acceptance:** `git push` triggers `moon ci` via hk; a deliberate lint break
  fails the push. `scripts/release.sh` cuts a release equivalently to the old
  `just release`. No `just` invocation remains in the repo (grep clean).

### Task M8 — remote CI workflow (Option A)

- **Consumes:** existing `.github/workflows/*` (to retire/replace).
- **Produces:** one workflow installing Nix (Determinate action) + devenv and
  running `moon ci` (affected via `MOON_BASE`/`MOON_HEAD`), with a stable rollup
  check context for branch protection; retire the per-tool workflows it subsumes
  (`jj-hooks.yml`, `jj-gt.yml`, `akiflow-cli.yml`, `nix-config.yml`, `tap.yml`,
  `docs.yml`, `ci-base-nix.yml`, `ci-base-rust.yml`). Keep `release.yml`,
  `nightly.yml`, `post-merge.yml`, `flake-update-*.yml`.
- **Acceptance:** a PR runs `moon ci` in CI and gates on one context; a broken
  check fails it; the darwin flake-eval runs on a macOS runner.

## Resolved decisions

- **Q1 — node pin:** **latest 24 LTS** (not sealed's 22.23.1). zireael is
  independent; akiflow-cli is the only node/bun consumer.
- **Q2 — remote CI shape:** **A** (one workflow installs devenv, runs `moon ci`).
- **Q3 — `just` disposition:** **retire entirely** (see M7).
- **Q4 — rust pin:** exact **`1.96.0`** (latest stable).

## Tasks

- [ ] M1 — proto + devenv + direnv foundation
- [ ] M2 — moon workspace scaffold (`.moon/workspace.yml`, no toolchain.yml)
- [ ] M3 — Rust projects `jj-hooks` + `jj-gt` moon.yml
- [ ] M4 — `akiflow-cli` moon.yml
- [ ] M5 — `nix-config` moon.yml (lints + 2 flake-evals; fixes stale host list)
- [ ] M6 — `tap` + `docs`/root meta tasks
- [ ] M7 — `hk` → `moon ci`; retire `just` entirely (relocate non-CI recipes)
- [ ] M8 — remote CI workflow (Option A: devenv-in-Actions `moon ci`)

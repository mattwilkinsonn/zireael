# Design: Distribution continuity before the zireael private flip

Status: **proposed**
Domain: tools

Amends the frozen extraction record
`docs/designs/tools/oss-tool-extraction-and-shared-tooling.md` (sibling, same
directory). That record stays frozen and unmodified; this record supersedes
exactly ONE of its decisions and adds the pre-flip release/comms continuity
plan its resolved decision 2 left as accepted breakage.

## Supersession

**Superseded decision:** the extraction record's per-repo tap choice —
`oss-tool-extraction-and-shared-tooling.md:80-82`: "Each standalone repo
becomes its own tap (`brew tap mattwilkinsonn/jj-hooks` etc.); the zireael tap
stops receiving bumps." Matt reopened it because jj-hooks is actively used,
and the breakage that decision (with resolved decision 2's "residual breakage
is accepted") tolerated — dead binstall URLs, a third tap path, no proven
standalone release pipeline — is too lossy for a tool with real users.
**Replacement:** ONE consolidated conventional tap,
`mattwilkinsonn/homebrew-tap` (`brew tap mattwilkinsonn/tap`), carrying both
`jj-hooks.rb` and `jj-gt.rb`, durable across future tool churn. Everything
else in the extraction record stands.

## Problem / Intent

The infra record's T8 (`docs/designs/platform/personal-infra-monorepo.md:537`:
"### T8 — flip zireael private (LAST)") makes the zireael repo private. On
that flip, every distribution channel anchored to zireael dies while the
channels users actually hold still point there:

- **crates.io is immutable and survives, but points at the corpse.** Both
  published 0.3.11 crates bake
  `repository = "https://github.com/mattwilkinsonn/zireael"` (verified from
  the published `jj-hooks-0.3.11.crate`'s embedded `Cargo.toml:26`), and the
  binstall templates are `{ repo }`-relative (embedded `Cargo.toml:32`:
  `pkg-url = "{ repo }/releases/download/v{ version }/jj-hooks-v{ version }-linux-x64.tar.gz"`).
  Post-flip, `cargo binstall jj-hooks` 404s and falls back to a slow source
  build. Only a NEW published version can re-anchor `{ repo }` — the
  standalone clones already carry the fix uncut:
  `jj-hooks/Cargo.toml:7` / `jj-gt/Cargo.toml:7` both read
  `repository = "https://github.com/mattwilkinsonn/jj-hooks"` (resp. `jj-gt`)
  at `version = "0.3.11"`, unpublished.
- **Current Homebrew users sit on the zireael tap**, whose repo AND asset
  URLs (`Formula/jj-hooks.rb:9` in zireael:
  `…/zireael/releases/download/…`) both die → hard break for
  `brew upgrade`/reinstall.
- **The standalone release pipeline has never succeeded.** The
  `mattwilkinsonn/jj-hooks` v0.3.11 run (id 33913136991, re-verified this
  session via `gh run view`) shows `Publish GitHub Release: failure`
  (release pre-existed from the manual asset mirror), cascading
  `Bump Homebrew tap formula: skipped` and `Publish to crates.io: skipped`.
  The tap-bump and crates-publish legs are unproven, and the App credentials
  they need are absent (`gh variable list` empty on both standalones; only
  pre-monorepo `CARGO_REGISTRY_TOKEN` + `HOMEBREW_TAP_TOKEN` secrets exist).

Intent: BEFORE the flip, while zireael is still a live fallback — consolidate
Homebrew distribution onto one durable tap, prove each standalone's release
pipeline end-to-end by cutting a real v0.3.12, and put migration comms on
surfaces that survive the flip.

**Sequence gate (cross-record):** this record's deliverables — v0.3.12
shipped through the standalone pipelines, the consolidated tap live and
validated, migration comms posted — MUST land before infra T8 executes. The
infra record's T8 already waits on the extraction record's mirror + comms
(`oss-tool-extraction-and-shared-tooling.md:706`: "record B's private flip
(its T8) is sequence-gated on both landing"); this record extends that same
gate with its own deliverables.

## Approach

Five moves, in dependency order.

### 1. Revive `mattwilkinsonn/homebrew-tap` as the single canonical tap

Live state (verified this session): the repo is PUBLIC and **not**
GitHub-archived (`gh repo view` → `"isArchived":false` — only its README
*claims* archival), carries a single stale `Formula/jj-hooks.rb` at
`version "0.2.1"` with
`disable! date: "2026-05-26", because: "moved to the mattwilkinsonn/zireael tap"`,
and has **no rulesets** (`gh api …/rulesets` → `[]`). So no un-archive admin
step exists; revival is a content PR plus admin-plane setup:

- Replace `Formula/jj-hooks.rb` with the current standalone formula
  (standalone `jj-hooks/Formula/jj-hooks.rb:4` `version "0.3.11"`, `:9`
  `url "https://github.com/mattwilkinsonn/jj-hooks/releases/download/…"` —
  the v0.3.11 assets are already mirrored onto both standalone Releases
  pages, 6 assets each, verified via `gh release view v0.3.11`). Add
  `Formula/jj-gt.rb` the same way (standalone `jj-gt/Formula/jj-gt.rb`).
- Rewrite the README: this IS the canonical tap
  (`brew tap mattwilkinsonn/tap`), with untap-then-retap migration commands
  for zireael-tap and per-repo-tap users.
- Admin plane (laptop): install the `zireael-release` App here, create a
  main-protection ruleset with the App as bypass actor (mirroring the
  standalones' "main protection" ruleset — jj-hooks ruleset id 16553812
  verified live).

### 2. Re-point each standalone's `bump-tap` at the tap repo

The core structural change. Today each standalone's `bump-tap` job bumps an
IN-REPO `Formula/` dir: `jj-hooks/.github/workflows/release.yml:234-240`
checks out `ref: main` of the SAME repo with the App token, `:281` runs
`python3 .github/scripts/bump-formulae.py` against its own `Formula/`, and
`:306` pushes `git push origin HEAD:main` to itself. `validate-tap` then taps
the repo itself (`release.yml:327`:
`brew tap mattwilkinsonn/jj-hooks https://github.com/mattwilkinsonn/jj-hooks`).
The jj-gt workflow is structurally identical
(`jj-gt/.github/workflows/release.yml:203-307,328`).

Change, per repo:

- **Scope the App token cross-repo.** The `create-github-app-token@v2` step
  (`release.yml:228-232`) currently passes only `app-id`+`private-key`, which
  mints a token scoped to the CURRENT repo — it cannot push to `homebrew-tap`
  even with the App installed there. Add `owner: mattwilkinsonn` +
  `repositories:` covering the tap repo (and self) so the minted token carries
  `contents:write` on the tap. (Tradeoff: that job's token then spans both
  repos — acceptable for a release job.)
- **Dual checkout.** Re-pointing the single `release.yml:234` checkout at the
  tap would delete the standalone's `.github/scripts/bump-formulae.py` from the
  workspace, so `python3 .github/scripts/bump-formulae.py` (`release.yml:281`)
  fails before any Formula logic. Instead keep the self checkout (the script
  lives there) and ADD a second `actions/checkout` for
  `mattwilkinsonn/homebrew-tap` under `path: tap` with the App token. Run the
  rewrite via the absolute script path with `working-directory: tap` (so
  `bump-formulae.py`'s CWD-relative `Path("Formula")` resolves to the tap's
  `Formula/`), and run the whole Commit+push step (`release.yml:282-306`) with
  `working-directory: tap` so `origin` is the tap remote. The fetch+rebase
  guard (`release.yml:304-306`) stays, but it only de-races main advancing
  under a SINGLE repo's bump — it does NOT serialize the two tools' separate
  workflows into the shared tap; that race is closed by sequencing the tag
  pushes (T6 step 4).
- `validate-tap` taps `mattwilkinsonn/tap` and installs
  `mattwilkinsonn/tap/<formula>` instead of the per-repo path
  (`release.yml:327-332`).

### 3. Harden `release.yml` for idempotent re-runs

The v0.3.11 lesson: `publish-release` hard-fails when the GitHub Release
already exists — `release.yml:199` is a bare `gh release create "$TAG" …`,
and its failure cascades into skipping the two jobs that actually needed
proving. Both workflows already have a `workflow_dispatch` re-run path
(`release.yml:12-16`) that this failure mode neuters. Fix in both repos:
check-then-create — `gh release view "$TAG" || gh release create "$TAG"
--verify-tag --title … --notes …` with NO asset args — then ALWAYS
`gh release upload "$TAG" <assets> --clobber` as a separate step, so both a
first run and a re-run refresh assets idempotently (never a partial
create-with-assets that a re-run duplicates). This makes the manual-mirror
state (release exists, assets present) a valid starting point, not a poison
pill.

### 4. Cut a real v0.3.12 through each standalone pipeline

Bump `0.3.11 → 0.3.12` in each standalone's `Cargo.toml` (+ lockfile +
CHANGELOG entry). Matt's laptop pushes the `v0.3.12` tag
(`release.yml:10-11` triggers on `tags: ["v*"]`) after the admin plane is
set. One green run per repo proves: native asset build, idempotent release
publish, App-token tap bump into the consolidated tap, tap validation on
macOS+Linux, and crates.io publish — which re-anchors the immutable crates.io
`repository` (and thus binstall's `{ repo }`) at the standalone for 0.3.12+.
jj-gt's crates dependency needs no ordering wait (its `release.yml:339-341`
comment: jj-hooks 0.3.11 is already on crates.io; the jj-gt 0.3.12 bump does
not require bumping its jj-hooks dependency).

### 5. Durable migration comms

Two surface classes survive the flip: web surfaces NOT on zireael, and a
CLIENT-durable surface that is on zireael but persists after it goes private.
Web surfaces (see OQ2): the two standalone READMEs (update the Homebrew section
— currently `jj-hooks/README.md:62-63` instructs
`brew tap mattwilkinsonn/jj-hooks …` — to `brew tap mattwilkinsonn/tap`, plus a
short "migrating from the zireael tap" untap/retap block), the tap repo's own
README (move 1), and the crates.io README (free: crates.io renders the packaged
`README.md`, so the 0.3.12 publish ships it). Client-durable surface:
`disable!`-stamp zireael's own `Formula/*.rb` pointing at `mattwilkinsonn/tap`
BEFORE the flip (T9) — a Homebrew tap is a local git clone, so a user who runs
`brew update` pre-flip pulls the stamp into their machine, where it survives
the repo going private and turns their next install/upgrade into an actionable
error naming the new tap (instead of an opaque git-fetch failure). A transient
courtesy note on zireael's README covers the pre-flip window but is not
load-bearing.

### App identity (settled)

Reuse the `zireael-release` App, extending its installation to both
standalones and the tap repo. This is the extraction record's own assumption
(`oss-tool-extraction-and-shared-tooling.md:746-748`: "extend the existing
App's installation to the new repos and reuse it… its slug appearing as
`zireael-release[bot]` in standalone commit authorship is cosmetic"). Both
workflows already mint its token (`release.yml:228-232`:
`app-id: ${{ vars.RELEASE_BOT_APP_ID }}`,
`private-key: ${{ secrets.RELEASE_BOT_PRIVATE_KEY }}`) — but three things are
missing, not two: the var/secret values, the App installations on all three
repos, AND an `owner: mattwilkinsonn` + `repositories:` scope on the
token-mint step. `create-github-app-token@v2` with only `app-id`+`private-key`
scopes the token to the CURRENT repo — insufficient to push the cross-repo
tap bump (see move 2 / T2).

## Alternatives considered

**Per-repo taps (the superseded choice).** What the extraction record chose
and what already exists (both standalone repos carry an in-repo `Formula/` at
0.3.11, and `bump-tap` is wired for it). Lost because: three tap paths in the
wild simultaneously (zireael tap, jj-hooks tap, jj-gt tap) with no durable
one; every future tool means yet another tap name for users to learn; and the
conventional `mattwilkinsonn/tap` shorthand — the shortest, most durable
name — goes unused while its repo sits pointing users at a soon-dead
monorepo. Consolidation costs one extra checkout target in `bump-tap` and
buys one tap name across all future tool churn.

**No-new-release minimal continuity.** Skip v0.3.12; rely on the mirrored
0.3.11 assets + tap re-point alone. Cheapest, but leaves the crates.io
`repository`/binstall anchor pointing at zireael forever until the next
organic release, leaves the standalone release pipeline (App mint, tap bump,
crates publish) unproven until a future release discovers its failures with
no zireael fallback, and ships nothing that validates the consolidated tap
end-to-end. Rejected: the whole point is proving the machine while the safety
net exists.

**A new dedicated release App (`dev-release` or similar).** Cleaner slug in
commit authorship, but a second App to create, key, install, and rotate — for
a purely cosmetic gain the extraction record already dismissed
(`oss-tool-extraction-and-shared-tooling.md:748-749`). Rejected; reuse
`zireael-release`.

**PAT instead of App token for the cross-repo tap push.** The pre-monorepo
`HOMEBREW_TAP_TOKEN` secret still exists on both standalones. Rejected:
long-lived PATs are the thing the App replaced (short-lived installation
tokens, least-privilege `contents:write`, ruleset bypass scoped to one
identity); reviving the PAT path would fork the auth model the workflows
already document (`release.yml:214-227`). The stale secret should be deleted
during admin cleanup.

## Global Constraints

- **Version floor:** the next published version of each crate is `0.3.12`;
  nothing re-publishes 0.3.11 (crates.io is immutable).
- **Tap:** `mattwilkinsonn/homebrew-tap`, user-facing name
  `brew tap mattwilkinsonn/tap`. It is the ONLY tap receiving bumps after
  this record lands.
- **App identity:** `zireael-release` (App ID via `vars.RELEASE_BOT_APP_ID`,
  key via `secrets.RELEASE_BOT_PRIVATE_KEY`) on all three repos. No new App,
  no PATs.
- **VCS discipline:** jj-vine is the sole push path; PRs open as drafts and
  are promoted with `gh pr ready` after review; never `gh pr create`; never a
  direct push to main; agents never merge — Matt merges.
- **Box has no admin creds** (by design). Every admin-plane step (App
  installs, secrets/vars, rulesets, tag pushes) is a LAPTOP task handed to
  Matt via a `~/notes/scratch/zireael/` runbook, never executed on the box.
- **Commits:** Conventional Commits subject; body paragraphs one-per-line;
  `Co-authored-by: Matt Wilkinson <mattwilki17@gmail.com>` trailer.
- **Sequence gate:** infra T8 (private flip) does not execute until every
  task below is done and the v0.3.12 runs are green.
- The extraction record stays frozen; this record is the only place the tap
  decision changes.

## Plan

Each task is tagged **BOX-DOABLE** (agent work, shipped as a jj-vine draft PR
per Global Constraints) or **LAPTOP** (Matt, driven by a runbook written to
`~/notes/scratch/zireael/`). Dependency order is stated per task; T1-T5 are
parallelizable except where noted, T6 gates the tag push, T7-T8 trail the
green runs.

### T1 — Revive the consolidated tap repo (BOX-DOABLE)

PR against `mattwilkinsonn/homebrew-tap`: replace the stale disabled
`Formula/jj-hooks.rb` (0.2.1, `disable!`-stamped) with the current standalone
formula at 0.3.11 (copy of `jj-hooks/Formula/jj-hooks.rb`, whose URLs already
point at the standalone Releases where the v0.3.11 assets are mirrored); add
`Formula/jj-gt.rb` (copy of `jj-gt/Formula/jj-gt.rb`); rewrite `README.md` as
the canonical-tap README (`brew tap mattwilkinsonn/tap`, install commands for
both formulae, and an explicit migration block: `brew untap
mattwilkinsonn/zireael` / `brew untap mattwilkinsonn/jj-hooks` / `brew untap
mattwilkinsonn/jj-gt`, then retap). Note: the repo is NOT GitHub-archived
(verified: `isArchived: false`) — only its README claims archival — so no
un-archive admin step exists; this PR alone revives it.

Interfaces:

- Consumes: `jj-hooks/Formula/jj-hooks.rb`, `jj-gt/Formula/jj-gt.rb`
  (v0.3.11, standalone URLs); the mirrored v0.3.11 release assets on both
  standalone Releases pages.
- Produces: `mattwilkinsonn/homebrew-tap` `Formula/{jj-hooks,jj-gt}.rb` @
  0.3.11 + canonical README. `brew tap mattwilkinsonn/tap` installable at
  0.3.11 immediately on merge.
- Depends on: the bot having push to `homebrew-tap` — SATISFIED (T6 step 0
  done; `push:true` verified). Blocks: T6 (ruleset needs the repo
  live-shaped) and the v0.3.12 `validate-tap` legs.

### T2 — Harden + re-point jj-hooks `release.yml` (BOX-DOABLE)

PR against `mattwilkinsonn/jj-hooks`, two concerns in one workflow file:

1. **Idempotent re-run:** replace the bare `gh release create` in
   `publish-release` (`release.yml:199-201`) with check-then-create
   (`gh release view "$TAG" || gh release create …`) plus
   `gh release upload "$TAG" release/jj-hooks-v*.tar.gz
   release/jj-hooks-v*.sha256 --clobber`, so a `workflow_dispatch` re-run
   (`release.yml:12-16`) over an existing release refreshes assets instead
   of failing (the v0.3.11 failure mode).
2. **Tap re-point (three coordinated changes, per Approach §2 — a naive
   single-checkout re-point ships a red run):**
   - **Token scope:** extend the `create-github-app-token@v2` step
     (`release.yml:228-232`) with `owner: mattwilkinsonn` + `repositories:`
     covering `homebrew-tap` — without it the minted token is CURRENT-repo
     scoped and the tap push 403s.
   - **Dual checkout:** keep the self checkout (`release.yml:234`, for
     `.github/scripts/bump-formulae.py`) and ADD a second `actions/checkout`
     for `mattwilkinsonn/homebrew-tap` at `path: tap` with the App token. Run
     the rewrite (`release.yml:273-281`) via the absolute script path with
     `working-directory: tap`, and run the Commit+push step
     (`release.yml:282-306`) with `working-directory: tap` so `origin` is the
     tap remote. Add a pre-bump guard: fail if `tap/Formula/jj-hooks.rb` is
     absent or still contains `disable!` (catches a premature run over the
     un-revived tap — see the T1 gate).
   - **validate-tap:** change `release.yml:327-332` to `brew tap
     mattwilkinsonn/tap`, `brew trust mattwilkinsonn/tap`, install/test
     `mattwilkinsonn/tap/jj-hooks`. Update the in-file comments that still say
     "Installed on: this repo only" and describe the in-repo bump.

Interfaces:

- Consumes: `jj-hooks/.github/workflows/release.yml` (jobs
  `publish-release`, `bump-tap`, `validate-tap`),
  `jj-hooks/.github/scripts/bump-formulae.py` (unchanged logic; invoked
  against the tap checkout).
- Produces: a `release.yml` whose tag run publishes idempotently and bumps
  `mattwilkinsonn/homebrew-tap` instead of the in-repo `Formula/`.
- Depends on: nothing to merge (T1 must be merged before a *run* can
  validate). Blocks: T6 tag push.

### T3 — Harden + re-point jj-gt `release.yml` (BOX-DOABLE)

Same changes as T2 against `mattwilkinsonn/jj-gt`'s structurally identical
workflow — idempotent `publish-release`
(`jj-gt/.github/workflows/release.yml:198-200`), the SAME three coordinated
`bump-tap` changes (token `owner`/`repositories` scope at `release.yml:227-231`,
dual checkout with `working-directory: tap` at `:233-245`/`:283-307`, pre-bump
guard on `tap/Formula/jj-gt.rb`), and `validate-tap` → `mattwilkinsonn/tap`
(`release.yml:328-335`). jj-gt's script only touches `jj-gt.rb` (its per-tool
`TOOLS` dict, `bump-formulae.py:32-34`), so the two tools never clobber each
other's formula in the shared tap.

Interfaces:

- Consumes: `jj-gt/.github/workflows/release.yml` (same three jobs),
  `jj-gt/.github/scripts/bump-formulae.py`.
- Produces: jj-gt release pipeline targeting the consolidated tap.
- Depends on / blocks: as T2. (OQ1 resolved: cut jj-gt v0.3.12 now,
  alongside jj-hooks.)

### T4 — jj-hooks v0.3.12 bump + durable comms (BOX-DOABLE)

PR against `mattwilkinsonn/jj-hooks`: `Cargo.toml:4` `version = "0.3.12"`
(+ `Cargo.lock`), CHANGELOG entry (distribution-only release: re-anchors
crates.io `repository`/binstall at the standalone, first release through the
consolidated tap), and the README comms — rewrite the Homebrew section
(`README.md:59-64`) from `brew tap mattwilkinsonn/jj-hooks …` to `brew tap
mattwilkinsonn/tap` + `brew install mattwilkinsonn/tap/jj-hooks`, and add a
short "Migrating from the zireael monorepo tap" subsection (untap/retap +
the note that binstall for ≤0.3.11 falls back to source build). Because
crates.io renders the packaged README, the 0.3.12 publish carries these
comms onto crates.io for free.

Interfaces:

- Consumes: `jj-hooks/Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`,
  `README.md`.
- Produces: a main ready to tag `v0.3.12`; durable comms on the standalone
  README and (post-publish) crates.io.
- Depends on: T2 merged first (the tag must run the re-pointed workflow).
  Blocks: T6 tag push.

### T5 — jj-gt v0.3.12 bump + durable comms (BOX-DOABLE)

Mirror of T4 against `mattwilkinsonn/jj-gt`: `Cargo.toml:4` → `0.3.12`
(the `jj-hooks` dependency pin stays — 0.3.11 is on crates.io; no ordering
wait, per `release.yml:339-341`), CHANGELOG, README Homebrew section
(`README.md:110-115`) + migration subsection.

Interfaces:

- Consumes: `jj-gt/Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `README.md`.
- Produces: jj-gt main ready to tag `v0.3.12`.
- Depends on: T3 merged. Blocks: T6 tag push (jj-gt leg). (OQ1 resolved:
  both crates ship 0.3.12 pre-flip.)

### T6 — Admin plane + tag push (LAPTOP)

Runbook at `~/notes/scratch/zireael/distribution-continuity-runbook.md`
(box-writable; syncs to the laptop), authored by the agent as part of this
record's execution, executed by Matt. Ordered steps:

0. **Grant push to the tap repo** — DONE (Matt added `rigel-mintaka` as a
   write collaborator on `mattwilkinsonn/homebrew-tap`, verified `push:true`),
   so T1's revive PR pushes from the box like any other. (The release-time tap
   push uses the App token, not this grant; this is only for the agent's
   jj-vine PR branch.)
1. Install the `zireael-release` App
   (<https://github.com/apps/zireael-release> → settings) on
   `mattwilkinsonn/jj-hooks`, `mattwilkinsonn/jj-gt`,
   `mattwilkinsonn/homebrew-tap`.
2. On both standalones: set repo variable `RELEASE_BOT_APP_ID`, secret
   `RELEASE_BOT_PRIVATE_KEY`; verify `CARGO_REGISTRY_TOKEN` is valid
   (present but dated 2026-05 — re-mint if expired); delete the stale
   `HOMEBREW_TAP_TOKEN` secret (dead auth path).
3. On the tap repo: create a main-protection ruleset (mirror the
   standalones' "main protection", e.g. jj-hooks ruleset id 16553812) with
   `zireael-release` as bypass actor.
4. After T1-T5 are merged, push the tags ONE AT A TIME to avoid the shared-tap
   race (the two workflows aren't serialized): push jj-hooks `v0.3.12`, watch
   its `release` run to green (bump-tap has pushed `jj-hooks.rb` to the tap),
   THEN push jj-gt `v0.3.12` and watch it green. If a leg fails, fix-forward on
   the box and re-run via `workflow_dispatch` (now idempotent per T2/T3).

Interfaces:

- Consumes: the runbook file; App admin access; the merged T1-T5 mains.
- Produces: App installed + creds set on 3 repos; tap ruleset; two green
  `release` runs; jj-hooks/jj-gt 0.3.12 on crates.io with
  `repository` → standalone; consolidated tap bumped to 0.3.12 by
  `zireael-release[bot]`.
- Depends on: T1 (ruleset target), T2-T5 merged (before step 4). Blocks:
  T7, T8, and infra T8.

### T7 — Retire the per-repo standalone taps (BOX-DOABLE, post-green)

Per OQ3's recommendation: after the v0.3.12 runs are green, one PR per
standalone stamping its in-repo `Formula/<tool>.rb` with `disable!
date: …, because: "moved to the mattwilkinsonn/tap tap"` (the exact
pattern the old conventional tap used at 0.2.1), so per-repo-tap users get
an actionable error instead of a silently stale formula. `bump-tap` no
longer touches these files after T2/T3, so without this they rot silently.

Interfaces:

- Consumes: `jj-hooks/Formula/jj-hooks.rb`, `jj-gt/Formula/jj-gt.rb`.
- Produces: disabled per-repo formulae pointing at `mattwilkinsonn/tap`.
- Depends on: T6 green. Blocks: nothing (not on the T8 gate).

### T8 — Confirm the sequence gate to the infra record (BOX-DOABLE)

Verify and record (in the tracking issue for this record) that all gate
conditions hold: 0.3.12 live on crates.io for both crates with
`repository` → standalone, `brew tap mattwilkinsonn/tap` installs 0.3.12,
comms live on both standalone READMEs + tap README. Only then may infra T8
(`personal-infra-monorepo.md:537`) proceed.

Interfaces:

- Consumes: T6's green runs; crates.io API; a fresh `brew` check.
- Produces: the go-signal infra T8 waits on.
- Depends on: T6 (T7 explicitly NOT required).

### T9 — Client-durable retirement of the zireael tap (BOX-DOABLE, pre-flip)

PR against `mattwilkinsonn/zireael`: `disable!`-stamp its in-repo
`Formula/jj-hooks.rb` and `Formula/jj-gt.rb` with
`because: "moved to the mattwilkinsonn/tap tap"` (the exact pattern the old
conventional tap and T7 use), pointing current zireael-tap users at
`brew tap mattwilkinsonn/tap`. Because a tap is a local git clone, a user who
runs `brew update` before the flip pulls the stamp locally, where it persists
after zireael goes private — the only migration surface that reaches existing
zireael-tap users AFTER the flip. Pairs with T7 (which retires the never-used
per-repo taps); this retires the tap that has real users.

Interfaces:

- Consumes: `zireael/Formula/jj-hooks.rb`, `zireael/Formula/jj-gt.rb`.
- Produces: disabled zireael-tap formulae pointing at `mattwilkinsonn/tap`,
  merged to zireael main before the flip.
- Depends on: T1 (the pointed-to tap should be live). Blocks: nothing on the
  v0.3.12 green gate, but MUST merge before infra T8 (it is a pre-flip
  user-facing comm). Not gated on the green runs.

## Tasks

- [ ] T1: tap repo revived — both formulae @0.3.11 + canonical README
      (BOX-DOABLE)
- [ ] T2: jj-hooks release.yml idempotent + bump-tap/validate-tap →
      consolidated tap (BOX-DOABLE)
- [ ] T3: jj-gt release.yml idempotent + re-pointed (BOX-DOABLE, per OQ1)
- [ ] T4: jj-hooks v0.3.12 bump + CHANGELOG + README comms (BOX-DOABLE)
- [ ] T5: jj-gt v0.3.12 bump + CHANGELOG + README comms (BOX-DOABLE, per
      OQ1)
- [ ] T6: admin runbook executed — App installs, vars/secrets, tap ruleset,
      tag pushes, green runs (LAPTOP)
- [ ] T7: per-repo standalone formulae disable!-stamped (BOX-DOABLE,
      post-green)
- [ ] T8: sequence gate confirmed to the infra record (BOX-DOABLE)
- [ ] T9: zireael tap formulae disable!-stamped → mattwilkinsonn/tap
      (BOX-DOABLE, pre-flip, client-durable)

## Open Questions

Each is designed against the stated assumption; none stalls the box-doable
work. OQ1 was put to Matt and RESOLVED (below); OQ2-OQ4 stand as designed.

1. **Cut jj-gt v0.3.12 now, or lazily on its next feature release?**
   **RESOLVED (Matt): now — cut both v0.3.12 together (T3+T5).**
   LOAD-BEARING — it changes the task set and the T8 gate. The deferral
   rejected: cutting jj-gt lazily instead means
   its crates.io `repository`/binstall anchor stays pointed at
   soon-private zireael for every install until that future release; its
   pipeline (App mint, tap bump, crates publish — all currently unproven)
   gets its first real test only after the zireael fallback is gone; and the
   consolidated tap carries a jj-gt formula no pipeline has ever bumped. The
   marginal cost of doing it now is one small PR (T5) — T3 and the T6 admin
   steps are needed for jj-gt's *next* release regardless. Assumption
   designed against: both crates ship 0.3.12 pre-flip.
2. **Migration-comms surface set.** **Recommendation: three web-durable
   surfaces PLUS one client-durable surface.** Web: both standalone READMEs
   (T4/T5), the tap repo README (T1), and crates.io via the packaged README
   (free with the 0.3.12 publish). Client-durable: `disable!`-stamp zireael's
   own `Formula/*.rb` pre-flip (T9) — it lives on soon-private zireael but a
   tap is a local clone, so a pre-flip `brew update` carries the pointer onto
   the user's machine where it survives the flip. This is the ONLY surface that
   reaches existing zireael-tap users after the flip; an earlier draft wrongly
   classified all zireael surfaces as transient. Still NOT load-bearing: a
   zireael pinned issue or final-release note (invisible post-flip, serve only
   the pre-flip window). T9 is DEFERRABLE off the green gate but MUST land
   before infra T8.
3. **Retire the per-repo standalone taps or leave as silent dupes?**
   **Recommendation: retire explicitly (T7)** with the same `disable!` +
   pointer pattern the old conventional tap used — a user who tapped
   `mattwilkinsonn/jj-hooks` gets an error naming the new tap instead of
   silently pinning to 0.3.11 forever. Deleting `Formula/` outright is
   worse (brew reports the formula as vanished, no pointer). DEFERRABLE —
   post-green cleanup, not on the flip gate.
4. **Is a crates.io README/`repository` re-point worth a version bump on its
   own?** **Recommendation: moot under the plan** — 0.3.12 (T4/T5) carries
   it as a side effect; never bump solely for metadata. The question only
   re-arises if OQ1 is answered "jj-gt lazy", in which case the answer is
   still no: accept the stale jj-gt metadata until its next organic release
   (that acceptance is the real cost of "lazy", stated in OQ1). DEFERRABLE.

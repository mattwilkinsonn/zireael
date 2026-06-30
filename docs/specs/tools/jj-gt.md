# jj-gt

## Overview

`jj-gt` is the bridge between a [jj](https://jj-vcs.github.io)
bookmark stack and a [Graphite](https://graphite.dev/) (`gt`) PR
stack. jj never writes the `refs/branch-metadata/<branch>` parent
records that gt's stack model depends on, so a plain `gt submit`
targets every PR at `main`. jj-gt closes that gap: it walks jj's
revset graph to derive `(branch, parent_branch)` pairs from the
bookmarks, registers them with `gt track`, then drives
`gt submit --stack` end to end — "a jj-aware replacement for
`gt track`/`gt sync`."

Parent derivation is one revset per selected bookmark
(`stack.rs:1-3` — "Stack derivation: walk jj's revset graph to
figure out (bookmark, parent_bookmark) pairs for a set of selected
bookmarks"). `derive_one` builds it as
`stack.rs:90-94` — `"heads(::{b} & bookmarks() ~ {b} ~ ::{trunk})"`:
the head bookmark(s) that are ancestors of `B`, excluding `B` and
everything already under trunk. Zero parents means the bookmark
sits on trunk (`stack.rs:97-100` — `[] => Ok(StackedBookmark { …
parent: BookmarkOrTrunk::Trunk })`), one is the parent
(`stack.rs:101-104`), and more than one (a merge commit) is a hard
error (`stack.rs:105-111` — `"multiple parent bookmarks found …
Specify --parent manually."`).

The surface spans four jobs:

| Command | Role |
| --- | --- |
| `submit` | Track + drive `gt submit --stack`; reconcile, hoist links, restore `@`. |
| `track` | Sync `refs/branch-metadata/*` only — submit minus the `gt submit`. |
| `status` | Print stack-wide PR + queue state in one `gh` query. |
| `fetch` | Graphite-aware `jj git fetch`: trunk fetch, backfill, `gt sync`, rebase, prune. |

`submit`, `reconcile`, `fetch`, and `restack` are the mutating
flows; `status` and `log` are read-only. Hook execution itself is
delegated to the `jj_hooks` library
(`hooks.rs:1-2` — "Thin wrapper around
[`jj_hooks::hooks::run_for_update`] and its batch siblings"); jj-gt
owns only the per-bookmark orchestration.

## CLI surface

The binary is `clap`-derived (`cli.rs:13-35` — `struct Cli`). Two
global flags apply to every subcommand: `--log-level`
(`cli.rs:22-23`, env `JJ_GT_LOG`, default `"warn"`) and
`-v/--verbose` (`cli.rs:30-31` — "Print the full subprocess output
of every step").

### Subcommands

The `Command` enum (`cli.rs:37-275`) declares:

| Subcommand | Line | Purpose |
| --- | --- | --- |
| `submit` | `cli.rs:53` | Track + submit selected bookmarks as a stack. |
| `track` | `cli.rs:107` | Sync metadata refs without `gt submit`. |
| `fetch` | `cli.rs:128` | Graphite-aware replacement for `jj git fetch`. |
| `status` | `cli.rs:172` | Print stack-wide PR + queue state. |
| `log` | `cli.rs:185` | Print the derived stack (debug). |
| `reconcile` | `cli.rs:195` | Reconcile gt metadata + remote refs with jj. |
| `restack` | `cli.rs:230` | Rebase every local stack onto trunk. |
| `init` | `cli.rs:263` | Print aliases + optionally install jjui actions. |
| `completions` | `cli.rs:271` | Emit a shell completion script. |

### Bookmark selection (`BookmarkArgs`)

`submit`, `track`, and `status` share the flattened `BookmarkArgs`
(`cli.rs:279-322`):

| Flag | Line | Effect |
| --- | --- | --- |
| `-b/--bookmark` | `cli.rs:282-288` | Operate on this bookmark (repeatable). |
| `-r/--revision` | `cli.rs:291-292` | Bookmarks pointing at these commits. |
| `-c/--change` | `cli.rs:296-297` | Bookmarks pointing at these commits — selected like `-r` (`select.rs` queries `bookmarks() & (<revset>)`); an *unbookmarked* change matches nothing and errors, it does not create a bookmark. |
| `--all` | `cli.rs:304-305` | Every bookmark across every stack (`bookmarks() & trunk..`), minus trunk + `gtmq_*`. |
| `--tracked` | `cli.rs:308-309` | Every locally-tracked bookmark. |
| `--allow-new` | `cli.rs:312-313` | Allow bookmarks with no remote counterpart. |
| `--remote` | `cli.rs:316-321` | Remote to push to / read tracking from (default `origin`). |

### Submit passthrough (`SubmitArgs`)

`submit` flattens `SubmitArgs` (`cli.rs:328-427`) — flags that map
onto `gt submit` via `gt::build_submit_argv`:

| Flag | Line | Effect on `gt submit` argv |
| --- | --- | --- |
| `--draft` | `cli.rs:332-333` | Adds `--draft`, suppresses default `--publish`. |
| `--no-publish` | `cli.rs:336-337` | Suppresses default `--publish` without `--draft`. |
| `--restack` | `cli.rs:340-341` | Adds `--restack`. |
| `-n/--no-edit` | `cli.rs:344-345` | Adds `--no-edit`. |
| `--ai` | `cli.rs:348-349` | Adds `--ai`. |
| `--no-ai` | `cli.rs:352-353` | Adds `--no-ai`. |
| `-R/--reviewers` | `cli.rs:356-357` | Adds `--reviewers <csv>`. |
| `-t/--team-reviewers` | `cli.rs:360-361` | Adds `--team-reviewers <csv>`. |
| `-u/--update-only` | `cli.rs:365-366` | Adds `--update-only`. |
| `-m/--merge-when-ready` | `cli.rs:369-370` | Adds `--merge-when-ready`. |
| `--target-trunk` | `cli.rs:373-374` | Adds `--target-trunk <name>`. |
| `--view` | `cli.rs:377-378` | Adds `--view`. |
| `-w/--web` | `cli.rs:381-382` | Adds `--web`. |
| `--comment` | `cli.rs:385-386` | Adds `--comment <text>`. |
| `--rerequest-review` | `cli.rs:389-390` | Adds `--rerequest-review`. |
| `--no-always` | `cli.rs:400-401` | Suppresses default `--always`. |
| `-f/--force` | `cli.rs:404-405` | Adds `--force` (overrides force-with-lease). |
| `--dry-run` | `cli.rs:408-409` | Adds `--dry-run` and skips jj-gt's own later mutations — but `submit` still runs `jj git export` first unless `--no-export` (`lib.rs:424-428`), so refs can move. |
| `-C/--confirm` | `cli.rs:412-413` | Adds `--confirm`. |
| `--no-links` | `cli.rs:421-422` | Skip the issue-reference / co-author hoist. |
| `--gt-arg` | `cli.rs:425-426` | Append a verbatim arg to `gt submit` (repeatable). |

Submit also takes flow-control flags that do **not** map onto
`gt submit`: `--trunk` (`cli.rs:61-62`), `--no-export`
(`cli.rs:65-66`), `--no-restore-cwc` (`cli.rs:71-72`), `--no-hooks`
(`cli.rs:76-77`), `--hooks-tip-only` (`cli.rs:85-86`,
`conflicts_with = "no_hooks"`), `--hooks-sequential`
(`cli.rs:96-97`, `conflicts_with_all = ["no_hooks",
"hooks_tip_only"]`), and `--hk-runner` (`cli.rs:101-102`).

### Trunk resolution

Every mutating command resolves trunk the same way
(`status.rs:154-162`): an explicit `--trunk` wins
(`status.rs:155-157`), else gt's repo config
(`status.rs:158-159` — `crate::gt::read_repo_config_trunk`, which
reads `.git/.graphite_repo_config`, `gt.rs:183-197`), else the
literal `"main"` (`status.rs:161` — `Ok("main".to_owned())`).

## Requirement: Submit drives a jj-aware gt stack push

`jj-gt submit` SHALL derive the stack from jj, register parents
with gt bottom-to-top, push via `gt submit --stack`, and leave the
working copy where it found it. The orchestration lives in
`submit_cmd` (`lib.rs:334-806`).

### Scenario: Sheltering uncommitted edits before mutating refs

- **GIVEN** the working copy `@` has uncommitted changes
- **WHEN** `jj-gt submit` runs (not `--dry-run`)
- **THEN** it SHALL run `jj new @` to shelter those edits behind a
  fresh empty `@` before touching any ref.

`submit_cmd` gates the shelter on
`lib.rs:413` — `if !submit.dry_run && jj::has_uncommitted_changes(jj)?`
and reports it as the step `lib.rs:414` — `"Sheltering uncommitted
edits (jj new @)"`. The shelter itself is
`jj.rs:277-279` — `pub fn shelter_uncommitted_edits(jj: &JjCli) ->
Result<()> { let _ = jj_run(jj, &["new", "@"])?; Ok(()) }`. The
rationale is recorded inline (`lib.rs:399-406` — "`jj new @`
snapshots those edits into the (now-frozen) old `@` and leaves us
operating on a clean empty `@` above them"). Dry-run skips it to
keep the "zero workspace mutations" promise (`lib.rs:408-412`).

### Scenario: Exporting bookmarks, then tracking parents bottom-up

- **GIVEN** a derived, partitioned stack
- **WHEN** submit proceeds past the shelter
- **THEN** it SHALL `jj git export` the bookmarks to git refs
  (unless `--no-export`), then `gt track` each bookmark with its
  derived parent in bottom-to-top order.

After sheltering, submit exports unless `--no-export`
(`lib.rs:425` — `if !no_export`), calling
`jj.rs:227-229` — `jj_run(jj, &["git", "export",
"--ignore-working-copy"])`. The selection is resolved
(`lib.rs:358` — `select::resolve_bookmarks`), expanded to the full
ancestor chain (`lib.rs:365` —
`select::expand_ancestors_for_submit(jj, &selected, &trunk)`),
parent-derived (`lib.rs:366` — `stack::derive_parents`), and split
into independent stacks (`lib.rs:370` —
`stack::partition_stacks(&stacked)`). Each partition is then
tracked in order (`lib.rs:562-579`); the parent passed to gt is the
derived parent, falling back to the trunk name for a bottom
bookmark (`lib.rs:563` — `let parent =
sb.parent.as_branch_name(&trunk);`, resolving via
`stack.rs:33-39` — `BookmarkOrTrunk::Trunk => trunk`).

`gt::track` is untrack-then-track so a re-parented bookmark
overwrites stale metadata
(`gt.rs:43` — `let _ = run_gt_captured(workspace_root, &["untrack",
branch, "--no-interactive"]);` then
`gt.rs:44-47` — `&["track", branch, "--parent", parent,
"--no-interactive"]`). Bottom-up ordering is mandatory because
`gt track <child> --parent <parent>` errors if the parent isn't
tracked yet (`stack.rs:249-253`); `partition_stacks` sorts each
partition via `sort_for_tracking` (`stack.rs:163-167`).

### Scenario: Submitting each partition via `gt submit --stack`

- **GIVEN** every bookmark in a partition is tracked
- **WHEN** submit reaches the push step
- **THEN** it SHALL run `gt submit --stack --branch <tip>` once per
  partition, with publish on by default and `--always` /
  `--no-verify` appended.

The push loop is `lib.rs:635-661`, building argv via
`lib.rs:649` — `let argv = gt::build_submit_argv(tip, &submit);`
and invoking `lib.rs:653` — `gt::submit(&workspace_root, &argv)`.
`build_submit_argv` always opens with
`gt.rs:61-66` — `vec!["submit".into(), "--stack".into(),
"--branch".into(), tip.into()]`.

Publish/draft is a three-way default
(`gt.rs:68-73`): `--draft` if `submit.draft`, else `--publish`
unless `submit.no_publish` — so **publish is the default**. Two
further defaults are forced on: `--always` unless `--no-always`
(`gt.rs:130-132` — `if !submit.no_always { argv.push("--always") }`,
so gt re-pushes every PR's base ref even when it thinks nothing
changed, `gt.rs:118-129`), and `--no-verify` unconditionally
(`gt.rs:143-146` — "Don't let gt re-run pre-push hooks — we ran
them already against the right revset"). jj-gt forwards `--ai` /
`--no-ai` only when set (`gt.rs:81-86`); when neither flag is
passed, gt keeps its own default.

Failures abort the run; already-submitted partitions stay
submitted because gt has no rollback (`lib.rs:632-634` — "Aborts on
the first failure — partitions already submitted stay submitted (gt
has no rollback; we don't either)").

### Scenario: Restoring `@` after the push

- **GIVEN** `--no-restore-cwc` was not passed
- **WHEN** gt's git-push triggers a jj ref-import that shifts `@`
- **THEN** submit SHALL restore `@` to the change recorded before
  submit, but only if it actually moved.

`@` is recorded before the push (`lib.rs:585-589` —
`Some(jj::current_change_id(jj)?)`, `None` when `no_restore_cwc`).
After hoisting and the track sweep, restore runs only on an actual
move (`lib.rs:792-796` — `if current.as_deref() !=
Some(saved.as_str())` then `jj::edit_change(jj, &saved)`), so a
no-op re-submit doesn't print spurious "Restoring" noise
(`lib.rs:788-791`).

### Scenario: Pre-submit reconcile and post-submit track sweep

Between tracking and pushing, submit reconciles gt metadata + remote
refs across all stacks (`lib.rs:596-630` —
`reconcile::reconcile(...)`), warning rather than aborting on
failure (`lib.rs:627-629` — `tracing::warn!("jj-gt: reconcile step
failed: {e}")`). After the push it sweeps every local bookmark and
`jj`-tracks any that have a remote ref but no tracking link
(`lib.rs:688-786`, gated on `!submit.dry_run`), skipping trunk
(`lib.rs:724`), `gtmq_*` branches (`lib.rs:727`), already-tracked
bookmarks (`lib.rs:730-733`), and pre-push WIP with no remote ref
(`lib.rs:743-751`).

## Requirement: Issue-reference and co-author hoisting

After `gt submit` creates/updates the PRs, `jj-gt submit` SHALL
hoist magic-word issue references AND `Co-authored-by:` trailers
from each bookmark's commit range into a machine-managed block at
the end of the PR description — unless `--no-links` is passed. This
exists because a Graphite squash-merge sets the merge body to the PR
title + description, so references and trailers living only in the
source commits are otherwise dropped on merge (`links.rs:4-19`).

The step runs at `lib.rs:669-671` — `if !submit.dry_run &&
!submit.no_links { hoist_links_step(...) }`. It is non-fatal: every
per-bookmark failure is a warning, never an abort
(`lib.rs:816-817` — "All failures are non-fatal warnings").

### Scenario: Hoisting and normalizing issue references

- **GIVEN** a bookmark's commit messages contain `Closes SEA-1`,
  `Fixes #42`, or `Refs DES-9`
- **WHEN** the hoist step runs
- **THEN** it SHALL emit one normalized line per distinct issue,
  `Closes <id>` when any commit closed it, else `Refs <id>`.

`extract_references` scans each message line for a magic phrase and
collects raw `(word, list)` pairs (`links.rs:244-254`), then
`group_and_normalize` collapses them (`links.rs:289-291` — "Closing
intent wins per issue; first-seen order preserved"). Closing vs
non-closing intent comes from two word sets:
`CLOSING_WORDS` (`links.rs:43-64` — `"close", "closes", … "fix",
"fixes", … "resolve", …`) and `NONCLOSING_WORDS`
(`links.rs:69-78` — `"ref", "refs", "references", "part of", …`).
The phrase regex is alternation-sorted longest-first with `\b`
anchors (`links.rs:141` — `r"(?im)\b(?P<word>{alt})\b[:\s]+(?P<list>
.+)$"`). A line renders as
`links.rs:97-103` — `Closes {id}` / `Refs {id}`; an unparseable
reference (a bare tracker URL) is hoisted verbatim
(`links.rs:104` — `HoistedRef::Verbatim(text) => text.clone()`).

### Scenario: Hoisting co-author trailers, deduped, rendered last

- **GIVEN** commits carry `Co-Authored-By: seal <…>` trailers
- **WHEN** the hoist step runs
- **THEN** it SHALL emit one trailer per distinct co-author
  (deduped by identity), placed in the final paragraph so a squash
  body keeps them parseable as trailers.

`extract_coauthors` matches every `co-authored-by:` line
case-insensitively (`links.rs:165-167` —
`r"(?im)^\s*co-authored-by:\s*(?P<value>.+?)\s*$"`) and dedupes by a
lowercased identity key, emitting the first-seen verbatim value
(`links.rs:280-283`). They render after the close fence — never
inside it — so the trailing run stays a contiguous GitHub-trailer
block (`links.rs:336-340`, `links.rs:524-525`).

### Scenario: Resolving the PR and writing only on change

- **GIVEN** a bookmark with references or co-authors
- **WHEN** the hoist step processes it
- **THEN** it SHALL read the bookmark's commit range, find the open
  PR, reconcile its body, and write back only if the body changed.

Per bookmark, `hoist_links_step` reads `parent..name` messages
(`lib.rs:834` — `jj::commit_messages_in_range(jj, parent,
&sb.name)`), extracts both kinds (`lib.rs:841-842`), and skips when
both are empty (`lib.rs:843-846`). It resolves the PR
(`lib.rs:847` — `gh::find_pr_for_branch`, which is
`gh pr list --head <branch> --state all … --limit 1`,
`gh.rs:90-115`), reads the body (`lib.rs:858` — `gh::pr_body`,
`gh pr view <n> --json body`, `gh.rs:195-208`), reconciles
(`lib.rs:868` — `links::reconcile_body(&body, &refs, &coauthors)`),
and writes back **only on a diff**
(`lib.rs:869-872` — `if new_body == body { unchanged += 1;
continue; }`) via `lib.rs:873` — `gh::set_pr_body` (`gh pr edit <n>
--body-file -`, `gh.rs:210-220`). A bookmark with no open PR is
silently counted, not failed (`lib.rs:849-851`).

## Requirement: Managed-block reconciliation preserves hand-written body

`reconcile_body` SHALL strip any prior managed content, preserve the
hand-written prose, union co-authors so manual attribution is never
dropped, and regenerate the fenced block idempotently. The managed
region is delimited by `links.rs:38-39` —
`BLOCK_OPEN = "<!-- jj-gt:links -->"` /
`BLOCK_CLOSE = "<!-- /jj-gt:links -->"`.

### Scenario: Round-trip idempotence

- **GIVEN** a previously-reconciled PR body
- **WHEN** `reconcile_body` runs again with the same inputs
- **THEN** the prose SHALL be unchanged and the block regenerated
  identically.

`reconcile_body` strips first (`links.rs:527` — `let (prose,
existing_coauthors) = strip_managed(body);`). `strip_managed` peels
the trailing co-author run, then the fence
(`links.rs:410-413` — `let (without_trailers, existing) =
strip_trailing_coauthors(body); (strip_fence(&without_trailers),
existing)`), documented idempotent
(`links.rs:394-395` — "feeding a previously-reconciled body back in
yields the original prose"). `strip_fence` cuts from the open marker
to after the close marker, tolerating a truncated block
(`links.rs:471-485`). It then re-emits prose, a fresh fenced block,
and the trailer paragraph as `\n\n`-joined sections, omitting any
empty one (`links.rs:534-545`).

### Scenario: Hand-written co-author trailers survive

- **GIVEN** a human typed `Co-Authored-By: …` into the PR body and
  the commits carry none
- **WHEN** `reconcile_body` runs
- **THEN** that trailer SHALL still appear in the regenerated block.

`strip_trailing_coauthors` only peels a paragraph composed *solely*
of trailer lines (`links.rs:415-419` — "A paragraph with any
non-trailer line is left untouched"), returning the extracted values
(`links.rs:442-455`). Those existing values are unioned ahead of the
hoisted ones (`links.rs:528` — `let all_coauthors =
union_coauthors(&existing_coauthors, coauthors);`).
`union_coauthors` dedupes case-insensitively, `existing` first
(`links.rs:492-500` — `for value in
existing.iter().chain(hoisted.iter()) { if
seen.insert(value.to_ascii_lowercase()) { out.push(value.clone()) }
}`), so the human's attribution is preserved and order-stable.

### Scenario: AI vs non-AI descriptions

The hoist is independent of gt's `--ai` choice: it reads the
authoritative source (commit messages) and reconciles into whatever
body gt produced (`links.rs:11-19`). When the AI body already
contains a hoisted line verbatim as a standalone line, the managed
block suppresses the duplicate — `body_already_has` matches only an
exact trimmed line (`links.rs:381-383` — `body.lines().any(|l|
l.trim() == line)`), so `Closes SEA-12` does not satisfy
`Closes SEA-1` and prose mentioning the line does not suppress it.

## Requirement: Pre-push hooks run per bookmark

Unless `--no-hooks` is set, `jj-gt submit` SHALL gate the push on
pre-push hooks. The default is one run per bookmark over its own
diff slice, executed in parallel; `--hooks-sequential` serializes
with live output and `--hooks-tip-only` collapses to one run per
stack tip. Orchestration is `hooks.rs:117-333`
(`run_pre_push_stack`); the dispatch lives in `lib.rs:445-549`.

### Scenario: Per-bookmark diff slicing

- **GIVEN** a bottom-to-top partition `[bottom, mid, head]`
- **WHEN** the per-bookmark gate builds its updates
- **THEN** each bookmark's hook diff SHALL run from its parent's
  tip (bookmark 0 from trunk), not the whole `trunk..tip` range.

`run_pre_push_stack` chains the from-ref: `prev_tip` starts at
trunk and advances per bookmark
(`hooks.rs:139-149` — `let mut prev_tip = trunk_commit.to_owned();
… updates.push(build_update(remote, name, &prev_tip, tip)); …
prev_tip = tip.clone();`). A bookmark sitting at its parent's commit
is skipped (`hooks.rs:141-144`). `build_update` makes the
old/new refs the diff window
(`hooks.rs:29-38` — "`--from-ref <trunk> --to-ref <tip>` so every
file changed across the bookmark's diff is in scope"). This contrasts
with `--hooks-tip-only`, which runs one `trunk..tip` range per stack
and can mask an intermediate-commit failure that CI would catch
(`cli.rs:79-84`).

### Scenario: Parallel by default, ephemeral worktree per bookmark

- **GIVEN** a multi-bookmark stack and no `--hooks-sequential`
- **WHEN** the gate runs
- **THEN** every bookmark's hooks SHALL run concurrently with output
  captured, each in its own ephemeral worktree.

The default is parallel: submit computes
`lib.rs:522` — `let parallel = !hooks_sequential;` and the parallel
arm sets `hooks.rs:157-161` — `RunOpts { … capture_output:
parallel }`, then calls
`hooks.rs:208-219` — `jj_hooks::hooks::run_for_partitioned_updates_parallel(...)`
behind a live spinner tracker (`hooks.rs:169-221`). The
documented contract is "one ephemeral worktree per bookmark, all
hook runners executing concurrently (output captured + replayed in
completion order)" (`cli.rs:88-95`); the worktree creation lives in
`jj_hooks`, jj-gt drives the partitioned batch. A single-bookmark
stack uses the unbatched live-output path instead
(`lib.rs:481-504`, "the batch API forces capture … unnecessary for
N=1").

### Scenario: Fail-fast scope and sequential fallback

- **GIVEN** two independent partitions A and B run in parallel
- **WHEN** a bookmark in A fails
- **THEN** A's remaining hooks SHALL cancel but B SHALL run to
  completion.

Fail-fast is per-partition by design
(`hooks.rs:106-110` — "If bookmark `mid` in stack A fails fmt, stack
A's `head` cancels its remaining hk steps — but stack B keeps
running"). The sequential path has **no** fail-fast — it flattens
all partitions and runs them with live output
(`hooks.rs:222-243` — "no fail-fast … aborting a 5-bookmark run on
bookmark 2 would surprise them",
`run_for_updates_sequential`). Outcomes are classified per bookmark
(`hooks.rs:254-293`); a failure or an autofix (`Fixup`) both block
the submit (`hooks.rs:300-308`), the summary table is rendered
(`hooks.rs:314-315` — `render_failure_report`), and the first
Failed/Fixup becomes the surfaced error
(`hooks.rs:321-328`). An autofix counts as blocking because the user
must squash the fixup before re-submitting
(`hooks.rs:386-394`, `interpret_outcome`).

## Reconcile, fetch, restack, status

These round out the surface; submit reuses the reconcile logic
internally.

**`reconcile`** (`reconcile.rs:1-20`) has two independent steps.
`retrack_adjacent_diverged` re-issues `gt track` for tracked
bookmarks whose jj-derived parent drifted from gt's record, scoped
to bookmarks gt already knows
(`reconcile.rs:101-108` — `candidates.intersection(&gt_known)`) and
using the lossy derivation so one zombie bookmark can't wedge the
sweep (`reconcile.rs:120` —
`stack::derive_parents_lossy(jj, &adjacent, &opts.trunk)`).
`push_rebased_tips` runs `jj git push --bookmark <name>` per
bookmark with jj's default force-with-lease
(`reconcile.rs:230` — `jj::git_push_bookmark(jj, &opts.remote,
name)`), classifying each as added/moved/already-in-sync
(`reconcile.rs:231-234`). The standalone subcommand pushes only when
`--push` is given (`cli.rs:205-212`); the submit path always pushes
the stack (`lib.rs:619-626`).

**`fetch`** is the Graphite-aware `jj git fetch`
(`README.md:153-157`): fetch trunk, backfill metadata refs, run
`gt sync` for branch cleanup, rebase orphaned children with
`jj rebase`, and prune `gtmq_*` queue artifacts. `gt sync` is always
`--no-restack` so gt's git-rebase never rewrites jj-tracked SHAs
(`gt.rs:162-171` — `run_gt_captured(workspace_root, &["sync",
"--no-restack", "--force"])`).

**`restack`** (`restack.rs:1-39`) rebases every local stack onto
trunk. Discovery is
`restack.rs:124` — `"bookmarks() & mine() & ~::<trunk_destination>"`
(user-authored bookmarks not yet reachable from trunk), partitioned
into independent stacks. `rebase_one` anchors the rebase at the
pre-rebase root (`restack.rs:243` — `let rebase_revset =
format!("{root_commit}::");`) and rebases onto `<trunk>@<remote>`
(`restack.rs:245` — `jj::rebase(jj, &rebase_revset,
trunk_destination)`); jj's conflict-in-commit model means rebases
always complete, so conflicts land as commit markers and surface in
the summary (`restack.rs:251-269`).

**`status`** (`cli.rs:171-182`) batches PR state for the stack in a
single `gh` query (`status.rs:164-166` —
`gh::find_prs_for_branches(workspace_root, branches, 100)`) and
prints a table, or JSON with `--json`.

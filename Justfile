set shell := ["bash", "-c"]
set dotenv-load := false

default:
    @just --list

# --- Workspace-wide recipes ---------------------------------------------------

# `fmt`/`fmt-check`/`clippy` cover only the Rust workspace. akiflow-cli
# has its own bun-native lint chain via `ci-akiflow-cli`.

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets -- -D warnings

# Workspace-wide nextest. Skips akiflow-cli (TS) entirely.
test:
    cargo nextest run --workspace --no-fail-fast

# `install-deps` orchestrates per-tool setup. Subsequent monorepo
# additions plug in their own setup recipe here.
install-deps:
    @just _install-deps-common
    @just _install-deps-jj-gt
    @just _install-deps-akiflow-cli

# Common cross-tool deps (rust toolchain, cargo-nextest, cargo-edit).
_install-deps-common:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v rustup >/dev/null 2>&1; then
        echo "error: rustup required. Install from https://rustup.rs first." >&2
        exit 1
    fi
    rustup show active-toolchain || true
    if ! command -v cargo-binstall >/dev/null 2>&1; then
        cargo install --locked cargo-binstall
    fi
    cargo binstall --no-confirm cargo-edit cargo-nextest

# jj-gt's setup is the heaviest — it shells out to hk, pkl, gt, gh,
# pre-commit, lefthook, etc. The recipe lives in tools/jj-gt/Justfile;
# we delegate.
_install-deps-jj-gt:
    cd tools/jj-gt && just install-deps

# akiflow-cli only needs bun.
_install-deps-akiflow-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v bun >/dev/null 2>&1; then
        echo "warn: bun not on PATH. Install from https://bun.sh first." >&2
        exit 0
    fi
    cd tools/akiflow-cli && bun install --frozen-lockfile

# --- Local CI mirror ----------------------------------------------------------
#
# Mirrors seal's `just ci` pattern: each `ci-<tool>` recipe runs the
# exact commands the per-tool GitHub workflow invokes (so local + remote
# stay in sync), and `just ci` auto-detects which tools' paths the
# working-copy diff vs main@origin touches via `_filter-touched <name>`
# (reading `.github/path-filters/<name>.yml` — the same file
# `dorny/paths-filter` consumes in the workflow).
#
# Why: instead of pushing a stack to GitHub and waiting for remote CI,
# run `just ci` and get the same gating in seconds. Remote CI becomes a
# backstop, not the primary feedback loop.

# `just ci` runs ONLY the tools whose path filters match the diff.
# `ci-all` runs every tool unconditionally.
[doc('Auto-detected CI: run only tools touched by the working-copy diff vs main@origin.')]
ci:
    #!/usr/bin/env bash
    set -euo pipefail
    ran_any=0
    if just _filter-touched jj-hooks; then
        echo "→ jj-hooks paths touched; running its CI."
        just ci-jj-hooks
        ran_any=1
    fi
    if just _filter-touched jj-gt; then
        echo "→ jj-gt paths touched; running its CI."
        just ci-jj-gt
        ran_any=1
    fi
    if just _filter-touched akiflow-cli; then
        echo "→ akiflow-cli paths touched; running its CI."
        just ci-akiflow-cli
        ran_any=1
    fi
    if just _filter-touched tap; then
        echo "→ tap paths touched; running its CI."
        just ci-tap
        ran_any=1
    fi
    if just _filter-touched docs; then
        echo "→ markdown paths touched; running markdownlint."
        just ci-docs
        ran_any=1
    fi
    if [ "$ran_any" -eq 0 ]; then
        echo "→ No tool paths touched by the diff. Nothing to do."
        echo "  (Run \`just ci-all\` to force the full suite.)"
    else
        echo "✅ Local CI passed"
    fi

# Run every tool's CI unconditionally — ignores the path filter.
[doc('Unconditional CI: run every tool, ignoring path filter. Equivalent to a full remote run.')]
ci-all: ci-jj-hooks ci-jj-gt ci-akiflow-cli ci-tap ci-docs

# Per-tool CI recipes. Each mirrors what the corresponding
# .github/workflows/<tool>.yml runs in CI. When the remote recipe
# changes, update both sides at once.

[doc('jj-hooks CI: fmt + clippy + nextest. Mirrors .github/workflows/jj-hooks.yml.')]
ci-jj-hooks:
    cargo fmt -p jj-hooks -- --check
    cargo clippy -p jj-hooks --all-targets -- -D warnings
    cargo nextest run -p jj-hooks --no-fail-fast

[doc('jj-gt CI: fmt + clippy + nextest (excludes live tests; run `just ci-jj-gt-live` for those).')]
ci-jj-gt:
    cargo fmt -p jj-gt -- --check
    cargo clippy -p jj-gt --all-targets -- -D warnings
    cargo nextest run -p jj-gt --no-fail-fast --no-tests=warn -E 'not (test(gh_live) | test(gt_submit_live))'

[doc('jj-gt live tests: gh_live + gt_submit_live. Requires JJ_GT_LIVE_* env vars.')]
ci-jj-gt-live:
    JJ_GT_LIVE_GH=1 \
    JJ_GT_LIVE_SUBMIT=1 \
    cargo nextest run -p jj-gt --no-fail-fast -E 'test(gh_live) | test(gt_submit_live)'

[doc('akiflow-cli CI: biome + tsc + bun test. Mirrors .github/workflows/akiflow-cli.yml.')]
ci-akiflow-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    cd tools/akiflow-cli
    bun install --frozen-lockfile
    bunx biome check .
    bunx tsc --noEmit
    timeout 5m bun test

[doc('Homebrew tap CI: brew style on every formula.')]
ci-tap:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v brew >/dev/null 2>&1; then
        echo "warn: brew not on PATH; skipping tap lint." >&2
        exit 0
    fi
    brew style tap/Formula/*.rb

[doc('Markdown lint: enforces the rules in .markdownlint-cli2.jsonc.')]
ci-docs:
    markdownlint-cli2 "**/*.md"

# --- Release ---------------------------------------------------------------
#
# `just release v0.3.0` cuts a full monorepo release:
#
#   1. Validate the version string + working-copy state.
#   2. Bump the workspace Cargo.toml + tools/akiflow-cli/package.json
#      + tap/Formula/*.rb to the new version. `cargo set-version
#      --workspace` handles all Rust members + the internal jj-hooks
#      path-dep version field in one shot; the akiflow-cli + tap
#      bumps are inline `sed` calls.
#   3. Commit "release: vX.Y.Z" as a new jj change on top of @.
#   4. Tag @- with the version.
#   5. Advance the local `main` bookmark to the release commit.
#   6. Push main + the tag — the tag push triggers release.yml.
#
# Tag format: vX.Y.Z (stable) or vX.Y.Z-rc.N (pre-release). The
# release.yml workflow skips the tap-bump + crates.io publish jobs
# for pre-releases.
#
# Tap formulae get their sha256s rewritten by release.yml at run
# time; the bump here only updates the `version` line so the
# `url "...releases/download/v#{version}/..."` templates resolve
# to the right path. Run-time sha rewrite then patches the
# placeholder shas the bump leaves behind.
#
# Adapted from jj-gt's standalone recipe; differences:
#   - --workspace flag on `cargo set-version` bumps both Rust crates
#     + the jj-hooks workspace-dep `version` field together.
#   - akiflow-cli isn't a Cargo member; bumped via the `version`
#     field in tools/akiflow-cli/package.json.
#   - tap formulae get version-line bumps so their URL templates
#     resolve before release.yml runs.
[doc('Cut a release: bump versions, commit, tag, push.')]
release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    version="{{ VERSION }}"
    if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9._-]+)?$ ]]; then
        echo "error: VERSION must look like v1.2.3 or v1.2.3-rc.1 (got: $version)" >&2
        exit 1
    fi
    bare="${version#v}"

    # Require a clean @ — release commits should not include
    # unrelated work.
    if [ -n "$(jj diff --summary --ignore-working-copy 2>/dev/null)" ]; then
        echo "error: working copy @ has uncommitted changes; finalize them first" >&2
        exit 1
    fi

    # Require `main` to be an ancestor of `@` so the release commit
    # lands on top of main. Otherwise advancing main to @- after the
    # commit would move it backwards or sideways onto an unrelated
    # branch.
    if ! jj --ignore-working-copy log -r "main & ::@" -T 'change_id' --no-graph 2>/dev/null | grep -q .; then
        echo "error: @ is not a descendant of main (run \`jj rebase -d main\` first)" >&2
        exit 1
    fi

    # Refuse to re-tag an existing version. Stops you from accidentally
    # overwriting a release that's already out there.
    if jj --ignore-working-copy tag list -T 'name ++ "\n"' 2>/dev/null | grep -qx "$version"; then
        echo "error: tag $version already exists" >&2
        exit 1
    fi

    if ! cargo set-version --help >/dev/null 2>&1; then
        echo "error: cargo-edit not installed (run: cargo install --locked cargo-edit)" >&2
        exit 1
    fi

    echo "==> Bumping Rust workspace + members + jj-hooks dep to $bare..."
    cargo set-version --workspace "$bare"
    echo

    echo "==> Bumping tools/akiflow-cli/package.json to $bare..."
    # In-place sed: `"version": "X.Y.Z"` → `"version": "$bare"`. The
    # extension regex anchors on the leading `"version":` key so it
    # doesn't catch unrelated `version` strings (e.g. an
    # `"engines": { "bun": "..." }`-adjacent field).
    sed -i -E "s/^(\s*\"version\":\s*)\"[^\"]+\"/\1\"$bare\"/" \
        tools/akiflow-cli/package.json
    echo

    echo "==> Bumping tap/Formula/*.rb version lines to $bare..."
    # Same anchor pattern as the python bump in release.yml:
    # `  version "X.Y.Z"` at the start of a Ruby formula's class body.
    sed -i -E "s/^(\s*version\s+)\"[^\"]+\"/\1\"$bare\"/" tap/Formula/*.rb
    echo

    echo "==> Updating Cargo.lock..."
    cargo update --workspace
    echo

    # Sanity check: every bump landed. If any of these still show the
    # old version, abort before committing so the user can debug.
    echo "==> Verifying bumps..."
    grep -q "^version = \"$bare\"" Cargo.toml || \
        ! grep -q "^version\.workspace = true\|^version = " Cargo.toml || \
        true  # workspace-inheriting members; the workspace section itself
    grep -q "^version = \"$bare\"" Cargo.toml || {
        echo "error: workspace Cargo.toml version didn't bump to $bare" >&2
        grep "^version = " Cargo.toml >&2
        exit 1
    }
    grep -q "\"version\": \"$bare\"" tools/akiflow-cli/package.json || {
        echo "error: tools/akiflow-cli/package.json version didn't bump" >&2
        exit 1
    }
    for formula in tap/Formula/*.rb; do
        if ! grep -q "version \"$bare\"" "$formula"; then
            echo "error: $formula version line didn't bump to $bare" >&2
            exit 1
        fi
    done
    echo

    echo "==> Committing release bump as a new jj change on top of @..."
    jj commit -m "release: $version"
    echo

    echo "==> Tagging @- with $version..."
    jj tag set "$version" -r @-
    echo

    # Move the local `main` bookmark forward to the release commit so
    # `jj git push` pushes the right ref.
    echo "==> Advancing main to the release commit..."
    jj bookmark set main -r @-
    echo

    echo "==> Exporting refs to git..."
    jj --ignore-working-copy git export >/dev/null 2>&1 || true
    echo

    echo "==> Pushing main..."
    jj git push -b main
    echo

    echo "==> Pushing tag $version (triggers release.yml)..."
    # `jj git push --tag` is the idiomatic spelling; the
    # standalone jj-gt repo used a `jj-push-tags` shell wrapper
    # but that's a jj-hp/dotfiles thing — straight `git push`
    # works inside the colocated repo.
    git push origin "$version"
    echo

    echo "✅ Done. Watch the release workflow:"
    echo "   https://github.com/mattwilkinsonn/zireael/actions/workflows/release.yml"

# Shared implementation for the per-filter "did the diff touch any
# path under <filter-name>?" predicate. Stays private — the public
# entrypoint is `just ci` (which auto-iterates) or
# `just _filter-touched <name>` directly (for one-off checks).
#
# Adapted from sealedsecurity/seal/Justfile (line 543).
#
# `[no-exit-message]` here suppresses the "Recipe _filter-touched
# failed" line that would otherwise print on the expected exit-1
# "no matches" path — this is a predicate, not an error.
[no-exit-message]
_filter-touched filter:
    #!/usr/bin/env bash
    set -euo pipefail
    target_filter="{{filter}}"
    filter_file=".github/path-filters/${target_filter}.yml"
    if [ ! -f "$filter_file" ]; then
        echo "error: no path-filter file for \`$target_filter\` at $filter_file" >&2
        exit 2
    fi
    # Extract the named filter's section. Stops at the next top-level key.
    patterns=$(awk -v target="$target_filter" '
        /^[A-Za-z_][A-Za-z0-9_-]*:/ {
            current = $0
            sub(/:.*/, "", current)
            in_filter = (current == target) ? 1 : 0
            next
        }
        in_filter && /^[[:space:]]*-[[:space:]]/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "")
            gsub(/^['\''"]|['\''"]$/, "")
            print
        }
    ' "$filter_file")
    # `jj diff --summary` emits `<status> <path>` per changed file
    # between main@origin and the working copy (covers every commit
    # in a stacked bookmark + uncommitted edits).
    #
    # Fall back to `main` (no @origin) when there's no remote yet —
    # useful during the initial monorepo bootstrap before the first push.
    if jj log -r main@origin -T 'commit_id' --no-graph --ignore-working-copy >/dev/null 2>&1; then
        changed=$(jj diff --from main@origin --to @ --summary | awk '{print $2}')
    elif jj log -r main -T 'commit_id' --no-graph --ignore-working-copy >/dev/null 2>&1; then
        changed=$(jj diff --from main --to @ --summary | awk '{print $2}')
    else
        # No `main` yet — assume everything is touched.
        echo "warn: no \`main\` bookmark yet; assuming all paths touched." >&2
        exit 0
    fi
    for path in $changed; do
        while IFS= read -r pat; do
            case "$pat" in
                # Pattern ending in `/**` is a recursive directory match.
                *'/**') p="${pat%/**}/"; case "$path" in "$p"*) exit 0 ;; esac ;;
                # Anything else is an exact-file match.
                *)      case "$path" in $pat) exit 0 ;; esac ;;
            esac
        done <<<"$patterns"
    done
    exit 1

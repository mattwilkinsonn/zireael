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

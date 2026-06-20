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

# `install-deps [tool]` sets up dev dependencies for one tool — or
# `all` (the default). Just has no enum type for recipe parameters
# (they're plain strings), so the recipe validates `tool` and prints
# the valid set on a miss.
#
#   just install-deps            # everything
#   just install-deps jj-hooks   # just that tool's stack (+ common)
install-deps tool="all":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ tool }}" in
        all)
            just _install-deps-common
            just _install-deps-jj-hooks
            just _install-deps-jj-gt
            just _install-deps-akiflow-cli
            ;;
        common)      just _install-deps-common ;;
        jj-hooks)    just _install-deps-common && just _install-deps-jj-hooks ;;
        jj-gt)       just _install-deps-common && just _install-deps-jj-gt ;;
        akiflow-cli) just _install-deps-akiflow-cli ;;
        *)
            echo "error: unknown tool '{{ tool }}'" >&2
            echo "valid: all | common | jj-hooks | jj-gt | akiflow-cli" >&2
            exit 1
            ;;
    esac

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

# jj-hooks's integration tests drive real hook frameworks, so its
# tests need pre-commit, prek, lefthook, and hk on PATH (hk in turn
# needs pkl to read hk.pkl), plus markdownlint-cli2 + actionlint for
# the doc/workflow hooks. macOS uses Homebrew; Linux uses uv for the
# Python backends, npm for markdownlint-cli2, pinned release tarballs
# for lefthook/actionlint/pkl, and cargo-binstall for hk. (`jj` itself
# is assumed present — it's the VCS this whole repo targets.)
_install-deps-jj-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
        Darwin)
            brew install pre-commit prek lefthook hk pkl markdownlint-cli2 actionlint
            ;;
        Linux)
            # Honour XDG_BIN_HOME when set (CI points it at one cacheable
            # dir); default to ~/.local/bin for local installs.
            bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
            mkdir -p "$bin_dir"
            export PATH="$bin_dir:$PATH"

            uv tool install pre-commit
            uv tool install prek

            arch="$(uname -m)"
            case "$arch" in
                x86_64)  lefthook_arch=x86_64; actionlint_arch=amd64; pkl_arch=amd64 ;;
                aarch64) lefthook_arch=arm64;  actionlint_arch=arm64; pkl_arch=aarch64 ;;
                *) echo "unsupported Linux arch: $arch" >&2; exit 1 ;;
            esac

            lefthook_version=2.1.6
            curl -fsSL "https://github.com/evilmartians/lefthook/releases/download/v${lefthook_version}/lefthook_${lefthook_version}_Linux_${lefthook_arch}" -o "$bin_dir/lefthook"
            chmod +x "$bin_dir/lefthook"

            actionlint_version=1.7.12
            curl -fsSL "https://github.com/rhysd/actionlint/releases/download/v${actionlint_version}/actionlint_${actionlint_version}_linux_${actionlint_arch}.tar.gz" | tar -xz -C "$bin_dir" actionlint

            # hk shells out to pkl to read hk.pkl; without pkl on PATH it
            # rejects every config as "no config".
            pkl_version=0.31.1
            curl -fsSL "https://github.com/apple/pkl/releases/download/${pkl_version}/pkl-linux-${pkl_arch}" -o "$bin_dir/pkl"
            chmod +x "$bin_dir/pkl"

            if command -v npm >/dev/null 2>&1; then
                npm config set prefix "$(dirname "$bin_dir")"
                npm install -g markdownlint-cli2
            else
                echo "warn: npm not on PATH; install Node.js to get markdownlint-cli2" >&2
            fi

            cargo binstall -y --install-path "$bin_dir" hk
            ;;
        *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
    esac

# jj-gt bridges jj bookmark stacks and Graphite, so its tests shell out
# to `gt` (the Graphite CLI), installed from npm. `jj`, `gh`, and `git`
# are assumed present (CI installs jj from a tarball; gh + git are
# preinstalled on runners and standard in a local dev setup).
_install-deps-jj-gt:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v gt >/dev/null 2>&1; then
        echo "gt already installed ($(gt --version 2>/dev/null | head -1))"
        exit 0
    fi
    if ! command -v npm >/dev/null 2>&1; then
        echo "error: npm required to install the Graphite CLI (gt); install Node.js first." >&2
        exit 1
    fi
    npm install -g @withgraphite/graphite-cli

# akiflow-cli only needs bun.
_install-deps-akiflow-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v bun >/dev/null 2>&1; then
        echo "warn: bun not on PATH. Install from https://bun.sh first." >&2
        exit 0
    fi
    cd tools/akiflow-cli && bun install --frozen-lockfile

# --- Install debug builds -----------------------------------------------------
#
# Each `install-debug-<tool>` recipe builds the tool in debug mode and
# drops the binary into `~/.cargo/bin` (or `$CARGO_HOME/bin`). Recipes
# are ETXTBSY-safe (unlink before cp, since Linux can't overwrite a
# running executable). Macs get codesigned automatically so the next
# run doesn't trigger a Gatekeeper prompt.
#
# Use this when you want to dogfood a local change without bumping
# the version or cutting a release. The `install` (release-build)
# variant ships via release.yml + Homebrew; install-debug is purely
# for the inner loop.

[doc('Install all debug builds (jj-hooks, jj-gt, akiflow-cli) to ~/.cargo/bin.')]
install-debug: install-debug-jj-hooks install-debug-jj-gt install-debug-akiflow-cli

[doc('Debug-build + install jj-hooks and jj-hp to ~/.cargo/bin.')]
install-debug-jj-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p jj-hooks --bin jj-hooks --bin jj-hp
    dest="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$dest"
    # On Linux, writing over an in-use executable fails with ETXTBSY
    # (text file busy). Unlink first so a running process keeps its
    # inode while we drop a fresh one at the path. macOS lets you
    # overwrite an active binary, so the unlink is a no-op there.
    for bin in jj-hooks jj-hp; do
        rm -f "$dest/$bin"
        cp "target/debug/$bin" "$dest/$bin"
        if [[ "$(uname)" == "Darwin" ]]; then
            codesign -s - "$dest/$bin" 2>/dev/null && echo "Codesigned $bin" || true
        fi
    done
    echo "Installed debug builds (jj-hooks + jj-hp) to $dest"

[doc('Debug-build + install jj-gt to ~/.cargo/bin.')]
install-debug-jj-gt:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p jj-gt --bin jj-gt
    dest="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$dest"
    rm -f "$dest/jj-gt"
    cp "target/debug/jj-gt" "$dest/jj-gt"
    if [[ "$(uname)" == "Darwin" ]]; then
        codesign -s - "$dest/jj-gt" 2>/dev/null && echo "Codesigned jj-gt" || true
    fi
    echo "Installed debug build (jj-gt) to $dest"

# akiflow-cli compiles to a single `af` binary via `bun build --compile`.
# Same ETXTBSY guard applies; no codesign needed (bun's compiled
# binaries don't trip Gatekeeper).
[doc('Debug-build + install akiflow-cli (`af`) to ~/.cargo/bin.')]
install-debug-akiflow-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    cd tools/akiflow-cli
    bun install --frozen-lockfile
    bun build src/index.ts --compile --outfile af
    dest="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$dest"
    rm -f "$dest/af"
    cp "af" "$dest/af"
    echo "Installed debug build (af) to $dest"

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
    if just _filter-touched nix-config; then
        echo "→ nix-config paths touched; running its CI."
        just ci-nix-config
        ran_any=1
        # Per-host flake-eval gated on the per-host filters. Skipped
        # by default (lints catch 90%); pass `EVAL=1 just ci` for the
        # full host-by-host eval. With EVAL=1, only hosts the diff
        # actually affects get evaluated — mirrors what
        # nix-config.yml's per-host gating does on PRs.
        if [ "${EVAL:-0}" = "1" ]; then
            HOSTS_FILTERED=1 just ci-nix-config-eval
        fi
    fi
    if [ "$ran_any" -eq 0 ]; then
        echo "→ No tool paths touched by the diff. Nothing to do."
        echo "  (Run \`just ci-all\` to force the full suite.)"
    else
        echo "✅ Local CI passed"
    fi

# Run every tool's CI unconditionally — ignores the path filter.
[doc('Unconditional CI: run every tool, ignoring path filter. Equivalent to a full remote run.')]
ci-all: ci-jj-hooks ci-jj-gt ci-akiflow-cli ci-tap ci-docs ci-nix-config

# Per-tool CI recipes. The granular `fmt-pkg`/`clippy-pkg`/`nextest-*`
# recipes below are the single source of truth for each check command:
# the GitHub workflows call them directly (lints job → fmt + clippy,
# test job → nextest) and the `ci-<tool>` aggregates run the same set
# locally. hk's pre-push mirrors these commands inline — it can't call
# `just` from inside a git hook without risking recursion.

# Per-package check building blocks — the GitHub jobs and `ci-<tool>`
# both call these, so each check is defined exactly once.

fmt-pkg PKG:
    cargo fmt -p {{ PKG }} -- --check

clippy-pkg PKG:
    cargo clippy -p {{ PKG }} --all-targets -- -D warnings

nextest-jj-hooks:
    cargo nextest run -p jj-hooks --no-fail-fast

# Excludes the live (network) tests; run `just ci-jj-gt-live` for those.
nextest-jj-gt:
    cargo nextest run -p jj-gt --no-fail-fast --no-tests=warn -E 'not (test(gh_live) | test(gt_submit_live))'

[doc('jj-hooks CI: fmt + clippy + nextest. Mirrors .github/workflows/jj-hooks.yml.')]
ci-jj-hooks: (fmt-pkg "jj-hooks") (clippy-pkg "jj-hooks") nextest-jj-hooks

[doc('jj-gt CI: fmt + clippy + nextest (excludes live tests; run `just ci-jj-gt-live` for those).')]
ci-jj-gt: (fmt-pkg "jj-gt") (clippy-pkg "jj-gt") nextest-jj-gt

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
    brew style Formula/*.rb

[doc('Markdown lint: enforces the rules in .markdownlint-cli2.jsonc.')]
ci-docs:
    markdownlint-cli2 "**/*.md"

# nix-config CI: nix lints + shell lints + toml lint + 6 flake-eval
# steps. Mirrors .github/workflows/nix-config.yml; commands match
# exactly what the per-job workflow runs (so local + remote feedback
# loops stay aligned).
#
# Skips the slow flake-eval steps by default (the linters catch 90%
# of issues in <5s). Pass `EVAL=1 just ci-nix-config` (or use
# `just ci-nix-config-eval`) for the full host-by-host eval.
#
# Tool deps (assumed on PATH): nixfmt, deadnix, statix, nil, shellcheck,
# shfmt, taplo. Local dev hosts get these via the nix-managed dev
# environment; CI installs them per-job.
[doc('nix-config CI: lints only. Set EVAL=1 to also run flake-eval. Mirrors .github/workflows/nix-config.yml.')]
ci-nix-config:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "→ nixfmt --check"
    find nix-config -type f -name '*.nix' -print0 | xargs -0 nixfmt --check
    echo "→ deadnix"
    deadnix --fail nix-config
    echo "→ statix check"
    statix check -c nix-config nix-config
    echo "→ nil diagnostics"
    find nix-config -type f -name '*.nix' -print0 | xargs -0 nil diagnostics
    echo "→ shellcheck (--external-sources)"
    find nix-config -type f -name '*.sh' -print0 | xargs -0 shellcheck --external-sources
    echo "→ shfmt -d -i 0 -s"
    find nix-config -type f -name '*.sh' -print0 | xargs -0 shfmt -d -i 0 -s
    echo "→ taplo fmt --check"
    find nix-config -type f -name '*.toml' -print0 | RUST_LOG=error xargs -0 taplo fmt --check
    if [ "${EVAL:-0}" = "1" ]; then
        just ci-nix-config-eval
    fi

[doc('nix-config flake-eval: nix eval each of the 6 host configs. Slow (~30s-2min per host cold). Honors per-host path filters when HOSTS_FILTERED=1.')]
ci-nix-config-eval:
    #!/usr/bin/env bash
    set -euo pipefail
    cd nix-config
    # When HOSTS_FILTERED=1 (set by the `ci` dispatcher), skip hosts
    # whose per-host path filter doesn't match the diff. When unset
    # (default for explicit `just ci-nix-config-eval` calls), eval
    # everything — that's the "I'm cutting a release / pre-PR
    # full-matrix sanity check" workflow.
    should_eval() {
        if [ "${HOSTS_FILTERED:-0}" != "1" ]; then
            return 0
        fi
        # The filter file lives at the repo root, not at nix-config/.
        (cd .. && just _filter-touched nix-config-hosts "$1") >/dev/null
    }
    for host in rpi4 rpi5 mattfw mattserver mattlinuxpro mattpc-wsl; do
        if should_eval "$host"; then
            echo "→ nix eval nixosConfigurations.$host"
            nix eval --raw ".#nixosConfigurations.$host.config.system.build.toplevel.outPath" \
                --no-warn-dirty --accept-flake-config >/dev/null
        else
            echo "→ skipping $host (no path-filter match)"
        fi
    done
    # Darwin only evaluates on macOS hosts; skip gracefully on Linux.
    if [ "$(uname -s)" = "Darwin" ]; then
        if should_eval darwin-mbp; then
            echo "→ nix eval darwinConfigurations.Matts-MacBook-Pro"
            nix eval --raw '.#darwinConfigurations.Matts-MacBook-Pro.config.system.build.toplevel.outPath' \
                --no-warn-dirty --accept-flake-config >/dev/null
        else
            echo "→ skipping darwinConfigurations.Matts-MacBook-Pro (no path-filter match)"
        fi
        if should_eval mattmini; then
            echo "→ nix eval darwinConfigurations.mattmini"
            nix eval --raw '.#darwinConfigurations.mattmini.config.system.build.toplevel.outPath' \
                --no-warn-dirty --accept-flake-config >/dev/null
        else
            echo "→ skipping darwinConfigurations.mattmini (no path-filter match)"
        fi
    else
        echo "→ skipping darwinConfigurations (not on macOS)"
    fi

# --- Release ---------------------------------------------------------------
#
# `just release v0.3.0` cuts a full monorepo release:
#
#   1. Validate the version string + working-copy state.
#   2. Bump the workspace Cargo.toml + tools/akiflow-cli/package.json
#      + Formula/*.rb to the new version. `cargo set-version
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

    echo "==> Bumping Formula/*.rb version lines to $bare..."
    # Same anchor pattern as the python bump in release.yml:
    # `  version "X.Y.Z"` at the start of a Ruby formula's class body.
    sed -i -E "s/^(\s*version\s+)\"[^\"]+\"/\1\"$bare\"/" Formula/*.rb
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
    for formula in Formula/*.rb; do
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
    # jj has no native `jj git push --tag`, so we shell out to jj-hp's
    # push-tags subcommand which wraps `jj git export` (a no-op when
    # refs are in sync) + `git push refs/tags/<tag>` per tag.
    # Requires `jj-hp` on PATH (installed via `just install-debug-jj-hooks`
    # locally, or via Homebrew for non-dev hosts).
    jj-hp push-tags "$version"
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
#
# Two argument shapes:
#   _filter-touched <name>          → reads .github/path-filters/<name>.yml,
#                                     matches against the top-level <name>:
#                                     key.
#   _filter-touched <file> <name>   → reads .github/path-filters/<file>.yml,
#                                     matches against the <name>: key inside
#                                     it. Used for the per-host nix-config
#                                     filters (one file with multiple
#                                     named filters).
[no-exit-message]
_filter-touched filter sub='':
    #!/usr/bin/env bash
    set -euo pipefail
    target_filter="{{filter}}"
    sub_filter="{{sub}}"
    if [ -z "$sub_filter" ]; then
        filter_file=".github/path-filters/${target_filter}.yml"
        match_key="$target_filter"
    else
        filter_file=".github/path-filters/${target_filter}.yml"
        match_key="$sub_filter"
    fi
    if [ ! -f "$filter_file" ]; then
        echo "error: no path-filter file for \`$target_filter\` at $filter_file" >&2
        exit 2
    fi
    # Extract the named filter's section. Stops at the next top-level key.
    patterns=$(awk -v target="$match_key" '
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

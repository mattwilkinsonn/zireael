#!/usr/bin/env bash
# One-time setup for the jj-gt live-test fixture repo.
#
# Creates `<owner>/jj-gt-live-tests` on GitHub (public, empty),
# pushes a trivial main branch, and opens one fixture PR against a
# stable `fixture/persistent-pr` branch so the `gh pr list` live
# test has something to assert against.
#
# Idempotent: re-running on an already-set-up repo is a no-op.
#
# Usage:
#   ./scripts/setup-live-test-fixture.sh [owner]
#
# `owner` defaults to your `gh` auth-status username.

set -euo pipefail

OWNER="${1:-}"
if [ -z "$OWNER" ]; then
    OWNER=$(gh api user --jq .login)
fi
REPO="jj-gt-live-tests"
FULL="${OWNER}/${REPO}"

# 1. Create the repo if missing.
if gh repo view "$FULL" >/dev/null 2>&1; then
    echo "repo $FULL already exists; skipping creation"
else
    echo "creating $FULL ..."
    gh repo create "$FULL" \
        --public \
        --description "Live-test fixture repo for jj-gt. Do not delete; PRs/branches under fixture/ are referenced by tests." \
        --add-readme \
        --clone=false
fi

# 2. Clone (or refresh) a workspace we can push from. We use plain
# `git clone` rather than `gh repo clone` so we don't go through gh's
# HTTPS proxy (which some sandboxes block even when raw git is fine).
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
git clone --quiet --depth 1 "https://github.com/${FULL}.git" .

git config user.name  "jj-gt-fixture-bot"
git config user.email "jj-gt-fixture-bot@users.noreply.github.com"

# 3. Ensure the persistent fixture branch + PR exist.
PERSISTENT_BRANCH="fixture/persistent-pr"
PR_TITLE="[fixture] persistent PR for jj-gt live tests"

# Does the branch exist on origin? Probe via the GitHub REST API
# rather than `git ls-remote` so this works in sandboxes that block
# anonymous git-over-https probes but allow gh.
if gh api "repos/${FULL}/branches/${PERSISTENT_BRANCH}" --silent >/dev/null 2>&1; then
    echo "branch $PERSISTENT_BRANCH already exists on origin"
else
    echo "creating $PERSISTENT_BRANCH ..."
    git checkout -b "$PERSISTENT_BRANCH"
    mkdir -p fixture
    cat > fixture/README.md <<EOF
# jj-gt live-test fixture branch

This branch + the (intentionally closed) PR pointing at it are
referenced by jj-gt's live \`gh pr list\` tests. **Do not delete the
branch** and **do not reopen / merge the PR** without first updating
the test suite (\`tests/gh_live.rs\`). The setup script keeps the PR
in the \`CLOSED\` state on every re-run so it doesn't show up on the
Graphite home page; \`gh pr list --state all\` still returns it for
the tests.
EOF
    git add fixture/README.md
    git commit -m "fixture: persistent branch for jj-gt live tests"
    git push -u origin "$PERSISTENT_BRANCH"
fi

# Find any PR for the branch (open OR closed). We deliberately keep
# the fixture PR closed so it doesn't show up on the Graphite home
# page or in default `gh pr list` queries; the tests use
# `--state all` and assert against the record, not the open-ness.
#
# `gh` has jq built in via `--jq`, so we don't depend on a system
# jq. Emit just two scalars, space-separated, and read them straight
# into shell vars.
existing=$(gh pr list --repo "$FULL" --head "$PERSISTENT_BRANCH" --state all --json number,state --jq '.[0] | "\(.number) \(.state)"' 2>/dev/null || true)
if [ -z "$existing" ] || [ "$existing" = " " ]; then
    echo "opening fixture PR ..."
    gh pr create --repo "$FULL" \
        --head "$PERSISTENT_BRANCH" \
        --base main \
        --title "$PR_TITLE" \
        --body "Persistent fixture PR for jj-gt live tests. Do not merge. Will be re-closed on every setup run."
    pr_number=$(gh pr list --repo "$FULL" --head "$PERSISTENT_BRANCH" --state all --json number --jq '.[0].number')
    pr_state="OPEN"
else
    pr_number=$(echo "$existing" | awk '{print $1}')
    pr_state=$(echo "$existing" | awk '{print $2}')
    echo "PR for $PERSISTENT_BRANCH already exists (#$pr_number, state=$pr_state)"
fi

# Ensure the PR is CLOSED. Skipping the close call when already
# closed keeps the script idempotent without making a redundant API
# write on every run.
if [ "$pr_state" = "OPEN" ]; then
    echo "closing fixture PR #$pr_number to keep it off the Graphite home page ..."
    gh pr close "$pr_number" --repo "$FULL" \
        --comment "Auto-closed by scripts/setup-live-test-fixture.sh — fixture PRs stay closed to avoid cluttering the Graphite home page. Do not delete the branch."
fi

echo
echo "Done. To run the gh live tests against this fixture:"
echo
echo "  export JJ_GT_LIVE_GH=1"
echo "  export JJ_GT_LIVE_REPO=$FULL"
echo "  cargo nextest run --test gh_live"

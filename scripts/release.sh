#!/usr/bin/env bash
# Cut a full monorepo release: bump versions, commit, tag, push.
#
#   scripts/release.sh v0.3.0        (or: moon run root:release -- v0.3.0)
#
#   1. Validate the version string + working-copy state.
#   2. Bump the workspace Cargo.toml + tools/akiflow-cli/package.json
#      + Formula/*.rb to the new version. `cargo set-version --workspace`
#      handles all Rust members + the internal jj-hooks path-dep version
#      field in one shot; the akiflow-cli + tap bumps are inline sed.
#   3. Commit "release: vX.Y.Z" as a new jj change on top of @.
#   4. Tag @- with the version.
#   5. Advance the local `main` bookmark to the release commit.
#   6. Push main + the tag — the tag push triggers release.yml.
#
# Tag format: vX.Y.Z (stable) or vX.Y.Z-rc.N (pre-release). release.yml
# skips the tap-bump + crates.io publish jobs for pre-releases.
#
# Tap formulae get their sha256s rewritten by release.yml at run time;
# the bump here only updates the `version` line so the
# `url "...releases/download/v#{version}/..."` templates resolve.
set -euo pipefail

version="${1:-}"
if [[ ! $version =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9._-]+)?$ ]]; then
	echo "usage: scripts/release.sh vX.Y.Z (or vX.Y.Z-rc.1); got: '${version}'" >&2
	exit 1
fi
bare="${version#v}"

# Require a clean @ — release commits should not include unrelated work.
if [ -n "$(jj diff --summary --ignore-working-copy 2>/dev/null)" ]; then
	echo "error: working copy @ has uncommitted changes; finalize them first" >&2
	exit 1
fi

# Require `main` to be an ancestor of `@` so the release commit lands on
# top of main. Otherwise advancing main to @- would move it backwards or
# sideways onto an unrelated branch.
if ! jj --ignore-working-copy log -r "main & ::@" -T 'change_id' --no-graph 2>/dev/null | grep -q .; then
	echo "error: @ is not a descendant of main (run: jj rebase -d main)" >&2
	exit 1
fi

# Refuse to re-tag an existing version.
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
sed -i -E "s/^(\s*\"version\":\s*)\"[^\"]+\"/\1\"$bare\"/" \
	tools/akiflow-cli/package.json
echo

echo "==> Bumping Formula/*.rb version lines to $bare..."
sed -i -E "s/^(\s*version\s+)\"[^\"]+\"/\1\"$bare\"/" Formula/*.rb
echo

echo "==> Updating Cargo.lock..."
cargo update --workspace
echo

echo "==> Verifying bumps..."
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
# jj has no native `jj git push --tag`, so shell out to jj-hp's push-tags
# subcommand which wraps `jj git export` + `git push refs/tags/<tag>` per
# tag. Requires `jj-hp` on PATH (scripts/install-debug.sh jj-hooks
# locally, or Homebrew for non-dev hosts).
jj-hp push-tags "$version"
echo

echo "✅ Done. Watch the release workflow:"
echo "   https://github.com/mattwilkinsonn/zireael/actions/workflows/release.yml"

// Cut a full monorepo release: bump versions, commit, tag, push.
//
//   release v0.3.0        (or: moon run root:release -- v0.3.0)
//
//   1. Validate the version string + working-copy state.
//   2. Bump the workspace Cargo.toml + tools/akiflow-cli/package.json
//      + Formula/*.rb to the new version. `cargo set-version --workspace`
//      handles all Rust members + the internal jj-hooks path-dep version
//      field in one shot; the akiflow-cli + tap bumps are inline string
//      transforms (bumpPackageJsonVersion / bumpFormulaVersion).
//   3. Commit "release: vX.Y.Z" as a new jj change on top of @.
//   4. Tag @- with the version.
//   5. Advance the local `main` bookmark to the release commit.
//   6. Push main + the tag — the tag push triggers release.yml.
//
// Tag format: vX.Y.Z (stable) or vX.Y.Z-rc.N (pre-release). release.yml
// skips the tap-bump + crates.io publish jobs for pre-releases.
//
// Tap formulae get their sha256s rewritten by release.yml at run time;
// the bump here only updates the `version` line so the
// `url "...releases/download/v#{version}/..."` templates resolve.
//
// Exit codes:
//   0 - release cut successfully
//   1 - a guard failed (bad version, dirty @, non-descendant, existing
//       tag, missing cargo-edit, or a bump that didn't take)
//   N - a shelled release step (cargo/jj/jj-hp) exited N; propagated
//       verbatim (bash `set -e`).

import { $ } from "bun";

// Version tag shape: vX.Y.Z (stable) or vX.Y.Z-<pre> (pre-release).
const VERSION_RE = /^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9._-]+)?$/;

// Validate the CLI version arg and strip the leading `v` to the bare
// SemVer used by cargo/package.json/formula version fields. Returns the
// exact usage string bash emitted on a bad arg.
function validateVersion(v: string): { bare: string } | { error: string } {
	if (!VERSION_RE.test(v)) {
		return {
			error: `usage: moon run root:release -- vX.Y.Z (or vX.Y.Z-rc.1); got: '${v}'`,
		};
	}
	return { bare: v.slice(1) };
}

// Port of `sed -i -E "s/^(\s*\"version\":\s*)\"[^\"]+\"/\1\"$bare\"/"`:
// rewrite the value of every top-of-line `"version": "..."` field.
// Anchored per line (m flag), so only the field name at line start is
// touched — not a `"version"` appearing mid-value.
function bumpPackageJsonVersion(text: string, bare: string): string {
	return text.replace(/^(\s*"version":\s*)"[^"]+"/gm, `$1"${bare}"`);
}

// Port of `sed -i -E "s/^(\s*version\s+)\"[^\"]+\"/\1\"$bare\"/"`:
// rewrite the Ruby `version "..."` line. The `url "...#{version}..."`
// lines start with `url`, not `version`, so they are left untouched.
function bumpFormulaVersion(text: string, bare: string): string {
	return text.replace(/^(\s*version\s+)"[^"]+"/gm, `$1"${bare}"`);
}

// Port of the post-bump `grep -q` verification block. Returns the exact
// stderr lines bash would emit for the FIRST failing check (bash exits
// at the first failure), or [] when every bump took. `formulas` carries
// each formula's path (for the error message) + its on-disk text.
function verifyBumps(
	cargoToml: string,
	pkgJson: string,
	formulas: { name: string; text: string }[],
	bare: string,
): string[] {
	// grep -q "^version = \"$bare\"" Cargo.toml
	const wanted = `version = "${bare}"`;
	if (!cargoToml.split("\n").some((line) => line.startsWith(wanted))) {
		const msgs = [`error: workspace Cargo.toml version didn't bump to ${bare}`];
		// grep "^version = " Cargo.toml >&2
		for (const line of cargoToml.split("\n")) {
			if (line.startsWith("version = ")) msgs.push(line);
		}
		return msgs;
	}
	// grep -q "\"version\": \"$bare\"" tools/akiflow-cli/package.json
	if (!pkgJson.includes(`"version": "${bare}"`)) {
		return ["error: tools/akiflow-cli/package.json version didn't bump"];
	}
	// for formula in Formula/*.rb; do grep -q "version \"$bare\"" ...
	for (const f of formulas) {
		if (!f.text.includes(`version "${bare}"`)) {
			return [`error: ${f.name} version line didn't bump to ${bare}`];
		}
	}
	return [];
}

type ShResult = { exitCode: number; stdout: string; stderr: string };
type ShOpts = { capture?: boolean };

// Dependencies runOnce takes from its environment. Tests pass fakes;
// production wires real Bun.$/Bun.file/Bun.Glob + console.
type Deps = {
	env: Record<string, string | undefined>;
	// Run a binary. `capture: true` mirrors bash `$(...)` / `>/dev/null`
	// (stdout/stderr captured, exit code inspected); otherwise stdio is
	// inherited so cargo/jj stream to the terminal like the bash did.
	sh: (cmd: string, args: string[], opts?: ShOpts) => Promise<ShResult>;
	readFile: (path: string) => Promise<string>;
	writeFile: (path: string, content: string) => Promise<void>;
	glob: (pattern: string) => Promise<string[]>;
	log: (msg: string) => void;
	err: (msg: string) => void;
};

async function runOnce(deps: Deps, argv: string[]): Promise<number> {
	const { sh, readFile, writeFile, glob, log, err } = deps;

	const version = argv[0] ?? "";
	const parsed = validateVersion(version);
	if ("error" in parsed) {
		err(parsed.error);
		return 1;
	}
	const bare = parsed.bare;

	// Require a clean @ — release commits should not include unrelated work.
	const diff = await sh("jj", ["diff", "--summary", "--ignore-working-copy"], {
		capture: true,
	});
	if (diff.stdout.replace(/\n+$/, "").length > 0) {
		err("error: working copy @ has uncommitted changes; finalize them first");
		return 1;
	}

	// Require `main` to be an ancestor of `@` so the release commit lands
	// on top of main. Otherwise advancing main to @- would move it
	// backwards or sideways onto an unrelated branch.
	const ancestor = await sh(
		"jj",
		[
			"--ignore-working-copy",
			"log",
			"-r",
			"main & ::@",
			"-T",
			"change_id",
			"--no-graph",
		],
		{ capture: true },
	);
	// bash: `... | grep -q .` — succeeds iff some line has ≥1 char.
	if (!ancestor.stdout.split("\n").some((line) => line.length > 0)) {
		err("error: @ is not a descendant of main (run: jj rebase -d main)");
		return 1;
	}

	// Refuse to re-tag an existing version.
	const tags = await sh(
		"jj",
		["--ignore-working-copy", "tag", "list", "-T", 'name ++ "\\n"'],
		{ capture: true },
	);
	// bash: `... | grep -qx "$version"` — a whole line equal to version.
	if (tags.stdout.split("\n").some((line) => line === version)) {
		err(`error: tag ${version} already exists`);
		return 1;
	}

	const cargoEdit = await sh("cargo", ["set-version", "--help"], {
		capture: true,
	});
	if (cargoEdit.exitCode !== 0) {
		err(
			"error: cargo-edit not installed (run: cargo install --locked cargo-edit)",
		);
		return 1;
	}

	log(`==> Bumping Rust workspace + members + jj-hooks dep to ${bare}...`);
	const setVersion = await sh("cargo", ["set-version", "--workspace", bare]);
	if (setVersion.exitCode !== 0) return setVersion.exitCode;
	log("");

	log(`==> Bumping tools/akiflow-cli/package.json to ${bare}...`);
	const pkgPath = "tools/akiflow-cli/package.json";
	await writeFile(
		pkgPath,
		bumpPackageJsonVersion(await readFile(pkgPath), bare),
	);
	log("");

	log(`==> Bumping Formula/*.rb version lines to ${bare}...`);
	for (const formula of await glob("Formula/*.rb")) {
		await writeFile(formula, bumpFormulaVersion(await readFile(formula), bare));
	}
	log("");

	log("==> Updating Cargo.lock...");
	const cargoUpdate = await sh("cargo", ["update", "--workspace"]);
	if (cargoUpdate.exitCode !== 0) return cargoUpdate.exitCode;
	log("");

	log("==> Verifying bumps...");
	const formulaFiles = await Promise.all(
		(await glob("Formula/*.rb")).map(async (name) => ({
			name,
			text: await readFile(name),
		})),
	);
	const failures = verifyBumps(
		await readFile("Cargo.toml"),
		await readFile(pkgPath),
		formulaFiles,
		bare,
	);
	if (failures.length > 0) {
		for (const line of failures) err(line);
		return 1;
	}
	log("");

	log("==> Committing release bump as a new jj change on top of @...");
	const commit = await sh("jj", ["commit", "-m", `release: ${version}`]);
	if (commit.exitCode !== 0) return commit.exitCode;
	log("");

	log(`==> Tagging @- with ${version}...`);
	const tag = await sh("jj", ["tag", "set", version, "-r", "@-"]);
	if (tag.exitCode !== 0) return tag.exitCode;
	log("");

	log("==> Advancing main to the release commit...");
	const bookmark = await sh("jj", ["bookmark", "set", "main", "-r", "@-"]);
	if (bookmark.exitCode !== 0) return bookmark.exitCode;
	log("");

	log("==> Exporting refs to git...");
	// bash: `... >/dev/null 2>&1 || true` — output discarded, failure ignored.
	await sh("jj", ["--ignore-working-copy", "git", "export"], { capture: true });
	log("");

	log("==> Pushing main...");
	const pushMain = await sh("jj", ["git", "push", "-b", "main"]);
	if (pushMain.exitCode !== 0) return pushMain.exitCode;
	log("");

	log(`==> Pushing tag ${version} (triggers release.yml)...`);
	const pushTags = await sh("jj-hp", ["push-tags", version]);
	if (pushTags.exitCode !== 0) return pushTags.exitCode;
	log("");

	log("✅ Done. Watch the release workflow:");
	log(
		"   https://github.com/mattwilkinsonn/zireael/actions/workflows/release.yml",
	);
	return 0;
}

export type { Deps, ShOpts, ShResult };
export {
	bumpFormulaVersion,
	bumpPackageJsonVersion,
	runOnce,
	validateVersion,
	verifyBumps,
};

// `import.meta.main` is true only when this file is the entry point
// (`bun run index.ts`). Under `bun test`, the test file is the entry and
// this stays false, so main never runs.
if (import.meta.main) {
	process.exit(
		await runOnce(
			{
				env: process.env,
				sh: async (cmd, args, opts) => {
					if (opts?.capture) {
						const r = await $`${cmd} ${args}`.nothrow().quiet();
						return {
							exitCode: r.exitCode,
							stdout: r.stdout.toString(),
							stderr: r.stderr.toString(),
						};
					}
					const r = await $`${cmd} ${args}`.nothrow();
					return { exitCode: r.exitCode, stdout: "", stderr: "" };
				},
				readFile: (path) => Bun.file(path).text(),
				writeFile: async (path, content) => {
					await Bun.write(path, content);
				},
				glob: async (pattern) => {
					const out: string[] = [];
					for await (const p of new Bun.Glob(pattern).scan(".")) out.push(p);
					out.sort();
					return out;
				},
				log: (msg) => console.log(msg),
				err: (msg) => console.error(msg),
			},
			process.argv.slice(2),
		),
	);
}

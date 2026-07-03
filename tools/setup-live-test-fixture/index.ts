// One-time setup for the jj-gt live-test fixture repo.
//
// Creates `<owner>/jj-gt-live-tests` on GitHub (public, empty),
// pushes a trivial main branch, and opens one fixture PR against a
// stable `fixture/persistent-pr` branch so the `gh pr list` live
// test has something to assert against.
//
// Idempotent: re-running on an already-set-up repo is a no-op.
//
// Usage:
//   setup-live-test-fixture [owner]
//
// `owner` defaults to your `gh` auth-status username
// (`gh api user --jq .login`).
//
// The *condition* probes (`gh repo view`, `gh api .../branches/...`,
// `gh pr list`) run with `.nothrow()` and are read via their exit code /
// stdout, so a missing repo/branch/PR is a normal "create it" signal, not a
// failure; every other shell call is mandatory and aborts the run on a
// nonzero exit.

import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { $ } from "bun";

const REPO = "jj-gt-live-tests";
const PERSISTENT_BRANCH = "fixture/persistent-pr";
const PR_TITLE = "[fixture] persistent PR for jj-gt live tests";

const REPO_DESCRIPTION =
	"Live-test fixture repo for jj-gt. Do not delete; PRs/branches under fixture/ are referenced by tests.";

const PR_BODY =
	"Persistent fixture PR for jj-gt live tests. Do not merge. Will be re-closed on every setup run.";

const CLOSE_COMMENT =
	"Auto-closed by tools/setup-live-test-fixture/index.ts — fixture PRs stay closed to avoid cluttering the Graphite home page. Do not delete the branch.";

// Heredoc content of fixture/README.md, verbatim (backticks and the
// trailing newline are part of the fixture the tests reference).
const FIXTURE_README = `# jj-gt live-test fixture branch

This branch + the (intentionally closed) PR pointing at it are
referenced by jj-gt's live \`gh pr list\` tests. **Do not delete the
branch** and **do not reopen / merge the PR** without first updating
the test suite (\`tests/gh_live.rs\`). The setup script keeps the PR
in the \`CLOSED\` state on every re-run so it doesn't show up on the
Graphite home page; \`gh pr list --state all\` still returns it for
the tests.
`;

// Result of a shelled-out command run with `.nothrow()`.
type ShResult = { exitCode: number; stdout: string; stderr: string };

// Everything runOnce touches from its environment. Tests pass fakes
// (recording arrays, canned ShResults); production wires the real
// `Bun.$` + node:fs + console.
type Deps = {
	// Run `gh` with the given argv, never throwing.
	gh(args: string[]): Promise<ShResult>;
	// Run `git` with the given argv inside `cwd`, never throwing.
	git(args: string[], cwd: string): Promise<ShResult>;
	// `mktemp -d` — returns the created workspace directory.
	mkdtemp(): Promise<string>;
	// Write a file, creating parent directories (bash `mkdir -p`).
	writeFile(path: string, content: string): Promise<void>;
	// `rm -rf` — the EXIT-trap cleanup.
	rm(dir: string): Promise<void>;
	log(message: string): void;
	err(message: string): void;
};

// owner = first positional arg, else the resolved gh auth user.
// Pure: `authUser` is resolved (and only fetched) by the caller.
function parseOwner(argv: string[], authUser: string): string {
	const arg = argv[0];
	return arg !== undefined && arg.length > 0 ? arg : authUser;
}

// Parse the `gh pr list --json number,state` output (a JSON array).
// Returns the first record's {number,state}, or null when there is no
// PR. This replaces the bash `[ -z "$existing" ] || [ "$existing" = " " ]`
// guard: empty / whitespace / null / `[]` / malformed all mean "none".
function parseExistingPr(
	jsonText: string,
): { number: number; state: string } | null {
	const trimmed = jsonText.trim();
	if (trimmed.length === 0) {
		return null;
	}
	let parsed: unknown;
	try {
		parsed = JSON.parse(trimmed);
	} catch {
		return null;
	}
	if (!Array.isArray(parsed)) {
		return null;
	}
	const first: unknown = parsed[0];
	if (typeof first !== "object" || first === null) {
		return null;
	}
	const record = first as Record<string, unknown>;
	const number = record.number;
	const state = record.state;
	if (typeof number !== "number" || typeof state !== "string") {
		return null;
	}
	return { number, state };
}

// Abort the run (bash `set -e`) when a mandatory command fails.
function must(result: ShResult, what: string): void {
	if (result.exitCode !== 0) {
		const detail = result.stderr.trim();
		throw new Error(
			detail.length > 0
				? `${what}: ${detail}`
				: `${what} (exit ${result.exitCode})`,
		);
	}
}

async function runOnce(deps: Deps, argv: string[]): Promise<number> {
	// Resolve owner. Only shell out to `gh api user` when no arg was
	// given, mirroring the bash `OWNER="${1:-}"` fallback.
	const argOwner = argv[0] ?? "";
	let authUser = "";
	if (argOwner.length === 0) {
		const login = await deps.gh(["api", "user", "--jq", ".login"]);
		must(login, "gh api user");
		authUser = login.stdout.trim();
	}
	const owner = parseOwner(argv, authUser);
	const full = `${owner}/${REPO}`;

	// 1. Create the repo if missing.
	const view = await deps.gh(["repo", "view", full]);
	if (view.exitCode === 0) {
		deps.log(`repo ${full} already exists; skipping creation`);
	} else {
		deps.log(`creating ${full} ...`);
		must(
			await deps.gh([
				"repo",
				"create",
				full,
				"--public",
				"--description",
				REPO_DESCRIPTION,
				"--add-readme",
				"--clone=false",
			]),
			"gh repo create",
		);
	}

	// 2. Clone a workspace we can push from. Plain `git clone` (not
	// `gh repo clone`) so we skip gh's HTTPS proxy. Cleaned up on exit.
	const work = await deps.mkdtemp();
	try {
		must(
			await deps.git(
				[
					"clone",
					"--quiet",
					"--depth",
					"1",
					`https://github.com/${full}.git`,
					".",
				],
				work,
			),
			"git clone",
		);
		must(
			await deps.git(["config", "user.name", "jj-gt-fixture-bot"], work),
			"git config user.name",
		);
		must(
			await deps.git(
				["config", "user.email", "jj-gt-fixture-bot@users.noreply.github.com"],
				work,
			),
			"git config user.email",
		);

		// 3. Ensure the persistent fixture branch exists. Probe via the
		// REST API rather than git so it works where anonymous
		// git-over-https is blocked but gh is allowed.
		const branchProbe = await deps.gh([
			"api",
			`repos/${full}/branches/${PERSISTENT_BRANCH}`,
			"--silent",
		]);
		if (branchProbe.exitCode === 0) {
			deps.log(`branch ${PERSISTENT_BRANCH} already exists on origin`);
		} else {
			deps.log(`creating ${PERSISTENT_BRANCH} ...`);
			must(
				await deps.git(["checkout", "-b", PERSISTENT_BRANCH], work),
				"git checkout -b",
			);
			await deps.writeFile(join(work, "fixture", "README.md"), FIXTURE_README);
			must(await deps.git(["add", "fixture/README.md"], work), "git add");
			must(
				await deps.git(
					["commit", "-m", "fixture: persistent branch for jj-gt live tests"],
					work,
				),
				"git commit",
			);
			must(
				await deps.git(["push", "-u", "origin", PERSISTENT_BRANCH], work),
				"git push",
			);
		}

		// Find any PR for the branch (open OR closed). We keep the
		// fixture PR closed so it stays off the Graphite home page; the
		// tests use `--state all` and assert against the record.
		const listed = await deps.gh([
			"pr",
			"list",
			"--repo",
			full,
			"--head",
			PERSISTENT_BRANCH,
			"--state",
			"all",
			"--json",
			"number,state",
		]);
		const existing =
			listed.exitCode === 0 ? parseExistingPr(listed.stdout) : null;

		let prNumber: number;
		let prState: string;
		if (existing === null) {
			deps.log("opening fixture PR ...");
			must(
				await deps.gh([
					"pr",
					"create",
					"--repo",
					full,
					"--head",
					PERSISTENT_BRANCH,
					"--base",
					"main",
					"--title",
					PR_TITLE,
					"--body",
					PR_BODY,
				]),
				"gh pr create",
			);
			const relisted = await deps.gh([
				"pr",
				"list",
				"--repo",
				full,
				"--head",
				PERSISTENT_BRANCH,
				"--state",
				"all",
				"--json",
				"number,state",
			]);
			must(relisted, "gh pr list");
			const created = parseExistingPr(relisted.stdout);
			prNumber = created?.number ?? 0;
			prState = "OPEN";
		} else {
			prNumber = existing.number;
			prState = existing.state;
			deps.log(
				`PR for ${PERSISTENT_BRANCH} already exists (#${prNumber}, state=${prState})`,
			);
		}

		// Ensure the PR is CLOSED. Skipping the close when already
		// closed keeps the run idempotent without a redundant API write.
		if (prState === "OPEN") {
			deps.log(
				`closing fixture PR #${prNumber} to keep it off the Graphite home page ...`,
			);
			must(
				await deps.gh([
					"pr",
					"close",
					String(prNumber),
					"--repo",
					full,
					"--comment",
					CLOSE_COMMENT,
				]),
				"gh pr close",
			);
		}

		deps.log("");
		deps.log("Done. To run the gh live tests against this fixture:");
		deps.log("");
		deps.log("  export JJ_GT_LIVE_GH=1");
		deps.log(`  export JJ_GT_LIVE_REPO=${full}`);
		deps.log("  cargo nextest run --test gh_live");
	} finally {
		// bash `trap 'rm -rf "$work"' EXIT`.
		await deps.rm(work);
	}

	return 0;
}

export type { Deps, ShResult };
export { parseExistingPr, parseOwner, runOnce };

// `import.meta.main` is true only when this file is the entry point
// (`bun run index.ts`). Under `bun test` the test file is the entry,
// so this stays false and main never runs.
if (import.meta.main) {
	const toSh = (r: {
		exitCode: number;
		stdout: string | Buffer;
		stderr: string | Buffer;
	}): ShResult => ({
		exitCode: r.exitCode,
		stdout: r.stdout.toString(),
		stderr: r.stderr.toString(),
	});
	const deps: Deps = {
		gh: async (args) => toSh(await $`gh ${args}`.nothrow().quiet()),
		git: async (args, cwd) =>
			toSh(await $`git ${args}`.cwd(cwd).nothrow().quiet()),
		mkdtemp: () => mkdtemp(join(tmpdir(), "jj-gt-fixture-")),
		writeFile: async (path, content) => {
			await mkdir(dirname(path), { recursive: true });
			await writeFile(path, content);
		},
		rm: (dir) => rm(dir, { recursive: true, force: true }),
		log: (message) => {
			console.log(message);
		},
		err: (message) => {
			console.error(message);
		},
	};
	try {
		process.exit(await runOnce(deps, process.argv.slice(2)));
	} catch (error) {
		deps.err(error instanceof Error ? error.message : String(error));
		process.exit(1);
	}
}

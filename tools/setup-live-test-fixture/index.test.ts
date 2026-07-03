import { describe, expect, test } from "bun:test";
import {
	type Deps,
	parseExistingPr,
	parseOwner,
	runOnce,
	type ShResult,
} from "./index.ts";

const ok = (stdout = ""): ShResult => ({ exitCode: 0, stdout, stderr: "" });
const fail = (stderr = ""): ShResult => ({ exitCode: 1, stdout: "", stderr });

// A recorded shell invocation: the tool ("gh"/"git") + its argv.
type Call = { tool: string; args: string[] };

type Scenario = {
	// Canned `gh api user --jq .login` output.
	authUser?: string;
	// repo view exit: 0 = exists (skip create), 1 = missing (create).
	repoExists: boolean;
	// branch probe exit: 0 = exists (skip), 1 = missing (create).
	branchExists: boolean;
	// stdout returned by the FIRST `gh pr list`.
	prListJson: string;
	// stdout returned by any subsequent `gh pr list` (post-create).
	prListJsonAfterCreate?: string;
};

// Build a fake Deps that answers gh/git deterministically per the
// scenario and records every call + log line for assertion.
function harness(scenario: Scenario): {
	deps: Deps;
	calls: Call[];
	logs: string[];
} {
	const calls: Call[] = [];
	const logs: string[] = [];
	let prListSeen = 0;

	const gh = async (args: string[]): Promise<ShResult> => {
		calls.push({ tool: "gh", args });
		const [a0, a1] = args;
		if (a0 === "api" && a1 === "user") {
			return ok(`${scenario.authUser ?? "defaultuser"}\n`);
		}
		if (a0 === "repo" && a1 === "view") {
			return scenario.repoExists ? ok() : fail("not found");
		}
		if (a0 === "api" && typeof a1 === "string" && a1.startsWith("repos/")) {
			return scenario.branchExists ? ok() : fail("Branch not found");
		}
		if (a0 === "pr" && a1 === "list") {
			prListSeen += 1;
			if (prListSeen === 1) {
				return ok(scenario.prListJson);
			}
			return ok(scenario.prListJsonAfterCreate ?? scenario.prListJson);
		}
		// pr create / pr close / repo create — all succeed.
		return ok();
	};

	const git = async (args: string[]): Promise<ShResult> => {
		calls.push({ tool: "git", args });
		return ok();
	};

	const deps: Deps = {
		gh,
		git,
		mkdtemp: async () => "/tmp/fake-work",
		writeFile: async () => {},
		rm: async () => {},
		log: (message) => {
			logs.push(message);
		},
		err: (message) => {
			logs.push(`ERR:${message}`);
		},
	};
	return { deps, calls, logs };
}

const ghCalls = (calls: Call[]): string[][] =>
	calls.filter((c) => c.tool === "gh").map((c) => c.args);
const hasGh = (calls: Call[], ...prefix: string[]): boolean =>
	ghCalls(calls).some((args) => prefix.every((p, i) => args[i] === p));

describe("parseOwner", () => {
	test("explicit positional arg wins over auth user", () => {
		expect(parseOwner(["acme"], "authbot")).toBe("acme");
	});

	test("falls back to auth user when no arg", () => {
		expect(parseOwner([], "authbot")).toBe("authbot");
	});

	test("falls back to auth user when arg is empty string", () => {
		expect(parseOwner([""], "authbot")).toBe("authbot");
	});
});

describe("parseExistingPr", () => {
	test("well-formed array → first record", () => {
		expect(parseExistingPr('[{"number":42,"state":"CLOSED"}]')).toEqual({
			number: 42,
			state: "CLOSED",
		});
	});

	test("first of several records", () => {
		expect(
			parseExistingPr(
				'[{"number":7,"state":"OPEN"},{"number":8,"state":"CLOSED"}]',
			),
		).toEqual({
			number: 7,
			state: "OPEN",
		});
	});

	test("empty array → null (replaces bash -z guard)", () => {
		expect(parseExistingPr("[]")).toBeNull();
	});

	test("literal null → null", () => {
		expect(parseExistingPr("null")).toBeNull();
	});

	test("whitespace → null (replaces bash '= \" \"' guard)", () => {
		expect(parseExistingPr("   \n ")).toBeNull();
	});

	test("empty string → null", () => {
		expect(parseExistingPr("")).toBeNull();
	});

	test("malformed json → null", () => {
		expect(parseExistingPr("{not json")).toBeNull();
	});

	test("missing fields → null", () => {
		expect(parseExistingPr('[{"number":1}]')).toBeNull();
		expect(parseExistingPr('[{"state":"OPEN"}]')).toBeNull();
	});
});

describe("runOnce idempotent branches", () => {
	test("repo missing → creates it", async () => {
		const { deps, calls } = harness({
			repoExists: false,
			branchExists: true,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		expect(await runOnce(deps, ["acme"])).toBe(0);
		expect(hasGh(calls, "repo", "create")).toBe(true);
	});

	test("repo exists → skips creation with message", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		await runOnce(deps, ["acme"]);
		expect(hasGh(calls, "repo", "create")).toBe(false);
		expect(logs).toContain(
			"repo acme/jj-gt-live-tests already exists; skipping creation",
		);
	});

	test("branch missing → creates branch (checkout/add/commit/push)", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: false,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		await runOnce(deps, ["acme"]);
		const gitArgs = calls.filter((c) => c.tool === "git").map((c) => c.args);
		expect(gitArgs).toContainEqual(["checkout", "-b", "fixture/persistent-pr"]);
		expect(gitArgs).toContainEqual([
			"push",
			"-u",
			"origin",
			"fixture/persistent-pr",
		]);
		expect(logs).toContain("creating fixture/persistent-pr ...");
	});

	test("branch exists → skips branch creation", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		await runOnce(deps, ["acme"]);
		const gitArgs = calls.filter((c) => c.tool === "git").map((c) => c.args);
		expect(gitArgs).not.toContainEqual([
			"checkout",
			"-b",
			"fixture/persistent-pr",
		]);
		expect(logs).toContain(
			"branch fixture/persistent-pr already exists on origin",
		);
	});

	test("PR open → gets closed", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":9,"state":"OPEN"}]',
		});
		await runOnce(deps, ["acme"]);
		expect(hasGh(calls, "pr", "close", "9")).toBe(true);
		expect(logs).toContain(
			"closing fixture PR #9 to keep it off the Graphite home page ...",
		);
	});

	test("PR already closed → no close call", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":9,"state":"CLOSED"}]',
		});
		await runOnce(deps, ["acme"]);
		expect(hasGh(calls, "pr", "close")).toBe(false);
		expect(logs).toContain(
			"PR for fixture/persistent-pr already exists (#9, state=CLOSED)",
		);
	});

	test("no PR → creates then closes (created as OPEN)", async () => {
		const { deps, calls, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: "[]",
			prListJsonAfterCreate: '[{"number":11,"state":"OPEN"}]',
		});
		await runOnce(deps, ["acme"]);
		expect(hasGh(calls, "pr", "create")).toBe(true);
		expect(hasGh(calls, "pr", "close", "11")).toBe(true);
		expect(logs).toContain("opening fixture PR ...");
	});

	test("no arg → resolves owner via gh api user", async () => {
		const { deps, calls } = harness({
			authUser: "authbot",
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		await runOnce(deps, []);
		expect(hasGh(calls, "api", "user")).toBe(true);
		expect(hasGh(calls, "repo", "view", "authbot/jj-gt-live-tests")).toBe(true);
	});

	test("final instructions echoed with resolved full repo", async () => {
		const { deps, logs } = harness({
			repoExists: true,
			branchExists: true,
			prListJson: '[{"number":5,"state":"CLOSED"}]',
		});
		await runOnce(deps, ["acme"]);
		expect(logs).toContain(
			"Done. To run the gh live tests against this fixture:",
		);
		expect(logs).toContain("  export JJ_GT_LIVE_REPO=acme/jj-gt-live-tests");
		expect(logs).toContain("  cargo nextest run --test gh_live");
	});
});

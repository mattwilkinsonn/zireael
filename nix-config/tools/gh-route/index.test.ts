// Unit tests for gh-route — the GitHub API bucket router (SEA-1083 / SEA-932).
//
// Ported from dotfiles/scripts/tests/gh-route.test.sh. The bash tests ran the
// real script as a subprocess with a fake `gh` on PATH and a pre-seeded cache;
// here we call the exported pure functions directly, and drive the command
// dispatch through runMain() with a fully faked Deps (no network, no real gh,
// no real clock/sleep).
//
// What is defended here:
//   - pick: routes to the bucket with the higher remaining/limit fraction;
//     ties and a REST lead resolve to REST; the ≤ FLOOR guard runs before the
//     fraction compare.
//   - remaining: reports the right bucket's count.
//   - shape parity: reviews/comments/review-comments/head-sha/check-runs/pr-list
//     emit the identical normalized REST JSON whether served by REST or GraphQL,
//     including the GraphQL-only `[bot]` login suffix for Bot authors.
//   - back-off: both buckets below FLOOR emits the floor warning and returns
//     within the capped wait; headroom present returns immediately, no warning.
//   - GraphQL 100-item truncation warning.
//   - unknown command exits 2; empty command exits 2; missing arg exits 1.

import { describe, expect, test } from "bun:test";
import {
	awaitHeadroom,
	type Config,
	checkRunsFromGraphql,
	checkRunsFromRest,
	commentsFromGraphql,
	commentsFromRest,
	type Deps,
	frac,
	gqlLogin,
	hasNextPage,
	headShaFromGraphql,
	headShaFromRest,
	parseJsonStream,
	pick,
	prListFromGraphql,
	prListFromRest,
	type RateLimit,
	readConfig,
	remaining,
	reviewCommentsFromGraphql,
	reviewCommentsFromRest,
	reviewsFromGraphql,
	reviewsFromRest,
	runMain,
	slurpAdd,
	splitRepo,
} from "./index.ts";

// ── Fixtures — the SAME logical rows in REST and GraphQL native shapes, so the
// router's normalization must converge them (from gh-route.test.sh). ──────────

const REVIEWS_REST = [
	{
		user: { login: "alice" },
		state: "APPROVED",
		submitted_at: "2024-01-01T00:00:00Z",
		commit_id: "aaa111",
		body: "lgtm",
	},
	{
		user: { login: "seal-bot[bot]" },
		state: "COMMENTED",
		submitted_at: "2024-01-02T00:00:00Z",
		commit_id: "bbb222",
		body: "nit: rename",
	},
];
const REVIEWS_GQL = {
	data: {
		repository: {
			pullRequest: {
				reviews: {
					nodes: [
						{
							author: { login: "alice", __typename: "User" },
							state: "APPROVED",
							submittedAt: "2024-01-01T00:00:00Z",
							commit: { oid: "aaa111" },
							body: "lgtm",
						},
						{
							author: { login: "seal-bot", __typename: "Bot" },
							state: "COMMENTED",
							submittedAt: "2024-01-02T00:00:00Z",
							commit: { oid: "bbb222" },
							body: "nit: rename",
						},
					],
				},
			},
		},
	},
};

const COMMENTS_REST = [
	{
		user: { login: "alice" },
		body: "hi",
		created_at: "2024-01-01T00:00:00Z",
		updated_at: "2024-01-01T01:00:00Z",
	},
	{
		user: { login: "seal-bot[bot]" },
		body: "CI passed",
		created_at: "2024-01-02T00:00:00Z",
		updated_at: "2024-01-02T00:00:00Z",
	},
];
const COMMENTS_GQL = {
	data: {
		repository: {
			pullRequest: {
				comments: {
					nodes: [
						{
							author: { login: "alice", __typename: "User" },
							body: "hi",
							createdAt: "2024-01-01T00:00:00Z",
							updatedAt: "2024-01-01T01:00:00Z",
						},
						{
							author: { login: "seal-bot", __typename: "Bot" },
							body: "CI passed",
							createdAt: "2024-01-02T00:00:00Z",
							updatedAt: "2024-01-02T00:00:00Z",
						},
					],
				},
			},
		},
	},
};

const RC_REST = [
	{
		user: { login: "alice" },
		body: "style",
		path: "src/a.ts",
		line: 10,
		commit_id: "ccc333",
	},
	{
		user: { login: "seal-bot[bot]" },
		body: "unused var",
		path: "src/b.ts",
		line: 20,
		commit_id: "ddd444",
	},
];
const RC_GQL = {
	data: {
		repository: {
			pullRequest: {
				reviewThreads: {
					nodes: [
						{
							comments: {
								nodes: [
									{
										author: { login: "alice", __typename: "User" },
										body: "style",
										path: "src/a.ts",
										line: 10,
										commit: { oid: "ccc333" },
										originalCommit: { oid: "zzz000" },
									},
									{
										author: { login: "seal-bot", __typename: "Bot" },
										body: "unused var",
										path: "src/b.ts",
										line: 20,
										commit: { oid: "ddd444" },
										originalCommit: { oid: "zzz000" },
									},
								],
							},
						},
					],
				},
			},
		},
	},
};

const HEAD_SHA_REST = {
	head: { sha: "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00" },
	number: 1,
};
const HEAD_SHA_GQL = {
	data: {
		repository: {
			pullRequest: { headRefOid: "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00" },
		},
	},
};

const CHECK_RUNS_REST = {
	check_runs: [
		{ name: "build", status: "completed", conclusion: "success" },
		{ name: "lint", status: "queued", conclusion: null },
	],
};
const CHECK_RUNS_GQL = {
	data: {
		repository: {
			object: {
				checkSuites: {
					nodes: [
						{
							checkRuns: {
								nodes: [
									{ name: "build", status: "COMPLETED", conclusion: "SUCCESS" },
								],
							},
						},
						{
							checkRuns: {
								nodes: [{ name: "lint", status: "QUEUED", conclusion: null }],
							},
						},
					],
				},
			},
		},
	},
};

const PR_LIST_REST = [
	{
		number: 7,
		title: "Add feature",
		state: "open",
		head: { sha: "abc123" },
		user: { login: "alice" },
	},
	{
		number: 8,
		title: "Fix bug",
		state: "open",
		head: { sha: "def456" },
		user: { login: "dependabot[bot]" },
	},
];
const PR_LIST_GQL = {
	data: {
		repository: {
			pullRequests: {
				nodes: [
					{
						number: 7,
						title: "Add feature",
						state: "OPEN",
						headRefOid: "abc123",
						author: { login: "alice", __typename: "User" },
					},
					{
						number: 8,
						title: "Fix bug",
						state: "OPEN",
						headRefOid: "def456",
						author: { login: "dependabot", __typename: "Bot" },
					},
				],
			},
		},
	},
};

// Expected normalized outputs (from gh-route.test.sh EXP_* constants).
const EXP_REVIEWS = REVIEWS_REST;
const EXP_COMMENTS = COMMENTS_REST;
const EXP_RC = RC_REST;
const EXP_HEAD_SHA = "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00";
const EXP_CHECK_RUNS = CHECK_RUNS_REST;
const EXP_PR_LIST = PR_LIST_REST;

// ── rate_limit snapshot helpers ────────────────────────────────────────────────

// bash seed_cache: a rate_limit snapshot with core/graphql remaining/limit and a
// far-future reset so pick() never times out on stale resets.
function rl(
	cr: number,
	cl: number,
	gr: number,
	gl: number,
	reset = 9_999_999_999,
): RateLimit {
	return {
		resources: {
			core: { remaining: cr, limit: cl, reset },
			graphql: { remaining: gr, limit: gl, reset },
		},
	};
}

// ── Faked Deps ─────────────────────────────────────────────────────────────────

type GhStub = (args: string[]) => { stdout: string; exitCode: number };

type FakeState = {
	logs: string[];
	errs: string[];
	slept: number[];
	cache: string | null;
};

// A Deps whose `gh` dispatches like the bash fake `gh`: `gh api rate_limit`
// returns the seeded snapshot; `gh api graphql …` dispatches on the query
// string; `gh api <restpath>` dispatches on the path. The clock is frozen and
// sleep is virtual (records seconds, advances the clock instantly) so back-off
// never actually waits. The seeded snapshot doubles as the pre-warmed cache.
function makeDeps(opts: {
	snapshot: RateLimit;
	env?: Record<string, string | undefined>;
	// Extra gh dispatch for command tests (graphql/rest fixtures).
	gh?: GhStub;
	// If true, the cache starts empty so rateLimit() must call `gh api rate_limit`.
	emptyCache?: boolean;
	// Snapshots returned by successive `gh api rate_limit` calls (for back-off
	// where the cache is force-refreshed after each sleep).
	rlSequence?: RateLimit[];
}): { deps: Deps; state: FakeState } {
	let clock = 1_000_000;
	const state: FakeState = {
		logs: [],
		errs: [],
		slept: [],
		cache: opts.emptyCache
			? null
			: JSON.stringify({ ...opts.snapshot, _fetched_at: clock }),
	};
	let rlCalls = 0;
	const deps: Deps = {
		env: opts.env ?? {},
		gh: async (args) => {
			if (args[0] === "api" && args[1] === "rate_limit") {
				const snap = opts.rlSequence
					? (opts.rlSequence[Math.min(rlCalls, opts.rlSequence.length - 1)] ??
						opts.snapshot)
					: opts.snapshot;
				rlCalls++;
				return { stdout: JSON.stringify(snap), exitCode: 0 };
			}
			if (opts.gh) return opts.gh(args);
			return { stdout: "", exitCode: 0 };
		},
		log: (m) => state.logs.push(m),
		err: (m) => state.errs.push(m),
		now: () => clock,
		sleep: async (s) => {
			state.slept.push(s);
			clock += s; // advance virtual clock so resets eventually pass
		},
		readCache: () => state.cache,
		writeCache: (c) => {
			state.cache = c;
		},
	};
	return { deps, state };
}

// gh dispatch mirroring the bash fake `gh`: routes graphql by query substring and
// REST by path substring, returning the native-shape fixtures as JSON.
const commandGh: GhStub = (args) => {
	// args after "api": either ["graphql", "-f", "query=...", ...] or [restpath].
	const isGraphql = args.includes("graphql");
	if (isGraphql) {
		const q =
			args.find((a) => a.startsWith("query="))?.slice("query=".length) ?? "";
		if (q.includes("reviewThreads"))
			return { stdout: JSON.stringify(RC_GQL), exitCode: 0 };
		if (q.includes("reviews"))
			return { stdout: JSON.stringify(REVIEWS_GQL), exitCode: 0 };
		if (q.includes("comments"))
			return { stdout: JSON.stringify(COMMENTS_GQL), exitCode: 0 };
		if (q.includes("checkSuites"))
			return { stdout: JSON.stringify(CHECK_RUNS_GQL), exitCode: 0 };
		if (q.includes("states:OPEN"))
			return { stdout: JSON.stringify(PR_LIST_GQL), exitCode: 0 };
		if (q.includes("headRefOid"))
			return { stdout: JSON.stringify(HEAD_SHA_GQL), exitCode: 0 };
		return { stdout: "{}", exitCode: 0 };
	}
	const path = args.find((a) => a.startsWith("repos/")) ?? "";
	if (/\/issues\/.*\/comments/.test(path))
		return { stdout: JSON.stringify(COMMENTS_REST), exitCode: 0 };
	if (/\/pulls\/.*\/reviews/.test(path))
		return { stdout: JSON.stringify(REVIEWS_REST), exitCode: 0 };
	if (/\/pulls\/.*\/comments/.test(path))
		return { stdout: JSON.stringify(RC_REST), exitCode: 0 };
	if (/\/commits\/.*\/check-runs/.test(path))
		return { stdout: JSON.stringify(CHECK_RUNS_REST), exitCode: 0 };
	if (path.includes("state=open"))
		return { stdout: JSON.stringify(PR_LIST_REST), exitCode: 0 };
	if (/\/pulls\//.test(path))
		return { stdout: JSON.stringify(HEAD_SHA_REST), exitCode: 0 };
	return { stdout: "[]", exitCode: 0 };
};

// Route a command through runMain with the cache seeded so the chosen bucket
// both wins pick() AND clears FLOOR (so awaitHeadroom returns at once). Mirrors
// the bash run_route helper (rest → seed 4000/300; gql → seed 300/4000).
async function runRoute(
	mode: "rest" | "gql",
	argv: string[],
): Promise<{ logs: string[]; errs: string[] }> {
	const snapshot =
		mode === "rest" ? rl(4000, 5000, 300, 5000) : rl(300, 5000, 4000, 5000);
	const { deps, state } = makeDeps({ snapshot, gh: commandGh });
	await runMain(deps, argv);
	return { logs: state.logs, errs: state.errs };
}

// ── pick (fractional-headroom routing) ─────────────────────────────────────────

describe("pick (fractional-headroom routing)", () => {
	const FLOOR = 200;
	test("graphql-dominant fraction → graphql", () => {
		expect(pick(rl(100, 5000, 4000, 5000), FLOOR)).toBe("graphql");
	});
	test("rest-dominant fraction → rest", () => {
		expect(pick(rl(4000, 5000, 100, 5000), FLOOR)).toBe("rest");
	});
	test("equal fraction → rest (tie to idle bucket)", () => {
		expect(pick(rl(2500, 5000, 2500, 5000), FLOOR)).toBe("rest");
	});
	test("graphql exhausted → rest", () => {
		expect(pick(rl(500, 5000, 0, 5000), FLOOR)).toBe("rest");
	});
	// Floor guard: a bucket at/below FLOOR must not be chosen when the other has
	// room, even with a momentarily HIGHER fraction (0.199 vs 0.06 here). The
	// guard runs BEFORE the fraction compare; pre-hardening logic picks the
	// opposite.
	test("graphql at/below floor but higher fraction → rest (guard overrides)", () => {
		expect(pick(rl(300, 5000, 199, 1000), FLOOR)).toBe("rest");
	});
	test("rest at/below floor but higher fraction → graphql (guard overrides)", () => {
		expect(pick(rl(199, 1000, 300, 5000), FLOOR)).toBe("graphql");
	});
	// Boundary of the ≤ guard: exactly AT the floor is guarded; one above is
	// healthy and competes on fraction. The 200/201 pair straddles the boundary
	// with opposite outcomes, so an off-by-one reddens exactly one.
	test("graphql exactly at floor → rest (≤ boundary is guarded)", () => {
		expect(pick(rl(300, 5000, 200, 1000), FLOOR)).toBe("rest");
	});
	test("graphql one above floor → graphql (healthy, wins on fraction)", () => {
		expect(pick(rl(300, 5000, 201, 1000), FLOOR)).toBe("graphql");
	});
	// Fraction compare reached only once BOTH clear the floor: strictly higher
	// fraction wins (graphql 0.80 vs 0.60).
	test("both healthy, graphql higher fraction → graphql", () => {
		expect(pick(rl(3000, 5000, 4000, 5000), FLOOR)).toBe("graphql");
	});
	// frac() zero-limit guard: a missing/zero limit scores 0, so it never wins.
	test("zero-limit bucket scores 0 fraction", () => {
		expect(frac({ remaining: 100, limit: 0 })).toBe(0);
		expect(frac(undefined)).toBe(0);
		expect(frac({ remaining: 2500, limit: 5000 })).toBe(500);
	});
});

// ── remaining ──────────────────────────────────────────────────────────────────

describe("remaining", () => {
	const snap = rl(4321, 5000, 1234, 5000);
	test("remaining rest → core count", () => {
		expect(remaining(snap, "rest")).toBe(4321);
	});
	test("remaining graphql → graphql count", () => {
		expect(remaining(snap, "graphql")).toBe(1234);
	});
	test("any non-graphql arg maps to core", () => {
		expect(remaining(snap, "")).toBe(4321);
	});
	test("missing bucket → 0", () => {
		expect(remaining({}, "rest")).toBe(0);
		expect(remaining({}, "graphql")).toBe(0);
	});
});

// ── shape parity (REST branch == GraphQL branch, [bot] normalized) ─────────────

describe("shape parity: normalizers converge REST and GraphQL", () => {
	test("reviews via REST → normalized shape", () => {
		expect(reviewsFromRest([REVIEWS_REST])).toEqual(EXP_REVIEWS);
	});
	test("reviews via GraphQL → same shape ([bot] appended)", () => {
		expect(reviewsFromGraphql(REVIEWS_GQL)).toEqual(EXP_REVIEWS);
	});
	test("comments via REST → normalized shape", () => {
		expect(commentsFromRest([COMMENTS_REST])).toEqual(EXP_COMMENTS);
	});
	test("comments via GraphQL → same shape ([bot] appended)", () => {
		expect(commentsFromGraphql(COMMENTS_GQL)).toEqual(EXP_COMMENTS);
	});
	test("review-comments via REST → normalized shape", () => {
		expect(reviewCommentsFromRest([RC_REST])).toEqual(EXP_RC);
	});
	test("review-comments via GraphQL → same shape ([bot] appended)", () => {
		expect(reviewCommentsFromGraphql(RC_GQL)).toEqual(EXP_RC);
	});
	// head-sha: REST reads .head.sha; GraphQL reads headRefOid; both → bare SHA.
	test("head-sha via REST → bare head SHA", () => {
		expect(headShaFromRest(HEAD_SHA_REST)).toBe(EXP_HEAD_SHA);
	});
	test("head-sha via GraphQL → same SHA (headRefOid)", () => {
		expect(headShaFromGraphql(HEAD_SHA_GQL)).toBe(EXP_HEAD_SHA);
	});
	// check-runs: REST already lowercase, flattened across pages; GraphQL
	// ascii_downcases and flattens across checkSuites. null conclusion survives
	// as JSON null, not "null".
	test("check-runs via REST → normalized lowercase shape", () => {
		expect(checkRunsFromRest([CHECK_RUNS_REST])).toEqual(EXP_CHECK_RUNS);
	});
	test("check-runs via GraphQL → UPPERCASE downcased to same shape", () => {
		expect(checkRunsFromGraphql(CHECK_RUNS_GQL)).toEqual(EXP_CHECK_RUNS);
	});
	test("check-runs null conclusion stays JSON null (not string)", () => {
		const out = checkRunsFromGraphql(CHECK_RUNS_GQL);
		expect(out.check_runs[1]?.conclusion).toBeNull();
	});
	// pr-list: REST state lowercase + [bot] present; GraphQL downcases state and
	// appends [bot] from __typename.
	test("pr-list via REST → normalized shape", () => {
		expect(prListFromRest([PR_LIST_REST])).toEqual(EXP_PR_LIST);
	});
	test("pr-list via GraphQL → same shape (state downcased, [bot] appended)", () => {
		expect(prListFromGraphql(PR_LIST_GQL)).toEqual(EXP_PR_LIST);
	});
});

// ── [bot] login suffix (gqlLogin) ──────────────────────────────────────────────

describe("gqlLogin", () => {
	test("Bot author gets [bot] suffix", () => {
		expect(gqlLogin({ login: "seal-bot", __typename: "Bot" })).toBe(
			"seal-bot[bot]",
		);
	});
	test("User author unchanged", () => {
		expect(gqlLogin({ login: "alice", __typename: "User" })).toBe("alice");
	});
	test("missing author → empty login", () => {
		expect(gqlLogin(null)).toBe("");
		expect(gqlLogin(undefined)).toBe("");
	});
});

// ── shape parity through the full command dispatch (runMain) ───────────────────

describe("shape parity via runMain (REST branch == GraphQL branch)", () => {
	test("reviews via REST", async () => {
		const { logs } = await runRoute("rest", ["reviews", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_REVIEWS);
	});
	test("reviews via GraphQL", async () => {
		const { logs } = await runRoute("gql", ["reviews", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_REVIEWS);
	});
	test("comments via REST", async () => {
		const { logs } = await runRoute("rest", ["comments", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_COMMENTS);
	});
	test("comments via GraphQL", async () => {
		const { logs } = await runRoute("gql", ["comments", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_COMMENTS);
	});
	test("review-comments via REST", async () => {
		const { logs } = await runRoute("rest", ["review-comments", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_RC);
	});
	test("review-comments via GraphQL", async () => {
		const { logs } = await runRoute("gql", ["review-comments", "1", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_RC);
	});
	test("head-sha via REST → bare SHA", async () => {
		const { logs } = await runRoute("rest", ["head-sha", "1", "o/r"]);
		expect(logs[0]).toBe(EXP_HEAD_SHA);
	});
	test("head-sha via GraphQL → same SHA", async () => {
		const { logs } = await runRoute("gql", ["head-sha", "1", "o/r"]);
		expect(logs[0]).toBe(EXP_HEAD_SHA);
	});
	test("check-runs via REST", async () => {
		const { logs } = await runRoute("rest", ["check-runs", "abc", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_CHECK_RUNS);
	});
	test("check-runs via GraphQL", async () => {
		const { logs } = await runRoute("gql", ["check-runs", "abc", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_CHECK_RUNS);
	});
	test("pr-list via REST", async () => {
		const { logs } = await runRoute("rest", ["pr-list", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_PR_LIST);
	});
	test("pr-list via GraphQL", async () => {
		const { logs } = await runRoute("gql", ["pr-list", "o/r"]);
		expect(JSON.parse(logs[0] ?? "null")).toEqual(EXP_PR_LIST);
	});
	// pick / remaining through dispatch.
	test("pick command prints the routing decision", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(100, 5000, 4000, 5000) });
		expect(await runMain(deps, ["pick"])).toBe(0);
		expect(state.logs[0]).toBe("graphql");
	});
	test("remaining command prints the bucket count", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4321, 5000, 1234, 5000) });
		expect(await runMain(deps, ["remaining", "graphql"])).toBe(0);
		expect(state.logs[0]).toBe("1234");
	});
});

// ── back-off (both buckets < FLOOR) ────────────────────────────────────────────

describe("back-off (awaitHeadroom)", () => {
	const cfg = (over?: Partial<Config>): Config => ({
		floor: 200,
		cacheTtl: 15,
		maxWait: 1,
		...over,
	});
	test("drained emits the floor warning and returns within the cap", async () => {
		const { deps, state } = makeDeps({
			snapshot: rl(5, 5000, 5, 5000),
			env: { GH_ROUTE_MAX_WAIT: "1" },
		});
		await awaitHeadroom(deps, cfg());
		expect(state.errs.some((e) => e.includes("both buckets < 200"))).toBe(true);
		// MAX_WAIT=1: the loop returns before exceeding the cap; sleeps stay ≤ cap.
		expect(state.slept.every((s) => s <= 1)).toBe(true);
		const total = state.slept.reduce((a, b) => a + b, 0);
		expect(total).toBeLessThanOrEqual(1);
	});
	test("headroom present → no wait, no warning", async () => {
		const { deps, state } = makeDeps({
			snapshot: rl(4000, 5000, 4000, 5000),
			env: { GH_ROUTE_MAX_WAIT: "1" },
		});
		await awaitHeadroom(deps, cfg());
		expect(state.slept.length).toBe(0);
		expect(state.errs.some((e) => e.includes("both buckets"))).toBe(false);
	});
	test("sleeps to the nearest reset then clears when a bucket refills", async () => {
		// First read drained; after the sleep the forced refresh returns headroom.
		const drained = rl(5, 5000, 5, 5000, 1_000_050);
		const refilled = rl(4000, 5000, 4000, 5000, 1_000_050);
		const { deps, state } = makeDeps({
			snapshot: drained,
			emptyCache: true,
			rlSequence: [drained, refilled],
			env: { GH_ROUTE_MAX_WAIT: "300" },
		});
		await awaitHeadroom(deps, cfg({ maxWait: 300 }));
		// reset(1_000_050) - now(1_000_000) + 1 = 51s sleep, then headroom.
		expect(state.slept).toEqual([51]);
		expect(state.errs.some((e) => e.includes("both buckets < 200"))).toBe(true);
	});
});

// ── GraphQL 100-item truncation warning (hasNextPage) ──────────────────────────

describe("GraphQL truncation warning (hasNextPage)", () => {
	test("hasNextPage detects a true anywhere in the tree", () => {
		expect(hasNextPage({ a: { b: { pageInfo: { hasNextPage: true } } } })).toBe(
			true,
		);
	});
	test("hasNextPage ignores false (jq // drops it)", () => {
		expect(hasNextPage({ pageInfo: { hasNextPage: false } })).toBe(false);
	});
	test("hasNextPage over arrays", () => {
		expect(hasNextPage([{ x: 1 }, { hasNextPage: true }])).toBe(true);
	});
	test("runMain warns when a GraphQL list hit the 100-item page cap", async () => {
		const truncatedReviews = {
			data: {
				repository: {
					pullRequest: {
						reviews: {
							pageInfo: { hasNextPage: true },
							nodes: REVIEWS_GQL.data.repository.pullRequest.reviews.nodes,
						},
					},
				},
			},
		};
		const gh: GhStub = (args) => {
			if (args.includes("graphql"))
				return { stdout: JSON.stringify(truncatedReviews), exitCode: 0 };
			return { stdout: "[]", exitCode: 0 };
		};
		const { deps, state } = makeDeps({
			snapshot: rl(300, 5000, 4000, 5000), // force graphql, has headroom
			gh,
		});
		await runMain(deps, ["reviews", "1", "o/r"]);
		expect(
			state.errs.some((e) =>
				e.includes("truncated at the 100-item GraphQL page cap"),
			),
		).toBe(true);
	});
	test("no truncation warning when hasNextPage is false", async () => {
		const { errs } = await runRoute("gql", ["reviews", "1", "o/r"]);
		expect(errs.some((e) => e.includes("truncated"))).toBe(false);
	});
});

// ── dispatch: exit codes ───────────────────────────────────────────────────────

describe("dispatch exit codes", () => {
	test("unknown command exits 2 and warns", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, ["totally-bogus"])).toBe(2);
		expect(state.errs.some((e) => e.includes("unknown command"))).toBe(true);
	});
	test("empty command prints help and exits 2", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, [])).toBe(2);
		expect(state.logs[0]).toContain("gh-route <cmd> [args]");
	});
	test("-h prints help and exits 0", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, ["-h"])).toBe(0);
		expect(state.logs[0]).toContain("gh-route <cmd> [args]");
	});
	test("--help prints help and exits 0", async () => {
		const { deps } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, ["--help"])).toBe(0);
	});
	test("remaining with no bucket → usage + exit 1", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, ["remaining"])).toBe(1);
		expect(state.errs[0]).toContain("usage: gh-route remaining");
	});
	test("head-sha with no pr → usage + exit 1", async () => {
		const { deps, state } = makeDeps({ snapshot: rl(4000, 5000, 4000, 5000) });
		expect(await runMain(deps, ["head-sha"])).toBe(1);
		expect(state.errs[0]).toContain("usage: gh-route head-sha");
	});
});

// ── plumbing: parseJsonStream / slurpAdd / splitRepo / readConfig ──────────────

describe("plumbing", () => {
	test("parseJsonStream splits concatenated --paginate pages", () => {
		expect(parseJsonStream('[{"a":1}]\n[{"b":2}]')).toEqual([
			[{ a: 1 }],
			[{ b: 2 }],
		]);
	});
	test("parseJsonStream tolerates braces inside strings", () => {
		expect(parseJsonStream('[{"body":"a } { b"}]')).toEqual([
			[{ body: "a } { b" }],
		]);
	});
	test("parseJsonStream skips a balanced-but-invalid page instead of throwing", () => {
		// Brackets balance (so a slice is cut) but the content isn't valid JSON;
		// the guard must drop it, not crash. The good page still comes through.
		expect(parseJsonStream('[},{]\n[{"b":2}]')).toEqual([[{ b: 2 }]]);
	});
	test("slurpAdd flattens array pages (add // [])", () => {
		expect(slurpAdd([[{ a: 1 }], [{ b: 2 }, { c: 3 }]])).toEqual([
			{ a: 1 },
			{ b: 2 },
			{ c: 3 },
		]);
	});
	test("splitRepo splits owner/name", () => {
		expect(splitRepo("octocat/hello")).toEqual({
			owner: "octocat",
			name: "hello",
		});
	});
	test("readConfig defaults and overrides", () => {
		expect(readConfig({})).toEqual({ floor: 200, cacheTtl: 15, maxWait: 300 });
		expect(
			readConfig({
				GH_ROUTE_FLOOR: "50",
				GH_ROUTE_CACHE_TTL: "5",
				GH_ROUTE_MAX_WAIT: "10",
			}),
		).toEqual({ floor: 50, cacheTtl: 5, maxWait: 10 });
	});
});

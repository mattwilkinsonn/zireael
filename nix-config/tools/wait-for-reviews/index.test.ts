// Regression tests for wait-for-reviews, ported 1:1 from the bash suite
// (dotfiles/scripts/tests/wait-for-reviews.test.sh). Two seams:
//   1. The tern isError / JSON-RPC-error guard — here `extractTernText` fed
//      canned SSE `data:` frames. Load-bearing: a tern error must yield null
//      (caller falls back to gh-route) instead of emitting the error text as
//      review-state JSON.
//   2. Arg validation — `parseArgs` directly, and `runOnce` with injected
//      fakes (fake ghRoute + sleep + tiny backstop) so accepted args exit
//      fast, asserting the banner text and exit code. No network, no real gh.

import { describe, expect, test } from "bun:test";
import {
	classify,
	type Deps,
	extractTernText,
	parseArgs,
	runOnce,
} from "./index.ts";

// ─── tern isError/error guard (extractTernText) ──────────────────────────
describe("tern isError/error guard", () => {
	// Tool-level error: content text present but isError:true. Pre-fix this
	// text was emitted as $state and crashed the next parse. Must return null.
	test("tool isError:true → null (empty state → fallback)", () => {
		const frame =
			'event: message\ndata: {"result":{"content":[{"text":"github REST 404: pull request not found"}],"isError":true}}';
		expect(extractTernText(frame)).toBeNull();
	});

	// JSON-RPC transport error: no result, just .error. Same contract.
	test("JSON-RPC .error → null", () => {
		const frame =
			'event: message\ndata: {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"boom"}}';
		expect(extractTernText(frame)).toBeNull();
	});

	// Success: isError:false, content text is the review-state JSON. The guard
	// must let it through unchanged so the caller can parse .head_sha.
	test("success (isError:false) → inner head_sha JSON", () => {
		const frame =
			'event: message\ndata: {"result":{"content":[{"text":"{\\"head_sha\\":\\"abc\\"}"}],"isError":false}}';
		expect(extractTernText(frame)).toBe('{"head_sha":"abc"}');
	});

	// No data: frame at all → null (nothing to parse).
	test("no data: frame → null", () => {
		expect(extractTernText("event: message\n:keepalive")).toBeNull();
	});
});

// ─── arg validation (parseArgs) ──────────────────────────────────────────
describe("parseArgs", () => {
	test("no PR arg → not ok, no message (bare usage)", () => {
		const r = parseArgs([]);
		expect(r.ok).toBe(false);
		expect(r.ok === false && r.msg).toBeUndefined();
	});

	test("leading-dash where PR goes → not ok", () => {
		expect(parseArgs(["--repo"]).ok).toBe(false);
	});

	test("'305 --bogus' → unknown option", () => {
		const r = parseArgs(["305", "--bogus"]);
		expect(r.ok).toBe(false);
		expect(r.ok === false && r.msg).toContain("unknown option");
	});

	test("'305 --repo' (no value) → needs value", () => {
		const r = parseArgs(["305", "--repo"]);
		expect(r.ok).toBe(false);
		expect(r.ok === false && r.msg).toContain(
			"--repo needs an owner/repo value",
		);
	});

	test("'305 --repo --bogus' (dash value) → needs value", () => {
		const r = parseArgs(["305", "--repo", "--bogus"]);
		expect(r.ok).toBe(false);
		expect(r.ok === false && r.msg).toContain(
			"--repo needs an owner/repo value",
		);
	});

	test("'305 --repo=' (empty equals) → needs value", () => {
		const r = parseArgs(["305", "--repo="]);
		expect(r.ok).toBe(false);
		expect(r.ok === false && r.msg).toContain(
			"--repo needs an owner/repo value",
		);
	});

	test("'305 --repo owner/repo' (space form) → ok", () => {
		const r = parseArgs(["305", "--repo", "owner/repo"]);
		expect(r).toEqual({ ok: true, pr: "305", repo: "owner/repo" });
	});

	test("'305 --repo=owner/repo' (equals form) → ok", () => {
		const r = parseArgs(["305", "--repo=owner/repo"]);
		expect(r).toEqual({ ok: true, pr: "305", repo: "owner/repo" });
	});

	test("'305 owner/repo' (positional) → ok", () => {
		const r = parseArgs(["305", "owner/repo"]);
		expect(r).toEqual({ ok: true, pr: "305", repo: "owner/repo" });
	});

	test("'305' alone → ok, empty repo (default-resolve later)", () => {
		const r = parseArgs(["305"]);
		expect(r).toEqual({ ok: true, pr: "305", repo: "" });
	});
});

// ─── runOnce: arg exits + accepted banner (injected fakes, no network) ───
type Captured = { out: string[]; err: string[] };

function makeDeps(
	argv: string[],
	overrides: Partial<Deps> = {},
): {
	deps: Deps;
	cap: Captured;
} {
	const cap: Captured = { out: [], err: [] };
	// Monotonic clock: each call advances a second so the poll loop's elapsed
	// crosses a tiny backstop and exits deterministically.
	let t = 0;
	const deps: Deps = {
		argv,
		env: {
			WAIT_BACKSTOP_SECS: "1",
			WAIT_GRACE_SECS: "0",
			WAIT_POLL_SECS: "0",
		},
		fetch: (async () => {
			throw new Error("fetch must not be called in tests");
		}) as unknown as typeof fetch,
		log: (m) => cap.out.push(m),
		err: (m) => cap.err.push(m),
		now: () => t++,
		sleep: async () => {},
		// tern OFF (no gateway env) → head/reviews/comments come from gh-route.
		ghRoute: async (cmd) =>
			cmd === "head-sha" ? "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" : "[]",
		repoView: async () => "owner/repo",
		...overrides,
	};
	return { deps, cap };
}

describe("runOnce arg validation", () => {
	test("no PR arg → exit 2, 'usage' on stderr", async () => {
		const { deps, cap } = makeDeps([]);
		expect(await runOnce(deps)).toBe(2);
		expect(cap.err.join("\n")).toContain("usage");
	});

	test("'305 --bogus' → exit 2, 'unknown option' on stderr", async () => {
		const { deps, cap } = makeDeps(["305", "--bogus"]);
		expect(await runOnce(deps)).toBe(2);
		expect(cap.err.join("\n")).toContain("unknown option");
	});

	test("'305 --repo' (no value) → exit 2, needs-value on stderr", async () => {
		const { deps, cap } = makeDeps(["305", "--repo"]);
		expect(await runOnce(deps)).toBe(2);
		expect(cap.err.join("\n")).toContain("--repo needs an owner/repo value");
	});

	test("'305 --repo --bogus' (dash value) → exit 2, needs-value", async () => {
		const { deps, cap } = makeDeps(["305", "--repo", "--bogus"]);
		expect(await runOnce(deps)).toBe(2);
		expect(cap.err.join("\n")).toContain("--repo needs an owner/repo value");
	});

	test("'305 --repo=' (empty equals) → exit 2, needs-value", async () => {
		const { deps, cap } = makeDeps(["305", "--repo="]);
		expect(await runOnce(deps)).toBe(2);
		expect(cap.err.join("\n")).toContain("--repo needs an owner/repo value");
	});

	test("'305 --repo owner/repo' → not an arg exit + header prints", async () => {
		const { deps, cap } = makeDeps(["305", "--repo", "owner/repo"]);
		const rc = await runOnce(deps);
		expect(rc).not.toBe(2);
		expect(cap.out.join("\n")).toContain("wait-for-reviews: owner/repo#305");
	});

	test("'305 --repo=owner/repo' → not an arg exit + header prints", async () => {
		const { deps, cap } = makeDeps(["305", "--repo=owner/repo"]);
		const rc = await runOnce(deps);
		expect(rc).not.toBe(2);
		expect(cap.out.join("\n")).toContain("wait-for-reviews: owner/repo#305");
	});
});

// ─── banner format preservation ──────────────────────────────────────────
describe("banner format", () => {
	test("exact format with 10-char head, bots list, tern:off", async () => {
		const { deps, cap } = makeDeps(["305", "--repo=owner/repo"], {
			env: {
				WAIT_BOTS: "greptile-apps coderabbitai",
				WAIT_BACKSTOP_SECS: "1",
				WAIT_GRACE_SECS: "0",
				WAIT_POLL_SECS: "0",
			},
		});
		await runOnce(deps);
		expect(cap.out[0]).toBe(
			"wait-for-reviews: owner/repo#305 head=deadbeefde bots=[greptile-apps coderabbitai] grace=0s backstop=1s (tern:off)",
		);
	});

	test("backstop line names still-blocking bots", async () => {
		const { deps, cap } = makeDeps(["305", "--repo=owner/repo"], {
			env: {
				WAIT_BOTS: "greptile-apps",
				WAIT_BACKSTOP_SECS: "1",
				WAIT_GRACE_SECS: "0",
				WAIT_POLL_SECS: "0",
			},
		});
		expect(await runOnce(deps)).toBe(0);
		expect(cap.out.join("\n")).toContain(
			"backstop 1s reached; proceeding. still blocking: greptile-apps",
		);
	});
});

// ─── classify: per-bot verdicts ──────────────────────────────────────────
const HEAD = "abcdef0123456789abcdef0123456789abcdef01";
const SHORT = HEAD.slice(0, 7);

function review(login: string, commitId: string): unknown {
	return { user: { login }, commit_id: commitId };
}
function comment(login: string, body: string, updated = "2024-01-01"): unknown {
	return { user: { login }, body, updated_at: updated };
}

describe("classify", () => {
	test("no data → pending", () => {
		expect(
			classify("greptile-apps", { reviews: [], comments: [], head: HEAD }),
		).toBe("pending");
	});

	// default branch (cubic-dev-ai): review at head → done.
	test("default bot review at head → done", () => {
		const reviews = [review("cubic-dev-ai", HEAD)];
		expect(
			classify("cubic-dev-ai", { reviews, comments: [], head: HEAD }),
		).toBe("done");
	});

	test("default bot review at earlier commit → stale", () => {
		const reviews = [review("cubic-dev-ai", "old")];
		expect(
			classify("cubic-dev-ai", { reviews, comments: [], head: HEAD }),
		).toBe("stale");
	});

	// [bot] login suffix is stripped + lowercased before matching.
	test("[bot] suffix and case are normalized", () => {
		const reviews = [review("Cubic-Dev-AI[bot]", HEAD)];
		expect(
			classify("cubic-dev-ai", { reviews, comments: [], head: HEAD }),
		).toBe("done");
	});

	// coderabbitai: keys on its edited summary comment body naming the head.
	test("coderabbitai body names head → done", () => {
		const comments = [comment("coderabbitai", `Review of ${HEAD} done`)];
		expect(
			classify("coderabbitai", { reviews: [], comments, head: HEAD }),
		).toBe("done");
	});

	test("coderabbitai body names short head → done", () => {
		const comments = [comment("coderabbitai", `Reviewed ${SHORT}`)];
		expect(
			classify("coderabbitai", { reviews: [], comments, head: HEAD }),
		).toBe("done");
	});

	test("coderabbitai rate-limited → limited", () => {
		const comments = [
			comment("coderabbitai", "> [!WARNING] Rate limited by CodeRabbit.ai"),
		];
		expect(
			classify("coderabbitai", { reviews: [], comments, head: HEAD }),
		).toBe("limited");
	});

	test("coderabbitai old comment not naming head → stale", () => {
		const comments = [comment("coderabbitai", "Reviewed some earlier commit")];
		expect(
			classify("coderabbitai", { reviews: [], comments, head: HEAD }),
		).toBe("stale");
	});

	// chatgpt-codex-connector: trusts the review commit_id, ignores on usage limit.
	test("codex review at head → done", () => {
		const reviews = [review("chatgpt-codex-connector", HEAD)];
		expect(
			classify("chatgpt-codex-connector", {
				reviews,
				comments: [],
				head: HEAD,
			}),
		).toBe("done");
	});

	test("codex usage-limited comment → limited", () => {
		const comments = [
			comment(
				"chatgpt-codex-connector",
				"You have hit usage limits for code reviews.",
			),
		];
		expect(
			classify("chatgpt-codex-connector", {
				reviews: [],
				comments,
				head: HEAD,
			}),
		).toBe("limited");
	});

	test("codex review at earlier commit → stale", () => {
		const reviews = [review("chatgpt-codex-connector", "old")];
		expect(
			classify("chatgpt-codex-connector", {
				reviews,
				comments: [],
				head: HEAD,
			}),
		).toBe("stale");
	});

	// greptile-apps: review at head OR summary naming head → done.
	test("greptile review at head → done", () => {
		const reviews = [review("greptile-apps", HEAD)];
		expect(
			classify("greptile-apps", { reviews, comments: [], head: HEAD }),
		).toBe("done");
	});

	test("greptile summary comment names head → done", () => {
		const comments = [comment("greptile-apps", `head ${HEAD}`)];
		expect(
			classify("greptile-apps", { reviews: [], comments, head: HEAD }),
		).toBe("done");
	});

	test("greptile only an earlier comment → stale", () => {
		const comments = [comment("greptile-apps", "earlier note")];
		expect(
			classify("greptile-apps", { reviews: [], comments, head: HEAD }),
		).toBe("stale");
	});

	// Picks the latest comment by updated_at for the body test.
	test("coderabbitai keys on the newest comment body", () => {
		const comments = [
			comment("coderabbitai", "old note", "2024-01-01"),
			comment("coderabbitai", `now at ${HEAD}`, "2024-06-01"),
		];
		expect(
			classify("coderabbitai", { reviews: [], comments, head: HEAD }),
		).toBe("done");
	});
});

// ─── fail-closed on unresolved head (empty head SHA) ─────────────────────
// Regression: head === "" made classify()'s `cbody.includes(head)` always
// true (String.includes("")), so every commenting bot false-matched the head
// — a false all-clear, the worst output for a review gate. The gate must fail
// LOUD on an unresolved head, never emit a verdict. Two layers: runOnce aborts
// before classify (exit 1); classify itself returns the blocking verdict as
// defense-in-depth. Each test below FAILS on the pre-guard code and PASSES
// after.
describe("fail-closed on empty head", () => {
	// classify — comment-keyed bots (the always-true `includes("")` path): must
	// NOT be reported done on an empty head. Pre-guard these returned "done".
	test("coderabbitai with empty head → pending, never done", () => {
		const comments = [comment("coderabbitai", "Reviewed some commit")];
		expect(classify("coderabbitai", { reviews: [], comments, head: "" })).toBe(
			"pending",
		);
	});

	test("greptile-apps with empty head → pending, never done", () => {
		const comments = [comment("greptile-apps", "a review summary")];
		expect(classify("greptile-apps", { reviews: [], comments, head: "" })).toBe(
			"pending",
		);
	});

	// classify — formal-review bots yield no usable verdict on an empty head
	// either: a real-head review must not be salvaged into done/stale. Pre-guard
	// these returned "stale" (revHead=0 against ""), a misleading non-blocking-ish
	// signal; the guard forces the blocking "pending".
	test("cubic-dev-ai with empty head → pending even with a review present", () => {
		const reviews = [review("cubic-dev-ai", "abc1234")];
		expect(classify("cubic-dev-ai", { reviews, comments: [], head: "" })).toBe(
			"pending",
		);
	});

	test("codex with empty head → pending even with a review present", () => {
		const reviews = [review("chatgpt-codex-connector", "abc1234")];
		expect(
			classify("chatgpt-codex-connector", { reviews, comments: [], head: "" }),
		).toBe("pending");
	});

	// runOnce — the load-bearing gate fix: tern off AND gh-route head-sha empty
	// → abort loud (exit 1), never a false all-clear (pre-guard: looped to the
	// backstop and returned 0).
	test("runOnce aborts (exit 1) when tern + gh-route both give empty head", async () => {
		const { deps, cap } = makeDeps(["305", "--repo=owner/repo"], {
			ghRoute: async (cmd) => (cmd === "head-sha" ? "" : "[]"),
		});
		expect(await runOnce(deps)).toBe(1);
		expect(cap.err.join("\n")).toContain("failing closed");
	});

	// runOnce — whitespace-only head is empty after trim: same abort.
	test("runOnce aborts (exit 1) when gh-route head is whitespace only", async () => {
		const { deps, cap } = makeDeps(["305", "--repo=owner/repo"], {
			ghRoute: async (cmd) => (cmd === "head-sha" ? "  \n" : "[]"),
		});
		expect(await runOnce(deps)).toBe(1);
		expect(cap.err.join("\n")).toContain("failing closed");
	});

	// runOnce — no over-correction (condition #3): tern off but gh-route
	// resolves a real head → proceed normally, never exit 1.
	test("runOnce proceeds when gh-route resolves the head (tern off)", async () => {
		const { deps } = makeDeps(["305", "--repo=owner/repo"], {
			ghRoute: async (cmd) =>
				cmd === "head-sha" ? "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" : "[]",
		});
		expect(await runOnce(deps)).not.toBe(1);
	});
});

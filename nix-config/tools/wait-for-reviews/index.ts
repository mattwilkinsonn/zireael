// wait-for-reviews <pr> [owner/repo | --repo owner/repo] — block until the
// PR's review bots have reviewed the current head (or are rate/usage-limited,
// or reviewed an earlier commit and didn't re-trigger within a grace window,
// or a backstop elapses), so an agent can push → wait → triage in one
// autonomous loop.
//
// Per-bot state each poll:
//   done    — reviewed the CURRENT head (commit_id == head, or summary names it)
//   limited — rate/usage-limited (don't block on it)
//   stale   — reviewed an EARLIER commit but not the head
//   pending — no review from this bot yet
//
// A `stale` bot blocks only until WAIT_GRACE_SECS (reported as
// "stale(waiting)" while blocking), then counts as done-stale; a `pending`
// bot blocks until WAIT_BACKSTOP_SECS.
//
// Review-state is read tern-first (the caching review MCP via the LiteLLM
// gateway) to collapse the fleet's N reads into one batched upstream fetch;
// it falls back to the `gh-route` bin (bucket-balanced REST/GraphQL) when the
// gateway env is absent or tern is unreachable.
//
// Env: WAIT_BOTS (space list), WAIT_BACKSTOP_SECS (1200), WAIT_GRACE_SECS
//      (300), WAIT_POLL_SECS (30), LITELLM_MCP_URL, LITELLM_API_KEY.
//
// Exit codes:
//   0 - reviews ready, or backstop reached (proceed either way)
//   1 - head SHA unresolved (tern + gh-route both empty) — failed closed
//   2 - bad arguments (usage)

import { $ } from "bun";

// ─── small unknown-narrowing helpers ─────────────────────────────────────
// Review/comment JSON comes from outside (tern or gh-route), so we read every
// field through guards that verify the shape at runtime — never an inline cast.

function prop(o: unknown, key: string): unknown {
	if (o !== null && typeof o === "object") return Reflect.get(o, key);
	return undefined;
}

function readString(o: unknown, key: string): string | undefined {
	const v = prop(o, key);
	return typeof v === "string" ? v : undefined;
}

function asArray(v: unknown): unknown[] {
	return Array.isArray(v) ? v : [];
}

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

// A bot's login, lowercased with a trailing/embedded "[bot]" stripped —
// matches jq's `.user.login | ascii_downcase | sub("\\[bot\\]";"")`.
function normLogin(item: unknown): string {
	const login = readString(prop(item, "user"), "login") ?? "";
	return login.toLowerCase().replace("[bot]", "");
}

// ─── tern SSE extraction + isError guard ─────────────────────────────────
// The JSON-RPC result rides on an SSE `data:` line. A tool-level error
// (result.isError) or a JSON-RPC error (.error) MUST yield null so the caller
// falls back to gh-route instead of treating the error text as review-state
// JSON — the load-bearing regression this guard exists to prevent.
function extractTernText(resp: string): string | null {
	const frames: string[] = [];
	for (const line of resp.split(/\r?\n/)) {
		if (line.startsWith("data: ")) frames.push(line.slice("data: ".length));
	}
	if (frames.length === 0) return null;
	const texts: string[] = [];
	for (const frame of frames) {
		const parsed = safeJson(frame);
		if (parsed === undefined) return null;
		const errorField = prop(parsed, "error");
		if (errorField !== null && errorField !== undefined) return null;
		const result = prop(parsed, "result");
		if (prop(result, "isError") === true) return null;
		const content = prop(result, "content");
		const first = Array.isArray(content) ? content[0] : undefined;
		const text = readString(first, "text");
		if (text === undefined) return null;
		texts.push(text);
	}
	return texts.join("\n");
}

type TernOpts = { url: string; key: string; repo: string; pr: string };

// Fetch tern's full review-state JSON string, or null on any failure (missing
// gateway env, network/timeout, or a tern-level error caught by the guard).
async function ternState(
	deps: Pick<Deps, "fetch">,
	opts: TernOpts,
): Promise<string | null> {
	const { url, key, repo, pr } = opts;
	if (!url || !key) return null;
	const owner = repo.split("/")[0] ?? "";
	const name = repo.slice(repo.lastIndexOf("/") + 1);
	const payload = {
		jsonrpc: "2.0",
		id: 1,
		method: "tools/call",
		params: {
			name: "get_review_state",
			arguments: { owner, repo: name, number: Number(pr) },
		},
	};
	const controller = new AbortController();
	const timer = setTimeout(() => controller.abort(), 25_000);
	let text: string;
	try {
		const res = await deps.fetch(url, {
			method: "POST",
			headers: {
				"x-litellm-api-key": `Bearer ${key}`,
				"Content-Type": "application/json",
				Accept: "application/json, text/event-stream",
				"x-mcp-servers": "tern",
			},
			body: JSON.stringify(payload),
			signal: controller.signal,
		});
		text = await res.text();
	} catch {
		return null;
	} finally {
		clearTimeout(timer);
	}
	return extractTernText(text);
}

// ─── arg parsing ─────────────────────────────────────────────────────────
// PR positional (reject missing or a leading-dash where the number goes), then
// an optional repo as a positional `owner/repo`, `--repo owner/repo`, or
// `--repo=x`. Any other leading-dash token is rejected, not taken as the repo.
type ParseResult =
	| { ok: true; pr: string; repo: string }
	| { ok: false; msg?: string };

function parseArgs(argv: string[]): ParseResult {
	const pr = argv[0] ?? "";
	if (pr === "" || pr.startsWith("-")) return { ok: false };
	const next = argv[1] ?? "";
	let repo = "";
	if (next === "--repo") {
		const val = argv[2] ?? "";
		if (val === "" || val.startsWith("-")) {
			return {
				ok: false,
				msg: "wait-for-reviews: --repo needs an owner/repo value",
			};
		}
		repo = val;
	} else if (next.startsWith("--repo=")) {
		repo = next.slice("--repo=".length);
		if (repo === "") {
			return {
				ok: false,
				msg: "wait-for-reviews: --repo needs an owner/repo value",
			};
		}
	} else if (next.startsWith("-")) {
		return { ok: false, msg: `wait-for-reviews: unknown option '${next}'` };
	} else if (next !== "") {
		repo = next;
	}
	return { ok: true, pr, repo };
}

// ─── per-bot classification ──────────────────────────────────────────────
type Verdict = "done" | "limited" | "stale" | "pending";

function classify(
	bot: string,
	ctx: { reviews: unknown[]; comments: unknown[]; head: string },
): Verdict {
	const { reviews, comments, head } = ctx;
	// Fail-closed: an empty head must never yield "done". `cbody.includes("")`
	// is always true, so without this guard every commenting bot false-matches
	// the head — a false all-clear. Return the blocking verdict, never "done".
	if (head === "") return "pending";
	const byBot = reviews.filter((r) => normLogin(r) === bot);
	const revAny = byBot.length;
	const revHead = byBot.filter(
		(r) => readString(r, "commit_id") === head,
	).length;

	const botComments = comments.filter((c) => normLogin(c) === bot);
	botComments.sort((a, b) => {
		const av = readString(a, "updated_at") ?? "";
		const bv = readString(b, "updated_at") ?? "";
		return av < bv ? -1 : av > bv ? 1 : 0;
	});
	const last = botComments[botComments.length - 1];
	const cbody = last ? (readString(last, "body") ?? "") : "";
	// grep -qE "$head|${head:0:7}" — head or its short form appears in the body.
	const matchesHead = cbody.includes(head) || cbody.includes(head.slice(0, 7));

	switch (bot) {
		case "coderabbitai":
			if (cbody.toLowerCase().includes("rate limited by coderabbit.ai"))
				return "limited";
			if (matchesHead) return "done";
			if (cbody !== "") return "stale";
			break;
		case "chatgpt-codex-connector":
			if (cbody.toLowerCase().includes("usage limits for code reviews"))
				return "limited";
			if (revHead > 0) return "done";
			if (revAny > 0) return "stale";
			break;
		case "greptile-apps":
			if (revHead > 0) return "done";
			if (matchesHead) return "done";
			if (revAny > 0 || cbody !== "") return "stale";
			break;
		default:
			if (revHead > 0) return "done";
			if (revAny > 0) return "stale";
			break;
	}
	return "pending";
}

// ─── runner ──────────────────────────────────────────────────────────────
type Deps = {
	argv: string[];
	env: Record<string, string | undefined>;
	fetch: typeof fetch;
	log: (msg: string) => void;
	err: (msg: string) => void;
	now: () => number; // seconds
	sleep: (secs: number) => Promise<void>;
	ghRoute: (cmd: string, ...args: string[]) => Promise<string>;
	// Resolve the current checkout's owner/repo when none was given. Injected
	// so tests never shell out; production wires `gh repo view`.
	repoView?: () => Promise<string>;
};

async function runOnce(deps: Deps): Promise<number> {
	const { env, log, err } = deps;

	const parsed = parseArgs(deps.argv);
	if (!parsed.ok) {
		if (parsed.msg) err(parsed.msg);
		err("usage: wait-for-reviews <pr> [owner/repo | --repo owner/repo]");
		err("  env: WAIT_BOTS WAIT_GRACE_SECS WAIT_BACKSTOP_SECS WAIT_POLL_SECS");
		return 2;
	}

	const pr = parsed.pr;
	let repo = parsed.repo;
	if (repo === "") {
		const view = deps.repoView ?? defaultRepoView;
		repo = (await view()).trim();
	}

	const botsRaw =
		env.WAIT_BOTS ??
		"coderabbitai cubic-dev-ai greptile-apps chatgpt-codex-connector";
	const bots = botsRaw.split(/\s+/).filter((b) => b !== "");
	const backstop = Number(env.WAIT_BACKSTOP_SECS ?? "1200");
	const grace = Number(env.WAIT_GRACE_SECS ?? "300");
	const interval = Number(env.WAIT_POLL_SECS ?? "30");

	const ternOpts: TernOpts = {
		url: env.LITELLM_MCP_URL ?? "",
		key: env.LITELLM_API_KEY ?? "",
		repo,
		pr,
	};

	// Head SHA: prefer tern's cached state (no upstream call for the fleet);
	// else the adaptive router. An empty head must never reach the classifier
	// (an empty grep matches every line and misreports every bot as "done").
	let state = await ternState(deps, ternOpts);
	let head = "";
	if (state) head = readString(safeJson(state), "head_sha") ?? "";
	if (head === "") {
		head = (await deps.ghRoute("head-sha", pr, repo)).trim();
		state = null;
	}

	// Fail-closed: a review-gate tool that cannot resolve the head MUST NOT
	// emit a verdict. With head === "" the classifier's `cbody.includes(head)`
	// is always-true (String.includes("")), so every commenting bot would be
	// misreported "done" — a false all-clear, the worst output for a gate.
	// tern down AND gh-route head-sha empty → abort loud; never classify() on "".
	if (head === "") {
		err(
			`wait-for-reviews: could not resolve head SHA for ${repo}#${pr} (tern + gh-route both empty); failing closed — ground per-surface review state in the GitHub API on the real head SHA`,
		);
		return 1;
	}

	log(
		`wait-for-reviews: ${repo}#${pr} head=${head.slice(0, 10)} bots=[${botsRaw}] grace=${grace}s backstop=${backstop}s (tern:${state ? "on" : "off"})`,
	);
	const start = deps.now();

	// Seed the first iteration with the state already fetched for the head SHA;
	// cleared at each loop end to force a fresh read next poll.
	let stJson: string | null = state;
	for (;;) {
		if (!stJson) stJson = await ternState(deps, ternOpts);
		let reviews: unknown[];
		let comments: unknown[];
		if (stJson) {
			const st = safeJson(stJson);
			reviews = asArray(prop(st, "reviews"));
			comments = asArray(prop(st, "comments"));
		} else {
			reviews = asArray(safeJson(await deps.ghRoute("reviews", pr, repo)));
			comments = asArray(safeJson(await deps.ghRoute("comments", pr, repo)));
		}

		const elapsed = deps.now() - start;
		const blocking: string[] = [];
		for (const bot of bots) {
			let st: string = classify(bot, { reviews, comments, head });
			// A stale bot blocks only until the grace window passes.
			if (st === "stale" && elapsed < grace) st = "stale(waiting)";
			log(`  ${bot.padEnd(26)} ${st}`);
			if (st === "pending" || st === "stale(waiting)") blocking.push(bot);
		}

		if (blocking.length === 0) {
			log("wait-for-reviews: reviews ready (done / limited / stale)");
			return 0;
		}
		if (elapsed >= backstop) {
			log(
				`wait-for-reviews: backstop ${backstop}s reached; proceeding. still blocking: ${blocking.join(" ")}`,
			);
			return 0;
		}
		stJson = null; // force a fresh tern read on the next poll
		await deps.sleep(interval);
	}
}

// ─── production wiring (never reached under bun test) ─────────────────────
async function defaultRepoView(): Promise<string> {
	return await $`gh repo view --json nameWithOwner -q .nameWithOwner`
		.nothrow()
		.text();
}

async function realGhRoute(cmd: string, ...args: string[]): Promise<string> {
	// `|| true` / `2>/dev/null` in bash → tolerate nonzero exit, return stdout.
	const res = await $`gh-route ${cmd} ${args}`.nothrow().quiet();
	return res.stdout.toString();
}

export type { Deps, ParseResult, TernOpts, Verdict };
export { classify, extractTernText, parseArgs, runOnce, ternState };

if (import.meta.main) {
	process.exit(
		await runOnce({
			argv: process.argv.slice(2),
			env: process.env,
			fetch: globalThis.fetch,
			log: (msg) => console.log(msg),
			err: (msg) => console.error(msg),
			now: () => Math.floor(Date.now() / 1000),
			sleep: (secs) => Bun.sleep(secs * 1000),
			ghRoute: realGhRoute,
			repoView: defaultRepoView,
		}),
	);
}

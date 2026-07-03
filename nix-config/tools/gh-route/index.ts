// gh-route <cmd> [args] — route a GitHub read to the API bucket (REST `core` vs
// GraphQL) with more headroom, so a fleet of agents sharing one token drains
// both 5000/hr buckets evenly instead of exhausting GraphQL while REST sits
// idle. (bun/TypeScript port of dotfiles/scripts/gh-route — SEA-932 / SEA-1083.)
//
// Every read emits the GitHub REST JSON shape regardless of which bucket served
// it, so callers are bucket-agnostic (a drop-in for `gh api repos/O/R/...`).
// `pick` exposes the routing decision on its own, for an agent choosing between
// this tool (REST) and the github MCP (GraphQL) for an interactive read.
//
// Commands (mirror the bash main()):
//   pick                               → `rest` | `graphql` (higher headroom)
//   remaining <rest|graphql>           → that bucket's remaining request count
//   head-sha        <pr>  [owner/repo] → PR head SHA (bare string)
//   reviews         <pr>  [owner/repo] → REST-shaped review array
//   comments        <pr>  [owner/repo] → REST-shaped issue-comment array
//   review-comments <pr>  [owner/repo] → REST-shaped inline review-comment array
//   check-runs      <ref> [owner/repo] → REST-shaped {check_runs:[...]} for a ref
//   pr-list               [owner/repo] → REST-shaped open-PR array
//
// Routing: reads live remaining/limit for both buckets (`gh api rate_limit`,
// which counts against neither bucket, cached GH_ROUTE_CACHE_TTL=15s in a
// per-uid tmp file shared across agents) and sends each read to the bucket with
// the higher remaining/limit fraction — never a bucket at/below GH_ROUTE_FLOOR
// when the other has room. If both buckets sit below FLOOR it waits for the
// nearest reset (capped at GH_ROUTE_MAX_WAIT) rather than spinning.
//
// The GraphQL branch caps list fields at 100 (unpaginated) and warns to stderr
// when a connection reports hasNextPage; the REST branch `--paginate`s.
//
// Env: GH_ROUTE_FLOOR (200), GH_ROUTE_CACHE_TTL (15), GH_ROUTE_MAX_WAIT (300),
//      GH_ROUTE_CACHE (per-uid tmp path). `gh` resolves from the dev set.
//
// Exit codes:
//   0 - normal exit
//   1 - missing required subcommand argument (e.g. `remaining` with no bucket)
//   2 - empty or unknown command

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { $ } from "bun";

// ── Config ──────────────────────────────────────────────────────────────────

type Config = { floor: number; cacheTtl: number; maxWait: number };

function intEnv(v: string | undefined, def: number): number {
	if (v === undefined || v === "") return def;
	const n = Number.parseInt(v, 10);
	return Number.isNaN(n) ? def : n;
}

function readConfig(env: Record<string, string | undefined>): Config {
	return {
		floor: intEnv(env.GH_ROUTE_FLOOR, 200),
		cacheTtl: intEnv(env.GH_ROUTE_CACHE_TTL, 15),
		maxWait: intEnv(env.GH_ROUTE_MAX_WAIT, 300),
	};
}

// ── Rate-limit shapes ─────────────────────────────────────────────────────────

type Bucket = { remaining?: number; limit?: number; reset?: number };
type RateLimit = {
	resources?: { core?: Bucket; graphql?: Bucket };
	_fetched_at?: number;
};

// ── Normalized REST output shapes ─────────────────────────────────────────────

type User = { login: string };
type ReviewOut = {
	user: User;
	state: string;
	submitted_at: string;
	commit_id: string | null;
	body: string;
};
type CommentOut = {
	user: User;
	body: string;
	created_at: string;
	updated_at: string;
};
type ReviewCommentOut = {
	user: User;
	body: string;
	path: string;
	line: number | null;
	commit_id: string | null;
};
type CheckRun = { name: string; status: string; conclusion: string | null };
type CheckRunsOut = { check_runs: CheckRun[] };
type PrOut = {
	number: number;
	title: string;
	state: string;
	head: { sha: string };
	user: User;
};

// ── Pure routing logic (bash `pick` / `remaining`, lines 71-91) ───────────────

// remaining/limit as a per-mille fraction; a missing or zero-limit bucket
// scores 0 (jq: `if .limit > 0 then .remaining*1000/.limit else 0`).
function frac(bucket: Bucket | undefined): number {
	const limit = bucket?.limit ?? 0;
	if (limit > 0) return ((bucket?.remaining ?? 0) * 1000) / limit;
	return 0;
}

// `rest` | `graphql` — the bucket with the higher remaining/limit fraction, but
// never a bucket at/below FLOOR when the other still has room (the ≤ guard runs
// BEFORE the fraction compare). Ties and a REST lead resolve to REST.
function pick(rl: RateLimit, floor: number): "rest" | "graphql" {
	const restR = rl.resources?.core?.remaining ?? 0;
	const gqlR = rl.resources?.graphql?.remaining ?? 0;
	if (gqlR <= floor && restR > floor) return "rest";
	if (restR <= floor && gqlR > floor) return "graphql";
	if (frac(rl.resources?.graphql) > frac(rl.resources?.core)) return "graphql";
	return "rest";
}

function remaining(rl: RateLimit, which: string): number {
	const key = which === "graphql" ? "graphql" : "core";
	return rl.resources?.[key]?.remaining ?? 0;
}

// ── owner/repo split (bash `_repo` %%/* and ##*/, lines 122-131) ──────────────

function splitRepo(repo: string): { owner: string; name: string } {
	const slash = repo.indexOf("/");
	const last = repo.lastIndexOf("/");
	return {
		owner: slash >= 0 ? repo.slice(0, slash) : repo,
		name: last >= 0 ? repo.slice(last + 1) : repo,
	};
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

// `gh api --paginate` on an array endpoint emits one JSON value per page,
// concatenated. Split the stream into top-level JSON values (bash relied on
// `jq -s` to slurp them). Whitespace between values is skipped; strings and
// escapes are tracked so braces inside string literals don't fool the depth
// counter.
function parseJsonStream(text: string): unknown[] {
	const values: unknown[] = [];
	let depth = 0;
	let inStr = false;
	let esc = false;
	let start = -1;
	for (let i = 0; i < text.length; i++) {
		const c = text[i];
		if (c === undefined) continue;
		if (start === -1) {
			if (c === " " || c === "\n" || c === "\t" || c === "\r") continue;
			start = i;
		}
		if (inStr) {
			if (esc) esc = false;
			else if (c === "\\") esc = true;
			else if (c === '"') inStr = false;
			continue;
		}
		if (c === '"') {
			inStr = true;
			continue;
		}
		if (c === "{" || c === "[") depth++;
		else if (c === "}" || c === "]") {
			depth--;
			if (depth === 0) {
				try {
					values.push(JSON.parse(text.slice(start, i + 1)));
				} catch {
					// Malformed page (e.g. a truncated --paginate stream) — skip it,
					// matching `jq -s`'s tolerance rather than crashing the process.
				}
				start = -1;
			}
		}
	}
	return values;
}

// jq `add // []` over a slurped stream of page arrays: concatenate every array
// page into one flat list (non-array pages are ignored, matching `add`).
function slurpAdd(pages: unknown[]): Record<string, unknown>[] {
	const out: Record<string, unknown>[] = [];
	for (const page of pages) {
		if (Array.isArray(page)) {
			for (const item of page) out.push(item as Record<string, unknown>);
		}
	}
	return out;
}

// ── GraphQL field normalizers ─────────────────────────────────────────────────

type GqlAuthor =
	| { login?: string | null; __typename?: string }
	| null
	| undefined;

// GraphQL-only `[bot]` suffix for Bot authors (jq `login` def): REST logins
// already carry `[bot]`, so appending it here converges the two shapes.
function gqlLogin(author: GqlAuthor): string {
	const base = author?.login ?? "";
	return author?.__typename === "Bot" ? `${base}[bot]` : base;
}

function asStr(v: unknown): string {
	return typeof v === "string" ? v : "";
}

function asRec(v: unknown): Record<string, unknown> {
	return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}

function asArr(v: unknown): unknown[] {
	return Array.isArray(v) ? v : [];
}

// `[.. | objects | .hasNextPage? // empty] | any` — any connection in the
// response reporting hasNextPage=true (jq `//` drops the `false` values, so
// only a literal `true` counts).
function hasNextPage(node: unknown): boolean {
	if (Array.isArray(node)) return node.some(hasNextPage);
	if (node && typeof node === "object") {
		const rec = node as Record<string, unknown>;
		if (rec.hasNextPage === true) return true;
		return Object.values(rec).some(hasNextPage);
	}
	return false;
}

function reviewsFromGraphql(res: unknown): ReviewOut[] {
	const pr = asRec(asRec(asRec(asRec(res).data).repository).pullRequest);
	return asArr(asRec(pr.reviews).nodes).map((raw) => {
		const n = asRec(raw);
		return {
			user: { login: gqlLogin(n.author as GqlAuthor) },
			state: asStr(n.state),
			submitted_at: asStr(n.submittedAt),
			commit_id: (asRec(n.commit).oid as string | undefined) ?? null,
			body: asStr(n.body),
		};
	});
}

function reviewsFromRest(pages: unknown[]): ReviewOut[] {
	return slurpAdd(pages).map((it) => ({
		user: { login: asStr(asRec(it.user).login) },
		state: asStr(it.state),
		submitted_at: asStr(it.submitted_at),
		commit_id: (it.commit_id as string | null | undefined) ?? null,
		body: asStr(it.body),
	}));
}

function commentsFromGraphql(res: unknown): CommentOut[] {
	const pr = asRec(asRec(asRec(asRec(res).data).repository).pullRequest);
	return asArr(asRec(pr.comments).nodes).map((raw) => {
		const n = asRec(raw);
		return {
			user: { login: gqlLogin(n.author as GqlAuthor) },
			body: asStr(n.body),
			created_at: asStr(n.createdAt),
			updated_at: asStr(n.updatedAt),
		};
	});
}

function commentsFromRest(pages: unknown[]): CommentOut[] {
	return slurpAdd(pages).map((it) => ({
		user: { login: asStr(asRec(it.user).login) },
		body: asStr(it.body),
		created_at: asStr(it.created_at),
		updated_at: asStr(it.updated_at),
	}));
}

function reviewCommentNodes(res: unknown): unknown[] {
	const pr = asRec(asRec(asRec(asRec(res).data).repository).pullRequest);
	const threads = asArr(asRec(pr.reviewThreads).nodes);
	const out: unknown[] = [];
	for (const t of threads) {
		for (const c of asArr(asRec(asRec(t).comments).nodes)) out.push(c);
	}
	return out;
}

function reviewCommentsFromGraphql(res: unknown): ReviewCommentOut[] {
	return reviewCommentNodes(res).map((raw) => {
		const n = asRec(raw);
		const commitOid = asRec(n.commit).oid as string | undefined;
		const origOid = asRec(n.originalCommit).oid as string | undefined;
		return {
			user: { login: gqlLogin(n.author as GqlAuthor) },
			body: asStr(n.body),
			path: asStr(n.path),
			line: (n.line as number | null | undefined) ?? null,
			commit_id: commitOid ?? origOid ?? null,
		};
	});
}

function reviewCommentsFromRest(pages: unknown[]): ReviewCommentOut[] {
	return slurpAdd(pages).map((it) => ({
		user: { login: asStr(asRec(it.user).login) },
		body: asStr(it.body),
		path: asStr(it.path),
		line: (it.line as number | null | undefined) ?? null,
		commit_id: (it.commit_id as string | null | undefined) ?? null,
	}));
}

function checkRunsFromGraphql(res: unknown): CheckRunsOut {
	const obj = asRec(asRec(asRec(asRec(res).data).repository).object);
	const suites = asArr(asRec(obj.checkSuites).nodes);
	const runs: CheckRun[] = [];
	for (const s of suites) {
		for (const raw of asArr(asRec(asRec(s).checkRuns).nodes)) {
			const n = asRec(raw);
			const conclusion = n.conclusion;
			runs.push({
				name: asStr(n.name),
				status: asStr(n.status).toLowerCase(),
				conclusion:
					typeof conclusion === "string" ? conclusion.toLowerCase() : null,
			});
		}
	}
	return { check_runs: runs };
}

function checkRunsFromRest(pages: unknown[]): CheckRunsOut {
	const runs: CheckRun[] = [];
	for (const page of pages) {
		for (const raw of asArr(asRec(page).check_runs)) {
			const n = asRec(raw);
			runs.push({
				name: asStr(n.name),
				status: asStr(n.status),
				conclusion: (n.conclusion as string | null | undefined) ?? null,
			});
		}
	}
	return { check_runs: runs };
}

function prListFromGraphql(res: unknown): PrOut[] {
	const repo = asRec(asRec(asRec(res).data).repository);
	return asArr(asRec(repo.pullRequests).nodes).map((raw) => {
		const n = asRec(raw);
		return {
			number: n.number as number,
			title: asStr(n.title),
			state: asStr(n.state).toLowerCase(),
			head: { sha: asStr(n.headRefOid) },
			user: { login: gqlLogin(n.author as GqlAuthor) },
		};
	});
}

function prListFromRest(pages: unknown[]): PrOut[] {
	return slurpAdd(pages).map((it) => ({
		number: it.number as number,
		title: asStr(it.title),
		state: asStr(it.state),
		head: { sha: asStr(asRec(it.head).sha) },
		user: { login: asStr(asRec(it.user).login) },
	}));
}

function headShaFromGraphql(res: unknown): string {
	const pr = asRec(asRec(asRec(asRec(res).data).repository).pullRequest);
	return asStr(pr.headRefOid);
}

function headShaFromRest(res: unknown): string {
	return asStr(asRec(asRec(res).head).sha);
}

// ── Effectful dependencies ─────────────────────────────────────────────────────

type GhResult = { stdout: string; exitCode: number };

type Deps = {
	env: Record<string, string | undefined>;
	// Runs the `gh` CLI with the given argv; never throws (bash used `|| true` /
	// `2>/dev/null` in places), so callers inspect exitCode.
	gh: (args: string[]) => Promise<GhResult>;
	log: (msg: string) => void;
	err: (msg: string) => void;
	now: () => number; // unix seconds (bash `date +%s`)
	sleep: (seconds: number) => Promise<void>;
	readCache: () => string | null; // cache file contents, or null if absent/empty
	writeCache: (content: string) => void;
};

// Cached rate_limit JSON (bash `_rate_limit`, lines 54-69). The rate_limit
// endpoint counts against no bucket, so caching is a latency + shared-view
// optimization. `_fetched_at` is stored inside the JSON. `forceRefresh` mirrors
// the `rm -f CACHE_FILE` after a back-off sleep so the next read is fresh.
async function rateLimit(
	deps: Deps,
	cfg: Config,
	forceRefresh = false,
): Promise<RateLimit> {
	const now = deps.now();
	let cachedAt = 0;
	const cached = deps.readCache();
	if (cached && !forceRefresh) {
		try {
			cachedAt = (JSON.parse(cached) as RateLimit)._fetched_at ?? 0;
		} catch {
			cachedAt = 0;
		}
	}
	if (forceRefresh || now - cachedAt >= cfg.cacheTtl) {
		const res = await deps.gh(["api", "rate_limit"]);
		if (res.exitCode === 0 && res.stdout.trim() !== "") {
			try {
				const parsed = JSON.parse(res.stdout) as RateLimit;
				parsed._fetched_at = now;
				deps.writeCache(JSON.stringify(parsed));
			} catch {
				// Leave any existing cache in place (bash: mv only on success).
			}
		}
	}
	const finalRaw = deps.readCache();
	if (!finalRaw) return {};
	try {
		return JSON.parse(finalRaw) as RateLimit;
	} catch {
		return {};
	}
}

// Block until a bucket clears FLOOR, or MAX_WAIT elapses (bash `_await_headroom`,
// lines 95-120). Sleeps to the nearest reset when both are drained; warns to
// stderr each sleep. After sleeping, forces a fresh rate_limit read.
async function awaitHeadroom(deps: Deps, cfg: Config): Promise<void> {
	let waited = 0;
	let force = false;
	for (;;) {
		const rl = await rateLimit(deps, cfg, force);
		const restR = rl.resources?.core?.remaining ?? 0;
		const gqlR = rl.resources?.graphql?.remaining ?? 0;
		if (restR > cfg.floor || gqlR > cfg.floor) return;
		const now = deps.now();
		const resets = [
			rl.resources?.core?.reset,
			rl.resources?.graphql?.reset,
		].filter((r): r is number => r != null && r > now);
		const resetMin = resets.length > 0 ? Math.min(...resets) : now + 60;
		let sleepS = resetMin - now + 1;
		if (sleepS < 1) sleepS = 1;
		if (sleepS > cfg.maxWait) sleepS = cfg.maxWait;
		if (waited + sleepS > cfg.maxWait) return;
		deps.err(
			`gh-route: both buckets < ${cfg.floor} (rest=${restR} gql=${gqlR}); waiting ${sleepS}s for reset`,
		);
		await deps.sleep(sleepS);
		waited += sleepS;
		force = true; // fresh read after the reset (bash `rm -f CACHE_FILE`)
	}
}

// owner/repo — explicit arg wins; else resolve from the local checkout via
// `gh repo view` (bash `_repo`).
async function resolveRepo(
	deps: Deps,
	arg: string | undefined,
): Promise<string> {
	if (arg) return arg;
	const res = await deps.gh([
		"repo",
		"view",
		"--json",
		"nameWithOwner",
		"-q",
		".nameWithOwner",
	]);
	return res.stdout.trim();
}

// Run a GraphQL query and warn to stderr if any connection hit its 100-item page
// cap (bash `_gql`, lines 138-149). Returns the parsed response.
async function runGraphql(
	deps: Deps,
	label: string,
	query: string,
	fields: string[],
): Promise<unknown> {
	const args = ["api", "graphql", "-f", `query=${query}`, ...fields];
	const res = await deps.gh(args);
	let parsed: unknown = {};
	try {
		parsed = JSON.parse(res.stdout);
	} catch {
		parsed = {};
	}
	if (hasNextPage(parsed)) {
		deps.err(
			`gh-route: ${label} truncated at the 100-item GraphQL page cap; the same read via REST paginates fully — re-run when REST has headroom, or use the MCP for the complete list.`,
		);
	}
	return parsed;
}

async function restPages(deps: Deps, path: string): Promise<unknown[]> {
	const res = await deps.gh(["api", "--paginate", path]);
	return parseJsonStream(res.stdout);
}

// GraphQL query strings (identical to the bash heredocs; `$var` are GraphQL
// variables bound via `-F`, kept literal).
const Q_HEAD_SHA =
	"query($o:String!,$n:String!,$p:Int!){repository(owner:$o,name:$n){pullRequest(number:$p){headRefOid}}}";
const Q_REVIEWS =
	"query($o:String!,$n:String!,$p:Int!){repository(owner:$o,name:$n){pullRequest(number:$p){reviews(first:100){pageInfo{hasNextPage} nodes{author{login __typename} state submittedAt commit{oid} body}}}}}";
const Q_COMMENTS =
	"query($o:String!,$n:String!,$p:Int!){repository(owner:$o,name:$n){pullRequest(number:$p){comments(first:100){pageInfo{hasNextPage} nodes{author{login __typename} body createdAt updatedAt}}}}}";
const Q_REVIEW_COMMENTS =
	"query($o:String!,$n:String!,$p:Int!){repository(owner:$o,name:$n){pullRequest(number:$p){reviewThreads(first:100){pageInfo{hasNextPage} nodes{comments(first:100){pageInfo{hasNextPage} nodes{author{login __typename} body path line commit{oid} originalCommit{oid}}}}}}}}";
const Q_CHECK_RUNS =
	"query($o:String!,$n:String!,$r:String!){repository(owner:$o,name:$n){object(expression:$r){... on Commit{checkSuites(first:20){pageInfo{hasNextPage} nodes{checkRuns(first:100){pageInfo{hasNextPage} nodes{name status conclusion}}}}}}}}";
const Q_PR_LIST =
	"query($o:String!,$n:String!){repository(owner:$o,name:$n){pullRequests(states:OPEN,first:100){pageInfo{hasNextPage} nodes{number title state headRefOid author{login __typename}}}}}";

// ── Command handlers ───────────────────────────────────────────────────────────

async function cmdHeadSha(
	deps: Deps,
	cfg: Config,
	pr: string,
	repoArg?: string,
) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "head-sha", Q_HEAD_SHA, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
			"-F",
			`p=${pr}`,
		]);
		deps.log(headShaFromGraphql(res));
	} else {
		const res = await deps.gh(["api", `repos/${repo}/pulls/${pr}`]);
		let parsed: unknown = {};
		try {
			parsed = JSON.parse(res.stdout);
		} catch {
			parsed = {};
		}
		deps.log(headShaFromRest(parsed));
	}
}

async function cmdReviews(
	deps: Deps,
	cfg: Config,
	pr: string,
	repoArg?: string,
) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "reviews", Q_REVIEWS, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
			"-F",
			`p=${pr}`,
		]);
		deps.log(JSON.stringify(reviewsFromGraphql(res)));
	} else {
		const pages = await restPages(deps, `repos/${repo}/pulls/${pr}/reviews`);
		deps.log(JSON.stringify(reviewsFromRest(pages)));
	}
}

async function cmdComments(
	deps: Deps,
	cfg: Config,
	pr: string,
	repoArg?: string,
) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "comments", Q_COMMENTS, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
			"-F",
			`p=${pr}`,
		]);
		deps.log(JSON.stringify(commentsFromGraphql(res)));
	} else {
		const pages = await restPages(deps, `repos/${repo}/issues/${pr}/comments`);
		deps.log(JSON.stringify(commentsFromRest(pages)));
	}
}

async function cmdReviewComments(
	deps: Deps,
	cfg: Config,
	pr: string,
	repoArg?: string,
) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "review-comments", Q_REVIEW_COMMENTS, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
			"-F",
			`p=${pr}`,
		]);
		deps.log(JSON.stringify(reviewCommentsFromGraphql(res)));
	} else {
		const pages = await restPages(deps, `repos/${repo}/pulls/${pr}/comments`);
		deps.log(JSON.stringify(reviewCommentsFromRest(pages)));
	}
}

async function cmdCheckRuns(
	deps: Deps,
	cfg: Config,
	ref: string,
	repoArg?: string,
) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "check-runs", Q_CHECK_RUNS, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
			"-F",
			`r=${ref}`,
		]);
		deps.log(JSON.stringify(checkRunsFromGraphql(res)));
	} else {
		const pages = await restPages(
			deps,
			`repos/${repo}/commits/${ref}/check-runs`,
		);
		deps.log(JSON.stringify(checkRunsFromRest(pages)));
	}
}

async function cmdPrList(deps: Deps, cfg: Config, repoArg?: string) {
	const repo = await resolveRepo(deps, repoArg);
	const { owner, name } = splitRepo(repo);
	await awaitHeadroom(deps, cfg);
	if (pick(await rateLimit(deps, cfg), cfg.floor) === "graphql") {
		const res = await runGraphql(deps, "pr-list", Q_PR_LIST, [
			"-F",
			`o=${owner}`,
			"-F",
			`n=${name}`,
		]);
		deps.log(JSON.stringify(prListFromGraphql(res)));
	} else {
		const pages = await restPages(deps, `repos/${repo}/pulls?state=open`);
		deps.log(JSON.stringify(prListFromRest(pages)));
	}
}

// The header comment block, rendered exactly as the bash `-h`/`--help` path
// (`sed -n '1,40p' "$0" | sed 's/^# \{0,1\}//;s/^#$//'`).
const HELP_TEXT = [
	"gh-route <cmd> [args] — route a GitHub read to the API bucket (REST `core` vs",
	"GraphQL) with more headroom, so a fleet of agents sharing one token drains both",
	"5000/hr buckets evenly instead of exhausting GraphQL while REST sits idle.",
	"",
	"Every read emits the GitHub REST JSON shape regardless of which bucket served",
	"it, so callers are bucket-agnostic (a drop-in for `gh api repos/O/R/...`).",
	"`pick` exposes the routing decision on its own, for an agent choosing between",
	"this tool (REST) and the github MCP (GraphQL) for an interactive read.",
	"",
	"Commands:",
	"  pick                              → `rest` | `graphql` (higher fractional headroom)",
	"  remaining <rest|graphql>          → that bucket's remaining request count",
	"  head-sha        <pr>  [owner/repo] → PR head SHA (bare string)",
	"  reviews         <pr>  [owner/repo] → REST-shaped review array",
	"  comments        <pr>  [owner/repo] → REST-shaped issue-comment array",
	"  review-comments <pr>  [owner/repo] → REST-shaped inline review-comment array",
	"  check-runs      <ref> [owner/repo] → REST-shaped {check_runs:[...]} for a ref",
	"  pr-list               [owner/repo] → REST-shaped open-PR array",
	"",
	"Routing: reads live remaining/limit for both buckets (`gh api rate_limit`,",
	"which counts against neither bucket, cached GH_ROUTE_CACHE_TTL=15s in a per-uid",
	"tmp file shared across agents) and sends each read to the bucket with the higher",
	"remaining/limit fraction — so it flips either way and never merely relocates the",
	"wall onto REST. If both buckets sit below GH_ROUTE_FLOOR, it waits for the",
	"nearest reset (capped at GH_ROUTE_MAX_WAIT) rather than spinning against an",
	"exhausted API, per GitHub's rate-limit guidance.",
	"",
	"Shape note: emits the fields consumers use (login / commit_id / state / body /",
	"timestamps / path / line / conclusion), normalized across both APIs. Inline",
	"thread state (`is_resolved` / `is_outdated`) and thread node IDs live only on",
	"GraphQL and are NOT part of this shape — resolve/reply stay on the github MCP.",
	"The GraphQL branch caps list fields at 100 (unpaginated); the REST branch",
	"`--paginate`s, so prefer REST for a PR with >100 reviews/comments.",
	"",
	"Env: GH_ROUTE_FLOOR (200), GH_ROUTE_CACHE_TTL (15), GH_ROUTE_MAX_WAIT (300),",
	"     GH_ROUTE_CACHE (per-uid tmp path). gh resolves from the dev set.",
].join("\n");

// Command dispatch mirroring bash main() (lines 253-274). Returns the exit code.
async function runMain(deps: Deps, argv: string[]): Promise<number> {
	const cfg = readConfig(deps.env);
	const cmd = argv[0] ?? "";
	const a = argv.slice(1);
	switch (cmd) {
		case "pick":
			deps.log(pick(await rateLimit(deps, cfg), cfg.floor));
			return 0;
		case "remaining": {
			const which = a[0];
			if (which === undefined) {
				deps.err("usage: gh-route remaining <rest|graphql>");
				return 1;
			}
			deps.log(String(remaining(await rateLimit(deps, cfg), which)));
			return 0;
		}
		case "head-sha": {
			const pr = a[0];
			if (pr === undefined) {
				deps.err("usage: gh-route head-sha <pr> [owner/repo]");
				return 1;
			}
			await cmdHeadSha(deps, cfg, pr, a[1]);
			return 0;
		}
		case "reviews": {
			const pr = a[0];
			if (pr === undefined) {
				deps.err("usage: gh-route reviews <pr> [owner/repo]");
				return 1;
			}
			await cmdReviews(deps, cfg, pr, a[1]);
			return 0;
		}
		case "comments": {
			const pr = a[0];
			if (pr === undefined) {
				deps.err("usage: gh-route comments <pr> [owner/repo]");
				return 1;
			}
			await cmdComments(deps, cfg, pr, a[1]);
			return 0;
		}
		case "review-comments": {
			const pr = a[0];
			if (pr === undefined) {
				deps.err("usage: gh-route review-comments <pr> [owner/repo]");
				return 1;
			}
			await cmdReviewComments(deps, cfg, pr, a[1]);
			return 0;
		}
		case "check-runs": {
			const ref = a[0];
			if (ref === undefined) {
				deps.err("usage: gh-route check-runs <ref> [owner/repo]");
				return 1;
			}
			await cmdCheckRuns(deps, cfg, ref, a[1]);
			return 0;
		}
		case "pr-list":
			await cmdPrList(deps, cfg, a[0]);
			return 0;
		case "":
			deps.log(HELP_TEXT);
			return 2;
		case "-h":
		case "--help":
			deps.log(HELP_TEXT);
			return 0;
		default:
			deps.err(
				`gh-route: unknown command '${cmd}' (try: pick remaining head-sha reviews comments review-comments check-runs pr-list)`,
			);
			return 2;
	}
}

export type { Config, Deps, RateLimit };
export {
	awaitHeadroom,
	checkRunsFromGraphql,
	checkRunsFromRest,
	commentsFromGraphql,
	commentsFromRest,
	frac,
	gqlLogin,
	HELP_TEXT,
	hasNextPage,
	headShaFromGraphql,
	headShaFromRest,
	parseJsonStream,
	pick,
	prListFromGraphql,
	prListFromRest,
	rateLimit,
	readConfig,
	remaining,
	reviewCommentsFromGraphql,
	reviewCommentsFromRest,
	reviewsFromGraphql,
	reviewsFromRest,
	runMain,
	slurpAdd,
	splitRepo,
};

// Run only when executed directly (not imported by the test file).
if (import.meta.main) {
	const cachePath =
		process.env.GH_ROUTE_CACHE ??
		`${process.env.TMPDIR ?? "/tmp"}/gh-route-rl.${process.getuid?.() ?? 0}.json`;
	const deps: Deps = {
		env: process.env,
		gh: async (args) => {
			const res = await $`gh ${args}`.nothrow().quiet();
			return { stdout: res.stdout.toString(), exitCode: res.exitCode };
		},
		log: (msg) => console.log(msg),
		err: (msg) => console.error(msg),
		now: () => Math.floor(Date.now() / 1000),
		sleep: (seconds) => Bun.sleep(seconds * 1000),
		readCache: () => {
			try {
				if (!existsSync(cachePath)) return null;
				const content = readFileSync(cachePath, "utf8");
				return content.length > 0 ? content : null;
			} catch {
				return null;
			}
		},
		writeCache: (content) => {
			try {
				writeFileSync(cachePath, content);
			} catch {
				// A cache write failure is non-fatal (bash: rm the tmp, move on).
			}
		},
	};
	process.exit(await runMain(deps, process.argv.slice(2)));
}

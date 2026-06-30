import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// Hard guardrails that hold even if the model is confused or a prompt tries to
// override them. Under the seal-bot push model an agent MAY push feature
// branches, open/update PRs, and run its own review loop on allowlisted-owner
// repos — but these stay blocked, always:
//   - pushing or force-pushing `main` (merge is the human gate),
//   - merging a PR,
//   - any *write* (push / PR / issue) to a repo outside the owner allowlist
//     (an OSS upstream like `can1357/*`), and
//   - broad pattern-matching process kills.
// Identity + policy: rule://commit-conventions, skill://autonomous-review.

// The only GitHub owners an agent may push to or open PRs/issues on. Enforced
// here on top of the seal-bot PAT's own repo scope. Lowercase — GitHub owners
// are case-insensitive.
const ALLOWED_OWNERS: Record<string, true> = {
	mattwilkinsonn: true,
	sealedsecurity: true,
};
const PUSH =
	/\bgit(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+push\b|\bjj(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+git\s+push\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*(?:submit|ss?)\b/;
// Merge — the human gate, blocked for every repo.
const MERGE = /\bgh\b[^\n;|&]*\bmerge\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*merge\b/;
// A `gh` mutation: a write verb anywhere in the `gh` segment (so flags between
// the noun and the verb — `gh pr -R owner/repo create` — don't slip past).
const GH_WRITE_CMD =
	/\bgh\b[^\n;|&]*\b(?:create|edit|comment|close|delete|review|reopen|ready|lock|unlock|develop|transfer|rename|sync|fork)\b/;
// Pushing to a remote literally named `upstream` — the OSS-upstream vector a
// URL-owner check can't see (`git push upstream` carries no URL/owner).
const PUSH_UPSTREAM = /\bpush\b(?:\s+-\S+)*\s+upstream\b/;
// `main` as an explicit push target — `origin main`, `-b main`, `:main`. Scoped
// so it doesn't fire on a feature branch that merely contains "main".
const PUSH_MAIN = /:(?:refs\/heads\/)?main(?![\w-])|\brefs\/heads\/main(?![\w-])|(?:^|\s)(?:-b|--bookmark|--branch)\s+main(?![\w-])|\borigin\s+(?:-\S+\s+)*main(?![\w-])/;

// Explicit GitHub owners named in a command: full URLs + `gh -R owner/repo`.
function namedOwners(cmd: string): string[] {
	const owners = new Set<string>();
	for (const m of cmd.matchAll(/github\.com[/:]([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
	for (const m of cmd.matchAll(/(?:-R|--repo)[=\s]+([\w.-]+)\/[\w.-]+/g)) owners.add(m[1].toLowerCase());
	return [...owners];
}

// A `gh` command that *creates* an issue or PR. A bare `gh issue create` /
// `gh pr create` with no allowlisted `-R`/URL files on whatever repo the cwd
// resolves to — an OSS upstream included (the spam-bot vector). Require an
// explicit allowlisted target. Command-aware (`gh` is the actual command, not a
// word in a message) so `git commit -m "… gh issue create …"` doesn't trip it.
function ghCreateWithoutAllowedOwner(cmd: string): boolean {
	for (const seg of cmd.split(/[\n;|&]+/)) {
		const toks = seg.trim().split(/\s+/).filter(Boolean);
		let i = 0;
		while (i < toks.length && /^[A-Za-z_]\w*=/.test(toks[i])) i++; // skip env prefix
		if (i >= toks.length || !/(?:^|\/)gh$/.test(toks[i])) continue; // gh isn't the command
		const rest = toks.slice(i + 1);
		if (!rest.includes("create") || !(rest.includes("issue") || rest.includes("pr"))) continue;
		// An allowlisted -R/URL makes it safe; no owner (or a disallowed one) blocks.
		if (namedOwners(seg).some((o) => ALLOWED_OWNERS[o] === true)) continue;
		return true;
	}
	return false;
}

// pkill / killall are always pattern-based -> broad. For `kill`, skip the
// leading signal spec (-9, -KILL, -s NAME, -n NUM, ...) and block when a
// remaining TARGET is negative (-1 / -<pgid> = a process group / everything).
// `kill -1 <pid>` (SIGHUP to one PID) and `kill -9 <pid>` stay allowed; only a
// negative target like `kill -- -1` or `kill -TERM -1` is the broad form.
function hasBroadKill(cmd: string): boolean {
	for (const seg of cmd.split(/[\n;&|]+/)) {
		const toks = seg.trim().split(/\s+/).filter(Boolean);
		const idx = toks.findIndex((t) => /(?:^|\/)(?:pkill|killall|kill)$/.test(t));
		if (idx === -1) continue;
		if (!/(?:^|\/)kill$/.test(toks[idx])) return true; // pkill / killall
		let k = idx + 1;
		if (toks[k] === "-s" || toks[k] === "-n") k += 2;
		else if (k < toks.length && /^-[A-Za-z0-9]+$/.test(toks[k])) k += 1;
		if (toks[k] === "--") k += 1;
		if (toks.slice(k).some((t) => /^-\d+$/.test(t))) return true; // negative target
	}
	return false;
}

// OMP runs commands through several tools; guard all that can push/run — not
// just bash — so a push/merge can't slip through `ssh` or a `recipe`. The
// broad-kill check stays scoped to locally-executing tools (a kill over ssh
// runs on the remote, not the session's own runtime).
const CMD_TOOLS: Record<string, true> = { bash: true, ssh: true, recipe: true };
const LOCAL_TOOLS: Record<string, true> = { bash: true, recipe: true };
const GH_MCP = /^mcp__github_/;
// GitHub MCP write operations — verbs anchored to the `mcp__github_` prefix so
// the shared `pull_request_` infix doesn't misclassify reads. Everything else
// (pull_request_read, get_*) is a read, allowed on any repo incl. an upstream
// PR you're triaging.
const GH_MCP_WRITE = /^mcp__github_(?:create|update|delete|add|fork|push|dispatch|request|merge)_|_write\b/;

// Pure decision: a block, or null to allow. Unit-tested in push-guard.test.ts.
export function evaluate(toolName: string, input: Record<string, unknown>): Block | null {
	if (CMD_TOOLS[toolName] === true) {
		// bash/ssh carry the command in `command`; any other shape falls back to
		// the whole input so a push/merge in any field is still caught.
		const cmd = typeof input.command === "string" ? input.command : JSON.stringify(input ?? {});

		if (LOCAL_TOOLS[toolName] === true && hasBroadKill(cmd)) {
			return {
				block: true,
				reason:
					"Broad process kill blocked (pkill / killall / kill -1). These can take down " +
					"the session's own runtime or unrelated work. Kill a specific PID you started, " +
					"or ask. See rule://process-safety.",
			};
		}

		if (MERGE.test(cmd)) {
			return {
				block: true,
				reason:
					"Merge blocked: merging a PR is the human gate — the agent never merges. " +
					"Get the PR to merge-ready and hand off. See skill://autonomous-review.",
			};
		}

		const bad = namedOwners(cmd).filter((o) => ALLOWED_OWNERS[o] !== true);
		if (bad.length && (PUSH.test(cmd) || GH_WRITE_CMD.test(cmd))) {
			return {
				block: true,
				reason:
					`Write to ${bad.join(", ")}/* blocked: outside the owner allowlist ` +
					"(mattwilkinsonn, sealedsecurity). Never push / open a PR / file an issue on an " +
					"upstream or OSS repo. See rule://commit-conventions.",
			};
		}

		if (ghCreateWithoutAllowedOwner(cmd)) {
			return {
				block: true,
				reason:
					"GitHub issue/PR create blocked: pass an allowlisted `-R <owner>/<repo>` " +
					"(mattwilkinsonn, sealedsecurity). A bare `gh issue create` / `gh pr create` files " +
					"on whatever repo the cwd resolves to — an OSS upstream included. " +
					"See rule://commit-conventions.",
			};
		}

		if (PUSH.test(cmd)) {
			if (PUSH_UPSTREAM.test(cmd)) {
				return {
					block: true,
					reason:
						"Push to the `upstream` remote blocked: that's the OSS upstream, outside the " +
						"owner allowlist. Push your fork's `origin` only. See rule://commit-conventions.",
				};
			}
			if (PUSH_MAIN.test(cmd)) {
				return {
					block: true,
					reason:
						"Push to `main` blocked: never push or force-push `main`. Push a feature branch " +
						"and open a PR; merge is the human gate. See rule://commit-conventions.",
				};
			}
		}
		return null;
	}

	if (GH_MCP.test(toolName)) {
		if (/merge/.test(toolName)) {
			return {
				block: true,
				reason:
					"Merge blocked: merging a PR is the human gate — the agent never merges. " +
					"See skill://autonomous-review.",
			};
		}
		if (GH_MCP_WRITE.test(toolName)) {
			const owner = typeof input.owner === "string" ? input.owner.toLowerCase() : "";
			if (ALLOWED_OWNERS[owner] !== true) {
				return {
					block: true,
					reason:
						`GitHub write blocked: owner '${owner || "(missing)"}' is outside the allowlist ` +
						"(mattwilkinsonn, sealedsecurity) — fail-closed on an absent or disallowed owner. " +
						"Never open PRs/issues on an upstream or OSS repo. See rule://commit-conventions.",
				};
			}
		}
		return null;
	}

	return null;
}

export default function pushGuard(pi: ExtensionAPI): void {
	pi.setLabel("push-guard");

	pi.on("tool_call", async (event) => {
		const result = evaluate(event.toolName, (event.input ?? {}) as Record<string, unknown>);
		return result ?? undefined;
	});
}

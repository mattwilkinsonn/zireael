import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

// jj-aware status for the omp statusline. omp's built-in `git` segment shows
// "detached" under jj (jj parks git HEAD), so we push real jj state instead via
// setStatus: the nearest stack bookmark + the working-copy change-id + a `*`
// dirty marker, e.g. "  file-cleanup zoxwopnu *".
//
// Queries are read-only — `--ignore-working-copy` means a refresh never
// triggers a jj snapshot. Pair with `statusLine.segmentOptions.git.showBranch:
// false` to drop the git "detached". Tweak the templates/glyph to taste.

const BRANCH_GLYPH = "\uE0A0"; // powerline branch (nerd/powerline font)

// Nearest bookmarked ancestor of @ — the "stack" you're working on.
const NEAREST_BOOKMARK = [
	"log", "-r", "heads(::@ & bookmarks())", "--no-graph",
	"--ignore-working-copy", "--color", "never", "-T", 'local_bookmarks ++ "\\n"',
];
// The working-copy change itself: short change-id + `*` when it has changes.
const CHANGE = [
	"log", "-r", "@", "--no-graph",
	"--ignore-working-copy", "--color", "never", "-T",
	'separate(" ", change_id.shortest(8), if(empty, "", "*"))',
];

async function jj(cwd: string, args: string[]): Promise<string | undefined> {
	try {
		const proc = Bun.spawn(["jj", ...args], { cwd, stdout: "pipe", stderr: "ignore", stdin: "ignore" });
		const out = await new Response(proc.stdout).text();
		return (await proc.exited) === 0 ? out.trim() : undefined;
	} catch {
		return undefined;
	}
}

async function refresh(ctx: ExtensionContext): Promise<void> {
	const change = await jj(ctx.cwd, CHANGE);
	if (!change) {
		ctx.ui.setStatus("jj", undefined); // not a jj repo (or jj absent) — clear
		return;
	}
	const bookmark = (await jj(ctx.cwd, NEAREST_BOOKMARK))?.split("\n")[0]?.trim();
	const label = bookmark ? `${bookmark} ${change}` : change;
	ctx.ui.setStatus("jj", `${BRANCH_GLYPH} ${label}`);
}

export default function jjStatus(pi: ExtensionAPI): void {
	pi.setLabel("jj-status");
	pi.on("session_start", (_event, ctx) => refresh(ctx));
	pi.on("turn_end", (_event, ctx) => refresh(ctx));
}

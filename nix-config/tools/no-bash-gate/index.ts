// no-bash-gate — fail CI when a committed bash script is not in the allowlist.
//
// The repo's convention (AGENTS.md "Scripts: TypeScript over bash") is that
// real script logic is TypeScript run via bun, not bash. This gate gives that
// rule teeth: it enumerates committed shell scripts (`*.sh` plus extensionless
// files carrying a bash shebang / `shellcheck shell=` directive) and fails if
// any is not in ALLOWLIST below. The allowlist is meant to SHRINK — each entry
// is a script that hasn't been converted yet, with a reason; the end state is
// an empty allowlist (zero committed bash).
//
// A NEW bash script therefore fails CI immediately: convert it to a bun/TS tool
// (see nix-config/tools/wait-for-reviews for the pattern), or, if it genuinely
// must be bash (nix bootstrap that runs before bun exists, a POSIX-only host
// constraint), add it to ALLOWLIST with a one-line reason.
//
// Inputs (env):
//   GATE_ROOT  - directory to scan (default: git toplevel). Tests point this at
//                a fixture tree.
// Exit codes:
//   0 - every committed bash script is allowlisted (or none exist)
//   1 - a non-allowlisted bash script is committed (prints the offenders)
//   2 - usage / internal error (e.g. not a git tree)

import { $ } from "bun";

// Paths are repo-relative, POSIX-separated, as `git ls-files` emits them.
// Each entry MUST carry a reason: why this is still bash. Shrink toward zero.
export const ALLOWLIST: Record<string, string> = {
	// nix bootstrap — runs before bun/nix exist on a fresh host.
	"nixos/scripts/mattpc-wsl-bootstrap.sh": "nix bootstrap, pre-bun",
	"shared/scripts/bootstrap-common.sh": "nix bootstrap, pre-bun",
	"shared/scripts/migrate-from-dotfiles.sh": "one-shot nix migration, pre-bun",
	"darwin/scripts/mac-setup.sh": "macOS bootstrap, pre-bun (Xcode CLT / brew)",
	"darwin/scripts/nix-switch-all.sh": "nix bootstrap wrapper, pre-bun",
	"darwin/scripts/sync-ventoy.sh": "macOS-only disk util, pre-bun",
	// yabai — the window manager execs these hooks directly as shell.
	"dotfiles/yabai/aw-layout.sh": "yabai shell hook",
	"dotfiles/yabai/columns.sh": "yabai shell hook",
	"dotfiles/yabai/cycle-display.sh": "yabai shell hook",
	"dotfiles/yabai/display-event.sh": "yabai shell hook",
	"dotfiles/yabai/display-setup.sh": "yabai shell hook",
	"dotfiles/yabai/g9-layout.sh": "yabai shell hook",
	"dotfiles/yabai/move-to-display.sh": "yabai shell hook",
	"dotfiles/yabai/reset-splits.sh": "yabai shell hook",
	"dotfiles/yabai/rules.sh": "yabai shell hook",
	"dotfiles/yabai/yabairc": "yabai config, execed as shell by yabai",
	// bash test harnesses for the remaining bash scripts above. These retire
	// when their subjects convert.
	"shared/scripts/tests/bootstrap-args.test.sh": "tests bootstrap-common.sh",
	"shared/scripts/tests/fnm-path-repair.test.sh": "tests a bootstrap path fix",
};

export type Scan = {
	/** Committed bash scripts found under the root, repo-relative POSIX paths. */
	found: string[];
	/** Found scripts not present in the allowlist — the CI failures. */
	offenders: string[];
	/** Allowlist entries that no longer exist — stale, should be pruned. */
	stale: string[];
};

const SHEBANG_RE = /^#!.*\b(bash|sh)\b/;
const SHELLCHECK_RE = /shellcheck\s+shell=/;

/** True when an extensionless file's first two lines mark it as a shell script. */
export function looksLikeShell(head: string): boolean {
	const firstLines = head.split("\n", 2);
	return firstLines.some(
		(line) => SHEBANG_RE.test(line) || SHELLCHECK_RE.test(line),
	);
}

/**
 * Partition committed files into the bash set and compare against the allowlist.
 * `readHead` returns the first bytes of a file (for shebang sniffing); injected
 * so tests need no real filesystem.
 */
export function evaluate(
	files: string[],
	allow: Record<string, string>,
	isShell: (path: string) => boolean,
): Scan {
	const found = files
		.filter((path) => path.endsWith(".sh") || isShell(path))
		.sort();
	const offenders = found.filter((path) => !(path in allow));
	const stale = Object.keys(allow)
		.filter((path) => !found.includes(path))
		.sort();
	return { found, offenders, stale };
}

export type Deps = {
	root: string;
	/** Lists committed files (repo-relative). */
	lsFiles: (root: string) => Promise<string[]>;
	/** First bytes of a repo-relative file, for shebang sniffing. */
	readHead: (root: string, path: string) => Promise<string>;
	log: (msg: string) => void;
	err: (msg: string) => void;
};

export async function runOnce(deps: Deps): Promise<number> {
	const { root, lsFiles, readHead, log, err } = deps;

	let files: string[];
	try {
		files = await lsFiles(root);
	} catch (error) {
		err(`no-bash-gate: cannot list git files in ${root}`);
		err(error instanceof Error ? error.message : String(error));
		return 2;
	}

	// Sniff only the extensionless candidates — cheap, and `.sh` needs no sniff.
	// Test the BASENAME for a dot, not the whole path: an extensionless script in
	// a dot-named directory (e.g. `tools.v2/helper`) must still be sniffed.
	const shellCache = new Map<string, boolean>();
	const candidates = files.filter((p) => {
		const basename = p.slice(p.lastIndexOf("/") + 1);
		return !p.endsWith(".sh") && !basename.includes(".");
	});
	await Promise.all(
		candidates.map(async (path) => {
			try {
				shellCache.set(path, looksLikeShell(await readHead(root, path)));
			} catch {
				shellCache.set(path, false);
			}
		}),
	);
	const isShell = (path: string): boolean => shellCache.get(path) ?? false;

	const { found, offenders, stale } = evaluate(files, ALLOWLIST, isShell);

	if (stale.length > 0) {
		log(`no-bash-gate: ${stale.length} stale allowlist entr(y/ies) (prune):`);
		for (const path of stale) log(`  - ${path}`);
	}

	if (offenders.length === 0) {
		log(
			`no-bash-gate: OK — ${found.length} committed bash script(s), all allowlisted.`,
		);
		return 0;
	}

	err("");
	err(
		`no-bash-gate: ${offenders.length} committed bash script(s) not allowlisted:`,
	);
	for (const path of offenders) err(`  - ${path}`);
	err("");
	err("Scripts must be TypeScript run via bun, not bash.");
	err("Convert it (see nix-config/tools/wait-for-reviews for the pattern),");
	err("or, if it genuinely must be bash, add it to ALLOWLIST in");
	err("nix-config/tools/no-bash-gate/index.ts with a one-line reason.");
	return 1;
}

if (import.meta.main) {
	const root =
		process.env.GATE_ROOT ??
		(await $`git rev-parse --show-toplevel`.nothrow().quiet().text()).trim();
	process.exit(
		await runOnce({
			root,
			lsFiles: async (r) => {
				const out = await $`git -C ${r} ls-files`.nothrow().quiet();
				if (out.exitCode !== 0) throw new Error(out.stderr.toString());
				return out
					.text()
					.split("\n")
					.filter((line) => line.length > 0);
			},
			readHead: async (r, path) => {
				const file = Bun.file(`${r}/${path}`);
				const slice = file.slice(0, 256);
				return await slice.text();
			},
			log: (msg) => console.log(msg),
			err: (msg) => console.error(msg),
		}),
	);
}

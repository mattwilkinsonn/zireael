// Build debug binaries and install them to ~/.cargo/bin (a dir already on
// most PATHs). Dev convenience for running the just-built tools locally.
//
//   install-debug              # both tools
//   install-debug jj-hooks     # jj-hooks + jj-hp
//   install-debug jj-gt
//
// On Linux, writing over an in-use executable fails with ETXTBSY (text
// file busy), so each binary is unlinked before the fresh copy lands (a
// running process keeps its inode). macOS lets you overwrite an active
// binary, so the unlink is a harmless no-op there; macOS also gets an
// ad-hoc codesign so the binary doesn't trip Gatekeeper.
//
// Inputs (env):
//   CARGO_HOME - install dir base; falls back to $HOME/.cargo.
//   HOME       - used only for the CARGO_HOME fallback.
//
// Exit codes:
//   0 - all requested builds + installs succeeded.
//   1 - unknown tool argument.
//   * - a `cargo build` / `bun build` failure propagates that command's
//       exit code (mirrors the old script's `set -e`).

import { copyFile, mkdir as fsMkdir, rm as fsRm } from "node:fs/promises";
import { $ } from "bun";

type Target = "all" | "jj-hooks" | "jj-gt";

// A single unit of the install plan. `plan()` produces these in
// execution order so tests can assert the sequence without running
// cargo/bun/cp. `build` shells a compiler; `install` copies (and maybe
// codesigns) a produced binary; `log` echoes a status line.
type Step =
	| { kind: "build"; cmd: string[]; cwd?: string }
	| { kind: "install"; src: string; name: string; sign: boolean }
	| { kind: "log"; msg: string };

// Everything runOnce touches from its environment. Tests pass fakes;
// production wires process.env, real Bun.$, process.platform, console,
// and node:fs. `sh` returns only the exit code — no caller reads stdout.
type Deps = {
	env: Record<string, string | undefined>;
	sh: (cmd: string[], opts?: { cwd?: string }) => Promise<{ exitCode: number }>;
	platform: NodeJS.Platform;
	log: (msg: string) => void;
	err: (msg: string) => void;
	mkdir: (path: string) => Promise<void>;
	rm: (path: string) => Promise<void>;
	cp: (src: string, dest: string) => Promise<void>;
};

// `case "${1:-all}"`: no arg → "all"; unknown → error (caller exits 1).
function parseTarget(argv: string[]): Target | { error: string } {
	const arg = argv[0] ?? "all";
	switch (arg) {
		case "all":
		case "jj-hooks":
		case "jj-gt":
			return arg;
		default:
			return { error: arg };
	}
}

// `dest="${CARGO_HOME:-$HOME/.cargo}/bin"`. An empty CARGO_HOME falls
// back too (bash `:-` treats empty and unset alike).
function computeDest(env: Record<string, string | undefined>): string {
	const base = env.CARGO_HOME || `${env.HOME ?? ""}/.cargo`;
	return `${base}/bin`;
}

// Ordered build+install plan for a target: the compiler invocations and
// install_bin calls in the exact sequence the bash `case` ran them.
// `all` chains the two tools in bash order (jj-hooks, jj-gt).
function plan(target: Target, dest: string): Step[] {
	const jjHooks: Step[] = [
		{
			kind: "build",
			cmd: [
				"cargo",
				"build",
				"-p",
				"jj-hooks",
				"--bin",
				"jj-hooks",
				"--bin",
				"jj-hp",
			],
		},
		{
			kind: "install",
			src: "target/debug/jj-hooks",
			name: "jj-hooks",
			sign: true,
		},
		{ kind: "install", src: "target/debug/jj-hp", name: "jj-hp", sign: true },
		{
			kind: "log",
			msg: `Installed debug builds (jj-hooks + jj-hp) to ${dest}`,
		},
	];
	const jjGt: Step[] = [
		{ kind: "build", cmd: ["cargo", "build", "-p", "jj-gt", "--bin", "jj-gt"] },
		{ kind: "install", src: "target/debug/jj-gt", name: "jj-gt", sign: true },
		{ kind: "log", msg: `Installed debug build (jj-gt) to ${dest}` },
	];
	switch (target) {
		case "jj-hooks":
			return jjHooks;
		case "jj-gt":
			return jjGt;
		case "all":
			return [...jjHooks, ...jjGt];
	}
}

// rm -f dest/name (the ETXTBSY unlink; -f ignores a missing file), then
// cp src → dest/name, then — only when signing is requested AND we're on
// darwin — an ad-hoc codesign. codesign failure is swallowed (bash
// `2>/dev/null ... || true`); success echoes "Codesigned <name>".
async function installBin(
	deps: Deps,
	dest: string,
	src: string,
	name: string,
	sign: boolean,
): Promise<void> {
	const target = `${dest}/${name}`;
	await deps.rm(target);
	await deps.cp(src, target);
	if (sign && deps.platform === "darwin") {
		const res = await deps.sh(["codesign", "-s", "-", target]);
		if (res.exitCode === 0) {
			deps.log(`Codesigned ${name}`);
		}
	}
}

async function runOnce(deps: Deps, argv: string[]): Promise<number> {
	const target = parseTarget(argv);
	const dest = computeDest(deps.env);
	// `mkdir -p "$dest"` runs before the case in the bash, so it happens
	// even for an unknown tool — preserved here for side-effect parity.
	await deps.mkdir(dest);
	if (typeof target === "object") {
		deps.err(`error: unknown tool '${target.error}'`);
		deps.err("valid: all | jj-hooks | jj-gt");
		return 1;
	}
	for (const step of plan(target, dest)) {
		if (step.kind === "build") {
			const res = await deps.sh(
				step.cmd,
				step.cwd !== undefined ? { cwd: step.cwd } : undefined,
			);
			// `set -e`: a failed compile aborts with that command's code.
			if (res.exitCode !== 0) {
				return res.exitCode;
			}
		} else if (step.kind === "install") {
			await installBin(deps, dest, step.src, step.name, step.sign);
		} else {
			deps.log(step.msg);
		}
	}
	return 0;
}

export type { Deps, Step, Target };
export { computeDest, installBin, parseTarget, plan, runOnce };

if (import.meta.main) {
	const sh = async (
		cmd: string[],
		opts?: { cwd?: string },
	): Promise<{ exitCode: number }> => {
		const [bin, ...args] = cmd;
		if (bin === undefined) {
			return { exitCode: 0 };
		}
		// No .quiet(): install-debug is an interactive dev tool, so cargo/bun build
		// output (and errors) must stream to the terminal like the bash original did.
		let proc = $`${bin} ${args}`.nothrow();
		if (opts?.cwd !== undefined) {
			proc = proc.cwd(opts.cwd);
		}
		const res = await proc;
		return { exitCode: res.exitCode };
	};
	process.exit(
		await runOnce(
			{
				env: process.env,
				sh,
				platform: process.platform,
				log: (msg) => console.log(msg),
				err: (msg) => console.error(msg),
				mkdir: async (path) => {
					await fsMkdir(path, { recursive: true });
				},
				rm: async (path) => {
					await fsRm(path, { force: true });
				},
				cp: async (src, dest) => {
					await copyFile(src, dest);
				},
			},
			process.argv.slice(2),
		),
	);
}

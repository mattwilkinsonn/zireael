import { describe, expect, test } from "bun:test";
import {
	computeDest,
	type Deps,
	installBin,
	parseTarget,
	plan,
	runOnce,
	type Step,
} from "./index.ts";

// A recording Deps: captures every shell command, log, err, and fs
// side effect in order so tests can assert the sequence without cargo,
// bun, cp, or a real filesystem.
type Recorder = {
	deps: Deps;
	sh: Array<{ cmd: string[]; cwd?: string }>;
	logs: string[];
	errs: string[];
	rm: string[];
	cp: Array<{ src: string; dest: string }>;
	mkdir: string[];
};

function recorder(opts?: {
	platform?: NodeJS.Platform;
	env?: Record<string, string | undefined>;
	shExit?: (cmd: string[]) => number;
}): Recorder {
	const sh: Recorder["sh"] = [];
	const logs: string[] = [];
	const errs: string[] = [];
	const rm: string[] = [];
	const cp: Recorder["cp"] = [];
	const mkdir: string[] = [];
	const deps: Deps = {
		env: opts?.env ?? { CARGO_HOME: "/fake/cargo" },
		platform: opts?.platform ?? "linux",
		sh: async (cmd, o) => {
			sh.push(o?.cwd !== undefined ? { cmd, cwd: o.cwd } : { cmd });
			return { exitCode: opts?.shExit ? opts.shExit(cmd) : 0 };
		},
		log: (m) => logs.push(m),
		err: (m) => errs.push(m),
		mkdir: async (p) => {
			mkdir.push(p);
		},
		rm: async (p) => {
			rm.push(p);
		},
		cp: async (src, dest) => {
			cp.push({ src, dest });
		},
	};
	return { deps, sh, logs, errs, rm, cp, mkdir };
}

describe("parseTarget", () => {
	test("no arg defaults to all", () => {
		expect(parseTarget([])).toBe("all");
	});
	test("explicit all", () => {
		expect(parseTarget(["all"])).toBe("all");
	});
	test("each tool passes through", () => {
		expect(parseTarget(["jj-hooks"])).toBe("jj-hooks");
		expect(parseTarget(["jj-gt"])).toBe("jj-gt");
	});
	test("unknown tool returns error carrying the bad arg", () => {
		expect(parseTarget(["nope"])).toEqual({ error: "nope" });
	});
});

describe("computeDest", () => {
	test("uses CARGO_HOME when set", () => {
		expect(computeDest({ CARGO_HOME: "/opt/c" })).toBe("/opt/c/bin");
	});
	test("falls back to $HOME/.cargo when CARGO_HOME unset", () => {
		expect(computeDest({ HOME: "/home/x" })).toBe("/home/x/.cargo/bin");
	});
	test("empty CARGO_HOME falls back too (bash :- semantics)", () => {
		expect(computeDest({ CARGO_HOME: "", HOME: "/home/x" })).toBe(
			"/home/x/.cargo/bin",
		);
	});
});

// Helpers to slice the plan for structural assertions.
const builds = (steps: Step[]) =>
	steps.filter(
		(s): s is Extract<Step, { kind: "build" }> => s.kind === "build",
	);
const installs = (steps: Step[]) =>
	steps.filter(
		(s): s is Extract<Step, { kind: "install" }> => s.kind === "install",
	);

describe("plan", () => {
	const dest = "/d/bin";

	test("jj-hooks: one cargo build (both bins), installs jj-hooks + jj-hp signed", () => {
		const steps = plan("jj-hooks", dest);
		expect(builds(steps).map((b) => b.cmd)).toEqual([
			[
				"cargo",
				"build",
				"-p",
				"jj-hooks",
				"--bin",
				"jj-hooks",
				"--bin",
				"jj-hp",
			],
		]);
		expect(installs(steps)).toEqual([
			{
				kind: "install",
				src: "target/debug/jj-hooks",
				name: "jj-hooks",
				sign: true,
			},
			{ kind: "install", src: "target/debug/jj-hp", name: "jj-hp", sign: true },
		]);
		expect(steps.at(-1)).toEqual({
			kind: "log",
			msg: `Installed debug builds (jj-hooks + jj-hp) to ${dest}`,
		});
	});

	test("jj-gt: single build + single signed install", () => {
		const steps = plan("jj-gt", dest);
		expect(builds(steps).map((b) => b.cmd)).toEqual([
			["cargo", "build", "-p", "jj-gt", "--bin", "jj-gt"],
		]);
		expect(installs(steps)).toEqual([
			{ kind: "install", src: "target/debug/jj-gt", name: "jj-gt", sign: true },
		]);
	});

	test("all chains jj-hooks, jj-gt in bash order", () => {
		const steps = plan("all", dest);
		expect(installs(steps).map((i) => i.name)).toEqual([
			"jj-hooks",
			"jj-hp",
			"jj-gt",
		]);
		// sign flags: all cargo bins signed.
		expect(installs(steps).map((i) => i.sign)).toEqual([true, true, true]);
	});
});

describe("installBin", () => {
	test("rm-before-cp order, no codesign on linux", async () => {
		const r = recorder({ platform: "linux" });
		await installBin(r.deps, "/d/bin", "target/debug/jj-gt", "jj-gt", true);
		expect(r.rm).toEqual(["/d/bin/jj-gt"]);
		expect(r.cp).toEqual([{ src: "target/debug/jj-gt", dest: "/d/bin/jj-gt" }]);
		expect(r.sh).toEqual([]); // no codesign shelled on linux
		expect(r.logs).toEqual([]);
	});

	test("darwin + sign → codesign called and logged", async () => {
		const r = recorder({ platform: "darwin" });
		await installBin(r.deps, "/d/bin", "target/debug/jj-gt", "jj-gt", true);
		expect(r.sh).toEqual([{ cmd: ["codesign", "-s", "-", "/d/bin/jj-gt"] }]);
		expect(r.logs).toEqual(["Codesigned jj-gt"]);
	});

	test("darwin + sign=false → no codesign", async () => {
		const r = recorder({ platform: "darwin" });
		await installBin(r.deps, "/d/bin", "target/debug/jj-gt", "jj-gt", false);
		expect(r.sh).toEqual([]);
		expect(r.logs).toEqual([]);
	});

	test("darwin + failed codesign is swallowed (no log)", async () => {
		const r = recorder({ platform: "darwin", shExit: () => 1 });
		await installBin(r.deps, "/d/bin", "target/debug/jj-gt", "jj-gt", true);
		expect(r.sh).toHaveLength(1);
		expect(r.logs).toEqual([]);
	});
});

describe("runOnce", () => {
	test("unknown tool: mkdir still runs, errors to stderr, exit 1", async () => {
		const r = recorder();
		const code = await runOnce(r.deps, ["bogus"]);
		expect(code).toBe(1);
		expect(r.mkdir).toEqual(["/fake/cargo/bin"]);
		expect(r.errs).toEqual([
			"error: unknown tool 'bogus'",
			"valid: all | jj-hooks | jj-gt",
		]);
		expect(r.sh).toEqual([]);
	});

	test("jj-gt on linux: build then rm→cp, no codesign, success log, exit 0", async () => {
		const r = recorder({ platform: "linux" });
		const code = await runOnce(r.deps, ["jj-gt"]);
		expect(code).toBe(0);
		expect(r.sh).toEqual([
			{ cmd: ["cargo", "build", "-p", "jj-gt", "--bin", "jj-gt"] },
		]);
		expect(r.rm).toEqual(["/fake/cargo/bin/jj-gt"]);
		expect(r.cp).toEqual([
			{ src: "target/debug/jj-gt", dest: "/fake/cargo/bin/jj-gt" },
		]);
		expect(r.logs).toEqual([
			`Installed debug build (jj-gt) to /fake/cargo/bin`,
		]);
	});

	test("failed cargo build aborts with that exit code, no install", async () => {
		const r = recorder({ shExit: () => 42 });
		const code = await runOnce(r.deps, ["jj-gt"]);
		expect(code).toBe(42);
		expect(r.cp).toEqual([]); // never got to the copy
		expect(r.logs).toEqual([]);
	});

	test("all on darwin: codesigns the two cargo bin sets", async () => {
		const r = recorder({ platform: "darwin" });
		const code = await runOnce(r.deps, []);
		expect(code).toBe(0);
		const codesigned = r.logs.filter((l) => l.startsWith("Codesigned "));
		expect(codesigned).toEqual([
			"Codesigned jj-hooks",
			"Codesigned jj-hp",
			"Codesigned jj-gt",
		]);
	});
});

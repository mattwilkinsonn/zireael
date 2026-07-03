import { describe, expect, test } from "bun:test";
import { type Deps, evaluate, looksLikeShell, runOnce } from "./index.ts";

describe("looksLikeShell", () => {
	test("bash shebang", () => {
		expect(looksLikeShell("#!/usr/bin/env bash\nset -e")).toBe(true);
	});
	test("sh shebang", () => {
		expect(looksLikeShell("#!/bin/sh\n")).toBe(true);
	});
	test("shellcheck directive on second line", () => {
		expect(looksLikeShell("# a comment\n# shellcheck shell=bash")).toBe(true);
	});
	test("plain text / other language is not shell", () => {
		expect(looksLikeShell("#!/usr/bin/env bun\nconsole.log(1)")).toBe(false);
		expect(looksLikeShell("import { $ } from 'bun'")).toBe(false);
	});
});

describe("evaluate", () => {
	const noSniff = () => false;

	test("a .sh not in the allowlist is an offender", () => {
		const r = evaluate(["dotfiles/scripts/new-thing.sh"], {}, noSniff);
		expect(r.offenders).toEqual(["dotfiles/scripts/new-thing.sh"]);
		expect(r.found).toEqual(["dotfiles/scripts/new-thing.sh"]);
	});

	test("an allowlisted .sh is not an offender", () => {
		const r = evaluate(
			["yabai/rules.sh"],
			{ "yabai/rules.sh": "yabai hook" },
			noSniff,
		);
		expect(r.offenders).toEqual([]);
		expect(r.found).toEqual(["yabai/rules.sh"]);
	});

	test("extensionless bash (sniffed) is caught", () => {
		const isShell = (p: string) => p === "scripts/deploy";
		const r = evaluate(["scripts/deploy", "README.md"], {}, isShell);
		expect(r.offenders).toEqual(["scripts/deploy"]);
	});

	test("non-shell files are ignored", () => {
		const r = evaluate(["index.ts", "flake.nix", "README.md"], {}, noSniff);
		expect(r.found).toEqual([]);
		expect(r.offenders).toEqual([]);
	});

	test("stale allowlist entries are reported", () => {
		const r = evaluate([], { "gone/old.sh": "reason" }, noSniff);
		expect(r.stale).toEqual(["gone/old.sh"]);
	});

	test("mixed set partitions correctly and sorts", () => {
		const r = evaluate(
			["b/new.sh", "a/allowed.sh", "keep.ts"],
			{ "a/allowed.sh": "ok" },
			noSniff,
		);
		expect(r.found).toEqual(["a/allowed.sh", "b/new.sh"]);
		expect(r.offenders).toEqual(["b/new.sh"]);
	});
});

describe("runOnce", () => {
	function deps(overrides: Partial<Deps>): {
		d: Deps;
		out: string[];
		errs: string[];
	} {
		const out: string[] = [];
		const errs: string[] = [];
		const d: Deps = {
			root: "/fake",
			lsFiles: async () => [],
			readHead: async () => "",
			log: (m) => out.push(m),
			err: (m) => errs.push(m),
			...overrides,
		};
		return { d, out, errs };
	}

	test("all allowlisted → exit 0", async () => {
		// Only files that are actually in the real ALLOWLIST.
		const { d, out } = deps({
			lsFiles: async () => ["dotfiles/yabai/rules.sh", "index.ts"],
		});
		expect(await runOnce(d)).toBe(0);
		expect(out.some((l) => l.includes("OK"))).toBe(true);
	});

	test("a new .sh → exit 1 and names the offender", async () => {
		const { d, errs } = deps({
			lsFiles: async () => ["dotfiles/scripts/sneaky.sh"],
		});
		expect(await runOnce(d)).toBe(1);
		expect(errs.some((l) => l.includes("sneaky.sh"))).toBe(true);
		expect(errs.some((l) => l.includes("TypeScript"))).toBe(true);
	});

	test("extensionless bash sniffed from head → exit 1", async () => {
		const { d, errs } = deps({
			lsFiles: async () => ["dotfiles/scripts/router"],
			readHead: async () => "#!/usr/bin/env bash\n",
		});
		expect(await runOnce(d)).toBe(1);
		expect(errs.some((l) => l.includes("router"))).toBe(true);
	});

	test("extensionless NON-bash (bun script) → exit 0", async () => {
		const { d } = deps({
			lsFiles: async () => ["dotfiles/scripts/thing"],
			readHead: async () => "#!/usr/bin/env bun\n",
		});
		expect(await runOnce(d)).toBe(0);
	});

	test("git failure → exit 2", async () => {
		const { d, errs } = deps({
			lsFiles: async () => {
				throw new Error("not a git repo");
			},
		});
		expect(await runOnce(d)).toBe(2);
		expect(errs.some((l) => l.includes("cannot list"))).toBe(true);
	});
});

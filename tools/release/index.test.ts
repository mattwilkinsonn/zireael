import { describe, expect, test } from "bun:test";
import type { Deps, ShOpts, ShResult } from "./index.ts";
import {
	bumpFormulaVersion,
	bumpPackageJsonVersion,
	runOnce,
	validateVersion,
	verifyBumps,
} from "./index.ts";

describe("validateVersion", () => {
	test("accepts a stable vX.Y.Z", () => {
		expect(validateVersion("v0.3.0")).toEqual({ bare: "0.3.0" });
	});

	test("accepts a -rc.N prerelease", () => {
		expect(validateVersion("v1.2.3-rc.4")).toEqual({ bare: "1.2.3-rc.4" });
	});

	test("rejects a version missing the leading v", () => {
		expect(validateVersion("0.3.0")).toEqual({
			error:
				"usage: moon run root:release -- vX.Y.Z (or vX.Y.Z-rc.1); got: '0.3.0'",
		});
	});

	test("rejects garbage", () => {
		expect(validateVersion("banana")).toEqual({
			error:
				"usage: moon run root:release -- vX.Y.Z (or vX.Y.Z-rc.1); got: 'banana'",
		});
	});

	test("rejects a two-component version", () => {
		expect("error" in validateVersion("v1.2")).toBe(true);
	});
});

const PKG_JSON = `{
	"name": "akiflow-cli",
	"version": "0.3.7",
	"description": "Akiflow CLI - Task management from the command line"
}
`;

const FORMULA = `class AkiflowCli < Formula
  desc "Command-line task management"
  version "0.3.7"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/x/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      sha256 "9e70c1bafbbf8f212f1ecb90c7ab0824e43acca9bff2dec143f5ce35fb6cde34"
    end
  end
end
`;

describe("bumpPackageJsonVersion", () => {
	test("rewrites the version line and nothing else", () => {
		const out = bumpPackageJsonVersion(PKG_JSON, "0.4.0");
		expect(out).toBe(
			PKG_JSON.replace('"version": "0.3.7"', '"version": "0.4.0"'),
		);
		expect(out).toContain('"version": "0.4.0"');
		expect(out).toContain('"name": "akiflow-cli"');
		expect(out).not.toContain("0.3.7");
	});

	test("carries a prerelease bare version verbatim", () => {
		const out = bumpPackageJsonVersion(PKG_JSON, "0.4.0-rc.1");
		expect(out).toContain('"version": "0.4.0-rc.1"');
	});
});

describe("bumpFormulaVersion", () => {
	test("rewrites the version line only, leaving url templates intact", () => {
		const out = bumpFormulaVersion(FORMULA, "0.4.0");
		expect(out).toContain('  version "0.4.0"');
		// The url "...#{version}..." interpolation lines must NOT be touched.
		expect(out).toContain(
			'url "https://github.com/x/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"',
		);
		expect(out).toContain('license "MIT"');
		expect(out).not.toContain('version "0.3.7"');
	});
});

describe("verifyBumps", () => {
	const cargoOk = 'version = "0.4.0"\nedition = "2024"\n';
	const pkgOk = '{ "version": "0.4.0" }';
	const formulasOk = [
		{ name: "Formula/akiflow-cli.rb", text: 'version "0.4.0"' },
	];

	test("returns no errors when every bump took", () => {
		expect(verifyBumps(cargoOk, pkgOk, formulasOk, "0.4.0")).toEqual([]);
	});

	test("flags a stale Cargo.toml and echoes its version lines", () => {
		const cargoStale = 'version = "0.3.7"\nedition = "2024"\n';
		expect(verifyBumps(cargoStale, pkgOk, formulasOk, "0.4.0")).toEqual([
			"error: workspace Cargo.toml version didn't bump to 0.4.0",
			'version = "0.3.7"',
		]);
	});

	test("flags a stale package.json", () => {
		expect(
			verifyBumps(cargoOk, '{ "version": "0.3.7" }', formulasOk, "0.4.0"),
		).toEqual(["error: tools/akiflow-cli/package.json version didn't bump"]);
	});

	test("flags a stale formula by path", () => {
		const formulasStale = [
			{ name: "Formula/jj-gt.rb", text: 'version "0.3.7"' },
		];
		expect(verifyBumps(cargoOk, pkgOk, formulasStale, "0.4.0")).toEqual([
			"error: Formula/jj-gt.rb version line didn't bump to 0.4.0",
		]);
	});
});

// ---- runOnce with fake deps ------------------------------------------

type Recorded = { cmd: string; args: string[] };

type FakeOpts = {
	// Map a command key ("jj diff", "cargo set-version --help", ...) to a
	// canned ShResult; anything unset returns exit 0 / empty output.
	responses?: Record<string, Partial<ShResult>>;
	files?: Record<string, string>;
	glob?: Record<string, string[]>;
};

function makeDeps(opts: FakeOpts = {}) {
	const commands: Recorded[] = [];
	const logs: string[] = [];
	const errs: string[] = [];
	const files: Record<string, string> = { ...(opts.files ?? {}) };

	const deps: Deps = {
		env: {},
		sh: async (
			cmd: string,
			args: string[],
			_shOpts?: ShOpts,
		): Promise<ShResult> => {
			commands.push({ cmd, args });
			// Match against progressively-specific keys so a test can pin a
			// response on just the subcommand it cares about.
			const full = `${cmd} ${args.join(" ")}`;
			const responses = opts.responses ?? {};
			let picked: Partial<ShResult> | undefined;
			for (const key of Object.keys(responses)) {
				if (full.startsWith(key)) picked = responses[key];
			}
			return {
				exitCode: picked?.exitCode ?? 0,
				stdout: picked?.stdout ?? "",
				stderr: picked?.stderr ?? "",
			};
		},
		readFile: async (path: string): Promise<string> => files[path] ?? "",
		writeFile: async (path: string, content: string): Promise<void> => {
			files[path] = content;
		},
		glob: async (pattern: string): Promise<string[]> =>
			opts.glob?.[pattern] ?? [],
		log: (msg: string) => logs.push(msg),
		err: (msg: string) => errs.push(msg),
	};
	return { deps, commands, logs, errs, files };
}

// Responses that make the four guards pass so the happy path can run.
// - jj diff: empty stdout → clean @
// - jj ...log main & ::@: a change_id line → main is an ancestor
// - jj tag list: empty → tag doesn't exist yet
// - cargo set-version --help: exit 0 → cargo-edit present
const HAPPY_RESPONSES: Record<string, Partial<ShResult>> = {
	"jj diff": { stdout: "" },
	"jj --ignore-working-copy log": { stdout: "abcdef123456\n" },
	"jj --ignore-working-copy tag list": { stdout: "v0.1.0\nv0.2.0\n" },
	"cargo set-version --help": { exitCode: 0 },
};

const HAPPY_FILES = {
	"tools/akiflow-cli/package.json": '{\n\t"version": "0.3.7"\n}\n',
	"Cargo.toml": 'version = "0.4.0"\n',
	"Formula/akiflow-cli.rb": '  version "0.4.0"\n',
};

const HAPPY_GLOB = { "Formula/*.rb": ["Formula/akiflow-cli.rb"] };

describe("runOnce guards", () => {
	test("bad version exits 1 before any command runs", async () => {
		const { deps, commands, errs } = makeDeps();
		expect(await runOnce(deps, ["nope"])).toBe(1);
		expect(commands).toEqual([]);
		expect(errs).toEqual([
			"usage: moon run root:release -- vX.Y.Z (or vX.Y.Z-rc.1); got: 'nope'",
		]);
	});

	test("dirty @ exits 1 before any bump", async () => {
		const { deps, commands, errs, files } = makeDeps({
			responses: { "jj diff": { stdout: "M Cargo.toml\n" } },
			files: { ...HAPPY_FILES },
			glob: HAPPY_GLOB,
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(1);
		expect(errs).toEqual([
			"error: working copy @ has uncommitted changes; finalize them first",
		]);
		// The clean-@ check is the FIRST command; no cargo/write followed.
		expect(commands).toEqual([
			{ cmd: "jj", args: ["diff", "--summary", "--ignore-working-copy"] },
		]);
		expect(files["tools/akiflow-cli/package.json"]).toBe(
			HAPPY_FILES["tools/akiflow-cli/package.json"],
		);
	});

	test("@ not descendant of main exits 1", async () => {
		const { deps, commands, errs } = makeDeps({
			responses: {
				"jj diff": { stdout: "" },
				"jj --ignore-working-copy log": { stdout: "\n" },
			},
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(1);
		expect(errs).toEqual([
			"error: @ is not a descendant of main (run: jj rebase -d main)",
		]);
		expect(commands.map((c) => `${c.cmd} ${c.args[0]}`)).toEqual([
			"jj diff",
			"jj --ignore-working-copy",
		]);
	});

	test("existing tag exits 1 before any bump", async () => {
		const { deps, commands, errs } = makeDeps({
			responses: {
				...HAPPY_RESPONSES,
				"jj --ignore-working-copy tag list": { stdout: "v0.3.9\nv0.4.0\n" },
			},
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(1);
		expect(errs).toEqual(["error: tag v0.4.0 already exists"]);
		// Stopped at the tag-list guard; cargo set-version never ran.
		expect(commands.some((c) => c.cmd === "cargo")).toBe(false);
	});

	test("missing cargo-edit exits 1", async () => {
		const { deps, errs } = makeDeps({
			responses: {
				...HAPPY_RESPONSES,
				"cargo set-version --help": { exitCode: 1 },
			},
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(1);
		expect(errs).toEqual([
			"error: cargo-edit not installed (run: cargo install --locked cargo-edit)",
		]);
	});

	test("verify failure after bumps exits 1", async () => {
		const { deps, errs } = makeDeps({
			responses: HAPPY_RESPONSES,
			files: { ...HAPPY_FILES, "Cargo.toml": 'version = "0.3.7"\n' },
			glob: HAPPY_GLOB,
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(1);
		expect(errs).toEqual([
			"error: workspace Cargo.toml version didn't bump to 0.4.0",
			'version = "0.3.7"',
		]);
	});
});

describe("runOnce happy path", () => {
	test("runs every step in the exact bash order", async () => {
		const { deps, commands, logs, files } = makeDeps({
			responses: HAPPY_RESPONSES,
			files: { ...HAPPY_FILES },
			glob: HAPPY_GLOB,
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(0);

		// The full shelled-command sequence, in order, matches release.sh.
		const seq = commands.map((c) => `${c.cmd} ${c.args.join(" ")}`);
		expect(seq).toEqual([
			"jj diff --summary --ignore-working-copy",
			"jj --ignore-working-copy log -r main & ::@ -T change_id --no-graph",
			'jj --ignore-working-copy tag list -T name ++ "\\n"',
			"cargo set-version --help",
			"cargo set-version --workspace 0.4.0",
			"cargo update --workspace",
			"jj commit -m release: v0.4.0",
			"jj tag set v0.4.0 -r @-",
			"jj bookmark set main -r @-",
			"jj --ignore-working-copy git export",
			"jj git push -b main",
			"jj-hp push-tags v0.4.0",
		]);

		// Side-effect files were rewritten via the pure bump fns.
		expect(files["tools/akiflow-cli/package.json"]).toContain(
			'"version": "0.4.0"',
		);
		expect(files["Formula/akiflow-cli.rb"]).toContain('version "0.4.0"');

		// Final success echo carries the actions URL.
		expect(logs.at(-1)).toBe(
			"   https://github.com/mattwilkinsonn/zireael/actions/workflows/release.yml",
		);
	});

	test("propagates a nonzero exit from a shelled step", async () => {
		const { deps } = makeDeps({
			responses: {
				...HAPPY_RESPONSES,
				"cargo set-version --workspace": { exitCode: 101 },
			},
			files: { ...HAPPY_FILES },
			glob: HAPPY_GLOB,
		});
		expect(await runOnce(deps, ["v0.4.0"])).toBe(101);
	});
});

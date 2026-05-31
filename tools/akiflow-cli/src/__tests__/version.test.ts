// Pins the `version` exposed by `af --version` to the value in
// package.json. Without this, src/index.ts had a hardcoded `version:
// "0.1.1"` string that silently drifted from every release; the
// homebrew tap formula test caught it on v0.3.3 (see PR notes).
//
// Two-part check:
//   1. package.json itself has a sane version string (catches the
//      "release recipe forgot to bump akiflow-cli" failure).
//   2. src/index.ts's `meta.version` references pkg.version (not a
//      hardcoded literal). Structural assertion against the source
//      text — catches anyone re-hardcoding a version literal.
//
// We don't spawn the bun-compiled binary here (slow, needs build
// setup); the structural check above is enough since bun's
// `--compile` bundles the JSON import into the binary at build time,
// so what index.ts reads at parse time IS what the binary ships.

import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import pkg from "../../package.json" with { type: "json" };

describe("af --version source", () => {
	it("package.json carries a non-empty version", () => {
		expect(pkg.version).toBeTruthy();
		expect(typeof pkg.version).toBe("string");
	});

	it("version looks like semver (X.Y.Z, optionally with -prerelease)", () => {
		expect(pkg.version).toMatch(/^\d+\.\d+\.\d+(-[A-Za-z0-9.-]+)?$/);
	});

	it("index.ts wires meta.version to pkg.version (not a literal)", () => {
		const indexSource = readFileSync(
			join(import.meta.dir, "..", "index.ts"),
			"utf-8",
		);
		// The metadata block should reference pkg.version, not a
		// "X.Y.Z" literal. Match any line of the form
		// `version: pkg.version` (whitespace-tolerant) inside the
		// `meta:` object.
		expect(indexSource).toMatch(/version:\s*pkg\.version/);
		// And the import that backs it.
		expect(indexSource).toMatch(
			/import\s+pkg\s+from\s+["']\.\.\/package\.json["']/,
		);
		// Negative: no hardcoded "version: '<literal>'" line.
		const hardcoded = indexSource.match(
			/version:\s*["']\d+\.\d+\.\d+(-[A-Za-z0-9.-]+)?["']/,
		);
		expect(hardcoded).toBeNull();
	});
});

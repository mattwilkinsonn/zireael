import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createLogger } from "../../lib/log";

let dir: string;
beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "af-log-test-"));
	process.env.AF_CACHE_DIR = dir;
});
afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
	delete process.env.AF_CACHE_DIR;
	delete process.env.AF_LOG;
	delete process.env.AF_DEBUG;
});

describe("createLogger", () => {
	test("by default writes nothing to file", () => {
		const log = createLogger({ module: "test" });
		log.warn("phase1", "something went wrong");
		expect(existsSync(join(dir, "af.log"))).toBe(false);
	});

	test("with AF_LOG=1, writes JSON Lines to af.log", () => {
		process.env.AF_LOG = "1";
		const log = createLogger({ module: "test" });
		log.warn("phase1", "msg one", { key: "value" });
		log.error("phase2", "msg two");
		const lines = readFileSync(join(dir, "af.log"), "utf8")
			.split("\n")
			.filter(Boolean);
		expect(lines.length).toBe(2);
		const first = JSON.parse(lines[0]!) as Record<string, unknown>;
		expect(first.level).toBe("warn");
		expect(first.module).toBe("test");
		expect(first.phase).toBe("phase1");
		expect(first.msg).toBe("msg one");
		expect(first.data).toEqual({ key: "value" });
		expect(first.ts).toMatch(/^\d{4}-\d{2}-\d{2}T/);
	});

	test("auth-extract logger writes to auth.log when AF_LOG=1", () => {
		process.env.AF_LOG = "1";
		const log = createLogger({ module: "auth.extract", file: "auth.log" });
		log.warn("token_search", "regex mismatch");
		expect(existsSync(join(dir, "auth.log"))).toBe(true);
	});
});

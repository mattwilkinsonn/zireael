import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { withLock } from "../../../lib/cache/lock";

let dir: string;
beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "af-lock-test-"));
});
afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

describe("withLock", () => {
	test("runs the critical section + releases on success", async () => {
		const result = await withLock(join(dir, ".lock"), async () => 42);
		expect(result).toBe(42);
	});

	test("releases lock even if callback throws", async () => {
		await expect(
			withLock(join(dir, ".lock"), async () => {
				throw new Error("boom");
			}),
		).rejects.toThrow("boom");
		// Subsequent acquire should succeed
		const result = await withLock(join(dir, ".lock"), async () => "ok");
		expect(result).toBe("ok");
	});

	test("serializes concurrent calls", async () => {
		const order: number[] = [];
		const slow = (n: number) =>
			withLock(join(dir, ".lock"), async () => {
				order.push(n);
				await new Promise((r) => setTimeout(r, 30));
				order.push(-n);
			});
		await Promise.all([slow(1), slow(2)]);
		// Order is [a, -a, b, -b] for some a/b — never interleaved
		expect(order.length).toBe(4);
		expect(order[0]! + order[1]!).toBe(0);
		expect(order[2]! + order[3]!).toBe(0);
	});
});

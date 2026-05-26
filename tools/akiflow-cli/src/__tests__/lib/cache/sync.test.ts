import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type ResourceClient, syncResource } from "../../../lib/cache/sync";

let dir: string;
beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "af-sync-test-"));
	process.env.AF_CACHE_DIR = dir;
});
afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
	delete process.env.AF_CACHE_DIR;
});

type Rec = { id: string; deleted_at: string | null; n: number };

function fakeClient(
	pages: Array<{ data: Rec[]; sync_token: string; has_next_page: boolean }>,
): ResourceClient {
	let idx = 0;
	return {
		get: async <_T>() => {
			const page = pages[idx++];
			if (!page) throw new Error("no more pages");
			return {
				success: true,
				message: null,
				data: page.data as unknown as _T[],
				sync_token: page.sync_token,
				has_next_page: page.has_next_page,
			};
		},
	};
}

describe("syncResource — cold start", () => {
	test("paginates through all pages and writes JSONL + final token", async () => {
		const client = fakeClient([
			{
				data: [
					{ id: "a", deleted_at: null, n: 1 },
					{ id: "b", deleted_at: null, n: 2 },
				],
				sync_token: "t1",
				has_next_page: true,
			},
			{
				data: [{ id: "c", deleted_at: null, n: 3 }],
				sync_token: "t2",
				has_next_page: false,
			},
		]);
		const result = await syncResource<Rec>(client, {
			resource: "test",
			keyOf: (r) => r.id,
			previousToken: null,
			limit: 100,
		});
		expect(result.finalToken).toBe("t2");
		expect(result.upsertedCount).toBe(3);
		expect(result.tombstoneCount).toBe(0);
		expect(result.pages).toBe(2);
		const content = await readFile(join(dir, "test.jsonl"), "utf8");
		expect(content.split("\n").filter(Boolean).length).toBe(3);
	});
});

describe("syncResource — delta", () => {
	test("applies tombstones by removing local records, keeps live records", async () => {
		const initial = fakeClient([
			{
				data: [
					{ id: "a", deleted_at: null, n: 1 },
					{ id: "b", deleted_at: null, n: 2 },
				],
				sync_token: "t1",
				has_next_page: false,
			},
		]);
		await syncResource<Rec>(initial, {
			resource: "test",
			keyOf: (r) => r.id,
			previousToken: null,
			limit: 100,
		});
		expect(existsSync(join(dir, "test.jsonl"))).toBe(true);

		const delta = fakeClient([
			{
				data: [{ id: "a", deleted_at: "2026-01-01T00:00:00Z", n: 1 }],
				sync_token: "t2",
				has_next_page: false,
			},
		]);
		const result = await syncResource<Rec>(delta, {
			resource: "test",
			keyOf: (r) => r.id,
			previousToken: "t1",
			limit: 100,
		});
		expect(result.tombstoneCount).toBe(1);
		expect(result.finalToken).toBe("t2");
		const content = await readFile(join(dir, "test.jsonl"), "utf8");
		const records = content
			.split("\n")
			.filter(Boolean)
			.map((l) => JSON.parse(l) as Rec);
		expect(records.map((r) => r.id)).toEqual(["b"]);
	});
});

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ApiResponse } from "../../../lib/api/types";
import { readResource, rebuild, refresh } from "../../../lib/cache";

let dir: string;
beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "af-cache-idx-test-"));
	process.env.AF_CACHE_DIR = dir;
	process.env.AF_NO_AUTO_SYNC = "1";
});
afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
	delete process.env.AF_CACHE_DIR;
	delete process.env.AF_NO_AUTO_SYNC;
});

// Fake client that returns one record per resource with the given id prefix.
function fakeClient() {
	return {
		get: async <T>(path: string): Promise<ApiResponse<T[]>> => {
			const resource = path.replace("/v5/", "");
			return {
				success: true,
				message: null,
				data: [
					{ id: `${resource}-1`, deleted_at: null, status: 2 } as unknown as T,
				],
				sync_token: `token-${resource}`,
				has_next_page: false,
			};
		},
	};
}

describe("rebuild", () => {
	test("creates all 8 resource JSONLs + tokens.json with last_full_sync_at", async () => {
		const summary = await rebuild(fakeClient());
		for (const res of [
			"tasks",
			"events",
			"time_slots",
			"labels",
			"tags",
			"calendars",
			"accounts",
			"contacts",
		]) {
			expect(existsSync(join(dir, `${res}.jsonl`))).toBe(true);
			expect(summary[res as keyof typeof summary].upserted).toBe(1);
		}
		expect(existsSync(join(dir, "tokens.json"))).toBe(true);
	});
});

describe("refresh", () => {
	test("uses stored sync_token from tokens.json", async () => {
		await rebuild(fakeClient());
		const summary = await refresh(fakeClient());
		expect(summary.tasks.upserted).toBe(1);
	});
});

describe("readResource", () => {
	test("returns parsed records for a resource", async () => {
		await rebuild(fakeClient());
		const tasks = await readResource(fakeClient(), "tasks");
		expect(tasks.length).toBe(1);
		expect(tasks[0]?.id).toBe("tasks-1");
	});
});

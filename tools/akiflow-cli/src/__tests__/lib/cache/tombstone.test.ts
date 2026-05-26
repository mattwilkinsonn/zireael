import { describe, expect, test } from "bun:test";
import { applyTombstones, isTombstone } from "../../../lib/cache/tombstone";

describe("isTombstone", () => {
	test("deleted_at set → true", () => {
		expect(isTombstone({ id: "a", deleted_at: "2026-01-01T00:00:00Z" })).toBe(
			true,
		);
	});
	test("status=9 → true", () => {
		expect(isTombstone({ id: "a", deleted_at: null, status: 9 })).toBe(true);
	});
	test("active record → false", () => {
		expect(isTombstone({ id: "a", deleted_at: null, status: 2 })).toBe(false);
	});
});

describe("applyTombstones", () => {
	test("removes records matching incoming tombstones", () => {
		const existing = [
			{ id: "a", deleted_at: null, status: 2 },
			{ id: "b", deleted_at: null, status: 2 },
			{ id: "c", deleted_at: null, status: 2 },
		];
		const incoming = [
			{ id: "b", deleted_at: "2026-01-01T00:00:00Z" },
			{ id: "d", deleted_at: null, status: 2 },
		];
		const { kept, upserts } = applyTombstones(existing, incoming, (r) => r.id);
		expect(kept.map((r) => r.id).sort()).toEqual(["a", "c"]);
		expect(upserts.map((r) => r.id)).toEqual(["d"]);
	});

	test("also removes records being replaced by upserts (caller appends them)", () => {
		const existing = [{ id: "a", deleted_at: null, status: 2 }];
		const incoming = [{ id: "a", deleted_at: null, status: 2 }];
		const { kept, upserts } = applyTombstones(existing, incoming, (r) => r.id);
		expect(kept.map((r) => r.id)).toEqual([]);
		expect(upserts.map((r) => r.id)).toEqual(["a"]);
	});
});

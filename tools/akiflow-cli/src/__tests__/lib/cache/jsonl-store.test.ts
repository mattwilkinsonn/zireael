import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	appendRecords,
	readAllRecords,
	rewriteRecords,
	upsertRecords,
} from "../../../lib/cache/jsonl-store";

let dir: string;
beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "af-jsonl-test-"));
});
afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

const file = (): string => join(dir, "test.jsonl");

describe("jsonl-store", () => {
	test("readAllRecords on missing file returns []", async () => {
		expect(await readAllRecords<{ id: string }>(file())).toEqual([]);
	});

	test("appendRecords writes one JSON object per line", async () => {
		await appendRecords(file(), [{ id: "a" }, { id: "b" }]);
		const text = await Bun.file(file()).text();
		const lines = text.split("\n").filter(Boolean);
		expect(lines.length).toBe(2);
		expect(JSON.parse(lines[0]!)).toEqual({ id: "a" });
	});

	test("readAllRecords parses each line independently", async () => {
		await appendRecords(file(), [
			{ id: "a", n: 1 },
			{ id: "b", n: 2 },
		]);
		const records = await readAllRecords<{ id: string; n: number }>(file());
		expect(records).toEqual([
			{ id: "a", n: 1 },
			{ id: "b", n: 2 },
		]);
	});

	test("readAllRecords skips malformed lines", async () => {
		await Bun.write(file(), '{"id":"a"}\nNOT JSON\n{"id":"b"}\n');
		const records = await readAllRecords<{ id: string }>(file());
		expect(records).toEqual([{ id: "a" }, { id: "b" }]);
	});

	test("upsertRecords replaces existing records by id", async () => {
		await appendRecords(file(), [
			{ id: "a", n: 1 },
			{ id: "b", n: 2 },
		]);
		await upsertRecords(
			file(),
			[
				{ id: "a", n: 99 },
				{ id: "c", n: 3 },
			],
			(r) => r.id,
		);
		const records = await readAllRecords<{ id: string; n: number }>(file());
		expect(records).toEqual([
			{ id: "b", n: 2 },
			{ id: "a", n: 99 },
			{ id: "c", n: 3 },
		]);
	});

	test("rewriteRecords replaces file contents", async () => {
		await appendRecords(file(), [{ id: "a" }, { id: "b" }]);
		await rewriteRecords(file(), [{ id: "x" }]);
		const records = await readAllRecords<{ id: string }>(file());
		expect(records).toEqual([{ id: "x" }]);
	});
});

import { existsSync, mkdirSync } from "node:fs";
import { appendFile, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

/**
 * Read every record from a JSONL file. Skips malformed lines (with warning).
 * Returns empty array if the file doesn't exist.
 */
export async function readAllRecords<T>(filePath: string): Promise<T[]> {
	if (!existsSync(filePath)) return [];
	const text = await readFile(filePath, "utf8");
	const records: T[] = [];
	for (const line of text.split("\n")) {
		if (!line.trim()) continue;
		try {
			records.push(JSON.parse(line) as T);
		} catch {
			console.warn(`[jsonl-store] skipping malformed line in ${filePath}`);
		}
	}
	return records;
}

/**
 * Append records to a JSONL file (one JSON object per line). Creates the
 * directory if missing. No-op on empty input.
 */
export async function appendRecords<T>(
	filePath: string,
	records: T[],
): Promise<void> {
	if (records.length === 0) return;
	mkdirSync(dirname(filePath), { recursive: true });
	const payload = `${records.map((r) => JSON.stringify(r)).join("\n")}\n`;
	await appendFile(filePath, payload, "utf8");
}

/**
 * Upsert records by extracted key — replaces existing records with matching
 * keys, appends new records that don't match. Rewrites the file. O(n) on
 * file size.
 */
export async function upsertRecords<T>(
	filePath: string,
	newRecords: T[],
	keyOf: (r: T) => string,
): Promise<void> {
	const existing = await readAllRecords<T>(filePath);
	const newKeys = new Set(newRecords.map(keyOf));
	const kept = existing.filter((r) => !newKeys.has(keyOf(r)));
	const merged = [...kept, ...newRecords];
	mkdirSync(dirname(filePath), { recursive: true });
	await writeFile(
		filePath,
		`${merged.map((r) => JSON.stringify(r)).join("\n")}\n`,
		"utf8",
	);
}

/**
 * Replace the file's contents with the given records.
 */
export async function rewriteRecords<T>(
	filePath: string,
	records: T[],
): Promise<void> {
	mkdirSync(dirname(filePath), { recursive: true });
	await writeFile(
		filePath,
		`${records.map((r) => JSON.stringify(r)).join("\n")}\n`,
		"utf8",
	);
}

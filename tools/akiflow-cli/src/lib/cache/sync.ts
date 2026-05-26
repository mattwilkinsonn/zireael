import type { ApiResponse } from "../api/types";
import { cacheFile } from "../platform-config";
import { readAllRecords, rewriteRecords } from "./jsonl-store";
import { isTombstone } from "./tombstone";

export interface ResourceClient {
	get<T>(
		path: string,
		params: { sync_token?: string; limit?: number },
	): Promise<ApiResponse<T[]>>;
}

export interface SyncOptions<T extends { id: string }> {
	/** Resource name; becomes the JSONL filename `<resource>.jsonl`. */
	resource: string;
	/** Key extractor for deduplication. */
	keyOf: (r: T) => string;
	/** Previous sync_token; pass null for cold start. */
	previousToken: string | null;
	/** Per-page limit. Default 2500 (matches Akiflow webapp). */
	limit?: number;
	/** Override the API path. Default: `/v5/<resource>`. */
	apiPath?: string;
}

export interface SyncResult {
	finalToken: string;
	upsertedCount: number;
	tombstoneCount: number;
	pages: number;
}

/**
 * Sync one resource from the Akiflow API into local JSONL cache.
 *
 * - Paginates with sync_token until has_next_page = false.
 * - Tombstones (deleted_at != null OR status=9) remove matching local records.
 * - Upserts replace matching records (by key) or append if new.
 * - Single file rewrite at the end — no torn state.
 */
export async function syncResource<
	T extends { id: string; deleted_at?: string | null; status?: number | null },
>(client: ResourceClient, opts: SyncOptions<T>): Promise<SyncResult> {
	const path = opts.apiPath ?? `/v5/${opts.resource}`;
	const file = cacheFile(`${opts.resource}.jsonl`);
	const limit = opts.limit ?? 2500;

	let token: string | undefined = opts.previousToken ?? undefined;
	let pages = 0;
	let upsertedCount = 0;
	let tombstoneCount = 0;
	const allUpserts: T[] = [];
	const tombstoneIds = new Set<string>();

	// eslint-disable-next-line no-constant-condition
	while (true) {
		const params: { sync_token?: string; limit: number } = { limit };
		if (token) params.sync_token = token;
		const resp = await client.get<T[]>(path, params);
		if (!resp.success) {
			throw new Error(
				`sync ${opts.resource} failed: ${resp.message ?? "unknown error"}`,
			);
		}
		pages++;
		if (resp.sync_token) token = resp.sync_token;

		const dataRecords = resp.data as unknown as T[];
		for (const r of dataRecords) {
			if (isTombstone(r)) {
				tombstoneIds.add(opts.keyOf(r));
				tombstoneCount++;
			} else {
				allUpserts.push(r);
				upsertedCount++;
			}
		}

		if (!resp.has_next_page) break;
	}

	if (token === undefined) {
		throw new Error(
			`sync ${opts.resource}: no sync_token returned from server`,
		);
	}

	// Merge into local cache: drop tombstoned IDs + IDs being replaced by
	// upserts, then append upserts. Single rewrite for atomicity.
	const existing = await readAllRecords<T>(file);
	const upsertIds = new Set(allUpserts.map(opts.keyOf));
	const kept = existing.filter(
		(r) => !tombstoneIds.has(opts.keyOf(r)) && !upsertIds.has(opts.keyOf(r)),
	);
	await rewriteRecords(file, [...kept, ...allUpserts]);

	return { finalToken: token, upsertedCount, tombstoneCount, pages };
}

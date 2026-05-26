import { rmSync } from "node:fs";
import type {
	Account,
	ApiResponse,
	Calendar,
	Contact,
	Event,
	Label,
	Tag,
	Task,
	TimeSlot,
} from "../api/types";
import { cacheFile, cachePath } from "../platform-config";
import { readAllRecords } from "./jsonl-store";
import { withLock } from "./lock";
import { syncResource } from "./sync";
import { readTokens, type Tokens, writeTokens } from "./tokens";

const LOCK = (): string => cacheFile(".lock");

const RESOURCES = [
	"tasks",
	"events",
	"time_slots",
	"labels",
	"tags",
	"calendars",
	"accounts",
	"contacts",
] as const;
export type Resource = (typeof RESOURCES)[number];

export interface CacheClient {
	get<T>(
		path: string,
		params: { sync_token?: string; limit?: number },
	): Promise<ApiResponse<T[]>>;
}

export interface ResourceSyncSummary {
	upserted: number;
	tombstones: number;
	pages: number;
}

/**
 * Cold-start sync: delete the cache and rebuild every resource from scratch.
 */
export async function rebuild(
	client: CacheClient,
): Promise<Record<Resource, ResourceSyncSummary>> {
	return withLock(LOCK(), async () => {
		try {
			rmSync(cachePath(), { recursive: true, force: true });
		} catch {
			/* fresh */
		}
		const tokens: Tokens = {};
		const summary = {} as Record<Resource, ResourceSyncSummary>;
		for (const res of RESOURCES) {
			const result = await syncResource(client, {
				resource: res,
				keyOf: (r: { id: string }) => r.id,
				previousToken: null,
			});
			tokens[res] = result.finalToken;
			summary[res] = {
				upserted: result.upsertedCount,
				tombstones: result.tombstoneCount,
				pages: result.pages,
			};
		}
		tokens.last_full_sync_at = new Date().toISOString();
		await writeTokens(tokens);
		return summary;
	});
}

/**
 * Warm-path delta sync: fetch only changes since last token per resource.
 */
export async function refresh(
	client: CacheClient,
): Promise<Record<Resource, ResourceSyncSummary>> {
	return withLock(LOCK(), async () => {
		const tokens = await readTokens();
		const summary = {} as Record<Resource, ResourceSyncSummary>;
		for (const res of RESOURCES) {
			const result = await syncResource(client, {
				resource: res,
				keyOf: (r: { id: string }) => r.id,
				previousToken: tokens[res] ?? null,
			});
			tokens[res] = result.finalToken;
			summary[res] = {
				upserted: result.upsertedCount,
				tombstones: result.tombstoneCount,
				pages: result.pages,
			};
		}
		await writeTokens(tokens);
		return summary;
	});
}

/**
 * Read all records for a resource. Auto-triggers a refresh if the cache is
 * older than 24h (unless AF_NO_AUTO_SYNC=1). If the cache has never been
 * built, runs `refresh` first.
 */
export async function readResource(
	client: CacheClient,
	resource: "tasks",
): Promise<Task[]>;
export async function readResource(
	client: CacheClient,
	resource: "events",
): Promise<Event[]>;
export async function readResource(
	client: CacheClient,
	resource: "time_slots",
): Promise<TimeSlot[]>;
export async function readResource(
	client: CacheClient,
	resource: "labels",
): Promise<Label[]>;
export async function readResource(
	client: CacheClient,
	resource: "tags",
): Promise<Tag[]>;
export async function readResource(
	client: CacheClient,
	resource: "calendars",
): Promise<Calendar[]>;
export async function readResource(
	client: CacheClient,
	resource: "accounts",
): Promise<Account[]>;
export async function readResource(
	client: CacheClient,
	resource: "contacts",
): Promise<Contact[]>;
export async function readResource<T>(
	client: CacheClient,
	resource: Resource,
): Promise<T[]> {
	const tokens = await readTokens();
	const hasToken = tokens[resource] != null;
	const stale = shouldAutoRefresh(tokens);
	if ((!hasToken || stale) && !process.env.AF_NO_AUTO_SYNC) {
		await refresh(client);
	}
	return readAllRecords<T>(cacheFile(`${resource}.jsonl`));
}

function shouldAutoRefresh(tokens: Tokens): boolean {
	if (!tokens.last_full_sync_at) return true;
	const age = Date.now() - new Date(tokens.last_full_sync_at).getTime();
	return age > 24 * 60 * 60 * 1000;
}

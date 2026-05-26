import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { AkiflowClient } from "./api/client";
import type { Task } from "./api/types";

const CACHE_DIR = path.join(os.homedir(), ".cache", "af");
const TASKS_CACHE_FILE = path.join(CACHE_DIR, "tasks-cache.json");
const TASKS_CACHE_META_FILE = path.join(CACHE_DIR, "tasks-cache-meta.json");

// Bump this if we change cache shape.
const CACHE_VERSION = 1;

export interface TasksCacheMeta {
	version: number;
	// Akiflow sync token for incremental sync (delta)
	syncToken?: string;
	// Epoch millis
	lastFullSyncAt: number;
	lastSyncAt: number;
	taskCount: number;
}

interface TasksCacheFile {
	version: number;
	tasksById: Record<string, Task>;
}

async function ensureCacheDir(): Promise<void> {
	await fs.mkdir(CACHE_DIR, { recursive: true });
}

async function loadTasksCacheFile(): Promise<TasksCacheFile | null> {
	try {
		const content = await fs.readFile(TASKS_CACHE_FILE, "utf-8");
		const parsed = JSON.parse(content) as TasksCacheFile;

		if (!parsed || parsed.version !== CACHE_VERSION || !parsed.tasksById) {
			return null;
		}

		return parsed;
	} catch {
		return null;
	}
}

async function loadTasksCacheMeta(): Promise<TasksCacheMeta | null> {
	try {
		const content = await fs.readFile(TASKS_CACHE_META_FILE, "utf-8");
		const parsed = JSON.parse(content) as TasksCacheMeta;

		if (!parsed || parsed.version !== CACHE_VERSION) {
			return null;
		}

		return parsed;
	} catch {
		return null;
	}
}

async function saveTasksCache(
	tasksById: Record<string, Task>,
	meta: TasksCacheMeta,
): Promise<void> {
	await ensureCacheDir();

	const cacheFile: TasksCacheFile = {
		version: CACHE_VERSION,
		tasksById,
	};

	await fs.writeFile(TASKS_CACHE_FILE, JSON.stringify(cacheFile));
	await fs.writeFile(TASKS_CACHE_META_FILE, JSON.stringify(meta));
}

function applyDelta(
	tasksById: Record<string, Task>,
	delta: Task[],
): Record<string, Task> {
	for (const task of delta) {
		// Treat deleted/trashed tasks as removed from local cache.
		if (task.deleted_at || task.trashed_at) {
			delete tasksById[task.id];
			continue;
		}

		tasksById[task.id] = task;
	}

	return tasksById;
}

export interface SyncTasksCacheOptions {
	/** Force full resync (ignore existing cache + syncToken) */
	forceFull?: boolean;
	/** For commands that want to avoid noisy output */
	quiet?: boolean;
}

/**
 * Ensure local cache is present and up-to-date.
 *
 * - First run: full sync via offset pagination.
 * - Next runs: incremental sync via sync_token.
 */
export async function syncTasksCache(
	client: AkiflowClient,
	options: SyncTasksCacheOptions = {},
): Promise<{ tasks: Task[]; meta: TasksCacheMeta }> {
	const existingCache = await loadTasksCacheFile();
	const existingMeta = await loadTasksCacheMeta();

	const needsFull =
		options.forceFull ||
		!existingCache ||
		!existingMeta ||
		existingCache.version !== CACHE_VERSION ||
		existingMeta.version !== CACHE_VERSION ||
		!existingMeta.syncToken;

	if (needsFull) {
		const { tasksById, meta } = await fullSyncTasksCache(client, {
			quiet: options.quiet,
		});
		return { tasks: Object.values(tasksById), meta };
	}

	// Incremental sync
	const tasksById = { ...existingCache.tasksById };
	const { tasks: updated, syncToken } = await incrementalSyncTasksCache(
		client,
		tasksById,
		existingMeta.syncToken ?? "",
		{ quiet: options.quiet },
	);

	const now = Date.now();
	const meta: TasksCacheMeta = {
		version: CACHE_VERSION,
		syncToken,
		lastFullSyncAt: existingMeta.lastFullSyncAt,
		lastSyncAt: now,
		taskCount: Object.keys(updated).length,
	};

	await saveTasksCache(updated, meta);

	return { tasks: Object.values(updated), meta };
}

async function fullSyncTasksCache(
	client: AkiflowClient,
	options: { quiet?: boolean } = {},
): Promise<{ tasksById: Record<string, Task>; meta: TasksCacheMeta }> {
	const tasksById: Record<string, Task> = {};

	const limit = 2500;
	let cursor: string | undefined;

	let lastSyncToken: string | undefined;
	let safety = 0;

	// Full sync: page through the entire dataset using the sync_token cursor.
	while (true) {
		const resp = await client.getTasks({ limit, syncToken: cursor });
		const page = resp.data ?? [];

		for (const task of page) {
			if (task.deleted_at || task.trashed_at) continue;
			tasksById[task.id] = task;
		}

		if (resp.sync_token) {
			lastSyncToken = resp.sync_token;
		}

		const hasNext = resp.has_next_page === true;
		if (!hasNext) break;

		// Avoid infinite loops: token must advance.
		if (!resp.sync_token || resp.sync_token === cursor) break;

		cursor = resp.sync_token;
		safety += 1;
		if (safety > 1000) break;
	}

	const now = Date.now();
	const meta: TasksCacheMeta = {
		version: CACHE_VERSION,
		syncToken: lastSyncToken,
		lastFullSyncAt: now,
		lastSyncAt: now,
		taskCount: Object.keys(tasksById).length,
	};

	await saveTasksCache(tasksById, meta);

	if (!options.quiet) {
		// Intentionally lightweight output (stdout) for long first sync.
		// Commands can suppress via quiet.
		// eslint-disable-next-line no-console
		console.error(`Cache: full sync complete (${meta.taskCount} tasks)`);
	}

	return { tasksById, meta };
}

async function incrementalSyncTasksCache(
	client: AkiflowClient,
	tasksById: Record<string, Task>,
	syncToken: string,
	options: { quiet?: boolean } = {},
): Promise<{ tasks: Record<string, Task>; syncToken: string }> {
	const limit = 2500;

	let currentToken: string | undefined = syncToken;
	let newToken: string = syncToken;

	// Keep fetching while API indicates more pages.
	while (true) {
		const resp = await client.getTasks({ limit, syncToken: currentToken });
		const delta = resp.data ?? [];

		applyDelta(tasksById, delta);

		if (resp.sync_token) {
			newToken = resp.sync_token;
		}

		const hasNext = resp.has_next_page === true;
		if (!hasNext) break;

		// For cursor-like paging of deltas, advance the token.
		if (resp.sync_token) {
			currentToken = resp.sync_token;
		} else {
			break;
		}
	}

	if (!options.quiet) {
		// eslint-disable-next-line no-console
		console.error("Cache: incremental sync complete");
	}

	return { tasks: tasksById, syncToken: newToken };
}

export async function readTasksFromCacheOnly(): Promise<Task[] | null> {
	const cache = await loadTasksCacheFile();
	if (!cache) return null;
	return Object.values(cache.tasksById);
}

export async function readTasksCacheMetaOnly(): Promise<TasksCacheMeta | null> {
	return loadTasksCacheMeta();
}

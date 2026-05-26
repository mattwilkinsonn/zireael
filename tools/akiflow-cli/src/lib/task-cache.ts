import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { CreateTaskPayload, Task } from "./api/types";

const CACHE_DIR = path.join(os.homedir(), ".cache", "af");
const PENDING_TASKS_FILE = path.join(CACHE_DIR, "pending-tasks.json");

// TTL for pending tasks (5 minutes) - after this, assume API has caught up
const PENDING_TTL_MS = 5 * 60 * 1000;

interface PendingTask {
	task: Task;
	createdAt: number;
}

interface PendingTasksCache {
	tasks: PendingTask[];
}

async function ensureCacheDir(): Promise<void> {
	try {
		await fs.mkdir(CACHE_DIR, { recursive: true });
	} catch {
		// Directory might already exist
	}
}

export async function loadPendingTasks(): Promise<Task[]> {
	try {
		const content = await fs.readFile(PENDING_TASKS_FILE, "utf-8");
		const cache: PendingTasksCache = JSON.parse(content);

		const now = Date.now();
		// Filter out expired tasks
		const validTasks = cache.tasks.filter(
			(pt) => now - pt.createdAt < PENDING_TTL_MS,
		);

		// Update cache if we filtered any out
		if (validTasks.length !== cache.tasks.length) {
			await savePendingTasksCache({ tasks: validTasks });
		}

		return validTasks.map((pt) => pt.task);
	} catch {
		return [];
	}
}

async function savePendingTasksCache(cache: PendingTasksCache): Promise<void> {
	await ensureCacheDir();
	await fs.writeFile(PENDING_TASKS_FILE, JSON.stringify(cache, null, 2));
}

export async function addPendingTask(task: Task): Promise<void> {
	const existing = await loadPendingTasks();

	// Remove any existing task with same ID
	const filtered = existing.filter((t) => t.id !== task.id);

	const pendingTask: PendingTask = {
		task,
		createdAt: Date.now(),
	};

	const cache: PendingTasksCache = {
		tasks: [
			...filtered.map((t) => ({
				task: t,
				createdAt: Date.now(), // Refresh TTL for existing
			})),
			pendingTask,
		],
	};

	await savePendingTasksCache(cache);
}

export async function removePendingTask(taskId: string): Promise<void> {
	const existing = await loadPendingTasks();
	const filtered = existing.filter((t) => t.id !== taskId);

	const cache: PendingTasksCache = {
		tasks: filtered.map((t) => ({
			task: t,
			createdAt: Date.now(),
		})),
	};

	await savePendingTasksCache(cache);
}

export async function clearPendingTasks(): Promise<void> {
	await savePendingTasksCache({ tasks: [] });
}

/**
 * Merge pending tasks with API tasks.
 * Pending tasks take precedence if not found in API response.
 */
export function mergeTasks(apiTasks: Task[], pendingTasks: Task[]): Task[] {
	const apiTaskIds = new Set(apiTasks.map((t) => t.id));

	// Add pending tasks that aren't already in the API response
	const newPendingTasks = pendingTasks.filter((pt) => !apiTaskIds.has(pt.id));

	return [...apiTasks, ...newPendingTasks];
}

/**
 * Create a Task object from CreateTaskPayload and API response
 */
export function createTaskFromPayload(
	_payload: CreateTaskPayload,
	apiResponse: Task,
): Task {
	return apiResponse;
}

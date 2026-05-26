import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import type { UpdateTaskPayload } from "../lib/api/types";

interface ContextFile {
	tasks: Array<{
		shortId: number;
		id: string;
		title: string;
	}>;
	timestamp: number;
}

function getContextFilePath(): string {
	return join(homedir(), ".cache", "af", "last-list.json");
}

function readContextFile(): ContextFile | null {
	try {
		const path = getContextFilePath();
		const content = readFileSync(path, "utf-8");
		return JSON.parse(content) as ContextFile;
	} catch {
		return null;
	}
}

function resolveTaskId(
	identifier: string,
	context: ContextFile | null,
): string | null {
	const uuidRegex =
		/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
	if (uuidRegex.test(identifier)) {
		return identifier;
	}

	if (!context) {
		return null;
	}

	const shortId = parseInt(identifier, 10);
	if (!Number.isNaN(shortId)) {
		const task = context.tasks.find((t) => t.shortId === shortId);
		if (task) {
			return task.id;
		}
		return null;
	}

	const matchingTasks = context.tasks.filter((t) =>
		t.id.toLowerCase().startsWith(identifier.toLowerCase()),
	);

	if (matchingTasks.length === 1) {
		return matchingTasks[0]?.id ?? null;
	}

	if (matchingTasks.length > 1) {
		console.error(
			`Error: Ambiguous UUID "${identifier}" matches ${matchingTasks.length} tasks`,
		);
		return null;
	}

	return null;
}

function getTaskTitle(
	taskId: string,
	context: ContextFile,
): string | undefined {
	return context.tasks.find((t) => t.id === taskId)?.title;
}

export const doCommand = defineCommand({
	meta: {
		name: "do",
		description: "Mark tasks as complete by short ID or UUID",
	},
	args: {
		ids: {
			type: "string",
			description: "Task IDs to complete (short ID or UUID)",
			required: true,
		},
	},
	run: async (context) => {
		const idsArg = context.args.ids as unknown;
		const ids = Array.isArray(idsArg) ? idsArg : [idsArg];

		if (!ids || ids.length === 0) {
			console.error("Error: No task IDs provided");
			process.exit(1);
		}

		const contextFile = readContextFile();
		if (!contextFile) {
			console.error(
				"Error: No task context found. Run 'af ls' first to create context.",
			);
			process.exit(1);
		}

		const resolvedTasks: Array<{ id: string; title: string }> = [];
		const failedIds: string[] = [];

		for (const id of ids) {
			const resolvedId = resolveTaskId(id, contextFile);
			if (resolvedId) {
				const title = getTaskTitle(resolvedId, contextFile);
				resolvedTasks.push({
					id: resolvedId,
					title: title || "Unknown",
				});
			} else {
				failedIds.push(id);
			}
		}

		if (failedIds.length > 0) {
			console.error(
				`Error: Could not resolve task IDs: ${failedIds.join(", ")}`,
			);
			if (resolvedTasks.length === 0) {
				process.exit(1);
			}
		}

		const client = createClient();
		const now = Date.now();
		const timestamp = new Date(now).toISOString();

		const updatePayloads: UpdateTaskPayload[] = resolvedTasks.map((task) => ({
			id: task.id,
			done: true,
			done_at: timestamp,
			status: 2,
			global_updated_at: timestamp,
		}));

		try {
			const response = await client.upsertTasks(updatePayloads);

			if (response.success) {
				console.log(`✓ Completed ${resolvedTasks.length} task(s):`);
				for (const task of resolvedTasks) {
					console.log(`  • ${task.title}`);
				}
			} else {
				console.error("Error: Failed to complete tasks");
				console.error(response.message);
				process.exit(1);
			}
		} catch (error) {
			console.error("Error: Failed to complete tasks");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

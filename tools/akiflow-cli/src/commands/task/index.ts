import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { defineCommand } from "citty";
import { createClient } from "../../lib/api/client";
import type { UpdateTaskPayload } from "../../lib/api/types";
import {
	createDateTimeUTC,
	getLocalTimezone,
	getTodayDate,
	parseDate,
	parseTime,
} from "../../lib/date-parser";
import { parseDuration } from "../../lib/duration-parser";

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
	// If identifier looks like a full UUID (36 chars with dashes), return it directly
	const uuidRegex =
		/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
	if (uuidRegex.test(identifier)) {
		return identifier;
	}

	// If no context, can't resolve short IDs or partial UUIDs
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

function formatDate(date: Date): string {
	const year = date.getFullYear();
	const month = String(date.getMonth() + 1).padStart(2, "0");
	const day = String(date.getDate()).padStart(2, "0");
	return `${year}-${month}-${day}`;
}

export const taskEditCommand = defineCommand({
	meta: {
		name: "edit",
		description: "Show editable fields for a task",
	},
	args: {
		id: {
			type: "string",
			description: "Task ID (short ID or UUID)",
			required: true,
		},
	},
	run: async (context) => {
		const id = context.args.id as string;
		const contextFile = readContextFile();

		const taskId = resolveTaskId(id, contextFile);
		if (!taskId) {
			console.error(
				`Error: Could not resolve task ID "${id}". Run 'af ls' first or provide a full UUID.`,
			);
			process.exit(1);
		}

		const client = createClient();

		try {
			const response = await client.getTask(taskId);
			if (!response.success || !response.data) {
				console.error("Error: Failed to fetch task");
				process.exit(1);
			}

			const task = response.data;

			console.log(`Task: ${task.title}`);
			console.log(`ID: ${task.id}`);
			console.log(`Description: ${task.description || "(none)"}`);
			console.log(`Date: ${task.date || "(not scheduled)"}`);
			console.log(
				`Status: ${task.status === 2 ? "Done" : task.status === 0 ? "Deleted" : "Active"}`,
			);
			console.log(`Priority: ${task.priority || "(none)"}`);
			console.log(
				`Duration: ${task.duration ? `${task.duration}ms` : "(none)"}`,
			);
			console.log(`Due date: ${task.due_date || "(none)"}`);
			console.log(`Project ID: ${task.listId || "(none)"}`);
			console.log(
				`Tags: ${task.tags_ids.length > 0 ? task.tags_ids.join(", ") : "(none)"}`,
			);
			console.log("\nEditable fields:");
			console.log("  Use 'af task move <id> <project>' to change project");
			console.log("  Use 'af task plan <id> <date>' to schedule task");
			console.log("  Use 'af task snooze <id> <duration>' to push back task");
			console.log("  Use 'af task delete <id>' to delete task");
		} catch (error) {
			console.error("Error: Failed to edit task");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

export const taskMoveCommand = defineCommand({
	meta: {
		name: "move",
		description: "Move task to a project",
	},
	args: {
		id: {
			type: "string",
			description: "Task ID (short ID or UUID)",
			required: true,
		},
		project: {
			type: "string",
			description: "Project ID to move task to",
			required: true,
		},
	},
	run: async (context) => {
		const id = context.args.id as string;
		const projectId = context.args.project as string;
		const contextFile = readContextFile();

		const taskId = resolveTaskId(id, contextFile);
		if (!taskId) {
			console.error(
				`Error: Could not resolve task ID "${id}". Run 'af ls' first or provide a full UUID.`,
			);
			process.exit(1);
		}

		const client = createClient();
		const timestamp = new Date().toISOString();

		const updatePayload: UpdateTaskPayload = {
			id: taskId,
			listId: projectId,
			global_updated_at: timestamp,
		};

		try {
			const response = await client.upsertTasks([updatePayload]);

			if (response.success) {
				console.log(`✓ Moved task "${id}" to project "${projectId}"`);
			} else {
				console.error("Error: Failed to move task");
				console.error(response.message);
				process.exit(1);
			}
		} catch (error) {
			console.error("Error: Failed to move task");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

export const taskPlanCommand = defineCommand({
	meta: {
		name: "plan",
		description: "Schedule task for a specific date",
	},
	args: {
		id: {
			type: "string",
			description: "Task ID (short ID or UUID)",
			required: true,
		},
		date: {
			type: "string",
			description: "Date to schedule task (YYYY-MM-DD or natural language)",
			required: false,
		},
		at: {
			type: "string",
			description: "Time for scheduling (e.g., 21:00, 14:30)",
			required: false,
		},
	},
	run: async (context) => {
		const id = context.args.id as string;
		const dateArg = context.args.date as string | undefined;
		const atArg = context.args.at as string | undefined;
		const contextFile = readContextFile();

		const taskId = resolveTaskId(id, contextFile);
		if (!taskId) {
			console.error(
				`Error: Could not resolve task ID "${id}". Run 'af ls' first or provide a full UUID.`,
			);
			process.exit(1);
		}

		let dateStr: string;

		if (dateArg) {
			const dateMatch = dateArg.match(/^(\d{4})-(\d{2})-(\d{2})$/);
			if (dateMatch) {
				dateStr = dateMatch[0];
			} else {
				const parsedDate = parseDate(dateArg);
				if (parsedDate) {
					dateStr = parsedDate;
				} else {
					console.error(
						`Error: Invalid date format "${dateArg}". Use YYYY-MM-DD or natural language (e.g., "today", "tomorrow", "next friday").`,
					);
					process.exit(1);
				}
			}
		} else if (atArg) {
			dateStr = getTodayDate();
		} else {
			console.error("Error: Either --date or --at must be specified.");
			process.exit(1);
		}

		const scheduledDate = new Date(dateStr);
		if (Number.isNaN(scheduledDate.getTime())) {
			console.error(`Error: Invalid date "${dateArg}"`);
			process.exit(1);
		}

		const client = createClient();
		const timestamp = new Date().toISOString();

		const updatePayload: UpdateTaskPayload = {
			id: taskId,
			date: dateStr,
			global_updated_at: timestamp,
		};

		if (atArg) {
			const parsedTime = parseTime(atArg);
			if (!parsedTime) {
				console.error(
					`Error: Invalid time format "${atArg}". Use HH:MM format (e.g., "21:00", "14:30").`,
				);
				process.exit(1);
			}

			updatePayload.datetime = createDateTimeUTC(
				dateStr,
				parsedTime.hours,
				parsedTime.minutes,
			);
			updatePayload.datetime_tz = getLocalTimezone();
		}

		try {
			const response = await client.upsertTasks([updatePayload]);

			if (response.success) {
				if (atArg) {
					console.log(`✓ Scheduled task "${id}" for ${dateStr} at ${atArg}`);
				} else {
					console.log(`✓ Scheduled task "${id}" for ${dateStr}`);
				}
			} else {
				console.error("Error: Failed to schedule task");
				console.error(response.message);
				process.exit(1);
			}
		} catch (error) {
			console.error("Error: Failed to schedule task");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

export const taskSnoozeCommand = defineCommand({
	meta: {
		name: "snooze",
		description: "Push task back by a duration (e.g., 1h, 2d, 1w)",
	},
	args: {
		id: {
			type: "string",
			description: "Task ID (short ID or UUID)",
			required: true,
		},
		duration: {
			type: "string",
			description: "Duration to snooze (e.g., 1h, 2d, 1w)",
			required: true,
		},
	},
	run: async (context) => {
		const id = context.args.id as string;
		const durationArg = context.args.duration as string;
		const contextFile = readContextFile();

		const taskId = resolveTaskId(id, contextFile);
		if (!taskId) {
			console.error(
				`Error: Could not resolve task ID "${id}". Run 'af ls' first or provide a full UUID.`,
			);
			process.exit(1);
		}

		let snoozeDuration: number;
		try {
			snoozeDuration = parseDuration(durationArg);
		} catch (error) {
			console.error(
				`Error: ${error instanceof Error ? error.message : "Invalid duration"}`,
			);
			process.exit(1);
		}

		const client = createClient();
		const allTasksResponse = await client.getTasks();
		if (!allTasksResponse.success || !allTasksResponse.data) {
			console.error("Error: Failed to fetch tasks");
			process.exit(1);
		}

		const task = allTasksResponse.data.find((t) => t.id === taskId);
		if (!task) {
			console.error(`Error: Task with ID "${taskId}" not found`);
			process.exit(1);
		}

		let baseDate = task.date ? new Date(task.date) : new Date();
		if (Number.isNaN(baseDate.getTime())) {
			baseDate = new Date();
		}

		const newDate = new Date(baseDate.getTime() + snoozeDuration);
		const dateStr = formatDate(newDate);
		const timestamp = new Date().toISOString();

		const updatePayload: UpdateTaskPayload = {
			id: taskId,
			date: dateStr,
			global_updated_at: timestamp,
		};

		try {
			const response = await client.upsertTasks([updatePayload]);

			if (response.success) {
				console.log(`✓ Snoozed task "${id}" to ${dateStr}`);
			} else {
				console.error("Error: Failed to snooze task");
				console.error(response.message);
				process.exit(1);
			}
		} catch (error) {
			console.error("Error: Failed to snooze task");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

export const taskDeleteCommand = defineCommand({
	meta: {
		name: "delete",
		description: "Soft delete a task",
	},
	args: {
		id: {
			type: "string",
			description: "Task ID (short ID or UUID)",
			required: true,
		},
	},
	run: async (context) => {
		const id = context.args.id as string;
		const contextFile = readContextFile();

		const taskId = resolveTaskId(id, contextFile);
		if (!taskId) {
			console.error(
				`Error: Could not resolve task ID "${id}". Run 'af ls' first or provide a full UUID.`,
			);
			process.exit(1);
		}

		const client = createClient();
		const timestamp = new Date().toISOString();

		const updatePayload: UpdateTaskPayload = {
			id: taskId,
			deleted_at: timestamp,
			global_updated_at: timestamp,
		};

		try {
			const response = await client.upsertTasks([updatePayload]);

			if (response.success) {
				console.log(`✓ Deleted task "${id}"`);
			} else {
				console.error("Error: Failed to delete task");
				console.error(response.message);
				process.exit(1);
			}
		} catch (error) {
			console.error("Error: Failed to delete task");
			if (error instanceof Error) {
				console.error(error.message);
			}
			process.exit(1);
		}
	},
});

export const taskCommand = defineCommand({
	meta: {
		name: "task",
		description: "Task management subcommands",
	},
	subCommands: {
		edit: taskEditCommand,
		move: taskMoveCommand,
		plan: taskPlanCommand,
		snooze: taskSnoozeCommand,
		delete: taskDeleteCommand,
	},
	run: async () => {
		console.log("Task management subcommands:");
		console.log("  edit   - Show editable fields for a task");
		console.log("  move   - Move task to a project");
		console.log("  plan   - Schedule task for a specific date");
		console.log("  snooze - Push task back by a duration");
		console.log("  delete - Soft delete a task");
	},
});

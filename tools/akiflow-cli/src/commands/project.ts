import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import type { Label, Task, UpdateTaskPayload } from "../lib/api/types";
import { syncTasksCache } from "../lib/tasks-local-cache";

const COLOR_PALETTE: Record<string, string> = {
	red: "#FF6B6B",
	orange: "#FFA500",
	yellow: "#FFD93D",
	green: "#6BCB77",
	blue: "#4D96FF",
	purple: "#9D84B7",
	pink: "#FF69B4",
};

function isValidColor(color: string): boolean {
	if (color.startsWith("#")) {
		return /^#[0-9A-Fa-f]{6}$/.test(color);
	}
	return color in COLOR_PALETTE;
}

function normalizeColor(color: string): string {
	if (color.startsWith("#")) {
		return color.toUpperCase();
	}
	return COLOR_PALETTE[color] || color;
}

function colorizeProjectColor(hexColor: string | null): string {
	if (!hexColor) {
		return "⚪";
	}
	return "●";
}

function countTasksForProjectId(tasks: Task[], projectId: string): number {
	return tasks.filter((task) => task.listId === projectId && !task.deleted_at)
		.length;
}

const projectLsCommand = defineCommand({
	meta: {
		name: "ls",
		description: "List all projects with task counts",
	},
	run: async () => {
		const client = createClient();

		try {
			const response = await client.getLabels();
			const projects = response.data.filter((label) => !label.deleted_at);

			if (projects.length === 0) {
				console.log("No projects found.");
				return;
			}

			console.log("\nProjects:");
			console.log("─".repeat(50));

			const { tasks } = await syncTasksCache(client, { quiet: true });

			for (const project of projects) {
				const taskCount = countTasksForProjectId(tasks, project.id);
				const colorIndicator = colorizeProjectColor(project.color);
				const taskText = taskCount === 1 ? "task" : "tasks";
				console.log(
					`${colorIndicator} ${project.title.padEnd(30)} ${taskCount} ${taskText}`,
				);
			}

			console.log("─".repeat(50));
		} catch (error) {
			console.error(
				"Error:",
				error instanceof Error ? error.message : "Failed to list projects",
			);
			process.exit(1);
		}
	},
});

const projectCreateCommand = defineCommand({
	meta: {
		name: "create",
		description: "Create a new project",
	},
	args: {
		name: {
			type: "string",
			description: "Project name",
			required: true,
		},
		color: {
			type: "string",
			description:
				"Project color (red, orange, yellow, green, blue, purple, pink, or #RRGGBB)",
			default: "blue",
		},
	},
	run: async (context) => {
		const name = context.args.name as string;
		const colorFlag = context.args.color as string;

		if (!isValidColor(colorFlag)) {
			console.error(
				`Error: Invalid color "${colorFlag}". Valid colors: ${Object.keys(COLOR_PALETTE).join(", ")}, or #RRGGBB format`,
			);
			process.exit(1);
		}

		const normalizedColor = normalizeColor(colorFlag);
		const client = createClient();

		try {
			const now = new Date().toISOString();
			const newLabel: Label = {
				id: crypto.randomUUID(),
				user_id: 0,
				parent_id: null,
				title: name,
				icon: null,
				color: normalizedColor,
				sorting: 0,
				type: null,
				global_created_at: now,
				global_updated_at: now,
				data: {},
				deleted_at: null,
			};

			const response = await client.upsertTasks([
				newLabel as unknown as UpdateTaskPayload,
			]);

			if (response.success) {
				console.log(`✓ Project "${name}" created successfully`);
			} else {
				console.error("Error: Failed to create project");
				process.exit(1);
			}
		} catch (error) {
			console.error(
				"Error:",
				error instanceof Error ? error.message : "Failed to create project",
			);
			process.exit(1);
		}
	},
});

const projectDeleteCommand = defineCommand({
	meta: {
		name: "delete",
		description: "Delete a project (soft delete)",
	},
	args: {
		name: {
			type: "string",
			description: "Project name",
			required: true,
		},
	},
	run: async (context) => {
		const projectName = context.args.name as string;
		const client = createClient();

		try {
			const response = await client.getLabels();
			const project = response.data.find(
				(label) =>
					label.title.toLowerCase() === projectName.toLowerCase() &&
					!label.deleted_at,
			);

			if (!project) {
				console.error(`Error: Project "${projectName}" not found`);
				process.exit(1);
			}

			const now = new Date().toISOString();
			const updatePayload: UpdateTaskPayload = {
				id: project.id,
				global_updated_at: now,
				deleted_at: now,
			};

			const updateResponse = await client.upsertTasks([updatePayload]);

			if (updateResponse.success) {
				console.log(`✓ Project "${projectName}" deleted successfully`);
			} else {
				console.error("Error: Failed to delete project");
				process.exit(1);
			}
		} catch (error) {
			console.error(
				"Error:",
				error instanceof Error ? error.message : "Failed to delete project",
			);
			process.exit(1);
		}
	},
});

export const projectCommand = defineCommand({
	meta: {
		name: "project",
		description: "Manage projects",
	},
	subCommands: {
		ls: projectLsCommand,
		create: projectCreateCommand,
		delete: projectDeleteCommand,
	},
	run: async () => {
		console.log(
			"Use 'af project ls', 'af project create', or 'af project delete'",
		);
	},
});

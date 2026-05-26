import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import type { CreateTaskPayload } from "../lib/api/types";
import { getDefaultCalendarId } from "../lib/calendar";
import {
	createDateTimeUTC,
	getLocalTimezone,
	getTodayDate,
	getTomorrowDate,
	parseDate,
	parseTime,
} from "../lib/date-parser";
import { parseDurationToSeconds } from "../lib/duration-parser";
import { addPendingTask } from "../lib/task-cache";

export const add = defineCommand({
	meta: {
		name: "add",
		description: "Create a new task",
	},
	args: {
		title: {
			type: "positional",
			description: "Task title",
			required: true,
		},
		today: {
			type: "boolean",
			description: "Schedule task for today",
			alias: "t",
		},
		tomorrow: {
			type: "boolean",
			description: "Schedule task for tomorrow",
		},
		date: {
			type: "string",
			description: "Natural language date (e.g., 'next friday', 'in 3 days')",
			alias: "d",
		},
		project: {
			type: "string",
			description: "Assign to project/label by name",
			alias: "p",
		},
		at: {
			type: "string",
			description: "Time for time block (e.g., '21:00', '14:30')",
		},
		duration: {
			type: "string",
			description: "Duration for time block (e.g., '30m', '1h', '2h')",
		},
	},
	run: async (context) => {
		const client = createClient();

		const title = context.args.title as string;
		const today = context.args.today as boolean;
		const tomorrow = context.args.tomorrow as boolean;
		const dateInput = context.args.date as string | undefined;
		const projectName = context.args.project as string | undefined;
		const timeInput = context.args.at as string | undefined;
		const durationInput = context.args.duration as string | undefined;

		let taskDate: string | undefined;
		let taskDateTime: string | undefined;
		let taskDateTimeTz: string | undefined;
		let taskDuration: number | undefined;
		let calendarId: string | null = null;

		if (today) {
			taskDate = getTodayDate();
		} else if (tomorrow) {
			taskDate = getTomorrowDate();
		} else if (dateInput) {
			const parsedDate = parseDate(dateInput);
			if (!parsedDate) {
				console.error(`Error: Could not parse date "${dateInput}"`);
				process.exit(1);
			}
			taskDate = parsedDate;
		}

		if (timeInput) {
			const parsedTime = parseTime(timeInput);
			if (!parsedTime) {
				console.error(
					`Error: Invalid time format "${timeInput}". Expected format: HH:MM (e.g., 21:00, 14:30)`,
				);
				process.exit(1);
			}

			if (!taskDate) {
				taskDate = getTodayDate();
			}

			taskDateTime = createDateTimeUTC(
				taskDate,
				parsedTime.hours,
				parsedTime.minutes,
			);
			taskDateTimeTz = getLocalTimezone();
			calendarId = await getDefaultCalendarId(client);
		}

		if (durationInput) {
			try {
				taskDuration = parseDurationToSeconds(durationInput);
			} catch (error) {
				console.error(
					`Error: ${error instanceof Error ? error.message : "Invalid duration format"}`,
				);
				process.exit(1);
			}
		}

		let listId: string | undefined;
		if (projectName) {
			try {
				const labelsResponse = await client.getLabels();
				const label = labelsResponse.data.find(
					(l) => l.title.toLowerCase() === projectName.toLowerCase(),
				);

				if (label) {
					listId = label.id;
				} else {
					console.error(`Error: Project "${projectName}" not found`);
					process.exit(1);
				}
			} catch (error) {
				console.error(`Error: Failed to fetch projects`);
				console.error(error instanceof Error ? error.message : "Unknown error");
				process.exit(1);
			}
		}

		const now = new Date().toISOString();
		const taskId = crypto.randomUUID();

		const task: CreateTaskPayload = {
			id: taskId,
			title,
			global_created_at: now,
			global_updated_at: now,
		};

		if (taskDate) {
			task.date = taskDate;
		}

		if (taskDateTime) {
			task.datetime = taskDateTime;
		}

		if (taskDateTimeTz) {
			task.datetime_tz = taskDateTimeTz;
		}

		if (taskDuration !== undefined) {
			task.duration = taskDuration;
		}

		if (listId) {
			task.listId = listId;
		}

		if (calendarId) {
			task.calendar_id = calendarId;
			task.status = 2; // Time-blocked status
		}

		try {
			const response = await client.upsertTasks([task]);
			const createdTask = response.data[0];

			if (!createdTask) {
				console.error("Error: Failed to create task - no data returned");
				process.exit(1);
			}

			// Save to pending cache for immediate visibility in ls
			await addPendingTask(createdTask);

			console.log("✓ Task created successfully");
			console.log(`  ID: ${createdTask.id}`);
			console.log(`  Title: ${createdTask.title}`);

			if (createdTask.date) {
				console.log(`  Date: ${createdTask.date}`);
			}

			if (createdTask.datetime) {
				const localTime = new Date(createdTask.datetime).toLocaleTimeString(
					[],
					{ hour: "2-digit", minute: "2-digit" },
				);
				console.log(`  Time: ${localTime}`);
			}

			if (createdTask.duration) {
				const minutes = Math.floor(createdTask.duration / 60);
				const durationStr =
					minutes >= 60
						? `${Math.floor(minutes / 60)}h${minutes % 60 > 0 ? ` ${minutes % 60}m` : ""}`
						: `${minutes}m`;
				console.log(`  Duration: ${durationStr}`);
			}

			if (createdTask.listId && listId) {
				console.log(`  Project: ${projectName}`);
			}
		} catch (error) {
			if (error instanceof Error && error.name === "AuthError") {
				console.error(
					"Error: Authentication failed. Please run 'af auth' to login.",
				);
			} else {
				console.error(
					"Error: Failed to create task",
					error instanceof Error ? error.message : "Unknown error",
				);
			}
			process.exit(1);
		}
	},
});

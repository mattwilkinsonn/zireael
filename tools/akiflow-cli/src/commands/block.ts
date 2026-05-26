import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import type { CreateTaskPayload, TimeSlot } from "../lib/api/types";
import { parseDuration } from "../lib/duration-parser";

interface TimeRange {
	start: Date;
	end: Date;
}

function getTodayStart(): Date {
	const now = new Date();
	const start = new Date(
		now.getFullYear(),
		now.getMonth(),
		now.getDate(),
		0,
		0,
		0,
	);
	return start;
}

function getTodayEnd(): Date {
	const now = new Date();
	const end = new Date(
		now.getFullYear(),
		now.getMonth(),
		now.getDate(),
		23,
		59,
		59,
	);
	return end;
}

function isToday(timeSlot: TimeSlot): boolean {
	const startTime = new Date(timeSlot.start_time);
	const todayStart = getTodayStart();
	const todayEnd = getTodayEnd();
	return startTime >= todayStart && startTime <= todayEnd;
}

function findFreeSlots(slots: TimeSlot[]): TimeRange[] {
	const todayStart = getTodayStart();
	const todayEnd = getTodayEnd();

	const sortedSlots = [...slots]
		.filter(isToday)
		.sort(
			(a, b) =>
				new Date(a.start_time).getTime() - new Date(b.start_time).getTime(),
		);

	const freeSlots: TimeRange[] = [];
	let currentTime = todayStart;

	for (const slot of sortedSlots) {
		const slotStart = new Date(slot.start_time);

		if (currentTime < slotStart) {
			freeSlots.push({ start: currentTime, end: slotStart });
		}

		const slotEnd = new Date(slot.end_time);
		if (slotEnd > currentTime) {
			currentTime = slotEnd;
		}
	}

	if (currentTime < todayEnd) {
		freeSlots.push({ start: currentTime, end: todayEnd });
	}

	return freeSlots;
}

function formatTime(date: Date): string {
	const hours = String(date.getHours()).padStart(2, "0");
	const minutes = String(date.getMinutes()).padStart(2, "0");
	return `${hours}:${minutes}`;
}

function findSlotForDuration(
	freeSlots: TimeRange[],
	durationMs: number,
): TimeRange | null {
	for (const slot of freeSlots) {
		const slotDuration = slot.end.getTime() - slot.start.getTime();
		if (slotDuration >= durationMs) {
			return slot;
		}
	}
	return null;
}

export const block = defineCommand({
	meta: {
		name: "block",
		description: "Create time block",
	},
	args: {
		duration: {
			type: "positional",
			description: "Duration (e.g., 1h, 30m, 2h)",
			required: true,
		},
		title: {
			type: "positional",
			description: "Time block title",
			required: true,
		},
	},
	run: async (context) => {
		const client = createClient();

		const durationInput = context.args.duration as string;
		const title = context.args.title as string;

		let durationMs: number;
		try {
			durationMs = parseDuration(durationInput);
		} catch (error) {
			console.error(
				`Error: ${error instanceof Error ? error.message : "Invalid duration"}`,
			);
			process.exit(1);
		}

		try {
			const timeSlotsResponse = await client.getTimeSlots();
			const allTimeSlots = timeSlotsResponse.data;
			const freeSlots = findFreeSlots(allTimeSlots);

			const calendarId =
				allTimeSlots.length > 0 ? allTimeSlots[0]?.calendar_id : null;

			const selectedSlot = findSlotForDuration(freeSlots, durationMs);

			if (!selectedSlot) {
				console.error(
					`Error: No available time slot for ${durationInput} today`,
				);
				console.log("\nAvailable free time slots:");

				if (freeSlots.length === 0) {
					console.log("  None");
				} else {
					freeSlots.forEach((slot) => {
						const slotDurationMs = slot.end.getTime() - slot.start.getTime();
						const slotDurationMins = Math.round(slotDurationMs / (60 * 1000));
						const hours = Math.floor(slotDurationMins / 60);
						const mins = slotDurationMins % 60;

						let durationStr = "";
						if (hours > 0 && mins > 0) {
							durationStr = `${hours}h ${mins}m`;
						} else if (hours > 0) {
							durationStr = `${hours}h`;
						} else {
							durationStr = `${mins}m`;
						}

						console.log(
							`  ${formatTime(slot.start)} - ${formatTime(slot.end)}  (${durationStr})`,
						);
					});
				}

				process.exit(1);
			}

			const startTime = selectedSlot.start.toISOString();
			const now = new Date().toISOString();
			const taskId = crypto.randomUUID();

			const task: CreateTaskPayload = {
				id: taskId,
				title,
				datetime: startTime,
				datetime_tz: new Date().toISOString(),
				duration: durationMs,
				global_created_at: now,
				global_updated_at: now,
				...(calendarId && { calendar_id: calendarId }),
			};

			const response = await client.upsertTasks([task]);
			const createdTask = response.data[0];

			if (!createdTask) {
				console.error("Error: Failed to create time block - no data returned");
				process.exit(1);
			}

			console.log("✓ Time block created successfully");
			console.log(`  ID: ${createdTask.id}`);
			console.log(`  Title: ${createdTask.title}`);

			if (createdTask.datetime) {
				const taskTime = new Date(createdTask.datetime);
				const endTime = new Date(
					taskTime.getTime() + (createdTask.duration ?? 0),
				);
				const durationMins = Math.round(
					(createdTask.duration ?? 0) / (60 * 1000),
				);
				const hours = Math.floor(durationMins / 60);
				const mins = durationMins % 60;

				let durationStr = "";
				if (hours > 0 && mins > 0) {
					durationStr = `${hours}h ${mins}m`;
				} else if (hours > 0) {
					durationStr = `${hours}h`;
				} else {
					durationStr = `${mins}m`;
				}

				console.log(
					`  Time: ${formatTime(taskTime)} - ${formatTime(endTime)} (${durationStr})`,
				);
			}
		} catch (error) {
			if (error instanceof Error && error.name === "AuthError") {
				console.error(
					"Error: Authentication failed. Please run 'af auth' to login.",
				);
			} else {
				console.error(
					"Error: Failed to create time block",
					error instanceof Error ? error.message : "Unknown error",
				);
			}
			process.exit(1);
		}
	},
});

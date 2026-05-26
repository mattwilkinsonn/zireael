import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import type {
	Account,
	Calendar,
	Event,
	Task,
	TimeSlot,
} from "../lib/api/types";
import { type NamedRange, resolveRange } from "../lib/date-parser";
import {
	type EventFilter,
	filterEvents,
	mergeTimeline,
	type TimelineEntry,
} from "../lib/filters/event";
import { emptyContext, toCleanedCalView } from "../lib/format/cleaned-types";

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

function formatTime(date: Date): string {
	const hours = String(date.getHours()).padStart(2, "0");
	const minutes = String(date.getMinutes()).padStart(2, "0");
	return `${hours}:${minutes}`;
}

function getDurationMinutes(start: Date, end: Date): number {
	return Math.round((end.getTime() - start.getTime()) / (60 * 1000));
}

function formatDuration(minutes: number): string {
	const hours = Math.floor(minutes / 60);
	const mins = minutes % 60;

	if (hours > 0 && mins > 0) {
		return `${hours}h ${mins}m`;
	} else if (hours > 0) {
		return `${hours}h`;
	} else {
		return `${mins}m`;
	}
}

function formatTimeline(slots: TimeSlot[]): string {
	if (slots.length === 0) {
		return "No events or time blocks scheduled for today.";
	}

	const lines: string[] = [];
	lines.push("📅 Today's Schedule");
	lines.push("");

	const sortedSlots = [...slots].sort((a, b) => {
		const aTime = new Date(a.start_time).getTime();
		const bTime = new Date(b.start_time).getTime();
		return aTime - bTime;
	});

	sortedSlots.forEach((slot, index) => {
		const start = new Date(slot.start_time);
		const end = new Date(slot.end_time);
		const duration = getDurationMinutes(start, end);
		const prefix = index === 0 ? "┌" : "├";

		lines.push(
			`${prefix} ${formatTime(start)} - ${formatTime(end)}  ${slot.title} (${formatDuration(duration)})`,
		);
	});

	return lines.join("\n");
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

function formatFreeSlots(slots: TimeRange[]): string {
	if (slots.length === 0) {
		return "No free time slots available today.";
	}

	const lines: string[] = [];
	lines.push("🕐 Free Time Slots Today");
	lines.push("");

	slots.forEach((slot, index) => {
		const duration = getDurationMinutes(slot.start, slot.end);
		const prefix = index === 0 ? "┌" : "├";

		lines.push(
			`${prefix} ${formatTime(slot.start)} - ${formatTime(slot.end)}  ${formatDuration(duration)} available`,
		);
	});

	return lines.join("\n");
}

const NAMED_RANGE_FLAGS: ReadonlyArray<NamedRange> = [
	"today",
	"tomorrow",
	"yesterday",
	"this-week",
	"next-week",
	"this-month",
	"next-month",
];

function hasExtendedCalFlags(args: Record<string, unknown>): boolean {
	if (NAMED_RANGE_FLAGS.some((n) => args[n])) return true;
	if (args.date || args.from || args.to) return true;
	if (args.calendar || args.account || args.connector) return true;
	// citty rewrites --no-X as args.X = false (negation flag)
	if (args.events === false || args.tasks === false || args.slots === false)
		return true;
	if (args.declined || args["all-day-only"] || args["all-day"] === false)
		return true;
	if (args.json || args.raw) return true;
	return false;
}

function buildEventFilter(
	args: Record<string, unknown>,
): EventFilter & { range: { from: Date; to: Date } } {
	const f: EventFilter = {};
	const named = NAMED_RANGE_FLAGS.find((n) => args[n]);
	const range = named
		? resolveRange(named)
		: args.date
			? (() => {
					const d = new Date(args.date as string);
					return { from: d, to: d };
				})()
			: args.from || args.to
				? {
						from: args.from ? new Date(args.from as string) : new Date(0),
						to: args.to
							? new Date(args.to as string)
							: new Date(8640000000000000),
					}
				: resolveRange("today");

	f.from = range.from;
	f.to = range.to;
	if (args.calendar) f.calendar = args.calendar as string;
	if (args.account) f.account = args.account as string;
	if (args.connector) f.connector = args.connector as string;
	if (args.declined) f.includeDeclined = true;
	if (args["all-day-only"]) f.allDayOnly = true;
	if (args["all-day"] === false) f.noAllDay = true;
	return { ...f, range };
}

async function runMergedCalendar(args: Record<string, unknown>): Promise<void> {
	const client = createClient();
	const ef = buildEventFilter(args);

	// citty's negation: --no-events → args.events = false
	const eventsPromise =
		args.events === false
			? Promise.resolve({ data: [] as Event[] })
			: client.get<Event[]>("/v5/events", { limit: 2500 });
	const slotsPromise =
		args.slots === false
			? Promise.resolve({ data: [] as TimeSlot[] })
			: client.getTimeSlots();
	const tasksPromise =
		args.tasks === false
			? Promise.resolve({ data: [] as Task[] })
			: client.getTasks({ limit: 2500 });

	const [eventsResp, slotsResp, tasksResp] = (await Promise.all([
		eventsPromise,
		slotsPromise,
		tasksPromise,
	])) as [{ data: Event[] }, { data: TimeSlot[] }, { data: Task[] }];

	// Filter events
	const eventsFiltered = filterEvents(eventsResp.data, {
		from: ef.from,
		to: ef.to,
		calendar: ef.calendar,
		account: ef.account,
		connector: ef.connector,
		includeDeclined: ef.includeDeclined,
		allDayOnly: ef.allDayOnly,
		noAllDay: ef.noAllDay,
	});

	// Filter slots by date range
	const fromMs = ef.range.from.getTime();
	const toMs = ef.range.to.getTime();
	const slotsFiltered = slotsResp.data.filter((s) => {
		const t = new Date(s.start_time).getTime();
		return t >= fromMs && t <= toMs;
	});

	// Filter tasks: only those with a datetime in the range
	const tasksFiltered = tasksResp.data.filter((t) => {
		if (!t.datetime) return false;
		const ts = new Date(t.datetime).getTime();
		return ts >= fromMs && ts <= toMs;
	});

	const merged = mergeTimeline(eventsFiltered, slotsFiltered, tasksFiltered);

	if (args.raw) {
		console.log(
			JSON.stringify(
				{ result: merged, next_cursor: null, errors: [] },
				null,
				2,
			),
		);
		return;
	}

	if (args.json) {
		// Cleaned shape: resolve calendar + account names
		const ctx = emptyContext();
		try {
			const calsResp = await client.get<Calendar[]>("/v5/calendars", {
				limit: 2500,
			});
			for (const c of calsResp.data) ctx.calendarsById.set(c.id, c);
		} catch {
			/* empty context still works */
		}
		try {
			const accsResp = await client.get<Account[]>("/v5/accounts", {
				limit: 2500,
			});
			for (const a of accsResp.data) ctx.accountsById.set(a.id, a);
		} catch {
			/* same */
		}
		const cleaned = merged.map((m) => toCleanedCalView(m, ctx));
		console.log(
			JSON.stringify(
				{ result: cleaned, next_cursor: null, errors: [] },
				null,
				2,
			),
		);
		return;
	}

	console.log(formatMergedTimeline(merged));
}

function formatMergedTimeline(entries: TimelineEntry[]): string {
	if (entries.length === 0)
		return "(no events, slots, or scheduled tasks in range)";
	const byDay = new Map<string, TimelineEntry[]>();
	for (const e of entries) {
		const day = e.start.toISOString().slice(0, 10);
		const arr = byDay.get(day) ?? [];
		arr.push(e);
		byDay.set(day, arr);
	}
	const lines: string[] = [];
	for (const [day, dayEntries] of byDay) {
		lines.push(new Date(day).toDateString());
		lines.push("");
		for (const e of dayEntries) {
			const start = `${pad2(e.start.getHours())}:${pad2(e.start.getMinutes())}`;
			const end = e.end
				? `${pad2(e.end.getHours())}:${pad2(e.end.getMinutes())}`
				: "    ";
			const glyph =
				e.type === "event" ? "📅" : e.type === "time_slot" ? "⏰" : "📌";
			const title =
				e.type === "event"
					? ((e.record as Event).title ?? "(untitled event)")
					: e.type === "time_slot"
						? (e.record as TimeSlot).title
						: ((e.record as Task).title ?? "(untitled task)");
			lines.push(`  ${start} — ${end}   ${glyph}  ${title}`);
		}
		lines.push("");
	}
	return lines.join("\n");
}

function pad2(n: number): string {
	return n.toString().padStart(2, "0");
}

export const cal = defineCommand({
	meta: {
		name: "cal",
		description: "View calendar — events + time slots + scheduled tasks",
	},
	args: {
		free: {
			type: "boolean",
			description: "Find free time slots today (legacy mode)",
		},
		// Date range
		today: { type: "boolean", description: "Today only (default)" },
		tomorrow: { type: "boolean" },
		yesterday: { type: "boolean" },
		"this-week": { type: "boolean" },
		"next-week": { type: "boolean" },
		"this-month": { type: "boolean" },
		"next-month": { type: "boolean" },
		date: { type: "string", description: "Single day" },
		from: { type: "string", description: "Start date" },
		to: { type: "string", description: "End date" },
		// Resource filters
		calendar: { type: "string", description: "Filter by calendar id" },
		account: { type: "string", description: "Filter by akiflow_account_id" },
		connector: { type: "string", description: "google | microsoft | icloud" },
		// citty rewrites --no-events as args.events = false (negation)
		events: {
			type: "boolean",
			description: "Include events (use --no-events to exclude)",
		},
		tasks: {
			type: "boolean",
			description: "Include scheduled tasks (use --no-tasks to exclude)",
		},
		slots: {
			type: "boolean",
			description: "Include time slots (use --no-slots to exclude)",
		},
		declined: { type: "boolean", description: "Include declined events" },
		"all-day-only": { type: "boolean" },
		"all-day": {
			type: "boolean",
			description: "Include all-day events (use --no-all-day to exclude)",
		},
		// Output
		json: { type: "boolean", description: "Cleaned JSON" },
		raw: { type: "boolean", description: "Raw API records JSON" },
	},
	run: async (context) => {
		const args = context.args;
		const showFree = args.free as boolean;

		try {
			if (showFree) {
				// Legacy --free path: keep existing behavior (free-slot finder)
				const client = createClient();
				const response = await client.getTimeSlots();
				const allSlots = response.data;
				const freeSlots = findFreeSlots(allSlots);
				console.log(formatFreeSlots(freeSlots));
				return;
			}

			if (hasExtendedCalFlags(args)) {
				await runMergedCalendar(args);
				return;
			}

			// Default (no flags): preserve the upstream slot-only behavior so
			// existing tests + scripts that pipe `af cal` keep working. Use any
			// new flag (e.g. --today, --this-week, --json) to opt into the
			// merged events+slots+tasks timeline.
			const client = createClient();
			const response = await client.getTimeSlots();
			const todaySlots = response.data.filter(isToday);
			console.log(formatTimeline(todaySlots));
		} catch (error) {
			if (error instanceof Error && error.name === "AuthError") {
				console.error(
					"Error: Authentication failed. Please run 'af auth' to login.",
				);
			} else {
				console.error(
					"Error: Failed to fetch calendar",
					error instanceof Error ? error.message : "Unknown error",
				);
			}
			process.exit(1);
		}
	},
});

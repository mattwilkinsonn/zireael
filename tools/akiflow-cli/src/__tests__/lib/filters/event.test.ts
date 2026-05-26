import { describe, expect, test } from "bun:test";
import type { Event, Task, TimeSlot } from "../../../lib/api/types";
import { filterEvents, mergeTimeline } from "../../../lib/filters/event";

function ev(o: Partial<Event>): Event {
	return {
		id: "e1",
		user_id: 1,
		calendar_id: "cal1",
		declined: false,
		deleted_at: null,
		start_date: null,
		end_date: null,
		start_time: "2026-05-21T10:00:00Z",
		end_time: "2026-05-21T11:00:00Z",
		title: null,
		task_id: null,
		time_slot_id: null,
		...o,
	} as Event;
}

describe("filterEvents", () => {
	test("excludes declined by default", () => {
		const events = [
			ev({ id: "a", declined: false }),
			ev({ id: "b", declined: true }),
		];
		expect(filterEvents(events, {}).map((e) => e.id)).toEqual(["a"]);
	});

	test("--declined includes them", () => {
		const events = [
			ev({ id: "a", declined: false }),
			ev({ id: "b", declined: true }),
		];
		expect(
			filterEvents(events, { includeDeclined: true })
				.map((e) => e.id)
				.sort(),
		).toEqual(["a", "b"]);
	});

	test("date range filters by start_time", () => {
		const events = [
			ev({ id: "before", start_time: "2026-05-20T10:00:00Z" }),
			ev({ id: "in", start_time: "2026-05-21T10:00:00Z" }),
			ev({ id: "after", start_time: "2026-05-22T10:00:00Z" }),
		];
		expect(
			filterEvents(events, {
				from: new Date("2026-05-21T00:00:00Z"),
				to: new Date("2026-05-21T23:59:59Z"),
			}).map((e) => e.id),
		).toEqual(["in"]);
	});

	test("--all-day-only filters to date-only events", () => {
		const events = [
			ev({ id: "timed", start_time: "2026-05-21T10:00:00Z", start_date: null }),
			ev({ id: "allday", start_time: null, start_date: "2026-05-21" }),
		];
		expect(filterEvents(events, { allDayOnly: true }).map((e) => e.id)).toEqual(
			["allday"],
		);
	});
});

describe("mergeTimeline", () => {
	test("dedup: event with task_id skips matching task", () => {
		const events = [ev({ id: "evt1", task_id: "task1" })];
		const slots: TimeSlot[] = [];
		const tasks = [
			{
				id: "task1",
				datetime: "2026-05-21T10:00:00Z",
				duration: null,
			} as unknown as Task,
		];
		const merged = mergeTimeline(events, slots, tasks);
		expect(merged.length).toBe(1);
		expect(merged[0]?.type).toBe("event");
	});

	test("sorted by start time", () => {
		const events: Event[] = [];
		const slots: TimeSlot[] = [];
		const tasks = [
			{ id: "t-late", datetime: "2026-05-21T14:00:00Z", duration: null },
			{ id: "t-early", datetime: "2026-05-21T09:00:00Z", duration: 30 },
		] as unknown as Task[];
		const merged = mergeTimeline(events, slots, tasks);
		expect(merged.map((m) => (m.record as { id: string }).id)).toEqual([
			"t-early",
			"t-late",
		]);
	});
});
